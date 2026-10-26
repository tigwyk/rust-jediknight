# bevi-knight 0.1 — what you can play, and what is still rough

A single-player reconstruction: it reads the retail `.gob` archives and `JK.EXE`'s own
tables, and does not need the original engine, a cover mount or a crack. **No game assets
are shipped with it** — you need your own retail install (the GOG build is what this was
developed against, because its `JK.EXE` is plaintext).

## Build it

Windows x64, a Rust toolchain with edition-2024 support (1.85 or newer) and the MSVC
target, and a retail install of *Star Wars Jedi Knight: Dark Forces II*.

```sh
cargo build --release
```

Films need one extra step: the install's `SMACKW32.DLL` is a 32-bit image, so the port
decodes films in a small i686 helper. Without it the game runs but skips cutscenes with a
message.

```sh
cargo build --release --target i686-pc-windows-msvc --manifest-path tools/smkdec/Cargo.toml
```

## Run it

```sh
cargo run --release -- --game "G:\Games\Star Wars Jedi Knight - Dark Forces 2"
```

That boots the retail main menu. `New Game` plays the campaign's opening film and starts
the first level of the first single-player episode; a level that finishes plays its own
cutscene and loads the next, and the light-side chain reaches the finale film. Everything a
player touches is the shipped data: the levels, the scripts, the menus, the HUD, the sounds
and the films. Audio is on by default; `--mute` forces silence.

Useful flags while evaluating (each is also a gate):

| flag | what it shows |
|---|---|
| `--gates` | every behaviour check, one process each — **60 gates, 0 failures** |
| `--tour` | every campaign level: loads it, runs its scripts, checks its film and its successor |
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
  end to end (`--tour`: 25 level files, 28 campaign rows on both arms, 22 films, 0 script
  errors), but objectives on levels 2–21 have not been played through by a person. That is
  the next release gate, and the main caveat on "0.1".
- **The dark-side arm is unreachable.** The campaign's decision row after level 15 takes the
  neutral (light) arm because the choice screen is not built. The dark levels (`16b`, `17b`,
  `20b`, `21b`) load and chain, but no UI selects them. The **ending screen** is likewise
  absent: the ending-choice inventory bin stays neutral.
- **Multiplayer.** No networking, no protocol, no dedicated server. The roadmap scopes
  single-player on purpose.
- **A real instinct scheduler.** AI honours `PrimaryFire` — delay, facing cone, ranges, aim
  error, bursts — and routes through doorways, but there is no instinct scheduler, no
  hearing or alert propagation, and no reaction time before the first shot.
- **Health and shields do not carry between levels.** Inventory, force powers, rank and the
  ending choice do.
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

## Changes since `v0.1.0`

- **The HUD message bug is fixed.** `jkPrintUNIString(player, key)` was reading its first
  argument (the player's thing index) instead of its second (the `.uni` key), so level 1's
  first pickup printed `COG_00000` across the screen instead of "Shield Recharge". The verb
  now reads the key argument, with a unit test on both the two-argument and legacy forms.
- **Audio is on by default.** A play session is audible at `BK_VOLUME` (default 0.75, the
  engine's own mixer scale); captures and probes stay silent unless `--sound-on` is passed,
  and `--mute`/`BK_MUTE` always wins. `--audio-state` asserts all three decisions.
- **The campaign decision's transition plays both films.** `Episode::flow_after` now walks
  through the `DECIDE` row and keeps every film in order (level 15: `38a.smk` then
  `39a.smk`), takes the neutral arm (light), and only swaps the level when the queue is
  empty; a two-film transition can no longer be cut short.
- **`--tour` is the new release gate:** every campaign level file is loaded, its placed
  scripts run against a live world, and every film and successor it names is checked against
  the archives. 60 gates and 129 tests now pass on this install.

## Verified on this install

Everything below was run against the retail data while this note was written, not remembered
from an earlier build.

```
--gates            0 of 60 failed, 0 skipped   (windowed; BK_HEADLESS=1 reads 0 of 60 failed, 8 skipped)
cargo test --bins  129 passed
--tour             25 level file(s) (28 campaign rows, both arms), 22 cutscenes, 0 script errors, 1 sound missing from retail data
--audit            540 cogs parsed, 0 failed, 1854 labels, 7683 symbols; summary 35/35 levels clean
--walk all         0 level(s) left the body in the void, worst drop 0.00
--campaign         32 level step(s) walked, 30 distinct level file(s)
--ends             28 of 35 level(s) name a script that calls jkEndLevel; the rest are m*/static
--goal             objective completed, level complete after 203 frames, next 02narshadda.jkl
--save-slots       2 slot file(s), distinct true, both read back
--force-live       rose +0.63 at level 1, +0.71 at level 4 with the same press
--audio-state      on at gain 0.750 for a play session; muted for a capture
```

The two screens a player spends the most time in are photographed from the running build:
[force-powers.png](force-powers.png) (a power's level drawn beside its icon) and
[tally.png](tally.png) (three stars earned at the end of level 1).
