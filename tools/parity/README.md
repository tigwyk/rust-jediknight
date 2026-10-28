# Retail-vs-port parity captures

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

The retail side never touches your real installation: it works on a copy under
`target/parity/retailcopy/`. By default it stages dgVoodoo2 and runs the game's 16-bit 3D path;
`-Software` forces the 8-bit software path. With no working 3D path the world renders black and
only the HUD appears — see Troubleshooting.

## Requirements

- Windows 10/11 x64, PowerShell 7 (`pwsh`), `ffmpeg` on `PATH`.
- A retail **GOG** install of *Star Wars Jedi Knight: Dark Forces II* (the port's normal requirement).
- The port built: `cargo build --release` (the harness will build it if missing).
- **Keep hands off the mouse and keyboard while a retail run is in progress** — it is a fullscreen
  game with the mouse captured, and synthetic input is how the harness drives it.

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
is **red from damage tint**, though: the body is taking damage (level 1's door: health 100 → 44 over
the series; catwalk: 100/100 → 81/11), and the engine's damage path adds a red palette tint
(`sithActor_DamageActor` calls `sithPlayer_AddDynamicTint(damage * 0.04, 0, 0)`; it decays 0.4 per
second and is applied as `palette += round(palette * tint * 0.5)`). Both halves are ported now: the
state (adds with the engine's clamps, the 0.4/s decay, the damage flash — unit-tested) and the draw
(a fullscreen palette pass on the world camera that scales the finished picture's bytes by
`1 + tint/2`; a 1.0 red tint measures ×1.52). The rows stay `UNGRADED` at 0.45/0.42 for a different
reason: retail's own 1 s series turns red at ~5–6 s and its settled reference at ~10 s is saturated
(r/g 2.0), while the port's 10 s capture now takes **6 hits (72 damage)** and draws only a weak tint
(the red crop mean moves 46 → 51–63) — the damage is a third of retail's and the doorway geometry
dominates that pair. At the catwalk the port also cannot stand its body on the catwalk *model* —
that is a solid thing, not a sector floor, so the body falls out of the level. Those are named engine
gaps (Phase 1 steps 3 and 5), not renderer tolerance (see `docs/gaps.md` §5.9).

The numbers are tied to the capture policy above (600 frames, window from the retail sidecar, default
16-bit retail renderer) — changing framing or frame count invalidates them.

The gate needs a retail install and captured references on the machine, so it is a developer check, not
a CI job; the port's own 141 tests and 61 gates remain the CI-able surface.

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
- The game's menus are **mouse-only** for synthetic input. The 640x480 fullscreen window's screen
  coordinates equal game coordinates; the two clicks needed are `Single Player (320,195)` and
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
  The port's player warp still has two limits: it keeps the body where it is put only when a
  floor is within the idle-attach reach (at the catwalk pose there is none in reach and the catwalk
  model is 0.145 below, so the port's body falls while retail's `coords` holds z 2.80), and the
  world *draw* of the engine's damage tint is missing. Both show up on
  `route_catwalk`/`route_door`.
- **Damage tint.** The engine's `sithActor_DamageActor` adds `damage * 0.04` to the local player's
  red palette tint (`sithPlayer_AddDynamicTint`), `sithPlayer_Update` decays it by 0.4 per second,
  and `stdPalEffects_UpdatePalette` applies the sum as `byte += round(byte * tint * 0.5)` per channel.
  That is the red in the two ungraded route references; it is not a scripted effect. The port draws
  it with a fullscreen palette pass on the world camera (`BK_TINT=r,g,b` paints a fixed value for
  measuring it), but the body has to be *hit* for the tint to appear — which at the door only starts
  around 15–20 s, past the 10 s capture window.
- **Reading a pair's contents.** `BK_HIT_TRACE=1` makes the port print each shot (`fire: <template>
  dmg … owner <thing> from <pos> yaw/pitch`) and put the shot's owner on each hit line, so a capture's
  log says who hit whom; `BK_AI_TRACE=1` prints every actor-frame that has line of sight to the body
  (range, dot, `ready`); `BK_AI_ROSTER=1` dumps every actor near the body once — AI class, cone test,
  line of sight and how far off its facing the body is, which is the fastest way to see why a room is
  quiet. At level 1's door: the capture now lands 6 real hits on the body (32 shots); the rest fall into a
  Greedo standing in the lines; the roster shows 56 armed actors within 8 units, all with a clear line,
  most facing >100° away. (They turn onto a target they have now: an actor keeps its aim while walking
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
