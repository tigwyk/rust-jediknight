# bevi-knight

[![CI](https://github.com/tigwyk/rust-jediknight/actions/workflows/ci.yml/badge.svg)](https://github.com/tigwyk/rust-jediknight/actions/workflows/ci.yml)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)
[![Platform: Windows 10/11 x64](https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-0078d4.svg)](#requirements)

A from-scratch replacement engine for **Star Wars Jedi Knight: Dark Forces II**, written in
**Rust + Bevy 0.20**. Point it at the GOG install you already own and it plays the real game —
the shipped levels, menus, HUD, sounds and cutscenes — in a modern window.

It is not a patch, launcher or crack. The retail installation is never modified: the port reads
the `.gob` archives, the engine executable's own tables and the plaintext data directly, and
ships **no game data** of its own.

![`jkl\03katarn.jkl`, drawn from the retail install](docs/screenshot.png)

*`jkl\03katarn.jkl` from its own `walkplayer` start point — the level's geometry, materials and
sky, with its crate models at their real size.*

## Status at a glance

**Version 0.1.0 — Windows x64, single-player only.** The light-side campaign is playable from the
opening film to the finale, with the caveat that nobody has yet played levels 2–21 all the way
through by hand (the app itself completes all 21 levels in sequence as a check).

| Working today | Deliberately not there in 0.1 |
|---|---|
| Campaign: all 28 campaign rows, both finales, films and level hand-off | Dark-side choice and ending screens |
| Menus, HUD, inventory, force-power screen, save slots, audio | Multiplayer |
| Combat: weapons, damage, deaths, pickups, force jump/speed/heal/see/pull/grip | The full force-power set and several animation states are unverified |
| Level logic: doors, lifts, triggers, scripts running against the live world | AI instinct scheduler (actors see, chase and fire, but only `PrimaryFire`) |
| Cutscenes and music from the retail files | Cutscene palette: 29 of 768 bytes differ from the reference decoder |

The complete, evidence-backed list is in the [0.1 release note](docs/release-0.1.md); the
unit-by-unit state is [docs/engineering.md](docs/engineering.md).

## Screens

![The retail main menu, drawn from the game's own bitmap, font and strings](docs/menu-main.png)

*The main menu is the game's own `bkmain.bm` with the retail font, strings and layout read out of
`JK.EXE` — nothing on this screen is authored by the port.*

![The in-game status bar over a live level](docs/ingame-hud.png)

*The status bar in the running game: retail panel and meter artwork, frame indices driven by the
player's own shield, health, force and battery values.*

## Requirements

- **Windows 10 or 11, x64.**
- A **retail GOG install** of *Star Wars Jedi Knight: Dark Forces II* — the folder must contain
  `JK.EXE` and a `Resource\` directory. The GOG build is the one tested here; the Steam
  executable is DRM-wrapped and is not read by this port.
- About 100 MB of disk space once installed (the setup download itself is ~28 MB). No admin
  rights are needed and no separate runtime has to be installed.

## Install

CI builds the setup on every push, and every **version tag produces a GitHub release with the
Windows setup attached**. Download the newest one from
[Releases](https://github.com/tigwyk/rust-jediknight/releases).

If no release is published yet — or you want the newest unreleased build — get the
`bevi-knight-setup` artifact from the latest green
[CI run](https://github.com/tigwyk/rust-jediknight/actions/workflows/ci.yml) (Actions → **CI** →
newest run → **Artifacts**; a GitHub sign-in is needed for artifact downloads), or
[build it yourself](#build-from-source) with one command.

1. Run `bevi-knight-<version>-setup.exe`.
2. The setup finds your GOG install from its registry entry or usual folders. If it cannot, browse
   to the folder with `JK.EXE`, paste the path, or drag the folder onto the window. It installs
   per-user to `%LOCALAPPDATA%\Programs\bevi-knight`, adds a Start Menu shortcut, and registers in
   **Settings → Apps**.
3. Uninstall at any time from **Settings → Apps → bevi-knight**, or with
   `bevi-knight-uninstall.exe` in the install folder.

The setup is unsigned, so Windows SmartScreen may warn the first time: **More info → Run anyway**.
It does not touch the retail install and needs no administrator rights.

For scripted installs the setup also accepts:

```powershell
bevi-knight-<version>-setup.exe /S /GAMEPATH="C:\GOG Games\Star Wars Jedi Knight - Dark Forces 2" /DIR="D:\Games\bevi-knight" /NOSHORTCUTS
```

## First launch

Launch **bevi-knight** from the Start Menu. The retail main menu appears; choose **New Game** to
play the campaign from the opening film. Audio is on by default (`--mute` only silences it), and
the game finds the retail install on later runs through the saved game path.

## Controls

| Action | Input |
|---|---|
| Move | `W` `A` `S` `D` |
| Look | mouse, or the arrow keys |
| Run | `Shift` |
| Jump | `Space` |
| Crouch | `Ctrl` |
| Fire | left mouse |
| Use (doors, switches) | `E` |
| Inventory strip (weapons and force powers) | `I`, then arrows and `Enter` |
| Previous / next weapon | `[` / `]` |
| Map and field log | `M` |
| Third person | `V` (chase camera with its pull-in), `C` (simpler follow view) |
| Quick save / quick load | `F5` / `F9`; `,` and `.` change save slot |
| Pause / back | `Escape` |
| Menus | arrows and `Enter`, or the mouse |

Saves are written to the `saves\` folder inside your retail install (`BK_SAVE_DIR` overrides it),
so they survive uninstalling the port.

## Troubleshooting

| Symptom | Fix |
|---|---|
| “It cannot find my game” | Run `bevi-knight.exe --find-game` to see what it checked, then `--set-game "<folder with JK.EXE>"`; or set `BK_GAME`; or re-run the setup. The folder must contain `JK.EXE` and `Resource\`. |
| No sound | It is on by default; check the Windows per-app mixer. `--mute` / `BK_MUTE=1` force it off for captures. |
| Cutscenes are skipped | When building from source, the 32-bit film helper must be built too (see below); the setup bundles it. |
| SmartScreen blocks the setup | It is unsigned: **More info → Run anyway**. |
| Where did the setup put things? | `%LOCALAPPDATA%\Programs\bevi-knight`; its log is `%TEMP%\bevi-knight-setup.log`. |
| Want a diagnostic run | `bevi-knight.exe --check` (parses the level, prints and asserts the numbers), `--audit` (the whole install), `--find-game`. Non-zero exit tells you where it stopped. |

## Reporting issues

Please use the [issue tracker](https://github.com/tigwyk/rust-jediknight/issues/new). A useful report
includes:

- the bevi-knight version (shown in **Settings → Apps**; 0.1.0 is the current release),
- your Windows version and GPU,
- where the retail install lives and which edition it is (GOG, CD, other),
- what you did, what you expected, and what happened,
- a screenshot if it is visual, and the terminal output or `%TEMP%\bevi-knight-setup.log`.
- If a level fails, name the level file (`jkl\...jkl`) rather than attaching game data.

**Please do not attach retail game files** (`.gob`, `.3do`, `.jkl`, `.smk`, `JK.EXE`, saves).
They are copyrighted; describe the file or thing by name instead. The full list of known gaps is in
[docs/gaps.md](docs/gaps.md) — checking there first saves a round trip.

## Build from source

Prerequisites: Windows x64, a Rust toolchain **1.97.1 or newer** with the MSVC target, and the
retail install.

```powershell
cargo build --release

# Cutscenes need a 32-bit helper because the game's SMACKW32.DLL is 32-bit:
rustup target add i686-pc-windows-msvc
cargo build --release --target i686-pc-windows-msvc --manifest-path tools/smkdec/Cargo.toml

# Run it (it finds the install itself; --game overrides):
cargo run --release
```

Build the same Windows setup locally:

```powershell
powershell -ExecutionPolicy Bypass -File packaging\build-setup.ps1
# writes dist\bevi-knight-<version>-setup.exe, no Inno/WiX/NSIS/Squirrel needed
```

Run the unit tests and the install-wide checks (the latter need the retail data):

```powershell
cargo test --bins --release
cargo run --release -- --check          # level 1: parse and assert
cargo run --release -- --audit          # every level and every referenced asset
cargo run --release -- --gates          # 61 behaviour gates, one process each
```

## Verification

[docs/gates.txt](docs/gates.txt) records what every check printed against the retail install:
**61 gates, 0 failures** (windowed; `BK_HEADLESS=1` skips 9 that need a window), plus **132 unit
tests**. CI builds, tests and packages on Windows with the MSRV pinned in `Cargo.toml` and uploads
the setup artifact. The 61 gates are not in CI on purpose — they require the game data, which the
repository does not ship and CI does not download.

## Documentation

| Document | What it covers |
|---|---|
| [0.1 release note](docs/release-0.1.md) | What 0.1 does, its caveats, and the verified numbers |
| [docs/gates.txt](docs/gates.txt) | The full gate transcript from the retail install |
| [docs/engineering.md](docs/engineering.md) | The engineering record: coverage, reverse-engineering, camera recovery |
| [docs/formats.md](docs/formats.md) | Recovered file formats, the evidence for each, and the unknowns |
| [docs/gaps.md](docs/gaps.md) | Everything not ported yet, unit by unit |
| [docs/roadmap.md](docs/roadmap.md) | The dependency-ordered remainder |
| [docs/progress-log.md](docs/progress-log.md) | How each piece was recovered |
| [docs/release-checklist.md](docs/release-checklist.md) | Release state and what remains before publishing |

## License and game rights

The source, tools and setup scripts are **GPL-3.0-or-later** — see [LICENSE](LICENSE) and
[NOTICE](NOTICE). You may rebuild, study, modify and redistribute it, including selling copies, as
long as the corresponding source and the same freedoms travel with it; closed-source forks and
proprietary repackaging are not permitted. Contributions are accepted under the same license
(inbound = outbound, no CLA).

**No game data is included or licensed.** *Star Wars Jedi Knight: Dark Forces II* and everything in
it — levels, textures, sounds, films, names — belongs to Lucasfilm/Disney, and this project is not
affiliated with or endorsed by them. It only reads the retail game the player already owns. The
screenshots in `docs/` show retail artwork, are included as documentation, and are not covered by
the GPL grant.
