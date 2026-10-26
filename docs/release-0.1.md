# bevi-knight 0.1 — what you can play, and what is still rough

A single-player reconstruction: it reads the retail `.gob` archives and `JK.EXE`'s own
tables, and does not need the original engine, a cover mount or a crack. **No game assets
are shipped with it** — you need your own retail install (the GOG build is what this was
developed against, because its `JK.EXE` is plaintext).

## Build it

Windows x64, a Rust toolchain **1.97.1 or newer** (what Bevy 0.20 needs) with the MSVC
target, and a retail install of *Star Wars Jedi Knight: Dark Forces II*.

```sh
cargo build --release
```

Films need one extra step: the install's `SMACKW32.DLL` is a 32-bit image, so the port
decodes films in a small i686 helper. Without it the game runs but skips cutscenes with a
message.

```sh
rustup target add i686-pc-windows-msvc
cargo build --release --target i686-pc-windows-msvc --manifest-path tools/smkdec/Cargo.toml
```

The game finds the helper in the repo's `tools/smkdec/target/i686-pc-windows-msvc/release/`
whether it is started from the repo or from `target/release/`, and `BK_SMKDEC` names the
executable explicitly if it was moved.

The same steps run in [CI](../.github/workflows/ci.yml) on a Windows runner with this file's MSRV,
and the job uploads the native setup as a build artifact. The `--gates` checks are not in CI:
they need a retail install, which is not shipped here.

## Install it (Windows, no admin rights)

`packaging\build-setup.ps1` builds **one native setup executable**:

```sh
powershell -ExecutionPolicy Bypass -File packaging\build-setup.ps1
# writes dist\bevi-knight-<version>-setup.exe (~28 MB: wizard + the game, film helper and docs)
```

It is compiled with the C# compiler and UI libraries that ship with Windows — no Inno Setup,
WiX, NSIS or Squirrel — so it is a normal Windows dialog rather than a script. Run it and it:

- searches **GOG's registry entries and the usual GOG install folders** for *Jedi Knight*, and
  pre-fills the result;
- lets you **Browse**, paste the folder that contains `JK.EXE`, or **drag the folder onto the
  window**, and checks it before enabling Install;
- copies the port, the film helper and the docs to `%LOCALAPPDATA%\Programs\bevi-knight`
  (per-user, so no admin rights and no changes to the game install);
- saves the game path in `game-path.txt` beside the program, adds a Start Menu shortcut (and a
  desktop one if ticked), registers an **Apps & Features** uninstall entry, and can launch the
  game when it finishes.

Uninstall from **Settings → Apps → bevi-knight**, or run `bevi-knight-uninstall.exe` in the
install folder. Two switches make the setup scriptable:

| switch | effect |
|---|---|
| `/S /GAMEPATH="<folder>" /DIR="<folder>" /NOSHORTCUTS` | silent install (omit `/GAMEPATH` to auto-detect) |
| `/uninstall` | starts the installed uninstaller |

The setup is unsigned, so Windows SmartScreen may warn the first time (More info → Run anyway);
a code-signing certificate is what removes that. The game itself finds the install the same way
when the setup is not used: `--game` first, then `BK_GAME`, then `game-path.txt` (beside the
program or in `%APPDATA%\bevi-knight\`), then the GOG registry, then the usual GOG folders. Two
setup flags answer questions without loading the game:

| flag | what it does |
|---|---|
| `--find-game` | prints the install path it would use, and how it found it |
| `--set-game <path>` | checks a path and saves it for later launches |

## Run it

```sh
cargo run --release --            # no path needed once one is installed or saved
cargo run --release -- --game "G:\Games\Star Wars Jedi Knight - Dark Forces 2"
```

That boots the retail main menu. `New Game` plays the campaign's opening film and starts
the first level of the first single-player episode; a level that finishes plays its own
cutscene and loads the next, and the light-side chain reaches the finale film. Everything a
player touches is the shipped data: the levels, the scripts, the menus, the HUD, the sounds
and the films. Audio is on by default; `--mute` forces silence.

The whole light-side chain has also been walked **by the app itself**, without a player:
`--tour-live` finishes each level on a timer, skips its films by name and loads the next,
and reports `21 level(s) finished and swapped, through the end at 21ajerec.jkl`. What a
person has not done is *play* levels 2–21 (see below).

Useful flags while evaluating (each is also a gate):

| flag | what it shows |
|---|---|
| `--gates` | every behaviour check, one process each — **61 gates, 0 failures** |
| `--tour` | every campaign level row on both arms: loads it, runs its scripts, checks its film(s) and successor |
| `--tour-live` | the running app walks the campaign: 21 levels finished and swapped, ending at `21ajerec.jkl` |
| `--start <level>` | start inside a level, skipping the menu |
| `--campaign` | walk the episode graph and print where it goes |
| `--items` | the `misc\items.dat` table and which bins name a script |
| `--force-economy` | the recovered force-point model |
| `--force-grant --start 03katarn` | a level granting its own power, and a point being spent |

## What works

New Game plays the opening film, then the cantina. Move, look, jump, crouch-speed, fire,
take damage, die and respawn; six weapons with their own scripts, ammo and view models; force
powers granted by the campaign's own episode rows, held and spent through the pause menu's
Force Powers screen; actors that see you, chase you through doorways and shoot with their
class's own accuracy; pickups, secrets, sectors, doors, elevators and scripted set pieces;
save/load in the engine's own `.jks` container with eight slots and autosaves at story beats;
the menus, the HUD, the map and objectives panels; cutscenes decoded through the install's
own `SMACKW32.DLL`. The campaign's mid-game light/dark decision plays both of its films in
order and continues on the light-side arm.

## What is deliberately rough or not there

- **No human has played the campaign end to end yet.** The chain is gated level by level and
  end to end (`--tour`: 28 campaign rows on both arms for 25 level files, 23 film hops, two
  finale rows, 0 script errors; `--tour-live`: the app itself finishes and swaps all 21
  light-side levels), but objectives on levels 2–21 have not been played through by a person.
  That is the next release gate, and the main caveat on "0.1".
- **Two-film playback is checked by unit test and was watched once by hand, not by a gate.**
  The mid-campaign decision plays `38a.smk` then `39a.smk` before the next level; a gate that
  sat through all 23 film hops would be too slow, so `--tour-live` names each film and skips it. The
  queue's ordering is pinned by `film_step` tests.
- **The dark-side arm is unreachable.** The campaign's decision row after level 15 takes the
  neutral (light) arm because the choice screen is not built. The dark levels (`16b`, `17b`,
  `20b`, `21b`) parse, load and chain — `--tour` walks that arm row by row to `21bjerec` and
  its `57a.smk` finale — but no UI selects them. The **ending screen** is likewise absent:
  the ending-choice inventory bin stays neutral.
- **Multiplayer.** No networking, no protocol, no dedicated server. The roadmap scopes
  single-player on purpose.
- **A real instinct scheduler.** AI honours `PrimaryFire` — delay, facing cone, ranges, aim
  error, bursts — and routes through doorways, but there is no instinct scheduler, no
  hearing or alert propagation, and no reaction time before the first shot.
- **Health and shields carry between levels** since 0.1.1; inventory, force powers, rank and the
  ending choice always did.
- **The save-slot picker.** Saving and loading work in the engine's own `.jks` container, and
  eight slots exist — `,` and `.` step between them, `F5` writes and `F9` reads the current
  one — but retail's slot-list screen is not built.
- **Force powers beyond the gated six.** Jump, speed, healing, seeing, pull and grip run
  their shipped scripts and are checked; the others have scripts and verbs but no check that
  they change anything. Grip's disarm branch is simplified.
- **Animation is partial.** Walk/run/fire/hit/turn/death play; crouch, alt-fire, saber swing
  and layered upper/lower body do not, and the view weapon's exact retail framing is derived
  rather than recovered.
- **Level 1's first frame starts inside cantina furniture** (a table 0.16 units from the eye,
  a patron 0.15 — [first-frame-level1.png](first-frame-level1.png) is what that looks like).
  Whether retail opens the same way is unsettled and needs a retail reference frame;
  near-plane clipping is fixed, the blocked view is not.
- **Surface attachment.** Thing-to-thing parenting and landing are the engine's;
  `sithPhysics_UpdateAttachedThingPhysics` is still stood in for by `update_attachments`.
- **Cutscene palette.** 29 of 768 palette bytes still differ from libsmacker's decode.

[docs/gaps.md](gaps.md) is the unit-by-unit account with the evidence for every row;
[docs/roadmap.md](roadmap.md) is the dependency-ordered remainder;
[docs/progress-log.md](progress-log.md) is how each piece was recovered.

## What changed since the earlier 0.1 candidate

- **It finds your install, and there is a native Windows setup.** The game resolves `--game`,
  `BK_GAME`, a saved `game-path.txt`, GOG's registry entries and the usual GOG folders;
  `--find-game` shows the result and `--set-game "<path>"` saves one. `packaging\build-setup.ps1`
  builds one self-contained `bevi-knight-<version>-setup.exe` (a WinForms wizard compiled with
  the compiler Windows already has) that finds the GOG install, accepts a browsed/pasted/dragged
  game folder, installs per-user to `%LOCALAPPDATA%\Programs\bevi-knight`, creates shortcuts and
  an Apps & Features entry, and carries its uninstaller — no admin rights, and the game install
  is not modified.
- **The HUD message bug is fixed.** `jkPrintUNIString(player, key)` was reading its first
  argument (the player's thing index) instead of its second (the `.uni` key), so level 1's
  first pickup printed `COG_00000` across the screen instead of "Shield Recharge". The verb
  now reads the key argument, with a unit test on both the two-argument and legacy forms.
- **Audio is on by default.** A play session is audible at `BK_VOLUME` (default 0.75, the
  engine's own mixer scale); captures and probes stay silent unless `--sound-on` is passed,
  and `--mute`/`BK_MUTE` always wins. `--audio-state` asserts all three decisions.
- **The campaign now runs past level 2.** `swap_level` resolved the successor against a bare
  `episode.jk`, which every archive holds (`JK1`, `JK1MP`, `JK1CTF`) and which therefore
  returned the multiplayer manifest; the app called level 2 "end of campaign" and stopped.
  It now resolves the single-player episode explicitly. `--tour-live` is the gate over that.
- **Per-level completion state resets with the level.** `queue_next_level`'s "already asked"
  flag and `watch_level_complete`'s announced/tallied flags were process-wide, so only the
  first level of a session would end, show its tally and credit rank/stars. Each now resets
  when the next level starts.
- **The opening film no longer starts behind the main menu.** It used to be inserted at boot,
  hid the menu, and was never removed; it now waits for `New Game` (`overlay.started`) and a
  finished intro is retired.
- **The campaign decision's transition plays both films.** `Episode::flow_after` now walks
  through the `DECIDE` row and keeps every film in order (level 15: `38a.smk` then
  `39a.smk`), takes the neutral arm (light), and only swaps the level when the queue is
  empty; a two-film transition can no longer be cut short.
- **`--tour` checks both arms by row**, not by file name: 28 campaign rows for 25 distinct
  level files, every film and successor against the archives, both finale rows, and the
  decision's two films in order. `--tour-live` walks the same chain through the running app.
  61 gates and 133 tests now pass on this install.
- **The project is explicitly GPL-3.0-or-later** (`LICENSE` + `NOTICE` + `Cargo.toml`), and the
  notice spells out that the game, its marks and the documentation screenshots are not licensed
  here — no game data ships with it.
## Verified on this install

Everything below was run against the retail data while this note was written, not remembered
from an earlier build.

```
--gates            0 of 61 failed, 0 skipped   (windowed; BK_HEADLESS=1 reads 0 of 61 failed, 9 skipped)
cargo test --bins  133 passed
--tour             28 level row(s) on both arms (25 distinct files), 5543 things, 23 films, 2 terminal rows, 0 script errors, 1 sound missing from retail data
--tour-live        21 level(s) finished and swapped, through the end at 21ajerec.jkl
--audit            540 cogs parsed, 0 failed, 1854 labels, 7683 symbols; summary 35/35 levels clean, 10 unresolved asset references, 0 non-drawable models, 0 manifest LEVEL rows missing, 57 undecoded files
--walk all         0 level(s) left the body in the void, worst drop 0.00
--campaign         32 level step(s) walked, 30 distinct level file(s)
--ends             28 of 35 level(s) name a script that calls jkEndLevel; the rest are m*/static
--goal             objective completed, level complete in ~200 frames, next 02narshadda.jkl
--save-slots       2 slot file(s), distinct true, both read back
--force-live       the same press lifts the body at level 1 and at level 4; gate passes
--audio-state      on at gain 0.750 for a play session; muted for a capture
```

Also checked by hand on this build: normal launch shows the main menu and plays no film;
the `New Game` action plays `03-04a.smk` (2500 frames) and drops into the cantina; a level
15 completion was watched through `38a.smk` → `39a.smk` → `16aEscapeShip.jkl loaded`. The
native setup was built and run silently to a temporary folder both with an explicit game path
and with auto-discovery (it found the GOG install through the registry), the installed
executable then found the game through its saved `game-path.txt` and passed `--check`, its
Start Menu shortcut was created and removed, the GUI opened with the right game pre-filled,
and the uninstaller removed the folder, shortcut and Apps & Features entry.

The two screens a player spends the most time in are photographed from the running build:
[force-powers.png](force-powers.png) (a power's level drawn beside its icon) and
[tally.png](tally.png) (three stars earned at the end of level 1).

## Patch 0.1.1

- **Health and shields now cross level changes.** Finishing a level used to rebuild the player at
  the new level's template values; they are now part of `Runtime::carry`/`adopt`, clamped to the
  new level's maximum. Pinned by
  `runtime::tests::health_and_shields_cross_a_level_change`; the suite is 133 tests.

## License

The source, tools and the setup build scripts are **GPL-3.0-or-later**; see `LICENSE` and
`NOTICE` in the repository root. Rebuild it, study it, change it, redistribute it — including
selling copies — as long as the source and the same freedoms go with it. Closed-source forks
and proprietary repackaging are not allowed. Contributions are accepted under the same license.

No game data is licensed or included. *Star Wars Jedi Knight: Dark Forces II* and everything in
it belongs to Lucasfilm/Disney; this is an unaffiliated reconstruction that reads the retail
install the player owns. The screenshots under `docs/` show retail artwork, are documentation,
and are not covered by the GPL. Binary releases (including the setup) should be offered
alongside the corresponding source: that is this repository, which `Cargo.toml` names. The
setup also installs `LICENSE` and `NOTICE` beside the game.

