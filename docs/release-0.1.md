# bevi-knight 0.1 — what you can play, and what is still rough

A single-player reconstruction: it reads the retail `.gob` archives and `JK.EXE`'s own
tables, and does not need the original engine, a cover mount or a crack.

## Run it

```sh
cargo run --release -- --game "G:\Games\Star Wars Jedi Knight - Dark Forces 2"
```

That boots the retail main menu. `New Game` starts the first level of the first
single-player episode and the campaign runs from there; a level that finishes plays its
own cutscene and loads the next. Everything a player touches is the shipped data: the
levels, the scripts, the menus, the HUD, the sounds and the films.

Useful flags while evaluating (each is also a gate):

| flag | what it shows |
|---|---|
| `--gates` | every behaviour check, one process each — **56 gates, 0 failures** |
| `--start <level>` | start inside a level, skipping the menu |
| `--campaign` | walk the episode graph and print where it goes |
| `--items` | the `misc\items.dat` table and which bins name a script |
| `--force-economy` | the recovered force-point model |
| `--force-grant --start 03katarn` | a level granting its own power, and a point being spent |

## What works

Move, look, jump, crouch-speed, fire, take damage, die and respawn; six weapons with
their own scripts, ammo and view models; force powers granted by the campaign's own
episode rows, held and spent through the pause menu's Force Powers screen; actors that
see you, chase you through doorways and shoot with their class's own accuracy; pickups,
secrets, sectors, doors, elevators and scripted set pieces; save/load in the engine's own
`.jks` container; the menus, the HUD, the map and objectives panels; cutscenes decoded
through the install's own `SMACKW32.DLL`.

## What is deliberately not there

- **Multiplayer.** No networking, no protocol, no dedicated server. The roadmap scopes
  single-player on purpose.
- **The save-slot picker.** Saving and loading work in the engine's own `.jks` container, and eight
  slots exist — `,` and `.` step between them, `F5` writes and `F9` reads the current one — but
  retail's slot-list screen (a name field and a Delete row) is not built, so a slot is chosen with a
  key rather than from a list.
- **A real instinct scheduler.** AI honours `PrimaryFire` — delay, facing cone, ranges,
  aim error, bursts — but the other instincts in a class, and their weights, are parsed
  and ignored.
- **Surface attachment.** Thing-to-thing parenting and landing are the engine's;
  `sithPhysics_UpdateAttachedThingPhysics` is still stood in for by `update_attachments`.
- **Cutscene palette.** 29 of 768 palette bytes still differ from libsmacker's decode.

[docs/gaps.md](gaps.md) is the unit-by-unit account with the evidence for every row;
[docs/roadmap.md](roadmap.md) is the dependency-ordered remainder;
[docs/progress-log.md](progress-log.md) is how each piece was recovered.

## Verified on this install

Everything below was run against the retail data while this note was written, not remembered
from an earlier build.

```
--gates            0 of 58 failed        (one child process per check, every row a verdict)
cargo test --bins  122 passed
--audit            540 cogs parsed, 0 failed, 1854 labels, 7683 symbols
--walk all         0 level(s) left the body in the void, worst drop 0.00
--campaign         32 level step(s) walked, 30 distinct level file(s)
--ends             28 of 35 levels name a script that calls jkEndLevel; the rest are m*/static
--goal             objective completed, level complete after 203 frames, next 02narshadda.jkl
--save-slots       two slot files, distinct true, both read back
--force-live       rose +0.60 at level 1, +0.71 at level 4 with the same press
```

The two screens a player spends the most time in are photographed from the running build:
[force-powers.png](force-powers.png) (a power's level drawn beside its icon) and
[tally.png](tally.png) (three stars earned at the end of level 1).
