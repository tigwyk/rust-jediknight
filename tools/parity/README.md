# Retail-vs-port parity captures

> **Agents: do not run the retail side** (`retail-capture.ps1`, or any gate flag that captures retail
> on demand). Only the user starts a retail capture. The decompiled engine in `decomp/` is the spec for
> mechanics (`docs/porting-roadmap.md`, Principle 1); the retail fixtures committed here are frozen
> regression data. The port-side scripts and the gates that read committed fixtures are fine to run.
> Everything below documents how the harness works for those deliberate, user-run sessions.

Screenshot the **real GOG game** and **this port** from the same, known camera poses, then put
them side by side. Built to answer "does the port draw what the 1997 engine draws?" — the first
use was level 1's opening, where the port's spawn view looked buried in a prop. That turned out to
be a frozen decor animation plus a spawn camera 0.055 units low, both fixed; the pair is now part
of a thresholded gate (`compare.ps1 -Gate`) rather than a one-off investigation.

Both sides are driven headlessly (no human input) and the pose is recorded, so two runs are
comparable even if the mouse moves between them.

## What it does

| Step | Script | What happens |
|---|---|---|
| 1 | `retail-capture.ps1` | Copies the retail install to a scratch folder, launches the **GOG `JK.EXE`** in `-devMode`, drives its menus with the mouse, reads the player pose from the game's own `coords` console command, `warp`s to each pose in `shots.tsv`, presses F12 (the game writes `JSHOT*.PCX/BMP`) and converts the frames to PNG. |
| 2 | `port-capture.ps1` | Runs `bevi-knight.exe` once per shot with `--start`, `--shot`, and (for numeric poses) `--shot-at` plus `--warp`: the camera at the recorded eye, the **player** at the recorded pose, as retail's `warp` leaves it. |
| 3 | `compare.ps1` | Pairs `retail/<name>.png` with `port/<name>.png`, writes a side-by-side, a contact sheet, and `report.md` with framing facts and a triage score (SSIM / mean absolute difference). |
| 4 | `compare.ps1 -Gate` | Scores every row of `thresholds.tsv` against its SSIM/MAD floors and exits non-zero if one is missing or failed. This is the regression check. |
| 5 | `menu-gate.ps1` / `film-gate.ps1` | The two screens that are *not* the world: the retail main menu against the port's render of it, and retail's opening-film crawl against the port's `BK_FILM_HOLD` frame. `film-gate.ps1` also holds a frame whose content changes every frame (210) and scores it against **ffmpeg's** Smacker decode of that index, which pins the decode order and palette where the static crawl cannot. Both capture their own sides on demand and score with ffmpeg SSIM. |

The retail side never touches your real installation: it works on a copy under
`target/parity/retailcopy/`. By default it stages dgVoodoo2 and runs the game's 16-bit 3D path;
`-Software` forces the 8-bit software path. With no working 3D path the world renders black and
only the HUD appears — see Troubleshooting.

`-Windowed` writes dgVoodoo's windowed config (`FullScreenMode=false`, borderless, `CaptureMouse=true`,
640x480). **It does not make a retail capture non-invasive on this machine**: the window becomes
640x480 borderless, but the user still saw the game take over the whole display on every run
(2026-11-09), so retail runs remain invasive and are opt-in. Do not call a capture windowed on the
strength of the config alone.

## Requirements

- Windows 10/11 x64, PowerShell 7 (`pwsh`), `ffmpeg` on `PATH`.
- A retail **GOG** install of *Star Wars Jedi Knight: Dark Forces II* (the port's normal requirement).
- The port built: `cargo build --release` (the harness will build it if missing).
- **Keep hands off the mouse and keyboard while a retail run is in progress** — the game captures the
  mouse, and synthetic input is how the harness drives it.

## Quick start

From the repo root:

```powershell
# 1. capture the retail reference frames
pwsh -File tools/parity/retail-capture.ps1

# 2. capture the same poses in the port
pwsh -File tools/parity/port-capture.ps1

# 3. pair, score and lay them out
pwsh -File tools/parity/compare.ps1

# 4. the regression check: pass/fail against the per-shot thresholds
pwsh -File tools/parity/compare.ps1 -Gate

# 5. the two full-screen pairs (UI and film; each captures its own sides on demand)
pwsh -File tools/parity/menu-gate.ps1
pwsh -File tools/parity/film-gate.ps1
```

Outputs:

```
target/parity/retail/<shot>.png       retail frame, cropped to the world box (the 3D view inside the raw frame)
target/parity/retail/<shot>-raw.png   the full engine frame (display mode + letterbox + HUD corners)
target/parity/retail/<shot>-series-*.png  8 x 1 s frames after level load (camera/pose evidence)
target/parity/retail/<shot>.json      pose read from `coords`, world crop, source hashes, series + warp verdicts
target/parity/retail/retail.log
target/parity/display-pre.json        desktop display modes saved before the run (recovery baseline)
target/parity/port/<shot>.png         port frame, rendered at the retail view size (`BK_WINDOW` from the sidecar)
target/parity/compare/<shot>-side.png retail | port, side by side
target/parity/compare/contact.png     every pair, worst first
target/parity/compare/report.md       framing facts per pair + triage scores (worst first)
```

If a run is ever interrupted (Ctrl+C, crash, killed process), the game can leave the primary monitor
in its 640x480 mode. Put it back with:

```powershell
pwsh -File tools/parity/restore-display.ps1                   # last run's saved desktop mode
pwsh -File tools/parity/restore-display.ps1 -RegistryDefaults # Windows' saved default mode
```

`port-capture.ps1` also checks for that stuck mode before it captures and heals it, so the port does
not open on a 640x480 desktop.

## The gate: `compare.ps1 -Gate`

`tools/parity/thresholds.tsv` gives every gated shot a minimum SSIM and a maximum mean absolute
difference; `compare.ps1 -Gate` scores the captured pairs against it, prints `PASS`/`FAIL` per shot and
exits non-zero if a thresholded shot is missing or outside its bounds. A pair with no threshold row is
reported as `UNGRADED` (so a new shot does not break the gate before its floor is chosen).

```text
name           min_ssim  max_mean_abs_diff
spawn          0.55      28.0
spawn_pose     0.55      28.0
spawn_yaw30    0.55      28.0
route_cantina  0.65      30.0
route_lower    0.65      30.0
route_street   0.65      30.0
```

The floors are tolerances, not equality: retail (1997 renderer) and the port (Bevy) differ, and the
port's HUD is inside its frame while the retail world crop has none. The spawn pairs sit at 0.62–0.65
and the gated route pairs at 0.72–0.74; the floors sit under those, and the gate is known to fail on a
real regression: the pre-fix spawn camera (0.055 units low) scores 0.2284, the fixed one 0.62–0.65.

`route_catwalk` and `route_door` are captured but deliberately **ungraded**, and the reason is now
measured rather than assumed. The port warps the player to the row's pose (`--warp`), the way retail's
`warp` leaves its body, so scripts and AI do see a body there. Retail's reference in those two rooms
is **red from damage tint**, though: the body is taking damage (level 1's door: the settled raw HUD
reads 81 health / 0 shields; catwalk: 100/100 → 81/11), and the engine's damage path adds a red palette tint
(`sithActor_DamageActor` calls `sithPlayer_AddDynamicTint(damage * 0.04, 0, 0)`; it decays 0.4 per
second and is applied as `palette += round(palette * tint * 0.5)`). Both halves are ported now: the
state (adds with the engine's clamps, the 0.4/s decay, the damage flash — unit-tested) and the draw
(a fullscreen palette pass on the world camera that scales the finished picture's bytes by
`1 + tint/2`; a 1.0 red tint measures ×1.52). The rows stay `UNGRADED` at 0.45/0.42 for a different
reason: retail's own 1 s series turns red at ~5–6 s and its settled reference at ~10 s is saturated
(r/g 2.0), while two uninstrumented 10 s port captures read **83 health / 0 shields** against retail's
**81 / 0** and draw red 46 → 67 — the settled state is within 2 health, the picture is not: retail's
reference is a
deeper red and the doorway framing dominates that pair. At the catwalk the body now **stands on the
catwalk model** (a solid thing, whose top the loader registers and the idle attachment holds — the
engine's `FindFloor`/`AttachThingToSurface` behaviour), which moved the pair 0.4493 → **0.4603**;
the rest is retail's deeper red (Phase 1 step 3/5). Those are gaps, not renderer tolerance (see
`docs/gaps.md` §5.9).

Round 419 re-captured both rows and confirmed the static gate is the wrong instrument: the catwalk's
tint now matches (port r/g 1.469, retail 1.418) but its fighter is at a different animation frame; the
door's reference is redder than the port's even at 83 vs 81 health, and a ±60 px alignment search finds
its best offset at x=60 (catwalk: -2,+2), so the reference carries a camera pose/shake the row does not.
Neither is a renderer delta. `BK_TINT=0.929,0,0` matched the door's red mean 73.5/73.0 but left green/blue
47.5/55.8 against 31.6/33.7, and `Tonemapping::None` moved the static rows by <0.002 SSIM and was
reverted. Keep the rows ungraded until a state-matched retail capture exists.

The numbers are tied to the capture policy above (600 frames, window from the retail sidecar, default
16-bit retail renderer) — changing framing or frame count invalidates them.

The gate needs a retail install and captured references on the machine, so it is a developer check, not
a CI job; the port's own 143 tests and 61 gates remain the CI-able surface.

## The shot list: `tools/parity/shots.tsv`

Tab-separated, one shot per line. A pose of `-` means **the level's own spawn** (retail reads the
pose with `coords`; the port just uses `--start`). Numeric rows are **camera eyes** in world units,
angles in the retail `coords` convention (`PYR`, degrees):

```text
name	level	x	y	z	pitch	yaw
spawn	01narshadda	-	-	-	-	-
spawn_pose	01narshadda	-10.443433	-5.455094	-1.092886	0	-59.994
spawn_yaw30	01narshadda	-10.443433	-5.455094	-1.092886	0	-30
route_cantina	01narshadda	-2.200043	-2.900013	-1.142886	0	-74.999908
route_lower	01narshadda	4.074969	-6.875029	0.257114	0	15.000092
route_street	01narshadda	5.474939	-1.899975	0.657119	0	-134.988342
route_catwalk	01narshadda	7.749950	-1.175000	2.857114	0	0
route_door	01narshadda	1.400084	9.700065	6.357114	0	105.000092
```

The `route_*` rows are the level's own `walkplayer` placements — the multiplayer start points (things
167, 169, 170, 171 and 174 of `01narshadda`) — so each is an authored, standable camera covering the
cantina, the way up and the upper door. Retail does not draw those marker things in single player; the
port used to (a Kyle model filled the camera at each), and now skips them the same way. Both sides put
a body at those poses: retail through `warp`, the port through `--warp` (the sector the port derives
from the position matches the sector retail's `coords` reports on all eight rows: 339 ×3, 254, 137,
156, 175, 60). The two upper rows are the ungraded ones described above.

`spawn` (no warp) captures the level's own start; retail reads the pose with `coords` and the sidecar
marks it `first_frame_is_player_camera` (its frame is pixel-identical to the warp-verified pair), so it
is a valid pair — it scores ~0.63 now, the same as `spawn_pose` and `spawn_yaw30`, after the spawn
camera was fixed (0.23 before). Numeric rows are the ones whose pose `coords` proves through `warp`;
use them when a pair has to be trusted independently of the level's start.

- `z` is the **eye** height — the value the port's `--shot-at` takes. Retail's `warp` moves the player
  body, and the engine rides the camera at `thing + 0x168` (the template's `eyeoffset=(0,0,0.037)`),
  so the retail side warps to `z - 0.037`. It reads `coords` back and marks the row
  `warp_verified` when the position (offset restored) and yaw agree. The port's `--warp` takes the
  same eye and puts its body one template eye offset below it, so both sides' bodies are at the same
  recorded pose.
- `pitch` is the port camera's pitch in degrees, taken by `--shot-at` (a fifth number, 0 when omitted).
  Retail's `warp` cannot set pitch, so it keeps the level's own view angles (level 1 stays at `coords`
  PYR pitch 1.04, which the sidecar records) — and that PYR number is not the rendered camera pitch
  (a 0.25° port pitch already drops the spawn pair 0.63 → 0.59), so rows request 0. `--warp` is yaw
  only, like retail's.
- `warp` exists because the staged copy's `JK.EXE` is patched — see "How the retail side is driven".

## How the retail side is driven (for maintainers)

Everything below was verified against the GOG build (the harness works on a scratch copy under `target/`).

- `JK.EXE -devMode` opens a **dev console window** ("Jedi Console", a `#32770` dialog) whose list
  box logs level parses and command output; the harness reads it to know when a level has loaded
  (`parse section things`).
- The game's menus are **mouse-only** for synthetic input. The 640x480 game view has screen
  coordinates equal to game coordinates; the two clicks needed are `Single Player (320,195)` and
  `New Game (304,115)`. The intro film is skipped with `ESC`.
- **A menu click commits to that screen.** Once an option is clicked you are inside that submenu and
  the only way out is its own `Ok`/`Cancel`; clicking a main-menu item from inside a submenu does
  nothing. So driving menus must descend one level at a time and commit/leave before the next step —
  `retail-capture.ps1` always goes Players → `Ok` → Single Player → New Game (or Load → `Ok`), never
  sideways.
- In gameplay, synthetic keyboard works. `T` opens the in-game console, and typed commands must be
  sent with each key **held ≥300 ms** (the engine drops faster keystrokes). `coords` prints
  `Pos: (x,y,z) PYR: (p,y,r) Sector: n`; `warp x y z p y r` teleports.
- `F12` is the game's own screenshot key (control function 40, bound in `Controls\set0000.ctl`);
  the engine writes `JSHOT###.PCX` (8-bit software path) or `.BMP` (16-bit/3D path), which the
  harness converts with `ffmpeg`.
- **Gated dev-console verbs are unlocked in the staged copy.** `warp`, `cogstatus`, `aistatus`,
  `slowmo`, … register only when bit 0x100 of the debug-flags word at `0x8EE66C` is set, and nothing
  sets it before `sithCommand_Startup` registers the commands. `Set-JkDevCommandsEnabled`
  (`retail-lib.ps1`) verifies the unique 9-byte site (`test al,1` + `jz`, file offset `0xED0A4`) and
  NOPs the six jump bytes, then reads the file back to verify. Only `target/parity/retailcopy/` is
  ever patched; the real install is untouched.
- **3D acceleration runs through a staged dgVoodoo2 wrapper, 16-bit path.** The 1997 build's Direct3D
  calls the system `D3DIM.DLL`, which crashes on this Windows 11 machine entering a level, so the
  software renderer used to be forced (`b3DAccel = 0`, 8-bit `.PCX` with palette dither). The harness
  now stages dgVoodoo2's `DDraw.dll`/`D3DImm.dll` (D3D11 translation) into the scratch copy by default
  and sets `b3DAccel = 1`, which renders the game's **16-bit** path without dither: F12 then writes
  `JSHOT###.BMP` (24-bit). The wrapper is downloaded once into `target/parity/wrapper/` (pinned
  version/sha256) and only the scratch copy is modified. `retail-capture.ps1 -Software` keeps the old
  8-bit path available. The display-mode registry keys live in the game's virtualized key
  (`HKCU\Software\Classes\VirtualStore\MACHINE\...\JediKnight\v1.0`); `HKLM` is never touched.
- **The game is closed through its own shutdown, not killed.** `retail-capture.ps1` sends `WM_CLOSE`
  to the dev-console dialog (whose handler runs `Main_Shutdown` and exits, both in the menus and in
  gameplay — the game window itself only honours `WM_CLOSE` in the menus). The engine then restores
  the desktop display mode. As a safety net the harness saves the pre-run modes to
  `target/parity/display-pre.json` and puts them back if the desktop no longer matches (e.g. after a
  force-kill); `restore-display.ps1` does the same manually. Killing the process skips the engine's
  DirectDraw shutdown, which is what used to leave the monitor at 640x480.

## Movement traces (Phase 0.5 trace v0)

`coords` polling plus a replay is how "the same inputs produce the same positions" becomes
measurable (Phase 1 step 3's exit criterion). Three scripts and two fixtures:

- `tools/parity/walk-shots.tsv` - the legs: `name level x y z pitch yaw`, the eye convention of
  `shots.tsv`. A leg is "start the body at this pose, hold forward for a while".
- `retail-capture.ps1 -Shots tools/parity/walk-shots.tsv -Only <name> -WalkTraceMs 2400` - the retail
  half. It warps the body to the leg, then holds the UP arrow (the engine's forward binding) and reads
  `coords` between holds; output `target/parity/trace/<name>-walk.tsv`.
- Add `-UseAfterWalk <key>` to fire the player's own action binding after the walk and before the
  post-walk console read (`SPACE` is the profile's ACTIVATE binding; `MOUSEMIDDLE` is its mouse
  alternative; `-UseKey` changes the route plan's `use` key too). `walk_exitdoor` uses it at the
  authored placement 174 with the intent that the walk stops at the shut door and the hold opens it
  through `00_door.cog`, with `-ConsoleCommands 'cogstatus 0'` recording `doorstatus`/`movestatus`.
  That intent is **unverified in retail**: every run so far recorded `doorstatus 0`/`movestatus 0`
  and no crossing (`docs/gaps.md`), so no retail trace with the door open exists. Issue #15 no longer
  needs one — the use action, the message it sends and the sighted condition are read from `decomp/`,
  and the replay's exit segment is the port's own deterministic route.
- `port-trace.ps1 [-Only <name>]` - the port half: `--warp` to the same pose, `BK_WALK_FORWARD=1`,
  one line per 1/60 s frame, output `target/parity/trace/<name>.tsv`.
- `trace-compare.ps1 -Name <name> [-Epsilon <units>]` - interpolates the port path to the retail sample
  times and prints per-sample `d_xy`/`d_z` plus the sector sequence; without `-Epsilon` it reports, with
  it the exit code is the verdict.

The retail clock is **held-key time, not wall time**. Reading `coords` goes through the dev-console
dialog, and that dialog pauses the game - a "hold W and poll" walk moves only in the gaps between
polls, which is why the trace is built from hold segments: activate the game window (verify it is
foreground), hold the key for exactly `HoldMs`, release, then read the pose while the game is frozen.
`held_ms` is the sum of those segments - the *injected* hold, which is **not** game time: the engine
advances its clock per frame (`sithTime_Advance`, clamped to [10, 500] ms) and retail renders a few
frames a second here, so a 600 ms hold is 2-4 frames. Since 2026-11-01 the capture also writes
`game_ms`, the engine's own clock (`DAT_00836c14`, read with `ReadProcessMemory`; JK.EXE has a fixed
base) since the start pose. It is **not** the time axis: the first re-capture (round 294) measured
17 019 ms of engine clock for a 3 000 ms hold, because the clock keeps running through the console
dialog and the foreground waits. `held_ms` stays the axis (the key is down for exactly that span), and
`trace-compare.ps1` prints the engine clock beside it as drift. Replay the port for the walk's
`held_ms` (`port-trace.ps1`'s length is `HoldMs x Segments`); past its end the compare clamps to the
port's final frame. The port is deterministic by construction: capture-mode
`--shot` pins `dt` at 1/60 s and `BK_WALK_FORWARD` replaces the keyboard, so a rerun is identical.

Formats (tab separated, `#` comments allowed):

```text
<name>-walk.tsv    held_ms  x  y  z  sector  yaw  [game_ms]   (retail; one row per hold segment)
<name>.tsv         tick  t_s  x  y  z  sector  vy  health  shields   (port; one row per frame)
```

First measured leg (`walk_cantina`, 2.4 s of held forward, 2026-10-28):

- start poses agree exactly (0.000 at `held_ms` 0) - the eye/warp convention matches retail's;
- **z is 0.055 low at every moving sample** - the port's walking body settles to `floor + radius`
  while retail keeps the authored pose (`floor + model height`). This is the idle-only attachment
  ceiling in the deviation ledger, now measured against retail instead of asserted;
- **x/y diverges with distance travelled**: retail covers 2.17 units in 2.4 s (0.9 u/s) where the
  port covers 1.06 (0.44 u/s), and retail enters sector 103 while the port is still in 254. The
  port's derived speed (`maxthrust / surfdrag` = 0.667 u/s) is under retail's, and its effective
  acceleration is slower still; `walkplayer maxvel=1.0` is the likely law. That is the next movement
  fix, not a harness problem. Retail's own samples are a little irregular (`coords` latency and the
  refocus dance); the comparison reports per-sample deltas and marks samples where retail is stopped.

Second leg (`walk_street45`, the flat straight run): retail 2.20 units over 2.4 s held (0.9 u/s) and
the port 1.59; the port crosses the same first sectors but **steps off a ledge at x 3.27 where retail
is stopped by furniture**, so the leg measures the collision difference between the two (the port has
no thing-on-thing collision) as well as the movement rule. Two harness facts learned here: the trace
must not leave the movement key down between segments (the window re-activation sleeps while the engine
runs, which silently moved the body outside the counted segments and made the first traces read
1.5 u/s), and the first segment of a trace can include a few frames of deceleration coast after the
release - the later segments are the steady ones.
Dense per-tick tracing (the fallback the roadmap's Phase 0.5 step 4 names) is not needed yet: segment
density is enough for the first comparison, and the Frida `sithThing_SetPositionAndOrient` hook is the
plan if a finer trace is required.
### Cog state trace at a level's spawn (Phase 1 step 4 baseline)

`retail-capture.ps1` accepts a comma-joined command list where `wait:<ms>` pauses: that is how one
capture shows a cog's state before *and* after time passes, the shape Phase 1 step 4 needs at a
trigger. Level 1's door cog (`cogstatus 0`) reads **identically at t≈0 and t≈3 s** - `doorstatus 0`,
`movestatus 0`, `opendoors 0`, `closedoors 0` - so the door is idle at spawn and no timer fires before
a trigger. That snapshot is the fixture ([cogstatus0-t3-01narshadda.txt](cogstatus0-t3-01narshadda.txt));
the same capture machinery will record the state *after* the player walks through the trigger.
## Movement trace gate (Phase 1 step 3)

`trace-gate.ps1` records the port's walk traces and checks them against the retail fixtures
(`walk-shots.tsv` legs). It fails when a leg's displacement leaves the harness's noise band (port
travel between 75% and 125% of retail's) or when more than 2 of its 600 ms sector samples disagree.
Both numbers are documented in the script header with their reasons: retail's trace clock is held-key
game time (its first segment carries deceleration coast, so its average runs high) and the samples are
600 ms apart against 60 Hz traces.

```powershell
powershell -NoProfile -File tools/parity/trace-gate.ps1
PASS  walk_street45  port  2.21 / retail  2.20 units (  100%)  sectors off 0
PASS  walk_street90  port  1.84 / retail  2.21 units (   83%)  sectors off 2
PASS  walk_cantina   port  1.85 / retail  2.59 units (   71%)  sectors off 1
PASS  walk_door0     port  0.13 / retail  0.07 units (stop-pose)  sectors off 0
trace gate: 4 leg(s), 0 failure(s)
```

The table above is the gate's first four legs as it shipped. It now runs **15 legs** with **one tracked failure** -
`walk5_spawn`, the floor-search-scope deviation on the roadmap's ledger; `docs/gates.txt` carries the current numbers
each round rather than this snapshot.

## Scripted route replay (issue #8)

The shared level-1 oracle for the movement, telemetry and integration slices. `--replay` drives the
port from a committed input script at the fixed 1/60 s capture step, and `--route-trace` writes the
comparable trace.

```powershell
cargo build --release
pwsh -NoProfile -File tools/parity/route-gate.ps1            # determinism + committed-fixture gate
pwsh -NoProfile -File tools/parity/route-gate.ps1 -Perturb   # ... and prove a perturbed trace fails
```

Files:

| Path | What it is |
|---|---|
| `tools/parity/routes/level1-route.tsv` | The route script. `warp <name> x y z yaw` rows are the level's own retail-recorded route placements (`shots.tsv`'s `route_*` rows and `walk-shots.tsv`'s legs); `input <ticks> forward strafe` holds that intent. A tick is 1/60 s. `wait <ticks>` stands still (the door leaf needs game time to finish opening before the next hold); `seed <vx> <vy>` starts a leg at the speed the retail capture already had at its pose, and `use` presses the player's use key. `wait` takes ticks like an input; `seed`/`use` take none, so the 600 ms sample grid never moves. There is no `switch` row: retail's `cogstatus 11` reads `doorstatus 0` with that door already open, so the synthetic `activate` it used to send made the port measure a scenario retail never performed. |
| `tools/parity/routetrace/level1-route-retail.tsv` | Retail-derived reference, one row every 600 ms (36 ticks). Provenance is in the file header. |
| `tools/parity/routetrace/level1-route-port.tsv` | The port's committed oracle, produced by the replay. |
| `tools/parity/route-gate.ps1` | The gate: replay twice (identical bytes), check the exit's own events (use names things 4 and 47, `door0_opened` latches after the use, the body crosses the leaf, `goal_bin100` = 3, `level_end` = 1), assert the trace spans the whole plan (sample count and final tick) and carries the audio columns with no `-`, compare to the committed port fixture, reject a trace with one position moved 0.25 units, match the retail reference within the epsilon **the fixture states** (`# epsilon <units>`; `-Epsilon` overrides it) over the rows **before** the port-only `exit_door` warp, check the mechanic provenance below, and run `audio-gate.ps1` and `encounter-gate.ps1` as prerequisites (`-SkipPrereqs` for iteration). |
| `tools/parity/routes/level1-route-provenance.tsv` | Issue #14 AC2: every mechanic the route exercises names its engine function, decomp address, port symbol and source. A `deviated` row names its deviation-ledger entry in `docs/porting-roadmap.md`; the gate and the repo test both verify it. |

The trace format is tab-separated, `#` comments allowed, one header row:

```text
tick  t_s  x  y  z  sector  vel_x  vel_y  vel_z  health  shields
      door0_status  door0_movestatus  door0_opendoors  door0_closedoors
      switch_status  switch_movestatus  switch_opendoors  switch_closedoors
      door0_opened  goal_bin100  level_end
      music  music_end  music_loop  music_vol  ambient  ambient_vol  sounds
```

- `tick` counts 1/60 s steps; `t_s = tick / 60`. Rows are written at ticks `0, 36, 72, ...`
  (600 ms), the cadence a retail `coords` poll segments a walk into.
- `x/y/z` are the player **body** position — the value retail's `coords` prints (the camera rides one
  `eyeOffset` above it, `walkplayer`'s 0.037) — so a retail capture and this trace name the same point.
- `sector` is the tracked sector the body is in.
- `vel_x/vel_y/vel_z` are the position derivative over the 600 ms sample window, in level units per
  second — the same quantity as the retail fixture's columns (a body pinned against a wall reads zero).
- `health/shields` come from the player thing and `rt.player_shields`; the retail capture reads
  them from the player thing (`thing+0x134`) and inventory bin 60. The retail walk trace records
  both bars per hold segment, so the committed reference has real values here.
- `door0_*` and `switch_*` are placed cogs 0 (`00_door.cog`) and 11 (`00_doorswitch.cog`), four
  symbols each (`doorstatus`, `movestatus`, and the script's own open/close variables), read with
  `cogstatus` once per leg end in the retail capture (plus after every `warp`; the dev-console read
  is the fragile part of a long capture, so the rows between reads carry the last leg-end value) and
  from the running VM on every replay row. Level 1 has both cogs, so
  every field is present; the gate rejects any `-` (the comparator would skip it and measure the
  position columns only).
- `door0_opened`, `goal_bin100` and `level_end` are **events**, and the rule is different from the
  other columns: they are graded **only at the fixture's last row**, as identities, not measurements.
  `door0_opened` latches 1 once the door's own script reports nonzero `doorstatus`/`movestatus`
  (`00_door.cog` re-closes it two seconds after `arrived`, so a sample may miss the open frame);
  `goal_bin100` is inventory bin 100, 1 at the level start and 3 after `01_endlevel.cog`'s `user0:`
  completes goal 0; `level_end` is `sithMain_SetEndLevel`'s never-cleared latch (`0x4c4bf0`), i.e. the
  level's own `jkEndLevel` ran — a capture keeps the world it booted, so the app's own transition flag
  is cleared and is not what the trace reads. A 600 ms retail hold advances a variable number of
  engine frames while the replay advances exactly 36, so the row either side latches on is not
  comparable; the final state is. A disagreement on an intermediate row is ignored, a missing cell or
  a different final value fails. The gate also proves the trigger is real: the plan must press the
  player's use binding (never a synthetic `activate`), the replay log must name things 4 and 47, and
  `door0_opened` must be 0 on every row strictly before the first use tick; the use-tick row is
  sampled before the key press and must also read 0.
- `music`/`music_end`/`music_loop`, `music_vol`, `ambient`/`ambient_vol` and `sounds` are the
  mixer's state at the same sample — the columns `--audio-trace` writes, carried by the one route
  trace so the route's audio state is measured at the same points as its pose (issue #14).
  `music`/`music_end`/`music_loop` are `PlaySong`'s track range and loop flag, `music_vol` is
  `SetMusicVol`'s value, `ambient` is the loudest audible loop with its channel volume, and
  `sounds` lists every one-shot played since the previous sample as `wav@gain` (the row is
  sampled before the tick's own step, the point retail's `coords` poll reads, so a sound played in
  the sampled tick falls in the next window). A **silent** ambience or window is an empty cell, the
  mixer's no-song state is `0 0 0`, and no audio cell is `-`.
  `--route-trace` and `--audio-trace` are rejected together because both drain the one-shot log.
- **The comparator's coverage is the expected fixture's.** Its header names the columns it measured
  and its rows are the span it covers; a column it omits and the trace's rows past its last tick are
  not graded (they are reported). That is what lets the frozen retail capture — 19 columns, rows
  0-684, taken before the exit segment existed — stay a regression check on the route prefix without a
  new capture and without widening anything.

A **leg boundary is a `warp`**, not a walk: the retail reference is four separate per-leg captures
from the level's own authored placements, so the route is the same four walks. Issue #10 fixed what
each leg exposed — the spawn seat no longer lifts the body onto the cantina's `00table`, the walk
rate is the measured retail 1.2 u/s (2.4 run), a `warp` attaches the
body at `GetThingHeight` (0.120114, where retail's `coords` holds z), and a low solid model is a step
the walker climbs — so the legs hold retail's sectors and vertical travel.

The reference's **bars and cogs come from one continuous retail run**, not four independent leg
captures: `retail-capture.ps1 -Shots tools/parity/shots.tsv -Only spawn -RouteWalk` reads this same
route script and runs every leg in one game session (a `warp` between legs, a 600 ms forward hold per
segment), sampling the bars from engine memory on every row and the cogs once per leg end. Four
per-leg runs each started from a fresh level load with their own pre-warp damage, so their bars could
not be compared with the port's continuous replay. **Both sides capture with the AI off**: the
retail walk sets the `-devMode` toggles `disableai on` and `noaishots on` (explicitly, and again
after every warp, since they toggle), and the port's replay passes `--no-ai` (a capture switch
that requires `--replay`). The route measures movement and cog state, so the cantina ambush is not
part of it: it used to kill retail's player (100/100 to 0/0 between ticks 180 and 252) while the
port survived, which made every later bar a guaranteed mismatch. Combat parity is the door
encounter's job, and ordinary play keeps the AI.

**The exit segment (issue #15)** appends the level's own way to 8t88 to those four legs. It starts at
tick 684 — the frozen retail fixture's own last sample — with a port-only warp, so the gate grades
that fixture only over the rows before the warp and no measured row changes:

| leg | pose | what it measures |
|---|---|---|
| `exit_door` | the port's stop pose at the door, derived from the level geometry: eye `(1.28, 9.62, 6.357114)`, sector 60, yaw 105 (placement 174's committed eye is `(1.400084, 9.700065, 6.357114)`, 0.206 from the leaf and outside the reach, and no walktrace of the exit door exists) | The door leg's stop pose leaves the linked leaf 0.5 beyond the shut portal, outside the 0.165 use sweep (`sithPlayerActions_Activate`, `0x4fc0a0`), so a walk from it never activates the door. The warp starts at the derived stop pose, 0.086 from the leaf at a 0.81 facing dot. `use` presses the player's ACTIVATE binding and messages the linked leaf (thing 4 at `(1.200002, 9.650073, 6.35)`) with `activated`; `00_door.cog` reopens the adjoins and runs `OpenDoors`. `wait 72` lets the leaf reach frame 1, `yaw 0` turns the body to the doorway, and `input 108` **walks through the open leaf** — the trace's y passes 9.650073 (sector 60 spans both sides, so the sector column does not change). `door0_opened` reads 0 at and before the use tick and 1 after it. |
| `exit_upper` | placement 175 (body `-2.519, 11.001, 6.72`) | the corridor/ramp side of the upper route. |
| `exit_low` | placement 176 (body `-6.15, 9.65, 4.32`) | the low floor east of the 385 ramp. |
| `exit_tunnel` | tunnel mouth `(-4.05, 12.0, 4.32)`, sector 407 | `use` on the one-way door (thing 47 at `-4.05, 13.95, 4.375`; `00_1waydoor.cog` opens for a body inside 407), then the hold crosses 407 -> 223 -> 229 -> 215 and the renderer sights 8t88 (`sithRender_RenderThing`, `0x4c8070`, sends `sighted` to `01_goal0.cog`). |

The door is **used and crossed**: the warp puts the body inside the use sweep, and the hold after the
use walks it through the opened leaf instead of warping past it. `01_endlevel.cog`'s `user0` sets goal
bin 100 = 3, sleeps 3 s and calls `jkEndLevel(1)`, so the plan's trailing `wait 240` outlasts that
sleep and the last sample carries `level_end` 1.

The per-sample position tolerance is the **retail harness's**, stated in
`level1-route-retail.tsv`'s header: a 600 ms sample advances a variable amount of engine time, so
one sample's travel (at most ~1.4 units at level 1's 2.4 u/s run) is the measurement resolution.
Sectors are compared exactly and are never tolerated away.

`--route-compare <expected> <actual> [--events-per-row]` is the comparator the gate uses;
`BK_ROUTE_EPSILON=<units>` sets an absolute tolerance for every numeric column (sector stays exact,
and `-` fields are skipped). Without the flag the latched event columns are graded on the expected
trace's last row only, which is what the frozen retail prefix needs; `--events-per-row` grades them
on every row, which is the port oracle's rule (the replay and the oracle share the port's fixed grid).

A replay is reproducible by construction: the step is pinned at 1/60 s and the replay runs the Update
schedule on one executor thread. The AI's shared LCG is thread-local, so with the default
multi-threaded executor the same script could land a hit a frame apart between runs; one thread makes
every call site see the same sequence. Play keeps the default executor.
## Door encounter telemetry (issue #9)

The fixed level-1 door encounter (the `route_door` pose, `tools/parity/door-encounter.tsv`) is captured on
both builds as one TSV so AI and combat timing can be compared instead of inferred:

```text
t_ms  kind  thing  template  x  y  z  yaw  health  shields  detail
```

`kind` is `player`/`actor` at the row's cadence (100 ms here), `shot` when the port fires a shot that
lands on the player (the row is that shot's **launch**: time, shooter, template and muzzle, with
`detail` naming the shooter's target and `hit`), `miss` when a fired shot dies without landing on the
player (geometry or its own `timer=`, with the same target in `detail`), and `hit` when a shot's
damage lands — the hit row carries the health and shields **after** the hit and the split
(`amount ... health -... shields -...`). Retail can
only observe a shot through its impact on the player (its memory reader sees a bolt only if it exists
at a 100 ms sample), so its `shot` rows are a lossy subset of bolts alive at a sample and the port's
`shot` rows are the landed shots' launches. The gate therefore pairs retail's **`hit`** times with the
port's `shot` times: the same shots, one flight apart, and both counts are exact instead of a sampling
race. Both sides use the engine's game clock, not wall time, so the timeline is comparable at
different frame rates. The port samples **before** the frame's AI tick, so a bolt born and spent inside
one frame is invisible to retail's 100 ms points; only the impact-driven `shot`/`hit` rows are written
after the tick. The retail engine renders a few frames a second, so its `hit` row is the damage that
landed since the previous sample (two bolts in one frame are one row) and its rows
hold that frame's state between grid points; the port hits are its fixed 60 Hz step.

The trace window is the fixture's own: retail stops at the first sample whose elapsed time reaches
10 s, so its last row is at 9,900 ms. The port's row runs 594 fixed steps (9.9 s) for the same reason;
a full 10,000 ms adds a sample (and, at the gunner's cadence, a shot) retail never records.

- Retail: `pwsh -File tools/parity/retail-capture.ps1 -Shots tools/parity/shots.tsv -Only route_door -EncounterMs 10000 -EncounterCadenceMs 100 -EncounterName door_encounter`
  writes `target/parity/encounter/door_encounter.tsv` (the pose row is `route_door`; `-EncounterName`
  matches the encounter file's name). It samples the engine's memory directly (the dev console
  pauses the game, so polling it would change the fight): player health is the f32 at
  `thing+0x134` from the player pointer `0x008319A4`, shields are inventory bin 60
  (`*(thing+0x1C0) + 0x88 + 60*0x18`), and actors/projectiles come from the thing array
  (`world = 0x008339C8`, count at `+0xBC`, base at `+0xC4`, stride `0x2B0`). The addresses and their
  decompiled sources are in `Invoke-JkEncounterTrace`.
- Port: `pwsh -File tools/parity/port-encounter.ps1` runs the same pose for the row's fixed 1/60 s
  frames with `BK_ENCOUNTER_OUT` set; the app samples the player and the actors within 8 units the
  same way retail does, and writes a `shot` row for every fired shot that lands on the player and a
  `hit` row for the damage. The capture starts the player damageable: `kyle.cog`'s `init_kyle` freezes
  the body for 1.5 s after the level loads, and retail's capture begins after the level has settled, so
  without that the gunner's first two bolts vanish and its first measured hit lands at 817 ms.
- Gate: `pwsh -File tools/parity/encounter-gate.ps1` seeds the committed retail fixture from
  `tools/parity/encounter/`, records the port twice (the two runs must be **byte-identical**; the
  capture pins `dt` and runs a single-threaded schedule, so a difference is a nondeterminism bug) and
  reports the per-shot/per-hit timing deltas, each paired hit's own health/shield split, the kill
  order, the settled endpoint, and the actor position/facing/health deltas at every sample. It exits
  non-zero when any of them leaves its tolerance, and its thresholds are parameters with their reasons
  in the script header. Event counts are exact: an extra or missing hit is an unpaired hit and fails.
  Fire cadence is graded on consecutive-shot **intervals** (retail's impact and the port's launch of
  the same shot differ by a constant flight, which intervals cancel). Retail's timing is
  nondeterministic: `sithAICmd_PrimaryFire` jitters every delay by `(1.2 - 0.2 * rand())` and
  acquisition is a chance roll, so the gate compares the intervals as a **sorted multiset** inside the
  cadence band rather than in one run's order. The per-hit and settled-bar tolerances are the
  fixture's own per-hit noise (its per-hit amounts span 11.2-11.4), not a padded band.
  Actor rows are joined by **authored placement** — nearest position at the first sample — because
  retail's `thing` is a slot in the engine's thing pool while the port's is the level's placement
  order. An actor either side reports and the other does not fails; there is no missing-actor
  fraction.

The port samples before the frame's AI tick for its `player`/`actor` rows, matching retail's 100 ms
points; the `shot`/`hit` rows are written after the tick, evicted from the runtime's event queue, so
the shot that fires this frame keeps its launch time. A shot writes its `shot` row only when it
actually damages the player (an invulnerable body produces no retail row), and the row's time is the
launch recorded when the projectile spawned.

**Facings are graded only where retail's facing is stable.** A retail facing that moves more than
`ActorYawTolerance` during the encounter is animation or a random pick, not a facing the port can
derive. The door fixture's powerdroid facing is a model turntable that sweeps ~350° and holds, and the
canteen pedestrians whose classes carry `Roam` (`0x50b830`) draw the engine's `rand()` (`0x512d00`, a
single global stream shared with every detection roll and aim error) for the direction and distance
every interval. The port marks a roamer in the actor row's `detail` column, and the gate compares the
absolute yaw only when retail's own facing stayed within the tolerance and the port's class is not a
roamer; every actor's **position** is still graded strictly. A retail moving facing is reported, not
graded.


Kill order is read off the trace: both captures drop a thing once it is gone (the retail reader skips
an actor at 0 health, the port's trace a destroyed one), so the last sample each actor id appears in
is its death time. The ids are side-local (retail's array slot, the port's placement index) and are
used only within a side for this check; the cross-side join is the placement mapping above. The door
fixture kills nobody on either side in ten seconds and the gate prints that explicitly (`kill order:
retail [], port []`); a fixture whose actors walk out of the 8-unit sampling radius would read the
same way, which is why the position check above is what tells the two apart.

The player's damage split is the shipped `item_shields.cog`'s: the bar takes the whole hit, a bar above
50 stops it completely, and the rest is scaled by `1 - shieldsLeft / 50` (which is above 1 when the hit
spent the bar). Energy (2) and fire (4) share that branch and saber (16) gets half protection; every
other kind reaches health with the bar untouched. A fixture where the port's per-hit health rises as
the bar empties is the script working, not a bug.

The capture runs the `Update` schedule on one executor thread, the same rule #8's replay uses: the
shared cog LCG is thread-local, so with the default executor two runs of the same fixed step could
fire a different number of shots. Two captures are therefore byte-identical apart from the file name,
and the gate's repeatability check reports zero delta. The same pin applies to every fixed-step
capture (`--shot`, `BK_ENCOUNTER_OUT`, `--replay`): without it the walk traces drift too — `walk_cantina`
travelled 2.6 units on one run and 1.0 on the next, because an AI actor driven from the LCG stood in
the body's path. Play keeps the default executor.

The gate is the instrument for #11 (AI engagement timing) and #12 (combat timing). #11 measured its
door-gunner acquisition, cadence and facing against the fixture and fixed the timing differences
(landed-shot rows, the intro freeze at capture start, the 9.9 s window, the roamer facing exemption);
its default run passes. #12 (the player's damage split) is still the shipped `item_shields.cog`'s.

### Audio state gate (issue #13)

`--audio-trace <path>` writes the replay's audio state on the same 600 ms (36-tick) cadence as the
route trace:

```text
tick  t_s  music  music_end  music_loop  music_vol  ambient  ambient_vol  sounds
```

- `music`/`music_end` are `PlaySong(start, end)`'s track range and `music_loop` is the
  mixer's loop flag, 1/0 (`-` when nothing is
  playing); `music_vol` is `SetMusicVol`'s current value, which level 1's `01_SoundTrack.cog` drives
  from a 0.1 s pulse (0.05 a step up, 0.1 down).
- `ambient`/`ambient_vol` are the sector's own `SOUND` loop (a script's `SectorSound` wins), and the
  cell is empty when the sector names none.
- `sounds` lists every one-shot the window played as `wav@gain`; `gain` is the mixer's attenuated
  volume (`snd::volume`), so distance and facing are in the number. `-` means **not recorded**. The
  retail fixture records the capture's one-shots and marks the AI/combat and body-movement ones as
  dynamic above the header:

  ```text
  # dynamic: kwalk02.wav, trprsht1.wav, kshieldhit01.wav, ...
  ```

  The comparator drops every listed wav from **both** sides, so those remain recorded evidence but
  out of the pass/fail, while a route-controlled sound (door/switch/fade/script) still compares. The
  classifier's four categories are exercised by the self-check and the port oracle.

### One-shot schedule (issue #18)

The audio trace samples the mixer every 600 ms; the one-shot oracle is the **event list** beneath it.
`--audio-schedule <path>` (needs `--replay`) writes one row per played one-shot:

```text
tick  time_s  wav  gain  volume  distance  forward  min_radius  max_radius  max_volume  snd  event  source
```

- `tick`/`time_s` are the replay's own tick and engine clock (`sithTime_Advance`'s step) — the time
  the engine played it, not a wall clock.
- `gain` is the mixer's attenuated volume (`snd::volume`); `volume` is the request's base volume and
  `distance`/`forward` the listener geometry it was computed from, so the gain can be re-derived.
- `min_radius`/`max_radius`/`max_volume`, `snd` and `event` are the **shipped `.snd` row** the bank
  loaded for that wav (class basename and event name), so every event names the data behind it.
- `source` is the engine call that queued it, with its address: `sithSoundClass_PlayModeRandom`
  (`0x4e6b70`) for AI chatter and hurt cries, `sithWeapon_WeaponFireProjectile` (`0x4d45a0`) ->
  `sithSoundMixer_PlaySoundThing` (`0x4dba60`) for shots, and the cog `PlaySoundThing`/`PlaySoundPos`/
  `PlaySoundLocal` verbs through `sithSoundMixer_PlaySoundThing` (`0x4dba60`) /
  `sithSoundMixer_PlaySoundPos` (`0x4db880`) / `sithSoundMixer_PlaySound` (`0x4db4f0`).

The committed `level1-audio-schedule.tsv` is the route's expected schedule. The gate compares a fresh
replay against it (`--audio-schedule-compare`), re-derives every row from the install's `.snd` classes
with no replay (`--audio-schedule-verify`: the row's class/event must exist with that wav and those
radii and volume, and the mixer's own fade formula over them — worked out again in the verifier, not
`snd::volume` — must give the row's gain), and checks that the trace's windows carry exactly those
events (`--audio-trace-vs-schedule`). It then perturbs the replay's own schedule four ways — one row
dropped, one added, one moved a 600 ms window, one gain moved 0.25 — and requires
`missing`/`extra`/`mistimed`/`attenuated` respectively.

This replaces the retail side of `sounds` as the one-shot oracle: the retail capture's one-shots are
its own AI/body events on a different clock, so they stay evidence, and the schedule's **event times**
come from the port's decomp-derived engine clock and AI phase, with no retail capture. What the
schedule cannot show by itself is gameplay behaviour: an AI that fires at the wrong moment changes the
schedule, and that is what the committed fixture catches.

```powershell
pwsh -NoProfile -File tools/parity/audio-gate.ps1            # determinism + port oracle + retail + door
pwsh -NoProfile -File tools/parity/audio-gate.ps1 -Perturb   # ... and prove a 0.25 gain change fails
```

`--audio-compare <expected> <actual>` is the comparator; the gate's exit code is its verdict. It
classifies every sound difference as **missing** (the reference played it, the trace did not),
**extra**, **mistimed** (same wav in another 600 ms window) or **attenuated** (same window, different
gain), and reports per-sample music selection/volume and ambience differences. `BK_AUDIO_EPSILON`
(default 0.02) is the absolute tolerance for gains and volumes; the gate passes 0.12, two 0.1 s pulse
steps, because the VM clock accumulates 1/60 s in f32 and a pulse boundary lands one frame either
side of the model's 100 ms grid.

Files:

| Path | What it is |
|---|---|
| `tools/parity/routes/level1-audio-route.tsv` | Issue #8's four route legs unchanged, plus the bar's doorway walked both ways. The door adjoin is a **y = -5 plane** (`--collide --surfaces-at -8,-5,-1.05`: surface 206, sector 19, adjoin 73, x `-8.10..-7.90`, z `-1.150..-0.700`, normal `(0,-1,0)`; 213 is its opposite face), so the legs walk in y (`yaw 180` in, `yaw 0` out). |
| `tools/parity/routes/level1-audio-route-door.tsv` | The doorway alone, captured in a **fresh process** (`retail-capture.ps1 -Only route_door -AudioRoute … -AudioRouteOut …`). In a fresh process retail crosses: `bar_in` reaches `y -5.187` by tick 36 and `bar_out` fires `crossed`, taking `currvol` 0.25 at tick 288, 0.50 at 324 (0.75 at the script's end). In the full four-leg process retail's body wedges on the elevator car's shut leaf and never crosses, so the door leg's state is only measurable from a fresh process. |
| `tools/parity/audiotrace/level1-audio-retail.tsv` | **Captured from retail** by `retail-capture.ps1 -AudioRoute tools/parity/routes/level1-audio-route.tsv -AudioRouteOut tools/parity/audiotrace/level1-audio-retail.tsv`: the harness boots level 1 with `-devMode`, applies each route leg's `warp` through the dev console, holds forward for the leg, and reads the engine's own state at every 600 ms of route game time — song range and loop flag (0x008BBC74/0x008BBC70/0x00835FCC), `SetMusicVol`'s value (f32 0x0054A678), and the 32-entry mixer table at 0x00835FE8 (sound name from the sound struct's own char[32], loop bit 0x1, volume, radii, distance) for the ambience loop and the one-shots. Capture instrumentation only: no shipping behavior changes. |
| `tools/parity/audiotrace/level1-audio-port.tsv` | The port's committed oracle, produced by the replay. |
| `tools/parity/audiotrace/level1-audio-schedule.tsv` | Issue #18's expected one-shot schedule for `level1-audio-route.tsv`: one row per played one-shot with its engine time, wav, gain, the listener geometry and shipped `.snd` row behind it, and the engine call that queued it. The gate compares a fresh replay against it and re-derives every row from the install's `.snd` classes without a replay. |
| `tools/parity/audio-gate.ps1` | The gate above. |
| `tools/parity/audiotrace/level1-audio-door-port.tsv` / `-door-retail.tsv` | The bar doorway captured on its own in a **fresh process** (`level1-audio-route-door.tsv`), where retail crosses: `bar_in` reaches `y -5.187` by tick 36, `bar_out` fires `crossed`, and `currvol` takes 0.15 at tick 252 and 0.40 at 288. The port fades from tick 216 (0.05/0.35/0.65): the fade curve is ~0.2 ahead of retail's (phase -36 ticks). The gate compares this pair event-aligned (first fade change from 0, not raw tick numbers): 2 aligned samples, worst delta 0.100 within 0.12, and it passes. |

Capture limitation: on this workstation retail's redbook device never starts (`0x00835FD0` reads 0,
the MCI open fails), so the captured `music_vol` stays at the mixer's initial 1.0 and cannot show the
script's fade; the port's fixture carries the fade. That is a device limitation of the capture
environment, not a fixture choice, and the gate reports it as the retail delta.

Two port fixes came out of the fixture: a surface crossing now carries its `linkid=` as the
`GetSenderId()` the script reads (level 1's fade-up and fade-down are the same plane under two links,
and both were being reported), and a crossed test now only considers the surfaces of the sector the
body is leaving (the two coincident doorway polygons no longer cancel each other in one frame). The
cog clock was also advancing twice per frame (`run_pulses_and_timers` and `resume_sleeping_labels`
both added `dt`), which ran every shipped fade at double speed; it now advances once.

### Retail's cog namespace

`coglist` prints `<id>: <script> <script>` per instance. The console buffer only keeps the tail, which
on level 1 is **system cogs** at ids **70+** (`00_smoketrail.cog`, `pow_saber.cog`, ... `pow_bryar.cog`)
- weapon/trail/power-up scripts present in every level. The level's own placed scripts fill **0..41**
(the port's load line reports 42 instances), which is the 0-based range the port's placement produces.
Tail fixture: [coglist-01narshadda.txt](coglist-01narshadda.txt). The port's `level.cogs` holds only the
placed instances, so engine-wide ids above that range are not represented there yet.
### The retail dev console's full vocabulary

`help` in the retail console lists every command (saved as
[dev-console-commands.txt](dev-console-commands.txt)): `players coords trackshots ping kick tick
session help disableai notarget outline disablepuppet cogtrace coglist cogstatus noaishots aistatus
ailist fly mem dynamicmem memdump invul cogpause matlist warp activate slowmo jump version team
framerate dispstats kill endlevel`. There is **no level-loading command**; `endlevel` does not advance
the campaign. `cogtrace`/`coglist`/`ailist` are the useful message/AI tracing tools for Phase 1's
trigger work.
## Engine-clock route and encounter capture (issue #20)

A memory-sampled fixture for the encounter (#16), route-timing (#17) and carry (#19) slices. It is a
**user-run** capture: the agent builds and dry-runs it with synthetic data, the user runs it once on
the retail install, and this harness never launches retail on its own. It is not part of CI.

```powershell
# the door encounter: the route_door pose, 10 s of engine clock, no input
pwsh -File tools/parity/retail-capture.ps1 -Shots tools/parity/shots.tsv -Only route_door -EngineCaptureMs 10000
```

Writes `target/parity/engine/route_door.tsv` (override with `-EngineCaptureOut` / `-EngineCaptureName`).
The warped body is the one `door-encounter.tsv` uses (level 1's upper-door `walkplayer` placement), so
the fixture is the same encounter the committed telemetry covers, with the state the telemetry
fixture lacks: engine time, body velocity, inventory bins 72/73/75, the AI random stream, and each
nearby actor's next-fire/acquisition phase at the origin. A route leg uses the same mode with the key
held for the window:

```powershell
pwsh -File tools/parity/retail-capture.ps1 -Shots tools/parity/walk-shots.tsv -Only walk_cantina -EngineCaptureMs 2400 -EngineCaptureKey UP
```

**No console read happens inside the window.** The dev console is a dialog that pauses the engine, so
polling it would sample a frozen clock and change the fight; the whole window is `ReadProcessMemory`
reads on a short loop. A row is written whenever the engine clock (`DAT_00836c14`) changes, so the
time axis is simulation time, not wall or console-poll time. The window ends at the first engine
instant at or past `-EngineCaptureMs` (retail's frames are 10-500 ms apart), so its last row can be
one retail frame past the nominal length. The warp (and the level-load console
reads) happen before the window; the existing 900 ms post-warp wait runs first, so the window does
not include the console/foreground transition (the first row is the first engine instant the body is
sampled at, not necessarily the warp's landing frame).

Format (tab separated, `#` comments allowed):

```text
# engine-clock capture: route_door
# origin_ms 12000
# pose x=1.4001 y=9.7001 z=6.3201 sector=60 (first sample)
# window 10000 ms of engine clock, 20 sample(s), 30 origin actor(s)
# addresses: clock=0x00836C14 player=0x008319A4 rng=0x0054F750 ai_instances=0x0088AFE0 ai_count=0x008B4BE0
t_ms  kind  thing  template  x  y  z  sector  vx  vy  vz  bin72  bin73  bin75  rng
      ai_flags  fire_next_ms  acquire_next_ms  acquired_at_ms  target  last_target
      next_pass_ms  detail
```

- `kind = player` rows are the body at one engine instant: `t_ms` is `DAT_00836c14`, `x/y/z` the body
  position (`thing+0xB8`), `sector` the tracked sector (`*(thing+0x5C)`), `vx/vy/vz` the engine
  velocity (`thing+0x1D4`), `bin72/73/75` the inventory amounts
  (`*(thing+0x1C0) + 0x88 + bin*0x18`: kills credit, population credit, alignment), and `rng` the
  AI random stream state (`0x0054F750`, the LCG `state = state*0x343FD + 0x269EC3` that
  `sithAI_RandomFireVector`/acquisition rolls share). The AI columns are empty on these rows.
- `kind = origin` rows exist once, at `origin_ms`, for every AI-instance actor within 8 units of the
  player. `ai_flags` is the AI instance's flag word (the acquisition-state bits, `instance+0x08`),
  `fire_next_ms`/`acquire_next_ms` are the next-run times of the actor's `primaryfire` and
  `lookfortarget` instincts (the registry names matched through the class handler table), and
  `next_pass_ms` is the instance's next AI pass (`instance+0x190`). `target`/`last_target` are the
  current and last acquired thing pointers, `acquired_at_ms` the last acquisition's engine time. All
  are absolute engine ms, so the port can align its first AI update with retail's phase relative to
  `origin_ms` (the header repeats it). `detail` spells out which instincts were found and whether the
  target is the player. The player columns are empty on these rows.
- `thing` is retail's thing-pool slot and `template` its template index, the same convention the
  encounter trace uses; the reference reader pairs actors by authored position, not by id.

The capture validates its own output with the same schema check the self-check uses, and throws
instead of leaving a malformed fixture. Run the self-check (no retail install, no launch):

```powershell
pwsh -File tools/parity/retail-capture.ps1 -EngineCaptureSelfCheck
```

It writes one synthetic fixture through the real writer, requires it to be accepted, then corrupts it
seven ways (missing column, non-numeric position, unknown kind, backwards clock, missing origin
clock, origin row with no next-fire time, renamed header) and requires every corruption to be
rejected. It also runs `Get-JkAiOriginRows` itself over a synthetic memory image (one actor inside the
radius, one outside, and a run with no AI slots) and requires the rows' fields and the fixture they
make to be right: that builder is what the first user-run capture died in — `@()` over its
`List[object]` throws "Argument types do not match" on PowerShell 7.6 — and the schema cases alone did
not reach it. Exit code 0 is pass.

Addresses, with their decompilation sources, are in `Invoke-JkEngineCapture` and
`Get-JkAiOriginRows`: the clock is `sithTime_Advance` (`decomp/004dd640`), the sector
`GetThingSector` (`0x502B80`), velocity `GetThingVel` (`0x503160`), the AI instance array
`sithAI_Create`/`sithAI_Process` (`0x4E9CB0`/`0x4E9E20`), the instinct slots and next-pass field
`sithAI_InstinctUpdate` (`0x4E9E90`), the target fields the `primaryfire`/`lookfortarget` handlers
write (`0x509E40`/`0x50AA80`/`0x50B9B0`), the instinct registry `sithAI_RegisterInstinct`
(`0x4EA0A0`), and the AI random stream `FUN_00512d00`. The capture changes no gameplay state: it only
reads memory, and the optional `-EngineCaptureKey` is the same synthetic input the movement traces
use.

## Status commands (`cogstatus`, `aistatus`)

Phase 0.5 step 2 pins the two dev-console status formats from the decomp and captures retail output:

- `cogstatus <n>` - `sithCog_CogStatus` (`decomp/004e0480_sithCog_CogStatus.c`): prints
  `Cog #%d: Name:%s  Script %s` then one line per cog symbol, `  Symbol %d: '%s' ` followed by
  ` = %f` for float symbols or ` = %d` for integer ones (format strings at `0x54aaf4`, `0x54ab1c`,
  `0x54ab30`, `0x54ab38`). Out-of-range indices print nothing.
- `aistatus <n>` - `sithAI_AIStatus` (`decomp/004ea230_sithAI_AIStatus.c`): the argument is a position in the
  level's **AI actor list in thing-id order** (`0` is the first actor; a bare `aistatus` dumps level-load timings
  instead, and a thing id is rejected). An inactive block prints `That AI block is not currently active.`; an active
  one prints `AI Status dump for thing N (template).`, then `Class 'x.ai', Flags=0x…, Moods a/b/c, NextUpdate=n`,
  then `Current instincts:` and one row per instinct:
  `Instinct %d: Params: %f/%f/%f/%f, nextUpdate=%d, mask=0x%x, mode=0x%x.`
  **Row i is the class `.ai` file's i-th instinct line** (verified: eleven lines in `ai\tuproddefault.ai` ↔ eleven
  rows for a `tuskan`; read any class with `bevi-knight --asset ai\<name>.ai`), and the two words are the
  registration's event mask and accept word: `mask` is the **events** whose dispatch invokes the handler
  (`sithAI_RegisterInstinct`'s fifth argument, `Follow` = `0xe00`), and `mode` is the **accept word** the
  scheduler tests against the AI flag word (`mode & flags != 0`; `Follow` = `2`). `sithAI_InstinctUpdate`
  also tests a third, undisplayed **reject** word that must be clear (`Follow` = `0x800`). `sithAICmd_Startup`
  registers each handler with them (`lookfortarget` is handler `0x50aa80`, accept `0x204`, reject `0`, events
  `0x1f00`). `Params` are the four per-instance numbers, 0 on an
  idle actor. Example rows for a brown Tusken: Jump `mask 0x604`, LookForTarget `mode 0x204`, PrimaryFire
  `mask 0x100 mode 2`, Follow `mask 0xe00 mode 2`, Flee `mask 0xf05 mode 0x800`, Talk `mode 0xffff`.

Capture with `retail-capture.ps1 -ConsoleCommands 'cogstatus 1' ...` - each row's output lands in
`target/parity/trace/<name>-console.txt`. Level 1's first cog is captured in
[cogstatus-01narshadda.txt](cogstatus-01narshadda.txt):

```text
Cog #1: Name:00_std_elev.cog  Script 00_std_elev.cog
  Symbol 0: 'crossed'  = 8
  ...
  Symbol 8: 'speed'  = 4.000000
```

Level 1 also has a **cog 0** fixture (`cogstatus0-01narshadda.txt`, `00_door.cog`): retail ids are the
row index and start at 0. The port-side verb `bevi-knight --start 01narshadda --cogstatus <n>` reads
`cogs[n]` and reproduces `cogstatus 1` byte-for-byte; for cog 0 it matches the first twelve symbols,
the rest being runtime local values that need the script's `startup` to run.

That sample is the fixture the port-side verb reproduces format-wise: `bevi-knight --start
01narshadda --cogstatus 1` prints `Cog #1: Name:<script>  Script <script>` plus `  Symbol i:
\'<name>\'  = <value>` (floats to six decimals, integers bare). The ids now **do** line up: the port's
`--links` probe lists retail's own script at each id (`thing 6 -> cog 1 ... 00_std_elev.cog`), which is
the same `00_std_elev.cog` retail's `cogstatus 1` prints, so the earlier `00_door.cog`/`00_std_elev.cog`
mismatch recorded here has been fixed; `--links` is the check.
## Known limitations / follow-ups

- **Triage, not equality.** Retail runs its 16-bit DirectDraw/D3D path, translated by dgVoodoo2 to
  D3D11; the port uses Bevy. The report's SSIM/mean-difference is for ranking pairs; the side-by-side
  is the evidence. `-Gate` turns the tolerances in `thresholds.tsv` into a pass/fail floor — it is a
  regression check on this machine, not a claim that the pictures are equal.
- **Retail framing:** the game's `viewSize` letterboxes the world inside its display mode; today the
  display mode is 1440x900 (16bpp, 3D on, `viewSize = 6`) and the world box is `1170x731+135+84`.
  Retail capture saves the world box (`<shot>.png`) and the raw frame (`<shot>-raw.png`); the box is
  found by row/column content fill, so HUD corners and the "Screenshot Taken" notification in the
  black margins are excluded. `compare.ps1` prefers the sidecar's crop, and `port-capture.ps1`
  renders the port window at that crop's size so both frames have one shape.
- **HUD:** the 16-bit `JSHOT` BMP contains the world view plus a little HUD in the margins (shield and
  ammo corners); the world crop cuts those off, while the port frame still has its own HUD. Compare
  the world region.
- **Pose rows:** numeric rows work through the patched `warp` and are verified against `coords`
  (`warp_verified` in the sidecar). `warp` sets position and yaw; pitch/roll are not settable (see the
  next bullet), so rows request pitch 0 and the sidecar records whatever `coords` reports.
- **Pitch:** `--shot-at` takes a fifth number — pitch in degrees. The retail `warp` cannot set pitch,
  and the `coords` PYR pitch is not the rendered camera pitch: level 1 prints 1.04, yet its frame
  matches a port pitch of 0 (a 0.25° port pitch already drops the spawn pair 0.63 → 0.59). The rows
  therefore request 0. A pose whose retail camera is genuinely pitched needs a retail-side way to set
  it, which is not known yet.
- **The player's pose.** `--shot-at` moves the camera only; `--warp "x,y,z,yaw"` moves the *player*
  to the same recorded eye (body one eye offset below it) and re-derives its sector, which is what
  lets the room's scripts, AI and hazards act on a body the way retail's reference capture has them
  act. Without `--shot-at`, `--warp` alone lets the camera ride the warped body (retail's mode;
  identical framing at a standing pose); the batch passes both so the framing stays deterministic.
  A warped body keeps its pose while idle, as retail's does: the idle attachment reaches a solid
  thing's model top within `GetThingHeight * 1.1` (`sithPhysics_FindFloor`), and
  `sithThing_AttachThingToSurface` does not move the body — so at the catwalk the port's body now
  holds 2.8201141 (retail `coords` 2.857114 eye − 0.037) instead of falling out of the level. A body
  that *walks* still settles at `top + radius` (0.08 lower there), which is the movement port, not the
  warp. `BK_WARP_TRACE=1` prints what is placed near a warped body with each solid thing's model top;
  `BK_WALK_TRACE=1` prints the walker's per-frame sector/pos/support.
- **Damage tint.** The engine's `sithActor_DamageActor` adds `damage * 0.04` to the local player's
  red palette tint (`sithPlayer_AddDynamicTint`), `sithPlayer_Update` decays it by 0.4 per second,
  and `stdPalEffects_UpdatePalette` applies the sum as `byte += round(byte * tint * 0.5)` per channel.
  That is the red in the two ungraded route references; it is not a scripted effect. The port draws
  it with a fullscreen palette pass on the world camera (`BK_TINT=r,g,b` paints a fixed value for
  measuring it), but the body has to be *hit* for the tint to appear — at the door the first hits land around 6 s
   and the 10 s capture is red (crop red mean 46 → 67).
- **Reading a pair's contents.** `BK_HIT_TRACE=1` makes the port print each shot (`fire: <template>
  dmg … owner <thing> from <pos> yaw/pitch`) and put the shot's owner on each hit line, so a capture's
  log says who hit whom; `BK_AI_TRACE=1` prints every actor-frame that has line of sight to the body
  (range, dot, `ready`); `BK_AI_ROSTER=1` dumps every actor near the body once — AI class, cone test,
  line of sight and how far off its facing the body is, which is the fastest way to see why a room is
  quiet. At level 1's door the uninstrumented 600-frame capture lands 10 hits and ends at 83 health /
  0 shields (retail's settled raw frame: 81 / 0). Do not compare a `BK_AI_TRACE=2` run's damage total:
  its output volume changed the encounter in round 409. The rest of the shots fall into a
  Greedo standing in the lines; the roster shows 56 armed actors within 8 units, all with a clear line,
  most facing >100° away. (They turn onto a target they have now - rate-limited by `maxrotthrust` - and a side-mate's shot tells the rest where it is: `sithAIAwareness` modelled; an actor keeps its aim while walking
  and the chase keeps its class's `Follow` standoff.)
- **Retail script coverage:** the menu coordinates and key timing are tied to the GOG build (and
  to `player\Lee\Lee.plr`); a different build/player may need the constants updated.
- **Level 1's spawn — what the pair showed, and the fix.** Retail's spawn frame is the player camera:
  the 1 s series is static in its world regions (`first_frame_is_player_camera` in the sidecar) and it
  is pixel-identical to the warp-verified `spawn_pose` reference. The 0.23 pair was the *port's* spawn
  camera, 0.055 units low: setup replaced the level's recorded pose with a floor-seated one. The port
  now starts at the recorded pose and keeps it while idle, so `spawn` scores 0.62–0.65 — the equal of the
  warp-verified pairs — and `compare.ps1 -Gate` fails the pre-fix frame. The thing at the eye is one
  decor thing (`tuskensoused`, thing 215) that `01_TuskenTwitch.cog` animates with `PlayKey`; the
  loader used to skip script-named keyfiles, which is what froze it. Retail draws that patron in the
  same place the port does, so it is the level's own start, not a missing draw rule. (The decomp also
  shows the eye offset matches retail and that retail has no first-person ceiling clamp; see
  `docs/gaps.md` §5.9.)

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| Retail frames are black with only HUD visible | The 3D path did not render: make sure the wrapper is staged and `b3DAccel = 1` (the default), or run with `-Software` for the 8-bit path. |
| `retail-capture.ps1` fails downloading the 3D wrapper | No network to github.com: drop the pinned dgVoodoo2 zip at `target/parity/wrapper/` (or run with `-Software`). |
| `retail-capture.ps1` never sees a level load | The menu clicks missed (focus timing). Rerun; the harness waits for the console dialog and level-parse line. |
| F12 writes no file | The game is not in gameplay (menus do not take screenshots) or another window took focus. |
| Typed console command becomes a prefix garbage string | Keystrokes were sent too fast; keys must be held (≥300 ms). |
| `warp` / `cogstatus` / `slowmo` answered "not recognized" | The staged `JK.EXE` was not patched: check the staging log for the `dev-console commands unlocked` line, and that `Set-JkDevCommandsEnabled` found its unique site. |
| `port-capture.ps1` waits forever | A retail run is in progress (`JK.EXE` alive); it waits for the retail side to finish. |
| Primary monitor left at 640x480 (or another legacy mode) after a run | An interrupted/killed run skipped the engine's display-mode restore. Run `pwsh -File tools/parity/restore-display.ps1` (or `-RegistryDefaults`). Normal runs close the game cleanly and do not do this. |
| Port window looks clipped or wrong right after a retail run | The desktop was still in the game mode. `port-capture.ps1` now heals that before capturing; if it persists, run `restore-display.ps1`. |
| Port frames size changed | The port renders at the retail view size for each row (read from `target/parity/retail/<shot>.json`); with no retail sidecar yet it falls back to 1024x768. |
