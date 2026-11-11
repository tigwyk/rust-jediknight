[CmdletBinding()]
param(
  [string]$Shots = 'tools/parity/shots.tsv',
  [string]$Out = 'target/parity/retail',
  [string]$GamePath,
  [string]$WorkCopy = 'target/parity/retailcopy',
  [int]$SeriesFrames = 8,
  [int]$LevelTimeoutSec = 45,
  [switch]$Software,
  # Request dgVoodoo's windowed config (`FullScreenMode=false`, `WindowedAttributes=borderless`,
  # `CaptureMouse=true`). Measured 2026-11-09: the game window became a 640x480 borderless window, but
  # the user still saw the game take over the whole display on every run, so this does **not** make a
  # retail capture non-invasive for this DirectDraw title. Opt-in; do not call a capture windowed on
  # the strength of the config alone.
  [switch]$Windowed,
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
  # Issue #13's retail audio probe: after the level loads and the row's warp is applied, read the
  # engine's own music and mixer state for this many game milliseconds and log it. Capture
  # instrumentation only (see `Invoke-JkAudioProbe`); 0 disables it.
  [int]$AudioProbeMs = 0,
  [int]$AudioProbeCadenceMs = 100,
  # Issue #13's retail audio route: walk this route file in retail and write its audio trace to
  # `-AudioRouteOut` (port audio-trace schema). Capture instrumentation only.
  [string]$AudioRoute = '',
  [string]$AudioRouteOut = 'tools/parity/audiotrace/level1-audio-retail.tsv',
  [int]$EncounterCadenceMs = 100,
  [string]$EncounterOut = 'target/parity/encounter',
  # File name (without `.tsv`) for the encounter trace. Empty uses the shots row's name; the gate's
  # fixture is `door_encounter`, while the pose row is `route_door`.
  [string]$EncounterName = '',
  # Issue #20's engine-clock route/encounter capture: after the level loads and the row's warp is
  # applied, sample the player body/clock/inventory/AI random state from process memory for this many
  # engine milliseconds and write `<name>.tsv` into -EngineCaptureOut. There is NO dev-console read
  # inside the window (its dialog pauses the engine, so a poll would sample a frozen clock and
  # perturb the fight); a row is written whenever `DAT_00836c14` changes. 0 disables the mode.
  [int]$EngineCaptureMs = 0,
  # Hold this key down for the whole window, e.g. `UP` (the engine's forward binding) for a route
  # leg. Empty leaves the body still, which is the door encounter.
  [string]$EngineCaptureKey = '',
  [string]$EngineCaptureOut = 'target/parity/engine',
  # File name (without `.tsv`); empty uses the shots row's name.
  [string]$EngineCaptureName = '',
  # Validate the engine-capture row schema on synthetic data and exit. Never resolves or launches
  # retail; this is the mode's self-check (run it before the user-run capture).
  [switch]$EngineCaptureSelfCheck,
  # Issue #8's continuous retail route: run every row of `-RoutePlan` in ONE game session and sample
  # the bars on the 600 ms grid the port replays. The selected shots row must be the level's own
  # start (no pose), i.e. `-Only spawn` with `shots.tsv`.
  [switch]$RouteWalk,
  [string]$RoutePlan = 'tools/parity/routes/level1-route.tsv',
  [string]$RouteOut = 'target/parity/routetrace',
  [string]$RouteName = 'level1-route-retail-continuous',
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
  # Hold this key after a `-WalkTraceMs` leg and before the post-walk console read (e.g. 'SPACE' = the
  # player's ACTIVATE binding, or 'MOUSEMIDDLE'). The walk's `coords` then show the stop pose and
  # `cogstatus` shows the door the level's own script opened.
  [string]$UseAfterWalk = '',
  # The ACTIVATE binding for `-UseAfterWalk` and for the route plan's `use` rows. The profile's
  # `bind 12 57 0x2` stores scancode 57 = 0x39 = SPACE (the PLR codes are scancodes, not VK codes).
  [string]$UseKey = 'SPACE',
  # How long the game runs after a use before the next console read freezes it.
  [int]$UseSettleMs = 900,
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

function Invoke-JkStaging([string]$Source, [string]$Destination, [switch]$Windowed) {
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
    Install-Jk3DWrapper -Destination $Destination -LogPath $script:LogPath -Windowed:$Windowed
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
  # `SITHBIN_GOAL00` (100): `01_endlevel.cog`'s `user0:` ORs bit 2 over `startup`'s bit 1, so the
  # goal counter reads 1 at the level start and 3 once 8t88 is sighted. Same inventory stride.
  $goal = [JkRetail]::ReadFloat($ProcessId, $inv + 0x88 + 100 * 0x18)
  if ([double]::IsNaN($health) -or [double]::IsNaN($shields) -or [double]::IsNaN($goal)) {
    throw "player bars: the player bars could not be read (health $health, shields $shields, goal $goal)"
  }
  if ($health -lt 0.0 -or $health -gt 1000.0) { throw "player bars: implausible health $health at thing+0x134" }
  if ($shields -lt -0.01 -or $shields -gt 200.01) { throw "player bars: implausible shields $shields at inventory bin 60" }
  if ($goal -lt -0.01 -or $goal -gt 65535.0) { throw "player bars: implausible goal counter $goal at inventory bin 100" }
  [pscustomobject]@{ Health = $health; Shields = $shields; Goal = $goal }
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
# Press the player's ACTIVATE binding. The retail renderer runs at a few frames a second, so a tap
# can fall between frames; a use is a hold. Mouse bindings need the cursor inside the window first.
function Invoke-JkUseKey {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [string]$Key = 'SPACE',
    [int]$HoldMs = 600,
    [int]$SettleMs = 900
  )
  $keys = @($Key -split ',' | Where-Object { $_ -ne '' })
  $keyboard = @($keys | Where-Object { $_ -notmatch '^MOUSE' })
  # The dev-console dialog usually owns the focus after a `coords` read. The key state must go down
  # **after** the game window is focused: a pre-press while the console is foreground never reaches
  # the engine's DirectInput queue, so the use was lost even though the key was held.
  try {
    $gameWindow = Get-JkWindow -ProcessId $ProcessId
    if ($gameWindow -ne [IntPtr]::Zero) {
      Log ("use: focus before '{0}'" -f [JkRetail]::ForegroundTitle())
      $activated = [JkRetail]::Activate($gameWindow)
      for ($attempt = 0; $attempt -lt 8; $attempt++) {
        [void][JkRetail]::ShowWindow($gameWindow, 9)
        [void][JkRetail]::SetForegroundWindow($gameWindow)
        Start-Sleep -Milliseconds 120
        if ([JkRetail]::ForegroundTitle() -eq 'Jedi Knight') { break }
      }
      # The dev-console dialog is a separate top-level window and often still foreground after a
      # `coords` read; a click inside the game window is what actually hands it focus (the F12 helper
      # does the same). It may fire the weapon once, which is harmless with the AI off.
      $rect = [JkRetail]::WindowRectText($gameWindow)
      if ($rect -match '\((-?\d+),(-?\d+)\)-\((-?\d+),(-?\d+)\)') {
        Send-JkClick -X ([int](([int]$Matches[1] + [int]$Matches[3]) / 2)) -Y ([int](([int]$Matches[2] + [int]$Matches[4]) / 2))
      }
      Start-Sleep -Milliseconds 400
      Log ("use: activated={0} focus now '{1}'" -f $activated, [JkRetail]::ForegroundTitle())
    }
    foreach ($name in $keys) {
      if ($name -match '^MOUSE') {
        $rect = [JkRetail]::WindowRectText((Get-JkWindow -ProcessId $ProcessId))
        if ($rect -match '\((-?\d+),(-?\d+)\)-\((-?\d+),(-?\d+)\)') {
          $cx = ([int]$Matches[1] + [int]$Matches[3]) / 2
          $cy = ([int]$Matches[2] + [int]$Matches[4]) / 2
          [JkRetail]::MoveTo([int]$cx, [int]$cy)
          Start-Sleep -Milliseconds 120
        }
        Log ("use: pressing {0} for {1} ms" -f $name, $HoldMs)
        Send-JkMouseButton -Button ($name -replace '^MOUSE', '') -HoldMs $HoldMs
      } else {
        Log ("use: holding {0} for {1} ms" -f $name, $HoldMs)
        Send-JkKeyDown -Name $name
        Start-Sleep -Milliseconds $HoldMs
        Send-JkKeyUp -Name $name
      }
      # The engine advances a frame or two in the hold; give the message and the handler a moment
      # before the next `coords` read freezes the game.
      Start-Sleep -Milliseconds $SettleMs
    }
  } finally {
    foreach ($name in $keyboard) { Send-JkKeyUp -Name $name }
  }
  $gameWindow = Get-JkWindow -ProcessId $ProcessId
  if ($gameWindow -ne [IntPtr]::Zero) {
    [void][JkRetail]::Activate($gameWindow)
    [void][JkRetail]::SetForegroundWindow($gameWindow)
  }
}

# `-RouteWalk`: the retail half of issue #8's scripted route, in ONE game session.
#
# The port's replay is one continuous run. Four independent retail leg captures each started from a
# fresh level load, so their bars carried different pre-warp damage and could not be compared; this
# runs the same `tools/parity/routes/level1-route.tsv` rows without reloading. A `warp` row runs the
# console warp at its tick (the body goes one eye offset below the script's camera eye, as retail's
# `warp` leaves it), an `input` row holds forward for its ticks in 600 ms segments, and the bars and
# positions are read at every segment end — no console while the fight is live. A `use` row plays the
# player's own ACTIVATE binding (`-UseKey`), so the door opens through the level's script, not a
# synthetic message. Cogs are read with `cogstatus` at every sample; `seed`/`switch` rows need no
# action here.
#
# The trace is the port's 21-column route schema, including the two event columns:
# `door0_opened` latches nonzero `doorstatus`/`movestatus` (the door closes again two seconds after
# it opens), and `goal_bin100` is inventory bin 100 (1 at start, 3 after the sighting).
function Invoke-JkRouteTrace {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$Plan,
    [Parameter(Mandatory)][string]$OutDir,
    [Parameter(Mandatory)][string]$Name,
    [string]$UseKey = 'SPACE',
    [int]$UseHoldMs = 600,
    [int]$UseSettleMs = 900
  )
  $path = Join-Path $OutDir ("{0}.tsv" -f $Name)
  $fields = @('tick', 't_s', 'x', 'y', 'z', 'sector', 'vel_x', 'vel_y', 'vel_z', 'health', 'shields',
    'door0_status', 'door0_movestatus', 'door0_opendoors', 'door0_closedoors',
    'switch_status', 'switch_movestatus', 'switch_opendoors', 'switch_closedoors',
    'door0_opened', 'goal_bin100')
  $rows = New-Object System.Collections.Generic.List[string]
  $cogs = [ordered]@{
    door0_status = $null; door0_movestatus = $null; door0_opendoors = $null; door0_closedoors = $null
    switch_status = $null; switch_movestatus = $null; switch_opendoors = $null; switch_closedoors = $null
  }
  # A hashed latch: a `$script`-scope scalar assigned inside a scriptblock stays local to it.
  $state = @{ Door0Opened = $false }
  $wanted = @{
    0  = @{ doorstatus = 'door0_status'; movestatus = 'door0_movestatus'; opendoors = 'door0_opendoors'; closedoors = 'door0_closedoors' }
    11 = @{ doorstatus = 'switch_status'; movestatus = 'switch_movestatus'; open_doors = 'switch_opendoors'; close_doors = 'switch_closedoors' }
  }
  # A reply can carry the previous command's tail, so a symbol that does not appear keeps its value.
  $readCogs = {
    foreach ($cog in 0, 11) {
      foreach ($line in @(Invoke-JkDevConsoleCommand -ProcessId $ProcessId -Command ("cogstatus {0}" -f $cog))) {
        if ($line -match "Symbol \d+: '([a-zA-Z_0-9]+)'\s*=\s*([-0-9.]+)") {
          $symbol = $Matches[1]
          if ($wanted[$cog].ContainsKey($symbol)) { $cogs[$wanted[$cog][$symbol]] = [double]$Matches[2] }
        }
      }
    }
    if (($null -ne $cogs['door0_status'] -and [double]$cogs['door0_status'] -ne 0) -or
        ($null -ne $cogs['door0_movestatus'] -and [double]$cogs['door0_movestatus'] -ne 0)) {
      $state.Door0Opened = $true
    }
  }
  # One trace row. `$Coords` is a `Read-JkCoords` result (body position and tracked sector).
  $addRow = {
    param([int]$Tick, $Coords, [double]$VelX, [double]$VelY, [double]$VelZ, [switch]$Replace)
    $bars = Read-JkPlayerBars -ProcessId $ProcessId
    $cells = @(
      $Tick,
      ('{0:F4}' -f ($Tick / 60.0)),
      ('{0:F4}' -f [double]$Coords.Pos[0]),
      ('{0:F4}' -f [double]$Coords.Pos[1]),
      ('{0:F4}' -f [double]$Coords.Pos[2]),
      [int]$Coords.Sector,
      ('{0:F4}' -f $VelX),
      ('{0:F4}' -f $VelY),
      ('{0:F4}' -f $VelZ),
      ('{0:F1}' -f $bars.Health),
      ('{0:F1}' -f $bars.Shields)
    )
    foreach ($key in $cogs.Keys) {
      $value = $cogs[$key]
      $cells += $(if ($null -eq $value) { '-' } else { '{0:F3}' -f [double]$value })
    }
    $cells += $(if ($state.Door0Opened) { '1' } else { '0' })
    $cells += ('{0:F0}' -f $bars.Goal)
    $line = ($cells -join ([char]9))
    if ($Replace -and $rows.Count -gt 0) { $rows[$rows.Count - 1] = $line } else { $rows.Add($line) }
  }
  # Both sides measure movement with the AI off: retail's own `-devMode` switches are `disableai`
  # (the port's replay passes `--no-ai`) and `noaishots` (the port skips the AI tick, so no shot is
  # fired at all). Both are **toggles** in the engine, so each is set explicitly with `on`; a warp
  # also gets them re-applied in case it resets the debug flags.
  $disableAi = {
    foreach ($command in 'disableai on', 'noaishots on') {
      foreach ($reply in @(Invoke-JkDevConsoleCommand -ProcessId $ProcessId -Command $command)) {
        Log ("route: {0} -> {1}" -f $command, $reply)
      }
    }
  }
  # Parse the plan first: a plan that starts with a pose warp records that pose at tick 0, not the
  # level's start (the exit-segment capture does this to avoid replaying the earlier legs).
  $steps = New-Object System.Collections.Generic.List[object]
  $planTick = 0
  foreach ($line in Get-Content -LiteralPath $Plan) {
    if ($line -match '^\s*(#|$)') { continue }
    $f = $line -split "`t"
    switch ($f[0]) {
      'warp' {
        $pose = $null
        if ($f[3] -and $f[3] -ne '-') {
          $pose = [pscustomobject]@{ X = [double]$f[3]; Y = [double]$f[4]; Z = [double]$f[5]; Yaw = [double]$f[6] }
        }
        $steps.Add([pscustomobject]@{ Kind = 'warp'; Tick = $planTick; Pose = $pose })
      }
      'input' {
        $steps.Add([pscustomobject]@{
            Kind = 'input'; Tick = $planTick; Ticks = [int]$f[1]
            Forward = [double]$f[2]; Strafe = [double]$f[3]
          })
        $planTick += [int]$f[1]
      }
      'wait' {
        $steps.Add([pscustomobject]@{ Kind = 'wait'; Tick = $planTick; Ticks = [int]$f[1] })
        $planTick += [int]$f[1]
      }
      'use' { $steps.Add([pscustomobject]@{ Kind = 'use'; Tick = $planTick }) }
      'switch' { $steps.Add([pscustomobject]@{ Kind = 'switch'; Tick = $planTick }) }
      'seed' { $steps.Add([pscustomobject]@{ Kind = 'seed'; Tick = $planTick }) }
      default { throw "route trace: unknown row kind '$($f[0])'" }
    }
  }
  & $disableAi
  $tick = 0
  $prevCoords = $null
  $prevTick = 0
  $startsWithPose = ($steps.Count -gt 0 -and $steps[0].Kind -eq 'warp' -and $null -ne $steps[0].Pose)
  if (-not $startsWithPose) {
    $startCoords = Read-JkCoords -ProcessId $ProcessId
    if (-not $startCoords) { throw 'route trace: the start pose could not be read from coords' }
    & $readCogs
    & $addRow 0 $startCoords 0 0 0
    $prevCoords = $startCoords
  }
  foreach ($step in $steps) {
    switch ($step.Kind) {
      'warp' {
        if ($null -ne $step.Pose) {
          $bodyZ = $step.Pose.Z - $script:JkEyeOffset
          # `warp x y z pitch yaw roll` (the shots flow's own argument order): passing the route's yaw
          # in the pitch slot left the body facing nowhere and unable to walk.
          $command = 'warp {0} {1} {2} 0 {3} 0' -f $step.Pose.X, $step.Pose.Y, $bodyZ, $step.Pose.Yaw
          $reply = @(Invoke-JkDevConsoleCommand -ProcessId $ProcessId -Command $command)
          if ($reply -match 'Position not in world') { throw "route trace: warp failed: $command" }
          # The shots flow waits this long after a warp before a walk; the game is still finishing
          # the console/foreground transition and would swallow the first hold.
          Start-Sleep -Milliseconds 900
          & $disableAi
          $coords = Read-JkCoords -ProcessId $ProcessId
          if (-not $coords) { throw "route trace: coords unavailable after $command" }
          Log ("route: {0} -> {1}" -f $command, $coords.Raw)
          # The port samples the post-warp pose at this tick; replace the previous leg's last row.
          & $readCogs
          & $addRow $tick $coords 0 0 0 -Replace
          $prevCoords = $coords
          $prevTick = $tick
        }
      }
      'input' {
        if ($step.Strafe -ne 0) { throw 'route trace: strafe input is not supported' }
        $segments = [int]($step.Ticks / 36)
        # Like the walk trace: the key goes down before the window is raised, so the frame that
        # resumes the game after a console/foreground wait already sees it (a 600 ms press entirely
        # inside a paused window is one frame or none at the engine's few-fps render rate).
        Send-JkKeyDown -Name 'UP'
        try {
          for ($i = 1; $i -le $segments; $i++) {
            # A console read between segments takes the focus, so DInput only sees the hold after the
            # game window is raised and foreground (the walk trace's own loop; its body moves).
            $gameWindow = Get-JkWindow -ProcessId $ProcessId
            $active = [JkRetail]::Activate($gameWindow)
            Log ("route: focus activated={0} foreground={1}" -f $active, [JkRetail]::ForegroundTitle())
            for ($attempt = 0; $attempt -lt 8; $attempt++) {
              if ($gameWindow -eq [IntPtr]::Zero) { break }
              [void][JkRetail]::ShowWindow($gameWindow, 9)
              [void][JkRetail]::SetForegroundWindow($gameWindow)
              Start-Sleep -Milliseconds 120
              if ([JkRetail]::ForegroundTitle() -eq 'Jedi Knight') { break }
            }
            Send-JkKeyDown -Name 'UP'
            $clockBefore = Read-JkGameMs -ProcessId $ProcessId
            Start-Sleep -Milliseconds 600
            Send-JkKeyUp -Name 'UP'
            $clockAfter = Read-JkGameMs -ProcessId $ProcessId
            Log ("route: hold clock {0} -> {1} ms" -f $clockBefore, $clockAfter)
            $tick += 36
            # End the segment with a console read like the walk trace: it freezes the game between
            # holds, so the pre-pressed key cannot move the body during the foreground wait and each
            # 600 ms sample measures one hold (the committed walk traces' methodology).
            $coords = Read-JkCoords -ProcessId $ProcessId
            if (-not $coords) { throw "route trace: coords unavailable at tick $tick" }
            $window = [double]($tick - $prevTick) / 60.0
            $vx = if ($window -gt 0) { ([double]$coords.Pos[0] - [double]$prevCoords.Pos[0]) / $window } else { 0.0 }
            $vy = if ($window -gt 0) { ([double]$coords.Pos[1] - [double]$prevCoords.Pos[1]) / $window } else { 0.0 }
            $vz = if ($window -gt 0) { ([double]$coords.Pos[2] - [double]$prevCoords.Pos[2]) / $window } else { 0.0 }
            & $addRow $tick $coords $vx $vy $vz
            Log ("route: tick {0} ({1} ms held) pos ({2:N2}, {3:N2}, {4:N2}) sector {5}" -f `
                $tick, ($i * 600), $coords.Pos[0], $coords.Pos[1], $coords.Pos[2], $coords.Sector)
            $prevCoords = $coords
            $prevTick = $tick
          }
        } finally {
          Send-JkKeyUp -Name 'UP'
        }
        # One cog read per leg (not per 600 ms segment): the engine's dev console is the fragile part
        # of a long capture, and a per-segment read crashed it once; the door and goal latches only
        # need a read after the opening step and at the leg ends.
        & $readCogs
        $coords = Read-JkCoords -ProcessId $ProcessId
        if ($coords) { Log ("route: leg end -> " + $coords.Raw) } else { Log 'route: leg end coordinates unavailable' }
      }
      'wait' {
        # Stand still for the ticks, in 600 ms segments, reading the pose between them. The key is up
        # and the game runs, so a script's door leaf finishes opening before the next hold — the port
        # sample at this tick is the same standing pose, which keeps the crossing row on both sides
        # in the same sector.
        $segments = [int]($step.Ticks / 36)
        for ($i = 1; $i -le $segments; $i++) {
          $gameWindow = Get-JkWindow -ProcessId $ProcessId
          [void][JkRetail]::Activate($gameWindow)
          for ($attempt = 0; $attempt -lt 8; $attempt++) {
            if ($gameWindow -eq [IntPtr]::Zero) { break }
            [void][JkRetail]::ShowWindow($gameWindow, 9)
            [void][JkRetail]::SetForegroundWindow($gameWindow)
            Start-Sleep -Milliseconds 120
            if ([JkRetail]::ForegroundTitle() -eq 'Jedi Knight') { break }
          }
          $clockBefore = Read-JkGameMs -ProcessId $ProcessId
          Start-Sleep -Milliseconds 600
          $clockAfter = Read-JkGameMs -ProcessId $ProcessId
          Log ("route: wait clock {0} -> {1} ms" -f $clockBefore, $clockAfter)
          $tick += 36
          $coords = Read-JkCoords -ProcessId $ProcessId
          if (-not $coords) { throw "route trace: coords unavailable at tick $tick" }
          $window = [double]($tick - $prevTick) / 60.0
          $vx = if ($window -gt 0) { ([double]$coords.Pos[0] - [double]$prevCoords.Pos[0]) / $window } else { 0.0 }
          $vy = if ($window -gt 0) { ([double]$coords.Pos[1] - [double]$prevCoords.Pos[1]) / $window } else { 0.0 }
          $vz = if ($window -gt 0) { ([double]$coords.Pos[2] - [double]$prevCoords.Pos[2]) / $window } else { 0.0 }
          & $addRow $tick $coords $vx $vy $vz
          Log ("route: tick {0} (waited {1} ms) pos ({2:N2}, {3:N2}, {4:N2}) sector {5}" -f `
              $tick, ($i * 600), $coords.Pos[0], $coords.Pos[1], $coords.Pos[2], $coords.Sector)
          $prevCoords = $coords
          $prevTick = $tick
        }
      }
      'use' {
        # The player's own ACTIVATE binding. Retail only reports what its own script does with the
        # message; nothing here sends an `activate` on the script's behalf.
        Log ("route: use row at tick {0} -- pressing the ACTIVATE binding {1}" -f $tick, $UseKey)
        Invoke-JkUseKey -ProcessId $ProcessId -Key $UseKey -HoldMs $UseHoldMs -SettleMs $UseSettleMs
        & $readCogs
      }
      'switch' {
        # Retail opens that door through the level's own plate link. The port's synthetic `activate`
        # was removed from the plan; script it here only if retail's own reading ever needs it.
        Log 'route: switch row has no synthetic action'
      }
      'seed' { }
    }
  }
  $out = New-Object System.Collections.Generic.List[string]
  $out.Add(($fields -join ([char]9)))
  foreach ($row in $rows) { $out.Add($row) }
  Set-Content -LiteralPath $path -Value $out -Encoding utf8
  Log ("route trace: wrote {0} ({1} row(s))" -f $path, ($rows.Count))
  $finalBars = Read-JkPlayerBars -ProcessId $ProcessId
  Log ("route trace: final goal bin 100 = {0:F0}, health {1:F1}, shields {2:F1}" -f $finalBars.Goal, $finalBars.Health, $finalBars.Shields)
  # The level-end tally is retail's level-complete witness: `01_endlevel.cog`'s `user0` sleeps 3 s
  # after the sighting and calls `jkEndLevel`. Keep the frame; the trace's last row is the state
  # before the swap.
  Start-Sleep -Milliseconds 4000
  $endPng = Join-Path $OutDir ("{0}-end.png" -f $Name)
  Save-JkWindowPng -ProcessId $ProcessId -Path $endPng
  Log ("route trace: end-of-route frame {0}" -f $endPng)
  return $path
}
# `-AudioRoute`: the retail half of issue #13's audio fixture.
#
# Walks the same route file the port replays (`--replay`): each `warp` row is applied through the
# dev console (the game pauses for it), each `input` row holds forward for its tick count. Every
# 600 ms of route game time a row is written in the port's audio-trace schema, read from the
# engine's own globals (see `Invoke-JkAudioProbe` for the addresses), so the two files compare
# without a second format:
#
#   music        `PlaySong`'s start/end track; `music_loop` is the mixer's loop flag (1/0)
#   music_vol    f32 0x0054A678  (`sithSoundMixer_SetMusicVolume` 0x4DB080 clamps the script value
#                     here; 0x0054A67C is the user's global multiplier the two are applied through)
#   ambient      the loop channel (flags bit 0x1) with the largest gain; its script volume
#   sounds       non-loop channels that started in the window, as `name@gain`
#
# The window polls the mixer every 100 ms so a short one-shot cannot fall between samples.
function Invoke-JkAudioRoute {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$RoutePath,
    [Parameter(Mandatory)][string]$OutPath,
    [int]$CadenceMs = 600
  )
  $route = @()
  foreach ($line in (Get-Content -LiteralPath $RoutePath)) {
    if ($line -match '^\s*#' -or $line.Trim() -eq '') { continue }
    $cells = $line -split "`t"
    if ($cells[0] -eq 'warp') {
      # The file's header has a blank column between `name` and `x`, so read the numbers compactly.
      # A `-` pose is the level's own start: there is no warp to apply for it.
      $vals = @($cells | Select-Object -Skip 2 | Where-Object { $_ -ne '' })
      if ($vals.Count -lt 4 -or $vals[0] -eq '-') { continue }
      $route += [pscustomobject]@{ Kind = 'warp'; X = [double]$vals[0]; Y = [double]$vals[1]; Z = [double]$vals[2]; Yaw = [double]$vals[3]; Ticks = 0 }
    } elseif ($cells[0] -eq 'input') {
      $route += [pscustomobject]@{ Kind = 'input'; Ticks = [int]$cells[1] }
    }
  }
  if ($route.Count -eq 0) { throw "audio route: no warp/input rows in $RoutePath" }
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add((@('tick', 't_s', 'music', 'music_end', 'music_loop', 'music_vol', 'ambient', 'ambient_vol', 'sounds') -join ([char]9)))
  $legTick = 0
  $sampleMs = 0
  $windowEvents = New-Object System.Collections.Generic.List[string]
  $seen = @{}
  $clock0 = -1
  $sample = {
    param([int]$tick)
    $vol = [JkRetail]::ReadFloat($ProcessId, 0x0054A678)
    $from = [JkRetail]::ReadU32($ProcessId, 0x008BBC74)
    $to = [JkRetail]::ReadU32($ProcessId, 0x008BBC70)
    $loop = [JkRetail]::ReadU32($ProcessId, 0x00835FCC)
    $playing = [JkRetail]::ReadU32($ProcessId, 0x00835FD0)
    # The range is the selection; `0x00835FD0` is the MCI start result, which reads 0 when the
    # redbook device does not start -- the selection is still what the engine holds.
    $musicName = if ($from -ne 0 -or $to -ne 0) { "$from" } else { '-' }
    $musicEnd = if ($musicName -ne '-') { "$to" } else { '-' }
    $musicLoop = if ($musicName -ne '-') { if ($loop -ne 0) { '1' } else { '0' } } else { '-' }
    $ambient = $null
    $ambientVol = 0.0
    $ambientGain = 0.0
    for ($i = 0; $i -lt 32; $i++) {
      $base = 0x00835FE8 + $i * 0x5C
      $flags = [JkRetail]::ReadU32($ProcessId, $base + 0x0C)
      if (($flags -band 0x20000) -eq 0) { continue }
      $sound = [JkRetail]::ReadU32($ProcessId, $base + 0x08)
      if ($sound -le 0) { continue }
      $nameBytes = [JkRetail]::ReadBytes($ProcessId, $sound, 32)
      if ($null -eq $nameBytes) { continue }
      $zero = [Array]::IndexOf($nameBytes, [byte]0)
      if ($zero -lt 0) { $zero = 32 }
      $name = [Text.Encoding]::ASCII.GetString($nameBytes, 0, $zero)
      $isLoop = ($flags -band 0x1) -ne 0
      $volume = [JkRetail]::ReadFloat($ProcessId, $base + 0x14)
      $minR = [JkRetail]::ReadFloat($ProcessId, $base + 0x18)
      $maxR = [JkRetail]::ReadFloat($ProcessId, $base + 0x1C)
      $dist = [JkRetail]::ReadFloat($ProcessId, $base + 0x38)
      $fade = 0.0
      if ($maxR -gt $minR) { $fade = [math]::Min(1.0, [math]::Max(0.0, ($dist - $minR) / ($maxR - $minR))) }
      $gain = [math]::Min(1.0, [math]::Max(0.0, $volume * (1.0 - $fade))) * 0.75
      if ($isLoop) {
        if ($null -eq $ambient -or $gain -gt $ambientGain) {
          $ambient = $name
          $ambientVol = $volume
          $ambientGain = $gain
        }
      } elseif (-not $seen.ContainsKey($name)) {
        $windowEvents.Add(('{0}@{1:F3}' -f $name, $gain))
      }
      $seen[$name] = $true
    }
    # Empty cells are recorded silence (no loop channel in this window); `-` would mean the field
    # was not recorded, which is only true when the range itself is unset.
    $ambientName = if ($null -eq $ambient) { '' } else { $ambient }
    $ambientVolume = if ($null -eq $ambient) { '' } else { '{0:F3}' -f $ambientVol }
    $sounds = ($windowEvents -join ' ')
    $lines.Add((@($tick, ('{0:F4}' -f ([double]$tick / 60.0)), $musicName, $musicEnd, $musicLoop, ('{0:F3}' -f $vol), $ambientName, $ambientVolume, $sounds) -join ([char]9)))
    $pthing = [JkRetail]::ReadU32($ProcessId, 0x008319A4)
    if ($pthing -gt 0) {
      $px = [JkRetail]::ReadFloat($ProcessId, $pthing + 0xB8)
      $py = [JkRetail]::ReadFloat($ProcessId, $pthing + 0xBC)
      Log ("audio route: t={0} pos=({1:F3},{2:F3}) music={3} vol={4:F3}" -f $tick, $px, $py, $musicName, [double]$vol)
    }
    $windowEvents.Clear()
    $seen = @{}
  }
  $key = 'UP'
  foreach ($leg in $route) {
    if ($leg.Kind -eq 'warp') {
      $cmd = 'warp {0} {1} {2} {3} 0 0' -f $leg.X, $leg.Y, $leg.Z, $leg.Yaw
      $warpClock0 = Read-JkGameMs -ProcessId $ProcessId
      $reply = @(Invoke-JkDevConsoleCommand -ProcessId $ProcessId -Command $cmd)
      $warpClock1 = Read-JkGameMs -ProcessId $ProcessId
      Log ("audio route: warp game_ms {0} -> {1} ({2} ms of game time spent in the console call)" -f $warpClock0, $warpClock1, ($warpClock1 - $warpClock0))
      if ($reply -match 'Position not in world') { throw "audio route: $cmd was refused: $reply" }
      Log "audio route: warp $($leg.X),$($leg.Y),$($leg.Z) yaw $($leg.Yaw)"
      if ($clock0 -lt 0) {
        # The route clock starts after the first warp lands the body at the route's own start.
        Start-Sleep -Milliseconds 400
        $clock0 = Read-JkGameMs -ProcessId $ProcessId
        if ($clock0 -lt 0) { throw 'audio route: the game clock (0x00836C14) could not be read' }
      } elseif ($warpClock1 -gt $warpClock0 -and $warpClock0 -ge 0) {
        # The engine keeps running while the dev-console command is issued (~0.4-1.2 s of game
        # time per warp). The route's warps cost zero ticks in the port, so the spent time is
        # removed from the route clock; otherwise every later leg -- and the bar crossing -- lands
        # that much later than the port's tick grid and the audio samples compare different legs.
        $clock0 = $clock0 + ($warpClock1 - $warpClock0)
        Log ("audio route: route clock re-based by {0} ms for the warp" -f ($warpClock1 - $warpClock0))
      }
      continue
    }
    $legEndMs = ($legTick + $leg.Ticks) * 1000.0 / 60.0
    $gameWindow = Get-JkWindow -ProcessId $ProcessId
    if ($gameWindow -ne [IntPtr]::Zero) {
      [void][JkRetail]::ShowWindow($gameWindow, 9)
      [void][JkRetail]::SetForegroundWindow($gameWindow)
      Start-Sleep -Milliseconds 120
    }
    Send-JkKeyDown -Name $key
    try {
      while ($true) {
        $clock = Read-JkGameMs -ProcessId $ProcessId
        if ($clock -lt 0) { throw 'audio route: the game clock stopped reading' }
        $elapsed = [double]$clock - [double]$clock0
        # The engine clock advances in frame-sized jumps under software rendering; fill every grid
        # point the jump crossed, not just one, so the row count matches the port's timeline.
        while ($elapsed -ge $sampleMs -and $sampleMs -le $legEndMs) {
          & $sample ([int]($sampleMs / 1000.0 * 60.0))
          $sampleMs += $CadenceMs
        }
        if ($elapsed -ge $legEndMs) { break }
        Start-Sleep -Milliseconds 8
      }
    } finally {
      Send-JkKeyUp -Name $key
    }
    $legTick += $leg.Ticks
  }
  Set-Content -LiteralPath $OutPath -Value $lines -Encoding utf8
  Log ("audio route: wrote {0} ({1} row(s))" -f $OutPath, ($lines.Count - 1))
  return $OutPath
}
# `-AudioProbeMs`: the retail half of issue #13's audio fixture, probe form.
#
# Capture instrumentation only. The addresses are the engine's own globals (JK.EXE has no
# relocations, so they are fixed), recovered from the decompiled mixer:
#
#   song range        u32 start 0x008BBC74, u32 end 0x008BBC70, u32 loop-back 0x00835FCC,
#                     u32 playing 0x00835FD0                 (`sithSoundMixer_PlaySong`, 0x4DAE60)
#   music volume      f32 0x0054A678                           (`SetMusicVol` -> 0x4DB080)
#   mixer channels    base 0x00835FE8, 32 entries of 0x5C:
#                       +0x08 sound struct, +0x0C flags, +0x14 volume, +0x18 min radius,
#                       +0x1C max radius, +0x38 distance, +0x48 thing
#                     (`sithSoundMixer_PlaySoundThing`, 0x4DBA60)
#   sound name        the sound struct's own char[32] at +0 (`FUN_004EF1D0` copies the name in)
#
# Nothing is written to the process: the probe only reads. `music_vol` is the mixer's own value; a
# channel's `gain` mirrors the port's `snd::volume` (base, linear min..max falloff, 0.75 final
# scale) from the channel's own fields, without the rear-pan term (the channel keeps no listener
# basis; a sound behind the listener would read high).
function Invoke-JkAudioProbe {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [int]$Milliseconds = 4000,
    [int]$CadenceMs = 100
  )
  $clock0 = Read-JkGameMs -ProcessId $ProcessId
  if ($clock0 -lt 0) { throw 'audio probe: the game clock (0x00836C14) could not be read' }
  $vol0 = [JkRetail]::ReadFloat($ProcessId, 0x0054A678)
  Log ("audio probe: start song {0}..{1} (loop {2}, playing {3}) volume {4:N3}" -f [JkRetail]::ReadU32($ProcessId, 0x008BBC74), [JkRetail]::ReadU32($ProcessId, 0x008BBC70), [JkRetail]::ReadU32($ProcessId, 0x00835FCC), [JkRetail]::ReadU32($ProcessId, 0x00835FD0), $vol0)
  $next = 0
  while ($true) {
    $clock = Read-JkGameMs -ProcessId $ProcessId
    if ($clock -lt 0) { throw 'audio probe: the game clock stopped reading' }
    $elapsed = [double]$clock - [double]$clock0
    if ($elapsed -ge $Milliseconds) { break }
    if ($elapsed -lt $next) { Start-Sleep -Milliseconds 8; continue }
    $vol = [JkRetail]::ReadFloat($ProcessId, 0x0054A678)
    $from = [JkRetail]::ReadU32($ProcessId, 0x008BBC74)
    $to = [JkRetail]::ReadU32($ProcessId, 0x008BBC70)
    $num = [JkRetail]::ReadU32($ProcessId, 0x00835FCC)
    $playing = [JkRetail]::ReadU32($ProcessId, 0x00835FD0)
    $channels = @()
    for ($i = 0; $i -lt 32; $i++) {
      $base = 0x00835FE8 + $i * 0x5C
      $flags = [JkRetail]::ReadU32($ProcessId, $base + 0x0C)
      if (($flags -band 0x20000) -eq 0) { continue }
      $sound = [JkRetail]::ReadU32($ProcessId, $base + 0x08)
      if ($sound -le 0) { continue }
      $nameBytes = [JkRetail]::ReadBytes($ProcessId, $sound, 32)
      if ($null -eq $nameBytes) { continue }
      $zero = [Array]::IndexOf($nameBytes, [byte]0)
      if ($zero -lt 0) { $zero = 32 }
      $name = [Text.Encoding]::ASCII.GetString($nameBytes, 0, $zero)
      $volume = [JkRetail]::ReadFloat($ProcessId, $base + 0x14)
      $minR = [JkRetail]::ReadFloat($ProcessId, $base + 0x18)
      $maxR = [JkRetail]::ReadFloat($ProcessId, $base + 0x1C)
      $dist = [JkRetail]::ReadFloat($ProcessId, $base + 0x38)
      $thing = [JkRetail]::ReadU32($ProcessId, $base + 0x48)
      $fade = 0.0
      if ($maxR -gt $minR) { $fade = [math]::Min(1.0, [math]::Max(0.0, ($dist - $minR) / ($maxR - $minR))) }
      $gain = [math]::Min(1.0, [math]::Max(0.0, $volume * (1.0 - $fade))) * 0.75
      $channels += ('{0}@{1:F3}(v={2:F3} d={3:F2} r={4:F2}..{5:F2} thing={6} flags=0x{7:X})' -f $name, $gain, $volume, $dist, $minR, $maxR, $thing, $flags)
    }
    Log ("audio probe: t={0} music={1},{2},{3} playing={4} vol={5:F3} channels=[{6}]" -f [math]::Round($elapsed), $from, $to, $num, $playing, $vol, ($channels -join '; '))
    $next += $CadenceMs
  }
  Log 'audio probe: done'
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

# `-EngineCaptureMs`: issue #20's engine-clock route/encounter capture.
#
# One measurement window is sampled from the process on a short loop. The dev console is **not** read
# inside it: its dialog pauses the engine, so a console poll would sample a frozen clock and change
# the encounter. A row is written whenever `DAT_00836c14` changes, so the time axis is the engine's
# own simulation clock, not wall or console-poll time.
#
# Addresses (JK.EXE has a fixed 0x400000 base and no relocations, so a decompiled global's address is
# its runtime address). Every one is either reused from the encounter/audio readers or read from the
# decompilation:
#
#   game clock (ms)        u32 at 0x00836C14          `sithTime_Advance` 0x4DD640
#   player thing           u32 at 0x008319A4          (the pointer the other readers use)
#   position               f32x3 at thing+0xB8        `GetThingPos` 0x502EF0
#   velocity               f32x3 at thing+0x1D4       `GetThingVel` 0x503160 (also
#                                                     `sithCamera_Update` 0x4C56B0, `jkPlayer_DrawPov`)
#   sector                 u32 at *(thing+0x5C)       `GetThingSector` 0x502B80
#   inventory              u32 at thing+0x1C0; bin n's amount is the f32 at inv+0x88+n*0x18
#                                                     (`sithInventory_SetInventory` 0x4D2000; the
#                                                     shields reader's bin 60 proves the stride)
#   AI random state        u32 at 0x0054F750          `FUN_00512d00`: state = state*0x343FD + 0x269EC3
#   AI instances           base 0x0088AFE0, stride 0x29C, highest slot u32 at 0x008B4BE0
#                                                     (`sithAI_Create` 0x4E9CB0, `sithAI_Process` 0x4E9E20)
#   instinct registry      pointer u32 at 0x0084DE50, count u32 at 0x0084DE54, entry stride 0x30,
#                          name char[0x20] at +0, handler u32 at +0x20
#                                                     (`sithAI_RegisterInstinct` 0x4EA0A0)
#
# AI instance fields (`sithAI_InstinctUpdate` 0x4E9E90 and the handlers):
#   +0x00 actor thing, +0x04 AI class, +0x08 flag word, +0x18C instinct count, +0x190 next pass (ms),
#   +0x1C0 last acquired target, +0x1D0 current target, +0x204 last acquisition time (ms);
#   instinct i's next-run time is at +0x10+i*0x18 (the handler writes it: `param_3+4`), and class
#   instinct i's handler is at +0xB8+i*0x90. Registry names `primaryfire` (next fire) and
#   `lookfortarget` (next acquisition roll) therefore name the two slots the encounter reads.
$script:JkEngineCaptureColumns = @(
  't_ms', 'kind', 'thing', 'template', 'x', 'y', 'z', 'sector', 'vx', 'vy', 'vz',
  'bin72', 'bin73', 'bin75', 'rng', 'ai_flags', 'fire_next_ms', 'acquire_next_ms',
  'acquired_at_ms', 'target', 'last_target', 'next_pass_ms', 'detail'
)

# The fixture is parsed on other machines, so a comma decimal separator must never reach it.
function Format-JkEngineNumber {
  param([double]$Value, [int]$Digits = 4)
  return $Value.ToString(('F{0}' -f $Digits), [System.Globalization.CultureInfo]::InvariantCulture)
}

function New-JkEngineCaptureRow {
  param([hashtable]$Fields)
  $row = foreach ($column in $script:JkEngineCaptureColumns) {
    if ($Fields.ContainsKey($column)) { [string]$Fields[$column] } else { '' }
  }
  return ,$row
}

function Test-JkEngineNumber {
  param([string]$Text)
  $value = 0.0
  return [double]::TryParse($Text, [System.Globalization.NumberStyles]::Float, [System.Globalization.CultureInfo]::InvariantCulture, [ref]$value)
}

function Test-JkEngineInteger {
  param([string]$Text, [ref]$Value)
  $number = 0.0
  if (-not [double]::TryParse($Text, [System.Globalization.NumberStyles]::Float, [System.Globalization.CultureInfo]::InvariantCulture, [ref]$number)) { return $false }
  if ($number -ne [math]::Floor($number)) { return $false }
  $Value.Value = [long]$number
  return $true
}

# Schema check for the engine-capture fixture. Returns one message per problem (an empty list for a
# well-formed file). The capture runs it on its own output before it claims a fixture, and
# `-EngineCaptureSelfCheck` runs it against synthetic rows and deliberate corruptions.
function Test-JkEngineCaptureRows {
  param([string[]]$Lines)
  $errors = New-Object System.Collections.Generic.List[string]
  $columns = $script:JkEngineCaptureColumns
  $header = $columns -join "`t"
  $origin = $null
  $headerSeen = $false
  $rows = 0
  $previousMs = $null
  for ($n = 0; $n -lt $Lines.Count; $n++) {
    $line = $Lines[$n]
    $at = $n + 1
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    if ($line.StartsWith('#')) {
      if ($line -match '^#\s*origin_ms\s+(-?\d+)\s*$') { $origin = [long]$Matches[1] }
      continue
    }
    if (-not $headerSeen) {
      if ($line -ne $header) { $errors.Add("line ${at}: header is not the engine-capture schema"); return @($errors) }
      $headerSeen = $true
      continue
    }
    $f = $line -split "`t"
    if ($f.Count -ne $columns.Count) {
      $errors.Add("line ${at}: $($f.Count) column(s), expected $($columns.Count)")
      continue
    }
    $field = @{}
    for ($c = 0; $c -lt $columns.Count; $c++) { $field[$columns[$c]] = $f[$c] }
    $kind = $field['kind']
    if ($kind -ne 'player' -and $kind -ne 'origin') { $errors.Add("line ${at}: unknown kind '$kind'"); continue }
    $ms = [long]0
    if (-not (Test-JkEngineInteger $field['t_ms'] ([ref]$ms))) { $errors.Add("line ${at}: t_ms is not an integer"); continue }
    if ($rows -eq 0 -and $null -eq $origin) { $errors.Add("line ${at}: no '# origin_ms' line before the first row") }
    if ($rows -eq 0 -and $null -ne $origin -and $ms -ne $origin) { $errors.Add("line ${at}: first row t_ms $ms is not the origin clock $origin") }
    if ($null -ne $previousMs -and $ms -lt $previousMs) { $errors.Add("line ${at}: t_ms $ms goes backwards from $previousMs") }
    $previousMs = $ms
    $rows++
    if ($kind -eq 'player') {
      foreach ($c in 'x', 'y', 'z', 'vx', 'vy', 'vz', 'bin72', 'bin73', 'bin75') {
        if (-not (Test-JkEngineNumber $field[$c])) { $errors.Add("line ${at}: player $c is not a number") }
      }
      foreach ($c in 'sector', 'rng') {
        $value = [long]0
        if (-not (Test-JkEngineInteger $field[$c] ([ref]$value))) { $errors.Add("line ${at}: player $c is not an integer") }
      }
      if ($field['template'] -ne 'walkplayer') { $errors.Add("line ${at}: player template is not walkplayer") }
      foreach ($c in 'ai_flags', 'fire_next_ms', 'acquire_next_ms', 'acquired_at_ms', 'target', 'last_target', 'next_pass_ms') {
        if ($field[$c] -ne '') { $errors.Add("line ${at}: player row carries $c") }
      }
    } else {
      foreach ($c in 'x', 'y', 'z') {
        if (-not (Test-JkEngineNumber $field[$c])) { $errors.Add("line ${at}: origin $c is not a number") }
      }
      foreach ($c in 'ai_flags', 'fire_next_ms', 'acquire_next_ms', 'acquired_at_ms', 'target', 'last_target', 'next_pass_ms') {
        $value = [long]0
        if (-not (Test-JkEngineInteger $field[$c] ([ref]$value))) { $errors.Add("line ${at}: origin $c is not an integer") }
      }
      if ($null -ne $origin -and $ms -ne $origin) { $errors.Add("line ${at}: origin row is not at the origin clock $origin") }
      foreach ($c in 'sector', 'vx', 'vy', 'vz', 'bin72', 'bin73', 'bin75', 'rng') {
        if ($field[$c] -ne '') { $errors.Add("line ${at}: origin row carries $c") }
      }
    }
  }
  if (-not $headerSeen) { $errors.Add('no header line') }
  if ($rows -eq 0) { $errors.Add('no data rows') }
  return @($errors)
}

function Write-JkEngineCaptureFile {
  param(
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][long]$OriginMs,
    [Parameter(Mandatory)][string]$Name,
    [string[]]$HeaderComments = @(),
    [object[]]$Rows = @()
  )
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add(("# engine-clock capture: {0}" -f $Name))
  $lines.Add(("# origin_ms {0}" -f $OriginMs))
  foreach ($comment in $HeaderComments) { $lines.Add(("# " + $comment)) }
  $lines.Add(($script:JkEngineCaptureColumns -join "`t"))
  foreach ($row in $Rows) { $lines.Add((@($row) -join "`t")) }
  Set-Content -LiteralPath $Path -Value $lines -Encoding utf8
}

# The encounter-origin block: every AI-instance actor within `Radius` of the player, with the state
# that decides when its first shot and next acquisition can happen. Read once, at the origin, so the
# port can align its first AI update with retail's phase (all times are engine ms, absolute).
function Get-JkAiOriginRows {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][long]$PlayerThing,
    [float]$X,
    [float]$Y,
    [double]$Radius = 8.0,
    [Parameter(Mandatory)][long]$OriginMs,
    # `(address, count) -> byte[]`. Defaults to the process; `-EngineCaptureSelfCheck` passes a
    # synthetic memory image instead, so this path is exercised without retail.
    [scriptblock]$ReadBytes = $null
  )
  $rows = New-Object System.Collections.Generic.List[object]
  if (-not $ReadBytes) {
    $ReadBytes = { param([long]$Address, [int]$Count) , [JkRetail]::ReadBytes($ProcessId, $Address, $Count) }
  }
  # -1 / NaN on a failed read, like `[JkRetail]::ReadU32` / `ReadFloat`.
  $readU32 = {
    param([long]$Address)
    $bytes = & $ReadBytes $Address 4
    if ($null -eq $bytes) { return [long]-1 }
    return [long][BitConverter]::ToUInt32($bytes, 0)
  }
  $regPtr = & $readU32 0x0084DE50
  $regCount = & $readU32 0x0084DE54
  if ($regPtr -le 0 -or $regCount -le 0 -or $regCount -gt 0x20) {
    throw ("engine capture: the instinct registry (0x0084DE50/54) read ptr {0}, count {1}" -f $regPtr, $regCount)
  }
  $registry = & $ReadBytes $regPtr ([int]($regCount * 0x30))
  if ($null -eq $registry) { throw 'engine capture: the instinct registry block could not be read' }
  $byHandler = @{}
  for ($i = 0; $i -lt $regCount; $i++) {
    $at = $i * 0x30
    $end = 0
    while ($end -lt 0x20 -and $registry[$at + $end] -ne 0) { $end++ }
    $name = [System.Text.Encoding]::ASCII.GetString($registry, $at, $end)
    $byHandler[([string][BitConverter]::ToUInt32($registry, $at + 0x20))] = $name
  }
  $aiTop = & $readU32 0x008B4BE0
  if ($aiTop -lt 0 -or $aiTop -gt 0xFF) {
    Log ("engine capture: no AI instance slots (0x008B4BE0 read {0})" -f $aiTop)
    return @()
  }
  $instances = & $ReadBytes 0x0088AFE0 (([int]$aiTop + 1) * 0x29C)
  if ($null -eq $instances) { throw 'engine capture: the AI instance block could not be read' }
  for ($i = 0; $i -le $aiTop; $i++) {
    $at = $i * 0x29C
    $actor = [BitConverter]::ToUInt32($instances, $at)
    if ($actor -eq 0) { continue }
    $class = [BitConverter]::ToUInt32($instances, $at + 4)
    $flags = [BitConverter]::ToUInt32($instances, $at + 8)
    $count = [BitConverter]::ToUInt32($instances, $at + 0x18C)
    $nextPass = [BitConverter]::ToUInt32($instances, $at + 0x190)
    $target = [BitConverter]::ToUInt32($instances, $at + 0x1D0)
    $lastTarget = [BitConverter]::ToUInt32($instances, $at + 0x1C0)
    $acquiredAt = [BitConverter]::ToUInt32($instances, $at + 0x204)
    $thing = & $ReadBytes $actor 0x2B0
    if ($null -eq $thing) { continue }
    if ([BitConverter]::ToInt32($thing, 0x0C) -ne 2) { continue }
    $health = [BitConverter]::ToSingle($thing, 0x134)
    if ([double]::IsNaN($health) -or $health -le 0.0) { continue }
    $ax = [BitConverter]::ToSingle($thing, 0xB8)
    $ay = [BitConverter]::ToSingle($thing, 0xBC)
    $az = [BitConverter]::ToSingle($thing, 0xC0)
    $dx = [double]$ax - [double]$X
    $dy = [double]$ay - [double]$Y
    if (($dx * $dx + $dy * $dy) -gt ($Radius * $Radius)) { continue }
    $template = [BitConverter]::ToUInt16($thing, 0x04)
    $fire = -1
    $acquire = -1
    if ($class -ne 0 -and $count -gt 0 -and $count -le 0x10) {
      $classBytes = & $ReadBytes $class 0x94C
      if ($null -ne $classBytes) {
        for ($j = 0; $j -lt $count; $j++) {
          $handler = [string][BitConverter]::ToUInt32($classBytes, 0xB8 + $j * 0x90)
          if (-not $byHandler.ContainsKey($handler)) { continue }
          $name = $byHandler[$handler].ToLowerInvariant()
          if ($name -eq 'primaryfire' -and $fire -lt 0) { $fire = $j }
          elseif ($name -eq 'lookfortarget' -and $acquire -lt 0) { $acquire = $j }
        }
      }
    }
    $fireNext = if ($fire -ge 0) { [long][BitConverter]::ToUInt32($instances, $at + 0x10 + $fire * 0x18) } else { -1 }
    $acquireNext = if ($acquire -ge 0) { [long][BitConverter]::ToUInt32($instances, $at + 0x10 + $acquire * 0x18) } else { -1 }
    $targetKind = if ($target -eq 0) { 'none' } elseif ([long]$target -eq $PlayerThing) { 'player' } else { 'other' }
    $detail = 'fire={0} acquire={1} target={2}' -f `
      $(if ($fire -ge 0) { 'primaryfire' } else { 'none' }), `
      $(if ($acquire -ge 0) { 'lookfortarget' } else { 'none' }), $targetKind
    $rows.Add((New-JkEngineCaptureRow @{
            t_ms = [string]$OriginMs; kind = 'origin'; thing = [string]$actor; template = [string]$template
            x = (Format-JkEngineNumber $ax); y = (Format-JkEngineNumber $ay); z = (Format-JkEngineNumber $az)
            ai_flags = [string]$flags; fire_next_ms = [string]$fireNext; acquire_next_ms = [string]$acquireNext
            acquired_at_ms = [string]$acquiredAt; target = [string]$target; last_target = [string]$lastTarget
            next_pass_ms = [string]$nextPass; detail = $detail
          }))
    Log ("engine capture: origin actor {0} template {1} at ({2:N3}, {3:N3}, {4:N3}): flags 0x{5:X} fire_next {6} acquire_next {7} acquired_at {8} next_pass {9} {10}" -f `
        $actor, $template, $ax, $ay, $az, $flags, $fireNext, $acquireNext, $acquiredAt, $nextPass, $detail)
  }
  # `ToArray()`: an `@()` over a `List[object]` throws "Argument types do not match" on PowerShell 7.6
  # (`PSEnumerableBinder.MaybeDebase`), which aborted the whole capture at the origin sample.
  return $rows.ToArray()
}

# Serves the origin builder's reads from a synthetic memory image (`address -> block of bytes`) for
# `-EngineCaptureSelfCheck`: `Count` bytes from the block that contains the address, or $null, the way
# ReadProcessMemory answers. Never used by a retail capture.
function Read-JkSyntheticMemory {
  param(
    [Parameter(Mandatory)][hashtable]$Image,
    [Parameter(Mandatory)][long]$Address,
    [int]$Count
  )
  foreach ($base in @($Image.Keys)) {
    $block = $Image[$base]
    if ($base -le $Address -and ($Address + $Count) -le ($base + $block.Length)) {
      $slice = New-Object byte[] $Count
      [Array]::Copy($block, [int]($Address - $base), $slice, 0, $Count)
      return , $slice
    }
  }
  return $null
}

function Invoke-JkEngineCapture {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$Name,
    [int]$Milliseconds = 10000,
    [string]$Key = '',
    [double]$Radius = 8.0,
    [Parameter(Mandatory)][string]$OutDir
  )
  $path = Join-Path $OutDir ("{0}.tsv" -f $Name)
  $player = [JkRetail]::ReadU32($ProcessId, 0x008319A4)
  if ($player -le 0) { throw "engine capture: the player thing pointer (0x008319A4) read $player" }
  $health0 = [JkRetail]::ReadFloat($ProcessId, $player + 0x134)
  if ([double]::IsNaN($health0) -or $health0 -lt 0.0 -or $health0 -gt 1000.0) {
    throw "engine capture: implausible health $health0 at thing+0x134"
  }
  $clock0 = Read-JkGameMs -ProcessId $ProcessId
  if ($clock0 -lt 0) { throw 'engine capture: the game clock (0x00836C14) could not be read' }
  if ([JkRetail]::ReadU32($ProcessId, 0x0054F750) -lt 0) {
    throw 'engine capture: the AI random state (0x0054F750) could not be read'
  }

  $rows = New-Object System.Collections.Generic.List[object]
  $originMs = $null
  $originPose = ''
  $lastClock = [long]-1
  $sampleCount = 0
  $actorCount = 0
  $watch = [Diagnostics.Stopwatch]::StartNew()
  # A window is a run of simulation time. If the clock stops (a menu, a pause, a lost level) fail
  # instead of spinning: 4x the window plus 15 s is generous at retail's few frames per second.
  $deadlineMs = [math]::Max(15000, $Milliseconds * 4)
  if ($Key) {
    $gameWindow = Get-JkWindow -ProcessId $ProcessId
    for ($attempt = 0; $attempt -lt 8; $attempt++) {
      if ($gameWindow -eq [IntPtr]::Zero) { break }
      [void][JkRetail]::ShowWindow($gameWindow, 9)
      [void][JkRetail]::SetForegroundWindow($gameWindow)
      Start-Sleep -Milliseconds 120
      if ([JkRetail]::ForegroundTitle() -eq 'Jedi Knight') { break }
    }
    Send-JkKeyDown -Name $Key
    Log ("engine capture: holding {0} for the whole window (foreground {1})" -f $Key, [JkRetail]::ForegroundTitle())
  }
  try {
    while ($true) {
      $clock = Read-JkGameMs -ProcessId $ProcessId
      if ($clock -lt 0) { throw 'engine capture: the game clock stopped reading' }
      if ($clock -eq $lastClock) {
        if ($watch.ElapsedMilliseconds -gt $deadlineMs) {
          throw ("engine capture: the game clock stayed at {0} ms for {1} ms of wall time" -f $clock, $watch.ElapsedMilliseconds)
        }
        Start-Sleep -Milliseconds 4
        continue
      }
      if ($sampleCount -gt 0 -and ($clock - $clock0) -ge $Milliseconds) { break }
      $sampleCount++
      $lastClock = $clock

      $thing = [JkRetail]::ReadBytes($ProcessId, $player, 0x1E0)
      if ($null -eq $thing) { throw "engine capture: the player thing block (0x008319A4) could not be read at $clock ms" }
      $px = [BitConverter]::ToSingle($thing, 0xB8)
      $py = [BitConverter]::ToSingle($thing, 0xBC)
      $pz = [BitConverter]::ToSingle($thing, 0xC0)
      $vx = [BitConverter]::ToSingle($thing, 0x1D4)
      $vy = [BitConverter]::ToSingle($thing, 0x1D8)
      $vz = [BitConverter]::ToSingle($thing, 0x1DC)
      $health = [BitConverter]::ToSingle($thing, 0x134)
      $sectorPtr = [BitConverter]::ToUInt32($thing, 0x5C)
      $invPtr = [BitConverter]::ToUInt32($thing, 0x1C0)
      if ($invPtr -eq 0) { throw "engine capture: the player inventory pointer is 0 at $clock ms" }
      $inventory = [JkRetail]::ReadBytes($ProcessId, $invPtr, 0x88 + 76 * 0x18)
      if ($null -eq $inventory) { throw "engine capture: the player inventory block could not be read at $clock ms" }
      $bin72 = [BitConverter]::ToSingle($inventory, 0x88 + 72 * 0x18)
      $bin73 = [BitConverter]::ToSingle($inventory, 0x88 + 73 * 0x18)
      $bin75 = [BitConverter]::ToSingle($inventory, 0x88 + 75 * 0x18)
      $rng = [JkRetail]::ReadU32($ProcessId, 0x0054F750)
      if ($rng -lt 0) { throw "engine capture: the AI random state could not be read at $clock ms" }
      $sector = -1
      if ($sectorPtr -ne 0) {
        $sectorRead = [JkRetail]::ReadU32($ProcessId, $sectorPtr)
        if ($sectorRead -ge 0) { $sector = [long]$sectorRead }
      }
      foreach ($value in @($px, $py, $pz, $vx, $vy, $vz, $bin72, $bin73, $bin75)) {
        if ([double]::IsNaN($value)) { throw "engine capture: a sampled float is NaN at $clock ms" }
      }
      if ($health -lt 0.0 -or $health -gt 1000.0) { throw "engine capture: implausible health $health at $clock ms" }
      foreach ($pair in @(@('bin72', $bin72), @('bin73', $bin73), @('bin75', $bin75))) {
        if ($pair[1] -lt -0.01 -or $pair[1] -gt 65535.0) {
          throw ("engine capture: implausible {0} {1} at {2} ms" -f $pair[0], $pair[1], $clock)
        }
      }
      if ($sector -lt -1 -or $sector -gt 1000000) { throw "engine capture: implausible sector $sector at $clock ms" }

      $rows.Add((New-JkEngineCaptureRow @{
              t_ms = [string]$clock; kind = 'player'; thing = [string]$player; template = 'walkplayer'
              x = (Format-JkEngineNumber $px); y = (Format-JkEngineNumber $py); z = (Format-JkEngineNumber $pz)
              sector = [string]$sector
              vx = (Format-JkEngineNumber $vx); vy = (Format-JkEngineNumber $vy); vz = (Format-JkEngineNumber $vz)
              bin72 = (Format-JkEngineNumber $bin72 3); bin73 = (Format-JkEngineNumber $bin73 3); bin75 = (Format-JkEngineNumber $bin75 3)
              rng = [string]$rng
            }))
      if ($null -eq $originMs) {
        $originMs = [long]$clock
        $originPose = ('x={0} y={1} z={2} sector={3}' -f (Format-JkEngineNumber $px), (Format-JkEngineNumber $py), (Format-JkEngineNumber $pz), $sector)
        foreach ($actor in @(Get-JkAiOriginRows -ProcessId $ProcessId -PlayerThing ([long]$player) -X $px -Y $py -Radius $Radius -OriginMs ([long]$clock))) {
          $rows.Add($actor)
          $actorCount++
        }
        if ($actorCount -eq 0) {
          Log ("engine capture: WARNING no live actor within {0} units of the origin; the fixture has no origin actor rows" -f $Radius)
        }
      }
      Log ("engine capture: {0} ms pos ({1:N3}, {2:N3}, {3:N3}) sector {4} vel ({5:N3}, {6:N3}, {7:N3}) bins {8:F3}/{9:F3}/{10:F3} rng {11}" -f `
          $clock, $px, $py, $pz, $sector, $vx, $vy, $vz, $bin72, $bin73, $bin75, $rng)
    }
  } finally {
    if ($Key) { Send-JkKeyUp -Name $Key }
  }
  if ($null -eq $originMs -or $sampleCount -eq 0) { throw 'engine capture: no sample was taken' }
  $comments = @(
    ('pose {0} (first sample)' -f $originPose),
    ('window {0} ms of engine clock, {1} sample(s), {2} origin actor(s)' -f $Milliseconds, $sampleCount, $actorCount),
    'addresses: clock=0x00836C14 player=0x008319A4 rng=0x0054F750 ai_instances=0x0088AFE0 ai_count=0x008B4BE0'
  )
  Write-JkEngineCaptureFile -Path $path -OriginMs $originMs -Name $Name -HeaderComments $comments -Rows $rows
  $errors = @(Test-JkEngineCaptureRows -Lines @(Get-Content -LiteralPath $path))
  if ($errors.Count -gt 0) { throw ('engine capture: wrote a malformed fixture: ' + ($errors -join '; ')) }
  Log ("engine capture: wrote {0} ({1} sample(s), {2} origin actor(s), origin {3} ms, final {4} ms)" -f $path, $sampleCount, $actorCount, $originMs, $lastClock)
  return $path
}

# Synthetic self-check for the engine-capture schema. No retail install, no launch: it exercises the
# same writer and validator the capture uses, then deliberately corrupts the fixture one way at a time
# and requires each corruption to be rejected. Returns the failure count (0 = pass).
function Invoke-JkEngineCaptureSelfCheck {
  $dir = Resolve-FromRepo 'target/parity/engine'
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $path = Join-Path $dir 'self-check.tsv'
  $origin = [long]12000
  $rows = @(
    (New-JkEngineCaptureRow @{ t_ms = '12000'; kind = 'player'; thing = '127621528'; template = 'walkplayer'; x = '1.4001'; y = '9.7001'; z = '6.3201'; sector = '60'; vx = '0.0000'; vy = '0.0000'; vz = '0.0000'; bin72 = '0.000'; bin73 = '0.000'; bin75 = '20.000'; rng = '123456789' }),
    (New-JkEngineCaptureRow @{ t_ms = '12000'; kind = 'origin'; thing = '42'; template = '42'; x = '5.9941'; y = '9.1052'; z = '5.3563'; ai_flags = '515'; fire_next_ms = '13500'; acquire_next_ms = '12400'; acquired_at_ms = '12000'; target = '127621528'; last_target = '127621528'; next_pass_ms = '12100'; detail = 'fire=primaryfire acquire=lookfortarget target=player' }),
    (New-JkEngineCaptureRow @{ t_ms = '12120'; kind = 'player'; thing = '127621528'; template = 'walkplayer'; x = '1.4001'; y = '9.7001'; z = '6.3201'; sector = '60'; vx = '0.0000'; vy = '0.0000'; vz = '0.0000'; bin72 = '0.000'; bin73 = '0.500'; bin75 = '20.000'; rng = '456789123' }),
    (New-JkEngineCaptureRow @{ t_ms = '12500'; kind = 'player'; thing = '127621528'; template = 'walkplayer'; x = '1.4001'; y = '9.7001'; z = '6.3201'; sector = '60'; vx = '0.0000'; vy = '0.0000'; vz = '0.0000'; bin72 = '0.000'; bin73 = '0.500'; bin75 = '20.000'; rng = '789123456' })
  )
  Write-JkEngineCaptureFile -Path $path -OriginMs $origin -Name 'self-check' -HeaderComments @('synthetic rows; no retail process was involved') -Rows $rows
  $lines = @(Get-Content -LiteralPath $path)
  function Assert-JkEngineAccepted([string]$Label) {
    $errors = @(Test-JkEngineCaptureRows -Lines $lines)
    if ($errors.Count -eq 0) {
      Write-Host ("PASS  self-check: {0} is accepted" -f $Label)
    } else {
      Write-Host ("FAIL  self-check: {0} was rejected: {1}" -f $Label, ($errors -join '; '))
      $script:selfCheckFailures++
    }
  }
  $script:selfCheckFailures = 0
  Assert-JkEngineAccepted 'a synthetic fixture'
  # Each case is a distinct malformation the schema must catch; one passing case fails the check.
  function Assert-JkEngineRejected([string]$Label, [string[]]$Bad) {
    $errors = @(Test-JkEngineCaptureRows -Lines $Bad)
    if ($errors.Count -gt 0) {
      Write-Host ("PASS  self-check: {0} is rejected ({1})" -f $Label, $errors[0])
    } else {
      Write-Host ("FAIL  self-check: {0} passed the schema" -f $Label)
      $script:selfCheckFailures++
    }
  }
  $columns = $script:JkEngineCaptureColumns
  # first data row: the header's line plus one (comments precede the header)
  $playerLine = [array]::IndexOf($lines, ($columns -join "`t")) + 1
  # 1. a missing column
  $bad = @($lines)
  $f = $bad[$playerLine] -split "`t"
  $bad[$playerLine] = (@($f | Select-Object -First ($f.Count - 1)) -join "`t")
  Assert-JkEngineRejected 'a row with a missing column' $bad
  # 2. a non-numeric position
  $bad = @($lines)
  $f = $bad[$playerLine] -split "`t"
  $f[4] = '1.4x'
  $bad[$playerLine] = $f -join "`t"
  Assert-JkEngineRejected 'a non-numeric x' $bad
  # 3. an unknown row kind
  $bad = @($lines)
  $f = $bad[$playerLine] -split "`t"
  $f[1] = 'shot'
  $bad[$playerLine] = $f -join "`t"
  Assert-JkEngineRejected 'an unknown kind' $bad
  # 4. a clock that goes backwards
  $bad = @($lines)
  $f = $bad[$playerLine + 2] -split "`t"
  $f[0] = '11000'
  $bad[$playerLine + 2] = $f -join "`t"
  Assert-JkEngineRejected 'a backwards clock' $bad
  # 5. a missing origin clock
  $bad = @($lines | Where-Object { $_ -notmatch '^#\s*origin_ms\s' })
  Assert-JkEngineRejected 'a missing origin clock' $bad
  # 6. an origin row with no next-fire time
  $bad = @($lines)
  $f = $bad[$playerLine + 1] -split "`t"
  $f[[array]::IndexOf($columns, 'fire_next_ms')] = ''
  $bad[$playerLine + 1] = $f -join "`t"
  Assert-JkEngineRejected 'an origin row with no next-fire time' $bad
  # 7. a malformed header
  $bad = @($lines)
  $bad[$playerLine - 1] = ($columns -join "`t").Replace('fire_next_ms', 'next_fire_ms')
  Assert-JkEngineRejected 'a renamed header column' $bad
  Remove-Item -LiteralPath $path -ErrorAction SilentlyContinue

  # --- the origin-row builder (Issue #20) ---
  # The user-run capture died in this path with "Argument types do not match": `@()` over the
  # builder's `List[object]` throws on PowerShell 7.6, and no check reached the builder without
  # retail. `-ReadBytes` serves a synthetic memory image instead of the process, so the call the
  # capture makes -- `@(Get-JkAiOriginRows ...)` -- runs here.
  function Set-JkWord([byte[]]$Block, [int]$Offset, [uint32]$Value) {
    [Array]::Copy([BitConverter]::GetBytes($Value), 0, $Block, $Offset, 4)
  }
  function Set-JkFloat([byte[]]$Block, [int]$Offset, [single]$Value) {
    [Array]::Copy([BitConverter]::GetBytes($Value), 0, $Block, $Offset, 4)
  }
  $playerThing = [long]0x50000000
  $image = @{}
  $registry = New-Object byte[] 0x30
  $handlerName = [System.Text.Encoding]::ASCII.GetBytes('primaryfire')
  [Array]::Copy($handlerName, 0, $registry, 0, $handlerName.Length)
  Set-JkWord $registry 0x20 ([uint32]0x1111)
  $image[0x20000000] = $registry                                  # instinct registry block
  $image[0x0084DE50] = [BitConverter]::GetBytes([uint32]0x20000000) # registry pointer
  $image[0x0084DE54] = [BitConverter]::GetBytes([uint32]1)         # registry count
  $image[0x008B4BE0] = [BitConverter]::GetBytes([uint32]1)         # highest AI instance slot
  $slots = New-Object byte[] (2 * 0x29C)
  Set-JkWord $slots 0x00 ([uint32]0x30000000)      # slot 0: actor thing
  Set-JkWord $slots 0x04 ([uint32]0x40000000)      #          AI class
  Set-JkWord $slots 0x08 ([uint32]0x1004)          #          flag word
  Set-JkWord $slots 0x10 ([uint32]7000)            #          instinct 0's next run (fire)
  Set-JkWord $slots 0x18C ([uint32]1)              #          instinct count
  Set-JkWord $slots 0x190 ([uint32]5000)           #          next pass
  Set-JkWord $slots 0x1D0 ([uint32]0x50000000)     #          current target = the player
  Set-JkWord $slots 0x204 ([uint32]1234)           #          last acquisition time
  Set-JkWord $slots 0x29C ([uint32]0x30000100)     # slot 1: actor far outside the radius
  $image[0x0088AFE0] = $slots
  $class = New-Object byte[] 0x94C
  Set-JkWord $class 0xB8 ([uint32]0x1111)          # instinct 0's handler is `primaryfire`
  $image[0x40000000] = $class
  $near = New-Object byte[] 0x2B0
  Set-JkWord $near 0x04 ([uint32]42)               # template
  Set-JkWord $near 0x0C ([uint32]2)                # thing type 2 = AI actor
  Set-JkFloat $near 0x134 ([single]50.0)           # health
  Set-JkFloat $near 0xB8 ([single]5.0)
  Set-JkFloat $near 0xBC ([single]9.0)
  Set-JkFloat $near 0xC0 ([single]6.0)
  $image[0x30000000] = $near
  $far = New-Object byte[] 0x2B0
  Set-JkWord $far 0x0C ([uint32]2)
  Set-JkFloat $far 0x134 ([single]50.0)
  Set-JkFloat $far 0xB8 ([single]30.0)             # 29 units away, outside Radius 8
  Set-JkFloat $far 0xBC ([single]9.0)
  $image[0x30000100] = $far
  $readImage = {
    param([long]$Address, [int]$Count)
    Read-JkSyntheticMemory -Image $image -Address $Address -Count $Count
  }.GetNewClosure()
  function Assert-JkOrigin([string]$Label, [bool]$Ok, [string]$Detail = '') {
    if ($Ok) {
      Write-Host ("PASS  self-check: {0}" -f $Label)
    } else {
      Write-Host ("FAIL  self-check: {0} ({1})" -f $Label, $Detail)
      $script:selfCheckFailures++
    }
  }
  # The capture's own call shape: one memory pass over the synthetic image.
  try {
    $originRows = @(Get-JkAiOriginRows -ProcessId 0 -PlayerThing $playerThing -X ([single]1.0) -Y ([single]9.0) -Radius 8.0 -OriginMs $origin -ReadBytes $readImage)
  } catch {
    Write-Host ("FAIL  self-check: the origin builder threw: {0}" -f $_.Exception.Message)
    $script:selfCheckFailures++
    $originRows = @()
  }
  Assert-JkOrigin 'the origin builder returns one row for the one in-radius actor' ($originRows.Count -eq 1) ("count $($originRows.Count)")
  $field = @{}
  if ($originRows.Count -eq 1) {
    $values = @($originRows[0])
    for ($c = 0; $c -lt [math]::Min($columns.Count, $values.Count); $c++) { $field[$columns[$c]] = $values[$c] }
  }
  $expected = [ordered]@{
    t_ms = [string]$origin; kind = 'origin'; thing = '805306368'; template = '42'
    x = '5.0000'; y = '9.0000'; z = '6.0000'; ai_flags = '4100'
    fire_next_ms = '7000'; acquire_next_ms = '-1'; acquired_at_ms = '1234'
    target = '1342177280'; last_target = '0'; next_pass_ms = '5000'
    detail = 'fire=primaryfire acquire=none target=player'
  }
  foreach ($column in @($expected.Keys)) {
    Assert-JkOrigin ("the origin row's {0} is {1}" -f $column, $expected[$column]) ([string]$field[$column] -eq $expected[$column]) ("read '$($field[$column])'")
  }
  $builderPath = Join-Path $dir 'self-check-origin.tsv'
  $playerRow = New-JkEngineCaptureRow @{ t_ms = [string]$origin; kind = 'player'; thing = '127621528'; template = 'walkplayer'; x = '1.4001'; y = '9.7001'; z = '6.3201'; sector = '60'; vx = '0.0000'; vy = '0.0000'; vz = '0.0000'; bin72 = '0.000'; bin73 = '0.000'; bin75 = '20.000'; rng = '123456789' }
  $builderRows = @($playerRow, $originRows[0])
  Write-JkEngineCaptureFile -Path $builderPath -OriginMs $origin -Name 'self-check-origin' -HeaderComments @('rows built from a synthetic memory image; no retail process was involved') -Rows $builderRows
  $builderErrors = @(Test-JkEngineCaptureRows -Lines @(Get-Content -LiteralPath $builderPath))
  Assert-JkOrigin 'the builder rows pass the fixture schema' ($builderErrors.Count -eq 0) ($builderErrors -join '; ')
  Remove-Item -LiteralPath $builderPath -ErrorAction SilentlyContinue
  # A run with no AI slot must produce no origin rows, not an error.
  $noSlots = $image.Clone()
  $noSlots[0x008B4BE0] = [BitConverter]::GetBytes([uint32]0x100)
  $readNoSlots = {
    param([long]$Address, [int]$Count)
    Read-JkSyntheticMemory -Image $noSlots -Address $Address -Count $Count
  }.GetNewClosure()
  $emptyRows = @(Get-JkAiOriginRows -ProcessId 0 -PlayerThing $playerThing -X ([single]1.0) -Y ([single]9.0) -Radius 8.0 -OriginMs $origin -ReadBytes $readNoSlots)
  Assert-JkOrigin 'a run with no AI instance slots returns no origin rows' ($emptyRows.Count -eq 0) ("count $($emptyRows.Count)")

  if ($script:selfCheckFailures -gt 0) {
    Write-Host ("engine-capture self-check: {0} failure(s)" -f $script:selfCheckFailures)
  } else {
    Write-Host 'engine-capture self-check: 1 fixture accepted, 7 malformed fixtures rejected, origin rows built from a synthetic memory image'
  }
  return $script:selfCheckFailures
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
    # Capture instrumentation, before any level script runs: when the CD/MCI device is absent,
    # sithSoundMixer_Startup leaves the music-initialized flag (0x00835FD4) at 0 and every script
    # SetMusicVol is dropped, so the level's fade never reaches 0x0054A678. Writing the flag in the
    # launched game changes no shipping behavior; the process is closed after the capture.
    if ([JkRetail]::WriteU32($process.Id, 0x00835FD4, 1)) {
      Log 'music: forced the music-initialized flag (0x00835FD4=1) for the capture (no CD/MCI device)'
    } else {
      Log 'music: WARNING could not set 0x00835FD4; the script volume will stay at the mixer default'
    }

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
    # Capture instrumentation: another application can hold the foreground on this workstation, and
    # the click/ESC then goes to it (the intro never skips). Activate the game window first, the same
    # way the walk trace does before each held segment.
    $gameWindow = Get-JkWindow -ProcessId $process.Id
    for ($attempt = 0; $attempt -lt 8; $attempt++) {
      if ($gameWindow -eq [IntPtr]::Zero) { break }
      [void][JkRetail]::ShowWindow($gameWindow, 9)
      [void][JkRetail]::SetForegroundWindow($gameWindow)
      Start-Sleep -Milliseconds 150
      if ([JkRetail]::ForegroundTitle() -eq 'Jedi Knight') { break }
    }
    Send-JkKey 'ESC'
    Start-Sleep -Milliseconds 2000
    $bright = Wait-JkBrightScreen -TimeoutSec 3
    $escRetries = 0
    while (($null -eq $bright -or $bright -le 55.0) -and $escRetries -lt 8) {
      $escRetries++
      Log "intro still playing (window yavg=$bright); sending ESC again ($escRetries)"
      if ($gameWindow -ne [IntPtr]::Zero) {
        [void][JkRetail]::SetForegroundWindow($gameWindow)
        Start-Sleep -Milliseconds 120
      }
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

    if ($AudioRoute) {
      $routePath = Resolve-FromRepo $AudioRoute
      $routeOut = Resolve-FromRepo $AudioRouteOut
      New-Item -ItemType Directory -Force -Path (Split-Path -Parent $routeOut) | Out-Null
      $null = Invoke-JkAudioRoute -ProcessId $process.Id -RoutePath $routePath -OutPath $routeOut
      # `01_soundtrack.cog` is retail cog #39 on level 1; its variables record whether the script's
      # fade actually ran (`currVol`), which the mixer memory alone cannot prove.
      $null = Invoke-JkDevConsoleCommand -ProcessId $process.Id -Command 'cogstatus 39'
      Log "audio route mode: skipped the screenshot series/reference for '$name' (wrote $routeOut)"
      return
    }
    # Issue #13: the retail audio probe, before any console command or key so nothing pauses the
    # clock or perturbs what the mixer is doing.
    if ($AudioProbeMs -gt 0) {
      if ($null -ne $requestedPose) { Start-Sleep -Milliseconds 900 }
      Invoke-JkAudioProbe -ProcessId $process.Id -Milliseconds $AudioProbeMs -CadenceMs $AudioProbeCadenceMs
      Log "audio probe mode: skipped the screenshot series/reference for '$name'"
      return
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

    # Issue #20: the engine-clock route/encounter capture. Like the encounter trace it replaces the
    # screenshot series and runs before any console command or key, so nothing pauses the clock or
    # perturbs the encounter. Inside the window there is no dev-console traffic at all.
    if ($EngineCaptureMs -gt 0) {
      $engineDir = Resolve-FromRepo $EngineCaptureOut
      New-Item -ItemType Directory -Force -Path $engineDir | Out-Null
      $engineName = if ($EngineCaptureName) { $EngineCaptureName } else { $name }
      $enginePath = Invoke-JkEngineCapture -ProcessId $process.Id -Name $engineName -Milliseconds $EngineCaptureMs -Key $EngineCaptureKey -OutDir $engineDir
      Log "engine capture mode: skipped the screenshot series/reference for '$name' (wrote $enginePath)"
      return
    }

    # Issue #8: the continuous retail route replaces the screenshot series. The row must be the
    # level's own start — a warp row would make the spawn leg a different measurement.
    if ($RouteWalk) {
      if ($null -ne $requestedPose) { throw "route walk '$name' must be a level-start row (no pose), not a warp row" }
      $routeDir = Resolve-FromRepo $RouteOut
      New-Item -ItemType Directory -Force -Path $routeDir | Out-Null
      $routePath = Invoke-JkRouteTrace -ProcessId $process.Id -Plan (Resolve-FromRepo $RoutePlan) -OutDir $routeDir -Name $RouteName -UseKey $UseKey -UseHoldMs $ActionHoldMs -UseSettleMs $UseSettleMs
      Log "route trace mode: skipped the screenshot series/reference for '$name' (wrote $routePath)"
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
      # The trigger fires from the pose the walk stopped at, and the console read after it is the
      # evidence (`cogstatus 0` then reports the door's own state). This is a real binding press,
      # not an `activate` on the script's behalf.
      if ($UseAfterWalk) {
        Log "walk trace: pressing the use binding '$UseAfterWalk' after the walk, before the console read"
        Invoke-JkUseKey -ProcessId $process.Id -Key $UseAfterWalk -HoldMs $ActionHoldMs -SettleMs $UseSettleMs
      }
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
    $gameWindow = Get-JkWindow -ProcessId $ProcessId
    if ($gameWindow -ne [IntPtr]::Zero) {
      Log ("game window after load: " + [JkRetail]::WindowRectText($gameWindow) + $(if ($windowedMode) { ' (requested windowed; the title still took over the display)' } else { ' (fullscreen)' }))
    }
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

# `-EngineCaptureSelfCheck`: validate issue #20's row schema on synthetic data. Deliberately before
# the install lookup and the launch path: this mode never starts retail and is safe to run anywhere.
if ($EngineCaptureSelfCheck) {
    $selfCheckFailures = Invoke-JkEngineCaptureSelfCheck
    exit ([int]($selfCheckFailures -gt 0))
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

$windowedMode = [bool]$Windowed
Log ("render window: " + $(if ($windowedMode) { 'windowed config requested (-Windowed); the game still took over the display in the 2026-11-09 measurement' } else { 'fullscreen' }))
Invoke-JkStaging -Source $GamePath -Destination $WorkCopy -Windowed:$windowedMode
if ($Software -and $windowedMode) { Log 'note: -Windowed only changes the dgVoodoo path; -Software runs the 8-bit renderer' }

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
