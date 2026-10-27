# Retail-vs-port parity captures

Screenshot the **real GOG game** and **this port** from the same, known camera poses, then put
them side by side. Built to answer "does the port draw what the 1997 engine draws?" — the first
use is level 1's opening, where the port's spawn view is buried in a prop (`docs/gaps.md` §5.9).

Both sides are driven headlessly (no human input) and the pose is recorded, so two runs are
comparable even if the mouse moves between them.

## What it does

| Step | Script | What happens |
|---|---|---|
| 1 | `retail-capture.ps1` | Copies the retail install to a scratch folder, launches the **GOG `JK.EXE`** in `-devMode`, drives its menus with the mouse, reads the player pose from the game's own `coords` console command, `warp`s to each pose in `shots.tsv`, presses F12 (the game writes `JSHOT*.PCX/BMP`) and converts the frames to PNG. |
| 2 | `port-capture.ps1` | Runs `bevi-knight.exe` once per shot with `--start`, `--shot`, and (for numeric poses) `--shot-at`. |
| 3 | `compare.ps1` | Pairs `retail/<name>.png` with `port/<name>.png`, writes a side-by-side, a contact sheet, and `report.md` with framing facts and a triage score (SSIM / mean absolute difference). |

The retail side never touches your real installation: it works on a copy under
`target/parity/retailcopy/`. It also needs the game's 3D acceleration **off** (with it on, this
game renders a black world and only the HUD — see Troubleshooting).

## Requirements

- Windows 10/11 x64, PowerShell 7 (`pwsh`), `ffmpeg` on `PATH`.
- A retail **GOG** install of *Star Wars Jedi Knight: Dark Forces II* (the port's normal requirement).
- The port built: `cargo build --release` (the harness will build it if missing).
- **Keep hands off the mouse and keyboard while a retail run is in progress** — it is a fullscreen
  640x480 game with the mouse captured, and synthetic input is how the harness drives it.

## Quick start

From the repo root:

```powershell
# 1. capture the retail reference frames
pwsh -File tools/parity/retail-capture.ps1

# 2. capture the same poses in the port
pwsh -File tools/parity/port-capture.ps1

# 3. pair, score and lay them out
pwsh -File tools/parity/compare.ps1
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

## The shot list: `tools/parity/shots.tsv`

Tab-separated, one shot per line. A pose of `-` means **the level's own spawn** (retail reads the
pose with `coords`; the port just uses `--start`). Numeric rows are **camera eyes** in world units,
angles in the retail `coords` convention (`PYR`, degrees):

```text
name	level	x	y	z	pitch	yaw
spawn	01narshadda	-	-	-	-	-
spawn_pose	01narshadda	-10.443433	-5.455094	-1.092886	0	-59.994
spawn_yaw30	01narshadda	-10.443433	-5.455094	-1.092886	0	-30
```

`spawn` (no warp) captures whatever camera the level opens on, which for level 1 is the scripted
intro camera — it scores ~0.23 SSIM against the port. `spawn_pose` is the same spot as the level's
own `walkplayer` but captured through `warp`, i.e. the verified player camera: it scores ~0.63, the
same as `spawn_yaw30`. Use numeric rows when a pair has to be trusted.

- `z` is the **eye** height — the value the port's `--shot-at` takes. Retail's `warp` moves the player
  body, and the engine rides the camera at `thing + 0x168` (the template's `eyeoffset=(0,0,0.037)`),
  so the retail side warps to `z - 0.037`. It reads `coords` back and marks the row
  `warp_verified` when the position (offset restored) and yaw agree.
- `pitch` must be **0**: the port's `--shot-at` is horizontal aim only, and retail's `warp` sets
  position + yaw while pitch/roll keep the level's own view angles (level 1 stays at PYR pitch 1.04,
  which the sidecar records).
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
  is the evidence.
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
  (`warp_verified` in the sidecar). `warp` sets position and yaw only — pitch/roll keep the level's
  own view angles — so rows use `pitch 0` and the sidecar records the achieved pitch.
- **Pitch:** the port's `--shot-at` aims horizontally only; retail `PYR` pitch is recorded but
  cannot be reproduced yet. Spawn/level rows are unaffected.
- **Retail script coverage:** the menu coordinates and key timing are tied to the GOG build (and
  to `player\Lee\Lee.plr`); a different build/player may need the constants updated.
- **Level 1's spawn — what the pair shows:** the 1 s series is *not* static: the first seconds show a
  moving view (a scripted intro camera), so the settled last frame is the comparison reference and
  `spawn_yaw30` — captured after `warp` — is the pose-verified player-camera pair. That matters for
  the score: the unverified `spawn` reference still frames the intro camera (SSIM 0.23), while the
  same room at the verified pose scores 0.62. The port used to freeze one animated decor thing
  (`tuskensoused`, thing 215) in front of the eye; the level's `01_TuskenTwitch.cog` plays a keyframe
  on it via `PlayKey`, and the port's runtime now loads script-named keyfiles and honours the one-shot
  `PlayKey` flags, so it animates and settles. Once the pose is verified (`warp`), retail draws that
  patron in the same place the port does — the old clean spawn reference was the intro camera, not a
  missing draw rule. (The decomp also shows the port's eye offset matches retail and that retail has
  no first-person ceiling clamp; see `docs/gaps.md` §5.9.)

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
