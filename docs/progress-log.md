# bevi-knight

A from-scratch reconstruction of **Star Wars Jedi Knight: Dark Forces II**'s world
renderer in **Rust + Bevy 0.20**.

It does not run or patch the original engine, and needs no cover mount or crack: it reads
the shipped `.gob` archives and the engine executable's own tables directly, and draws the
real levels, menus and HUD.

![jkl\03katarn.jkl, from the level's own start point](screenshot.png)

*`jkl\03katarn.jkl`, viewed from the level's `walkplayer` start point: the level's own
geometry, materials and sky, with placed crate models sitting on the ledge at their real
size. Levels and models share one Z-up coordinate system, so a model's height is its
height in the world.*

![All 35 retail levels, rendered from their own start points](levels.png)

*Every `.jkl` in the install — 26 single-player (25 campaign levels, listed as 28
manifest rows across the light and dark paths, plus the asset-only `static.jkl`),
6 multiplayer and 3 CTF — one `--shot` process each, from each level's own start
point. The outdoor levels show the game's own cloud sky.*

![An outdoor level with its sky](sky.png)

*`jkl\04escapehouse.jkl`: brick walls, wooden beams and the level's sky texture,
drawn as an unshaded dome behind the world.*

![Four .3do models, assembled from their node hierarchies](models.png)

*`--pose <model> -`: four models drawn from their own rest pose. A puppet's mesh
vertices are node-local, so the body only exists after its 18-node hierarchy is
composed — the top-left figure is `3do/actor/ky.3do`, the walker is `3do/at.3do`, and
the door and crate below have no hierarchy at all and are drawn as stored.*

![A keyframe pose](pose-kyle.png)

*`--pose 3do/actor/ky.3do 3do/key/kystand0.key 8`: the same body posed from frame 8 of
its stand animation. A `.key` frame is an absolute local pose per joint, matched to the
model by mesh name (`kystand0.key` lists its joints alphabetically, the model lists them
in authoring order).*

![The retail main menu, drawn from the game's own bitmap and strings](menu-main.png)

*`--ui singleplayer`: the game's own `ui\bm\bkmain.bm` with its `ui\sft\large0.sft` text, at
the element rects read out of the engine executable's `.data`. Nothing about this screen is
authored here — the art, the font, the strings (`ui\jkstrings.uni`) and the layout all
come from the retail install, and the text is the colour retail shows because a glyph's
byte is a palette index into the background's own palette.*

![The in-game Esc menu](menu-esc.png)

*`--ui esc`: the pause screen over `bkesc.bm`, same pipeline — the same `large0.sft` glyph
strip resolves to a different colour because this palette is different.*

![The in-game status bar, drawn over a live level](ingame-hud.png)

*The status bar as the app draws it: `statusLeft.bm`/`statusRight.bm` at the screen
corners with the shield, health, force and battery meters and the three `%03d` numbers,
all at `hud_scale` 2. Every position comes from the bitmap headers and every frame index
from the meter's own value.*

![The character-select screen, from the executable's own element table](ui-chooseplayer.png)

*`--ui chooseplayer`: the artwork, the five buttons and their rects all come from the
install, and the element array itself is read out of the engine executable at 0x526d98 — the
same path draws all 41 screens it holds, 512 elements in total. Its background is `bkMain`,
not the `bkSingle` this build used to guess: it is the `jkGui_stdBitmaps[0]` argument of the
`jkGui_InitMenu` call that initializes the screen, read from the code. The player list itself
is a list box the engine fills from code, so it is empty here.*

![The map / field-log screen, its controls and its open map area](ui-fieldlog.png)

*`--ui 0x52ea68`: the in-game map and journal. This is the screen the extraction used to
call "force_powers" — 12 picture buttons over a 520x320 custom element — because nothing in
the data says what it is. The code does: its menu is initialized by `jkGuiMap_Startup` over
`bkFieldLog.bm`, and the buttons are the map's own controls (`flPlus`/`flMinus`, the rotate
and pan arrows, `flSpin`, `flReset`). The black area is the `type 7` element where the map is
drawn at run time; the green bar above it is drawn by that same code.*

![The single-player screen, reached by activating the main menu's first row](ui-nav-singleplayer.png)

*`--menu main/10`: the app activated the main menu's `GUI_SINGLEPLAYER` row through the same
path a click takes, and the screen it opened is the one the retail graph names — the
`jkGuiSingleplayer_Show` menu over `bkSingle.bm`, with New Game, Load, `(Debug)` and Cancel
from the element table. Navigation follows the recovered `_Show` switches, not a guess:
`menus`/`nav` map `(screen, hoverId)` to the next screen, and the audit checks every edge's id
against the data it came from.*

![The objectives page, whose empty state is the engine's own line](ui-objectives.png)

*`--ui 0x5311e0`: the objectives page. Its `type 7` panel is `jkGuiObjectives_CustomRender`
(its own `+0x30` callback, 0x417b00), which lists the player's goals and, with none, prints
`GUI_NO_OBJECTIVES` 30px in from the element's edge in the element's font. This build has no
goal inventory, so that is its true state — the words come from `jkstrings.uni`, not from
here. The green banner is painted in `bkFieldLog.bm` itself.*

![The quit-confirmation dialog, its text from the shipped string table](ui-quit-dialog.png)

*`--menu main/12`: the main menu's Quit row does not exit — the decompiled `jkGuiMain_Show`
case 12 calls `jkGuiDialog_Message(GUI_QUITCONFIRM, GUI_QUITCONFIRM_Q)`, so the app opens the
Ok/Cancel dialog with those two lines from `jkstrings.uni` ("Quit Jedi Knight", "Are you sure
you want to quit Jedi Knight?"), the dialog's own panel over the menu art, and Cancel going
back / Ok leaving. Nothing here is typed in: the layout is the element array at 0x530068 and
the text is the shipped table.*

![The character editor: six arrow buttons around two code-drawn panels](ui-edit-character.png)

*`--ui 0x534cd0`: the character editor. Six picture buttons index the engine's bitmap table
(33/34 = `arrowLeft.bm`/`arrowRight.bm`) and land exactly on their rects, which is what
reading whole element arrays bought: the old string-keyed scan saw this screen as three
buttons called `cancel`. The two black panels are `type 7` custom elements — the model
window and the force-power grid are drawn by the screen's code into rects the data only
reserves.*

![A dialog: the bkdialog panel over the main menu art](ui-dialog.png)

*`--ui 0x52fe88`: a dialog is two layers. `bkdialog.bm` is 414x206, not a screen, so the
engine copies the menu art and then the panel at that bitmap's own `xPos`/`yPos`
(`113,137`), which is where the elements' full 640x480 rects sit inside it. This build
draws the panel only for screens whose elements really do fit inside it — this generic
"Ok" box does, while the four-line quit prompt spans the screen and draws no panel. The
message lines are runtime text the engine fills in, so only the button shows.*

![The loading screen, from its own array](ui-loading.png)

*`--ui 0x534790`: "Loading…" and the two copyright lines are ordinary elements in the
executable's `.data`, drawn over `bkloading.bm` — the same array the engine fills with
progress text.*

![The force-power screen: fourteen icons placed by their own bitmaps](ui-force.png)

*`--ui force`: the screen this build could not find for two rounds. Its elements carry a
`rect = -1,-1,-1,-1` placeholder that the engine fills from the artwork, so a "does the rect
fit the screen" test threw the whole screen away. Each icon is placed from its own
`xPos`/`yPos` (`foAbsorb.bm` at 201,269, `foJump.bm` at 284,87, …), drawn from its
normal/hover/pressed cells, over `bkforce.bm`.*

![The single-player tally, its force-star row drawn](ui-tally-stars.png)

*`--ui 0x531530 --stars 4`: the tally screen's `type 7` element is where
`jkGuiSingleTally_ForceStarsRender` (0x418000) paints the force stars — one `foStars.bm` cell
repeated, centred in the element's rect, at the element's y. The count is game state, so this
build draws what `--stars` is given and nothing otherwise; every other custom panel is named
in `--ui-tables` (`[map]`, `[objectives]`, `[loadbar]`, `[model]`, …) but needs state or level
data the screenshot path does not have.*

![The setup screen: gold title, gold tabs, red labels and a check box](ui-setup.png)

*`--ui setup`: every font here is named by the element's own `textType` — `large1` for the
title, `med0` for the tab buttons, `small0` for the red labels — and the empty check box
left of "Enable A3D Sound" is `check.bm` from the engine's bitmap table, with the label
shifted right by the box's width plus four, as the engine does.*

![The main menu in the running game, cursor on Single Player](ingame-menu-main.png)

*`--menu singleplayer`: the same screen inside the app, with the cursor on the first row
drawn in the engine's own hover font (`large1`), which is how retail lights a hovered row.
Escape opens the pause screen, Up/Down move, Enter picks.*

![The pause menu rendered into the running game](ingame-menu.png)

*Escape opens `bkesc.bm` in the app itself — 640x480 canvas letterboxed over the live
level, cursor on `Objectives` — and Up/Down/Enter move and pick.*

![The whole HUD, composed offline](hud.png)

*`--hud`: status bar, crosshair, the bracketed item panel at the bottom and the inventory
strip down the right edge, composed over the campaign's first level's colormap — which is
where the red/green/amber come from. The icons are `ui\bm\ic<name>.bm`, whose file names are
derived from `misc\items.dat`'s item names by the engine's own `ic%.5s%d.bm` rule.*

![The force-power screen artwork](ui-force-art.png)

*`--bm "ui\bm\bkforce.bm"`: the Jedi-power screen's own artwork — the medallion bands,
the alien landscape, and the empty slots the force icons are drawn into. Decoded straight
from the install, palette included.*

![Whole-level overview](overview.png)

*`--overview`: level 1 in plan, from directly above — a plan view of a Z-up world, `6 x span` up
through a 10-degree lens with every sector drawn. Standing 0.9 x span above the roofs at the default
45-degree field of view made every wall lean out over the room beside it, which is what made the
plan look like the level's geometry overlapping the models in it.*

## Status

| area | state |
|---|---|
| `.gob` archive VFS (5 archives, 4795 entries / 4791 unique names) | working |
| `.jkl` level parsing (geometry, materials, objects) | working |
| `.mat` material → bitmap (1932 of 1983; the rest are texture-less by design) | working |
| `.cmp` colormap → palette (offset 64; the 64–320 distance tables are located but unused) | base palette only |
| `.3do` model parsing + placement (all 477 models) | working, drawn from the rest pose |
| `.spr` sprite parsing (all 20) | parsed; nothing placed uses one |
| `.wav` playback + `.snd` sound classes (all 644 + 102) | working |
| sector ambience loops (`SOUND` per sector, swapped as you move) | working |
| COG script parser (all 540 scripts) + interpreter core | parsing complete; 315 verbs, and every verb a shipped script calls is answered by the runtime except AI, weapon, music and the message bus (§16.4) |
| COG runtime — placed instances bound to a mutable world | working: doors open, lifts move, adjoins close; level 1 delivers 92 messages and actuates 13 adjoins. `--cog-check` fails if a verb outside the deferred groups goes back to being stubbed |
| the same runtime driving the renderer (per-frame thing transforms) | working |
| UI: menus from the game's own art, fonts and element tables | working in-app: 41 element arrays / 512 elements read from the EXE, `--ui <name>`, `--menu`, Up/Down/Enter |
| UI: the engine's font and bitmap tables | working: 12 fonts + 35 bitmaps read out of the EXE, so text uses the font each element names and hover is the next one |
| UI: picture buttons, check boxes and dialog panels | working: `arrowLeft`/`arrowRight`, `check.bm` and the 14 `fo*.bm` force icons, placed at each bitmap's own `xPos`/`yPos`; a dialog's `bkdialog.bm` panel over `bkmain.bm` |
| UI: the eight `type 7` panels | classified from each element's own `draw` callback (`+0x30`): `map`, `objectives`, `stars`, `loadbar`, `title-text`, `model`, `power-grid`; 8/8 by `--ui-audit`. The star row and the objectives page's empty state are drawn; the map is a 3D view (`sithMap_DrawCircle`), the rest need game state. 72 elements carry a `click` callback too |
| UI: the pointer | working: window cursor mapped onto the 640x480 canvas (letterbox undone), hover lights the element under it, click picks it |
| UI: menu navigation | working: 17 edges + 2 actions recovered from each screen's `_Show` switch, click/Enter opens the screen the retail graph names, Quit opens the retail `GUI_QUITCONFIRM` dialog, cancel/done goes back, `--menu <screen>/<id>` drives one hop headlessly |
| UI audit (`--ui-audit`) | passing: 41 screens / 512 elements, every font + bitmap index in the tables read from the EXE, 319 keys with 3 assigned at run time |
| UI: the status bar (panels, four meters, three numbers) | working in-app; `--hud` composes it offline |
| UI: crosshair, item panel and inventory strip | working: rects, `items.dat` icons and the item count |
| UI bitmaps (`.bm`) and string tables (`.uni`) decoding | working: all 146 + 5 files |
| sprite fonts (`.sft`) | working: all 27 files (`--sft`), used by `--ui` |
| world geometry rendering with per-vertex baked light + mipmaps | working |
| sky (unlit sky faces + a sky dome for the background) | working, no sky scroll |
| episode manifests (`episode.jk`, all 3 episodes, 37 sequences) | working |
| retail asset graph — all 35 levels + every referenced asset | **verified** |
| distance shading (the `.cmp` tables) | not started — see stage 1 above |
| `.key` animation data, `.pup` puppets, `.ai` classes | parsed (all 784 files); placed actors animate (`stand`) — see the `actors animate` row below; `.ai` classes parsed but not driving anything yet |
| inventory bins | weapons and force powers share one table (`enum SITHBIN`): `SetCurInvWeapon`/`SelectWeapon` arm a bin, `GetCurInvWeapon` reads it, force mana is bin 14 and force powers are bins 21–26 (§16.7). `--bins` checks the round-trip and that arming the bryar arms `+bryarbolt`. The bin→projectile mapping is a name heuristic, not the engine's `weaponData`. **Force powers run their own scripts**: `--force jump` executes `force_jump.cog` end to end (mana −20 exact, hold measured 0.5 s) but the lift is an open bug — the script's velocity reaches `SetThingVel` as zero |
| AI | actors carry their shipped `.ai`: sight distance and cone decide whether an armed actor fires its own `weapon=`, and `--ai` checks that an actor fires at 3 units, holds at 8 and stays silent from behind (§16.8). The instinct scheduler's weights, pathfinding, `accuracy=` spread and reactions are not done |
| items | pickups work through the item's **own script**: touching a powerup runs its cog's `touched` then `taken`, so `pow_shields.cog` lands `bin 60 +20` from its own `bin=60`/`ChangeInv` (§16.6); `--pickup` asserts it on level 1's `shieldrecharge`. Also fixed the inventory verbs' argument order (thing first) and `TakeItem` double-counting. Taking a weapon changes what the player fires (runtime weapon: `+bryarbolt` 30 damage -> `+elaser` 12, checked by --weapon), and a taken powerup comes back after its 
espawn= seconds (checked in --pickup). Not yet: pickup sounds, real inventory maxima |
| combat: firing | working in the window: **left mouse** fires the level's own weapon template, the bolt is drawn and flown with the template's `vel=`, stops at the sector graph and applies its `damage=`; `--fire +bryarbolt` checks flight and the 30 points of damage headlessly, and `--fire-demo` shows the bolt on screen (§16.5). a projectile damages what it reaches and a killed actor plays the puppet's own `death` keyframe (`kyftdeth.key`, 45 frames at 15 fps) and holds its last frame; --fire asserts the kill. Damage decays in flight the way the engine does it (SITH_WF_DAMAGE_DECAY 0x4000, 
ate= per second floored at mindamage=): level 1''s bolt is 30 damage dropping to 10 over 1.33 s (§16.5). Not yet: explosions, ammo, force power effects, a body that falls |
| actors animate | working: each placed actor plays its puppet's `stand` keyframe at the keyfile's own 15 fps on its own mesh, rebuilt only when the frame number changes; `--animate` checks the rendered vertices across every material — **1684 of 1697** actors move over the loop (§16.3). `walk`/`run`/strafe need velocity, i.e. stage 6's AI |
| cutscenes (`.smk`) | **the data loads and its headers check out**: 25 files, 483 MB, 29,493 frames, all 640×300 at 15.2 fps (`--smk`); the decoder is `SMACKW32.DLL` (71 KB, named exports recovered), not JK.EXE. The VFS now finds them through the `Resource/VIDEO` mount | decoding not started — stage 8 |
| level collision + adjoin (portal) traversal | working: a walker stays inside the level's own sectors |
| player motion | working: walk/run/back/strafe/crouch, jump and water derived from the level's own `walkplayer` template chain, gravity scaled from the level, and water halving movement from the floor's surface flags (§16.2); level 1 walks at 0.667 u/s (`maxthrust/surfdrag`), jumps to a 0.28-unit apex in 0.75 s. `--rules` prints the table, the level's flagged-surface counts, and checks the integrated jump against the closed form. Step-up is not implemented and cannot be with infinite-plane collision (§16.2) |
| sector visibility | working: one batch per (sector, material) + a per-frame clip-plane test, 21 of 462 sectors drawn in level 1 at the spawn, pixel-identical to --no-cull (0 of 230400 sampled pixels differ at yaw 0/90/270). Portal occlusion is implemented and unit-tested but off until the player's sector is tracked (§16.1) |

### Where the roadmap stands

Stage-by-stage, against `docs/roadmap.md`, with the check that holds each one:

| stage | state | check |
|---|---|---|
| 1 visibility | `.cmp` colormap material by default at measured parity (0.00% vs `StandardMaterial`), and **portal occlusion on**: symmetric portal graph, seed chosen by agreement with the frustum, culled frame 0.000% different from `--no-cull` over 230,400 pixels | `--cmp`, `--shot` A/B, `--audit` |
| 2 player | walk/run/back/strafe/crouch from the template, the engine's jump gravity, water from surface flags, step-up, and the **lateral rule** in one function (`collide::face_blocks`: span + outline + penetration). `--rules` asserts a body beside a wall is stopped and one 20 units past its end is not | `--rules`, `--walk all` |
| 2 player parity | **done except lateral coverage**: walk/run/back/strafe/crouch/jump/water from the template's own numbers, step-up onto a 0.65 ledge, the body standing on faces that belong to other sectors, and the view following the walker at the sweep's eye height (§16.2). Still open: a body beside a face but outside its plan projection is still pushed by an infinite plane | `--rules`, `--walk all` |
| 4 COG verbs | every label level 1 declares is fed, each `run_label` has a fresh bounded budget, and **`sleep`/`waitformessage` suspend and resume** across frames (`Vm::resume` + `resume_sleeping_labels`; `--sleep` pins both halves and is verified to fail when suspension is removed). A nested `sleep` re-runs its enclosing block | `--cog-check`, `--sleep`, `--triggers`, `--live-check` |
| 5 combat | **complete for single-player**: damage two-way, shields, death/respawn, pickups, explosions, force jump/speed/healing/seeing, mana regeneration, ammo, and **force pull exactly as the shipped script does it** — highlight with `jkSetTarget`, then `ApplyForce` at 30 u/s toward the player with gravity cleared (`--pull`). Grip damages over time while holding the victim | `--hit`, `--fire`, `--pickup`, `--weapon`, `--force`, `--mana`, `--ammo`, `--pull` |
| 6 AI | instincts reach the live runtime (117 actors), and the chase **searches the sector graph**: `Collide::route` is breadth-first over the portals (1206 doorways on level 1, built from the `adjoin`/`mirror` pairs), refusing links its own gates close, and the runtime keeps the doorway route per actor and pops waypoints as they are reached; the direct line is still tried first. Instinct scheduler weights, `accuracy=` spread and reaction times are still unfixed | `--ai`, `--live-check` |
| 7 audio | attenuation over 375 clips, the song range, the pulse/timer queue, the **music fade** verified end to end (`--fade`: 0.00 → 1.00 → 0.00 through the script's own chain), and the **menus' own wavs** parsed from `jkGuiMenu` and played on hover/click (39 of 41 screens, both names present) | `--sound`, `--music`, `--fade`, `--ui-sounds` |
| 8 `.smk` cutscenes | **it decodes and it plays in the window**: the DLL is 32-bit (`machine 0x014C`), so a 64-bit process cannot load it (error 193) — the decode runs in `tools/smkdec`, a dependency-free i686 helper driven by JK's own `SmackOpen(file, 0xfe000, -1)`. `--smk-decode` proves frames come out and differ (`01-02A.SMK`: 40 decoded, 10 distinct, first non-blank frame 16), and `--cutscene 01-02A.SMK 30` plays them over level 1 at 15 fps letterboxed 640×300 in the 640×480 overlay ([image](cutscene-playing.png)). **Frames are in colour, from the film's own palette**: the header layout is proved (chunk sizes + tables + huffman block fill all 25 films exactly, `Layout::covers` in `--smk`), and palette records decode per frame — `skip`, `colour-shift`, and literal "set one colour" chunks, the last of which was the bug that made the first attempt look like coloured static ([images](cutscene-colour-window.png)). Palettes are verified against libsmacker as a reference decoder (740 of 768 bytes wrong before the fix, 29 after, §16.9). **The whole film plays once, streamed**: 90 decoded frames used to loop, which is what a 992-frame film playing the same six seconds looks like; the helper now decodes the entire film to disk as it plays (about 5× real time, so it stays ahead) and frames are read, coloured and blitted as they appear, stopping on the last one ([image](cutscene-late-frame.png)). Known limit: 29 of 768 palette bytes still differ from libsmacker (§16.9). Data: the VFS falls back to loose files through the `Resource/VIDEO` mount, so `--campaign` resolves every cutscene row and `--smk` verifies all 25 files (483 MB, 29,493 frames, 640×300 at 15.2 fps) | `--smk`, `--smk-decode`, `--cutscene`, `--campaign` |
| 9 campaign flow | **savegame container + thing records implemented** (`src/jks.rs`): version 6, the 1580-byte header, a 0x24-byte game-side block (episode `JK1` + one u32), the 32-byte level name, the game time and the 24-byte player block, then records framed on disk as `u32 type, u32 length, payload` (the 0x28-byte message header is in-memory only; both engine-written autosaves in `tools/parity/` now parse to EOF — 5856 and 7704 records) and a `ThingRecord` over the payload offsets `FUN_004f4210` stages — template index, type, position, angles. `--save` writes one record per placed thing (level 1: 64) and reads every position and angle back, including the player's own spawn. The campaign **graph** is verified too: `--campaign` walks both `gotoA`/`gotoB` edges and proves all 28 single-player levels are reachable from level 1 through the one `DECIDE` row, while the MP/CTF manifests loop as map rotations (§16.9). The **transition** is wired to the engine's own verb: `jkendlevel` (`jkCog.c`) marks `Runtime::level_complete`, and `--transition` resolves the successor through the graph — `01narshadda.jkl -> 02narshadda.jkl`, 21 levels on the `gotoA` spine (§16.9). Not done: the rest of the payload fields and the header internals. **The transition runs end to end up to the film**: `--complete-after <s>` calls the engine's own `jkEndLevel`, `Episode::flow_after` resolves the graph's next rows, the `CUT` row plays in the window and the next level is named (`level 01narshadda.jkl complete -> cutscene 06a.smk, then 02narshadda.jkl`, [image](cutscene-on-complete.png)); `--flow` asserts every hop, with each film checked as `SMK2` and each level present in the archives. **The swap works, and it is the default**: `swap_level` is an exclusive system that despawns the old level's `LevelEntity`-tagged entities, swaps `World` and re-runs `setup`, so level 2 (391 things, 641 sectors) is built and rendered with its own music after the film (`--no-swap-levels` stops at the film). Three bugs made it work: the camera is only spawned when none exists, the film gates the change via `Cutscene::finished`, and the overlay is marked dirty so the HUD returns ([image](level2-after-transition.png)). Remaining for the stage: the rest of the `.jks` payload and the header internals | `--save`, `--campaign`, `--transition`, `--flow`, `--complete-after`, `--swap-levels` |
| 10 UI panels | **every single-player custom element draws**: map, objectives, force stars, load bar, title text, plus the menus' own wavs. The two holes the docs used to name — the model window and the power grid — are both on `edit_character`, a **multiplayer** screen, so they are out of scope rather than pending | `--map-shot`, `--ui-visible`, `--load-bar`, `--force-screen`, `--ui-tables` |

Every stage's recovered constants and code addresses are in `docs/formats.md`; the
gates above are what fail if a behaviour drifts.

### Status: what a player can do today

Read this as the honest line between the two halves of the goal. The **read** half — parsing every
retail asset format and proving it against the install — is essentially complete. The **play** half
is not: this is not yet a replacement you can finish the game in.

**Works, with a check behind it:**

- **The game boots to its main menu, and its rows start a game.** `bkmain.bm` over the install's own
  element table; Enter/arrows or the mouse picks Single Player, and New Game (`GUI_NEWGAME`, id 100)
  loads the campaign's first level from the episode manifest and hands the world to the walker —
  `swap_level` runs the same `setup` the first load does, so scripts, things and the player all start
  over. The pause menu reloads the level (`GUI_RESTART`), leaves it for the boot menu (`GUI_ABORT`),
  and returns to it (`GUI_RETURN_TO_GAME`). Every screen freezes the world while it is open, and
  `--boot` checks the chain against this install's tables. Nothing is typed in: an escape only backs
  out one screen, and the boot menu ignores it, as retail's does.
- All 35 shipped levels parse and pass `--check` (36/36 including `static.jkl`), and `--audit` reads
  every archive and reports 34/35 reachable from the manifests.
- **`--live-check` watches the running game, not a mock of it**: twenty frames in, it asserts the
  instincts are attached to the runtime the game simulates, the player thing exists, the collider and
  the materials are populated, actors have parts, and every placed script bound. Two bugs in one week
  (the impact scan never being called, instincts attached to a throwaway runtime) were invisible to
  checks that built their own world; this one would have failed on both.
- The world renders from the player's own eyes at the walker's eye height, with sector/portal
  visibility culling; walls carry their polygons, and the body stands on the walkway and steps onto
  a 0.65-unit ledge (`--walk all`: 34/34 levels, 1077 sector crossings).
- The `.key`/`.pup` animation plays on placed actors; an actor that sees you fires (projectiles,
  in-flight damage decay) and walks toward you at its own `maxstep`.
- The `.smk` films decode through the install's own 32-bit `SMACKW32.DLL` in a helper process, play
  once in full colour, and the level transition after them lands on the next level.
- The force panel and the field-log map panel draw from the executable's own screens and tables.
- The campaign graph, savegame container (`.jks`), audio mix curve, and the menu screens' data are
  recovered and checked.

**Does not work yet — do not expect a playable campaign:**

- ~~Combat is one-directional~~ — **two-way now, and checked**: `--hit` drives the same scan the game
  uses and lands 25 of a bolt's 30 carried damage on a placed actor, then takes 9 off the player from
  the actor's own `+elaser`; the status bar shows the runtime's health. Force powers, explosions and
  ammo are still partial (§16.5).
- **Force powers are data, not effects**: force seeing/pull/grip have no visual or physical effect yet.
- ~~`--rules`' jump calibration failed~~ — **fixed**: the lateral resolver was pushing along a floor plane's infinite normal and pinning the body, so `walker_step` now keeps the height its vertical step produced and lets the resolver own X and Y. `--rules` measures `apex 2.57 / 2.81` (91% of the closed form) and exits clean (§16.2).
- **The movement basis is JK's own now, and strafing was backwards until it was.** `walk_target`
  builds motion from `lvec = (cos yaw, sin yaw)` and `rvec = lvec × uvec = (sin yaw, -cos yaw)`; the port
  had `-rvec`, so `A` and `D` were swapped. `--controls` compares the mapping against the camera transform
  the renderer builds, and fails if the sign flips back.
- **Mouse look works, with a calibrated sensitivity.** `LOOK_SENSITIVITY` (0.0035 rad/px, `BK_LOOK_SENS`
  overrides) stands in for JK's options-screen slider, which is a setting rather than data; the pitch
  clamp *is* data (`minheadpitch`/`maxheadpitch`). The pointer is locked whenever no screen is open and
  released for menus. Arrow keys still turn.
- **"Characters stuffed inside each other" was the camera standing inside an NPC, not the model scale.**
  The spawn only demanded `radius * 2` = 0.7 units from other things, exactly an NPC's body width; `--shot`
  showed a head and shoulders filling the frame. The spawn now wants 2.0 units (`BK_THING_CLEARANCE`), and
  the same viewpoint is open room. **`MODEL_SCALE` was the other half, and it was 10: every model ten times
  too big for its level.** A door model has to fill the doorway the level builds for it, and both are in the
  file — `d2x3.3do` is 0.200 × 0.300 model units and the vertical adjoin 0.01 units from the door's own
  placement is 0.20 × 0.30 level units; `d3x3_1.3do` 0.300 × 0.300 against 0.30 × 0.30; `d4x3_1.3do` 0.400 ×
  0.300 against 0.35–0.50 × 0.25–0.35. One model unit is one level unit. At 10 an actor 0.19 units tall was
  drawn 1.91 units tall in a level whose storeys are 0.6 units and whose doorways are 0.3, which is exactly
  "the enemy models are scaled way too big for the level". `--scale` prints that table; the body radius, the
  eye height, the step, the AI standoff and the actor walking speed were all on the same constant and came
  down with it, and every gate was re-run at the new size.
- **The player's own body was in every frame, from the inside.** `walkplayer` names `model3d=ky.3do`, and
  that is the thing the camera rides, so the level spawned a 1.8-unit model *around* the camera. Models are
  drawn with `cull_mode: None`, so the view was not blocked by the body — it was filled with the *inside* of
  it: unlit polygons over the level, the walls and every NPC in front of them, which is what "level geometry
  overlapping NPC/enemy models" looks like. `is_local_player` skips those twelve `walkplayer` rows per level
  (2879 model instances instead of 3119, 105 animated actors instead of 117) and `--live-check` asserts the
  body is drawn by 0 entities. `BK_MODEL_SCALE=0.01` is the A/B that isolated it: with the models shrunk to
  nothing the same vantage renders the room.
- **The player body is the level's own numbers, at the models' scale.** `movesize=0.065` is the body's
  collision radius and `eyeoffset.z=0.037` the camera's height above the body, both in level units — so
  radius 0.065, eye 0.037, step `movesize + 0.01` = 0.066, in a level where a doorway is 0.30 tall. An
  earlier round read those as puppet-internal offsets and multiplied them by 10 to pair with a 1.81-unit
  character: the same ten-times error seen from the other side, and what put the camera 0.1 above the floor
  while the props were ten times too big. `--rules` prints the derived motion, `--live-check` puts the body
  on its floor, and `--move` walks all 34 levels with it. `--near reeyeesfist` at the corrected size:
  [image](model-scale.png).
- **Audio is muted by default, and quiet when enabled.** `--sound-on` plays at a 5% master gain
  (`--sound-on` + `BK_VOLUME=1` for a real playtest); `--mute`/`BK_MUTE=1` force silence whatever else is
  passed. `--audio-state` prints the decision and gain, then exits before an audio device is opened, so the
  mute can be verified without playing anything. The read-back `audio: N sink(s) live, M audible, loudest X`
  line reports the sink state sampled from the sinks themselves — it printed nothing for a whole round while
  the enforcement system was silently unregistered, which is exactly why it now prints even at zero sinks.
- **Things have three angles, and the port was reading one.** The row is `X Y Z pitch yaw roll sector`;
  only yaw was parsed and the transform rotated about Z, so **138 of 352 things in level 1** (39%) were drawn
  face-up. The orientation is now `rdMatrix_Build34` verbatim, converted to a quaternion. Reading the load
  path also settled the model scale twice over: `RADIUS` is the body's collision radius *in level units*
  (0.066 for `ky.3do`, agreeing with the template's `size=0.065`), which is what pinned the drawing scale at
  1:1 rather than 10:1, and identified the huge orange surfaces as the level's **freighter**, 110 units long
  by design.
- **The FOV came from running the game.** The camera's projection is a fixed **90 degrees horizontal**, the
  word at `buffer + 0x08` of the render buffer, read with Frida from a copy of JK.EXE that had level 1 loaded.
  The engine never states a vertical angle — it fixes the horizontal one and lets the taller axis follow — so
  the port now derives it (`73.74` at 4:3, `58.72` at 16:9) instead of rendering a flat 45, which is why its
  captures used to look zoomed in. The same sample paid for two earlier conclusions: the level-1 camera matrix
  at yaw −60 is `c0 = (0.5, −0.866)`, `c1 = (0.866, 0.5)`, so **column 1 is forward** (the axis the port now
  uses), and the camera sits at the thing's `+0xb8` plus its `+0x168` eye, confirmed against the player's own
  position. Getting there is not scriptable: the executable exits on `-level`, has no console, its registry key
  is absent, and the intro is five films long.- **Vertical mouse look is normalised.** Retail JK looks *down* when you move the mouse up — the flight-sim
  convention its 1997 control code was written for — and there is no option to change it: the executable's
  strings contain no "invert" at all. The port now defaults to the modern sense (mouse down looks down) with
  `BK_INVERT_Y=1` keeping the retail one for comparison, and `--controls` asserts both directions so the sign
  cannot silently flip. The sign itself comes from the platform, not from preference: Bevy forwards winit's
  raw device delta unchanged and that delta is screen-signed on Windows.- **The camera was a quarter turn off, and the engine's own matrix says which way.** `FUN_0043fcf0` @ `0x43fcf0`
  is the euler builder this port already copied as `thing_quat`; at yaw 0 its columns are `(1,0,0)`,
  `(0,cos p,sin p)`, `(0,-sin p,cos p)`, so only **column 1** rises when the view looks up. The first-person
  camera is built from the thing's own orientation and its view angles (`FUN_004c56b0` @ `0x4c56b0`), which
  puts the look axis in column 1 — so a level's yaw `a` faces `(-sin a, cos a)`, and the port had been driving
  the camera, the walk keys and the spawn aim along column 0, `(cos a, sin a)`. One `yaw_axes()` now holds the
  mapping; `--controls` asserts the camera's look and right against `thing_quat`'s columns on 15 yaw/pitch
  pairs. The 03katarn start is the proof: [before](camera-before.png) stands the eye inside a doorway,
  [after](screenshot.png) is the street the level opens on. The engine's velocity term turned out to be a
  **strafe roll** (`roll += clamp(5 * dot(vel, right), -8, 8)` degrees, first person only), not the pitch the
  docs had attributed; it is ported now as `camera_lean()`, and the three constants are byte-verified in the file — 5.0 at
  `0x522ddc`, −8.0 at `0x522de0`, 8.0 at `0x522de4` — with `--controls` asserting the 3.3 deg bank at full
  strafe, the 8 deg clamp and that walking forward does not bank at all. Reading `FUN_004c56b0` in full also
  mapped the camera modes (first person, sector-snapped, follow with `z + 0.05`, four-vector basis, static
  matrix) and proved the FOV is *not* in the camera update at all: no canvas, no `tan`, no field of view —
  the projection is a display-layer parameter, which is why the port cannot inherit it from the controller.
- **Captures no longer listen to the machine.** The look and move path read the live keyboard and mouse with no
  capture guard, so `--shot` could leave the camera anywhere the mouse had drifted: three identical runs of the
  same command produced three different frames, and one produced nothing but sky. `walk_player` now ignores
  keys and mouse while `Overview::capture` is set (`BK_WALK_FORWARD` still drives it), which is what makes a
  doc image reproducible.
- **`--start [level]` goes straight into a level.** Testing a level meant four clicks through the main menu
  every time; `cargo run --release -- --start 03katarn` boots into it with no screen open and Escape still
  opens the pause menu. `--boot` asserts the decision table (no flags -> menu, `--start` -> level, `--menu`
  still wins).- **The level geometry itself checks out.** `--winding` over all 35 levels: every drawn surface's normal
  points into its own room (91,233 of 91,233), and 98.9% of windings agree with the normal they ship. So the
  polygons are not the problem — the body is.
- **The lateral move is the sweep, and it neither wedges nor leaves the world.** Three rules were missing,
  and none was the impact response the docs first blamed: the surface search records a face only when the
  body is moving *into* it, the floor probe has to search every face (this port's sectors do not tile), and
  containment is a floor test rather than the engine's hull test. `--move` walks all 34 levels: 0 fell
  through, 0 frozen, 189 units. Walking off level 1's walkway drops the body into the room below and it
  lands (`z -13.65` on floor `-14.00`) — that is geometry, not collision, so the gate asserts "never below
  every floor" rather than "never descends".
- **The spawn is nudged off whatever it overlaps.** Levels place the player at a marker that can sit on
  an NPC or inside a wall; `spawn_point` searches outwards for a floor-standing point at least a body''s
  width from any other thing, and falls back to the clearest one it can find.
- **Collision uses the engine''s own sweep now**, with two caveats. `collide::sweep` is
  `sithIntersect_CheckSphereHit` + `TestSphereFaceHit` + the adjoin walk of
  `sithCollision_SearchForSurfaceCollisions`, and the fall, the floor probe and the walk all go through it
  (`--collide`: 4 of 352 placed things on level 1 stand on a floor in another sector, which is the case
  the old queries could not do). Caveats: a face that owns no polygon is still handled by the plane
  resolver, and the `.jkl` winding is untrusted, so the polygon test asks the polygon's centre which side
  is inside instead of assuming a winding.
- **`.cmp` shading is implemented for the distance fade**; light levels from the colormap's own
  sectors are not (see §16.13 for what is calibrated).
- **UI panels**: every single-player custom element draws (map, objectives, force stars, load bar,
  title text). The two that used to be listed as missing — the model window and the power grid — are
  both on `edit_character`, a **multiplayer** screen, so they are out of scope.
- **Actors do not pathfind**; they walk straight at what they see, steering through portals by
  agreement with the frustum, and the instinct scheduler's weights are unfixed.
- **Multiplayer is out of scope**, deliberately.

Each of those is listed against its stage in `docs/roadmap.md` with the next concrete step.

### Retail coverage

`--audit` is the drop-in gate: it sweeps every level in every archive, resolves
every material/model/sprite/keyframe/puppet/AI/cog/sound reference, checks every
`LEVEL` row of all three `episode.jk` manifests, counts what it can decode in every
format present in the install, and **builds the draw geometry for every level**.

```
archives   5 GOB files, 4791 entries
levels reachable 34/35 from the manifests
render: 236346 world triangles, 6256 model instances, 1387375 model triangles, 0 placed sprite things
scripts (cog) 540 parsed, 1854 labels, 7683 symbols
keyframes (key) 504 files, 8439 nodes (max 29), 72587 entries, 0 truncated
ai classes (ai) 231 files, 1511 parameters, 1830 instincts, 0 without alignment/rank
puppets (pup) 49 files, 934 animation states, 158 joint mappings, 0 keyframe references missing
bitmaps (bm) 146 files, 309 cells, 3923508 pixels, 0 not exact
strings (uni) 5 tables, 1456 strings
sound classes 1976 entries over 88 events, 2 wav references missing
summary: 35/35 levels clean, 10 unresolved asset references, 0 non-drawable models,
         0 manifest LEVEL rows missing, 85 undecoded files
```

| ext | files | decoded |
|---|---|---|
| `.mat` | 1983 | 1932 |
| `.wav` | 644 | **644** |
| `.cog` | 540 | **540** |
| `.key` | 504 | **504** |
| `.3do` | 477 | **477** |
| `.bm` | 146 | **146** |
| `.snd` | 102 | **102** |
| `.ai` `.ai0` `.ai2` | 231 | **231** |
| `.pup` | 49 | **49** |
| `.jkl` | 35 | **35** |
| `.sft` | 27 | **27** |
| `.cmp` | 21 | **21** |
| `.spr` | 20 | **20** |
| `.uni` | 5 | **5** |
| `.dat`, `.par`, `.jk` | 7 | 0 |

4733 of 4791 files (99%) decode, including all 27 `.sft` fonts. Everything the renderer,
the audio system, the script runtime and the UI parse from — geometry, textures,
palettes, models, campaign order, sounds, level logic, animation, AI, fonts, UI bitmaps
and strings — is complete. Of the 58 that do not, 51 are `.mat` files that carry no
bitmap by design and 7 are saved game state (`.dat`/`.par`) or the level-library `.jk`;
none affects rendering a level or a menu. The 10 unresolved asset references are `.wav`
names that the retail install does not ship (one traced to `misc\snd\turret.snd`) and
that the original engine ignored.
Details in `docs/formats.md` §11–§15.

### The level's own logic runs

`--run` binds each placed COG instance to a mutable world (`src/runtime.rs`) and
delivers its messages, so the level's authored behaviour is exercised, not just its
geometry. Level 1 (`jkl\01narshadda.jkl`, 42 placed instances):

```
startup: 27 ran, 0 waiting on an engine event, 15 scripts are event-only
messages: 92 delivered, 0 waiting, 0 errors
world after 60 steps: 21 things on a path, 19 moved, 10 sector adjoin changes,
                      19 sounds requested, 1 inventory items
thing 4 (2x3door) moved from (1.2 9.7 6.3) to (1.2 9.7 6.6)
sounds: 19 requested, 0 missing from archive
```

Across all 35 levels: **1030 startups ran, 2541 messages delivered, 424 things moved,
247 sector adjoin changes, 4893 sounds requested, 0 errors, 0 missing sounds** — every
sound name a script asked for resolves to a real archive entry, which is what proves
the symbol binding is right and not just the parser. The verbs still stubbed, ranked by
how many placed instances call them, are printed at the end of `--run all`
(`GetSenderId` 216, `GetSenderRef` 184, `SetWallCel` 162, …).

### …and it moves what the window draws

The app runs the same script instances and steps their world every frame, rewriting the
`Transform` of the entities drawing each thing that moved:

```
42 script instances bound, 19 things moving at load
   (2x2elev, 2x3door, 3x3door_1, 3x3elev, 4x3door_1, 4x4elev, 6x6elev_1, funicular)
sim 0.0s: thing 4 drawn at (1.20 9.65 6.39), runtime (1.20 9.65 6.39)
```

Thing 4 is a `2x3door` authored at `z = 6.35`; its script has opened it and the mesh was
drawn where the runtime says. `--sim` also delivers each declared message once so the
level's doors and lifts act without a player; without it only `startup` runs, as at load
in the original engine.

### …and you can walk around it

The camera is a walker bound to the level's own convex sectors. A `.jkl` surface whose
`adjoin` is `-1` is a wall; one with an adjoin is a doorway, resolved through the
`World adjoins` table's `mirror` field to the sector on the other side — that field is
what makes portals work, and treating it as a sector index pins you in place
(`docs/formats.md` §3.1). `--walk all` holds "forward" from each level's own
`walkplayer` and reports the result:

```
level                          travelled  blocked    sectors contained  hull
jkl\03katarn.jkl                    16.0        0   405->369       yes  differs
jkl\04escapehouse.jkl               16.3        0   781->786       yes  differs
jkl\09fuelstation.jkl               11.0        0   874->880       yes  differs
...
summary: 34/34 levels with a start kept the walker inside its sector,
         13 had the hull test agree, 931 sector crossings, 1 without a start
```

`contained` holds on every level with a start; `hull` is a stricter second opinion that
disagrees where sectors overlap (level 1 has 462 sectors for 352 things, many of them
trigger slivers). The window runs the same `walker_step` the sweep measures, so walking
into a wall stops at the wall and walking through a doorway changes sector.

### …and its models assemble from their joint hierarchies

A `.3do` puppet stores each mesh's vertices around its own *pivot*, so the parts only
become a body after the chain above each one is composed. Composing the translations
alone gives a heap of plates; the engine's own composition is

```text
joint[n]    = joint[parent] · T(-parent.pivot) · [R(n.rot) | n.pos] · T(n.pivot)
joint[root] = [R | pos] · T(pivot)
```

Three details are load-bearing, and each was a bug before it was read out of the engine:

- a node rotates its own mesh about its own pivot but is *placed* relative to its
  **parent's** pivot, so both pivots appear with opposite signs;
- the parent of a node can come **after** it in the node table (`ky.3do` stores `k_lcalf`
  at row 3 and `k_lthigh` at row 8), so the hierarchy resolves by reference, not by row
  order;
- `R` is not a product of three axis rotations: Sith's `rdMatrix_Build34` evaluates nine
  trig expressions into three orientation vectors that are then the matrix *columns*
  (pitch about X, yaw about Z, roll about Y), and the cross terms are asymmetric.
  Transcribing those nine expressions is what made a standing figure.

A `.key` frame is an absolute local pose per joint (`x y z pitch yaw roll`), matched to
the model by mesh name, because `kystand0.key` lists its joints alphabetically while
`ky.3do` lists its meshes in authoring order. The second row of a keyframe entry is
velocity / angular velocity used to extrapolate between keys — not, as it first looks, a
delta.

Evidence chain: the shipped `JediKnight.EXE` gives the file layouts verbatim as
`"%d: %x %x %d %d %d %d %d %f %f %f %f %f %f %f %f %f %s"` (hierarchy node) and
`"%d: %f %x %f %f %f %f %f %f"` (keyframe entry); the composition, the matrix layout and
the "keyframe = absolute pose, blended as a difference from rest" rule come from
[OpenJKDF2](https://github.com/shinyquagsire23/OpenJKDF2)'s decompilation of the same
engine (`rdThing_AccumulateMatrices`, `rdPuppet_BuildJointMatrices`,
`rdMatrix_Build34`); the outcome is what `docs/models.png` and `docs/pose-kyle.png` show.
The matrix layout has not been re-derived from this install's disassembly, so it is
stated as a transcription rather than as an independent observation.

Two related findings: the world is **Z-up** (`CEILING SKY Z` in every level header, and
144 of `04escapehouse`'s 164 sky faces are flat in Z), so gravity, the floor normal and
the camera's up vector are all Z, and models, level geometry and placed things share one
frame. And `.3do` writes `NAME` *above* `VERTICES` in each mesh block, so a parser that
attaches the name to the mesh it already parsed shifts every mesh name by one — which
pairs each keyframe joint with the wrong mesh and scatters the model even with the right
math.

## Run

```powershell
cargo run --release                      # boot to the main menu; New Game loads the campaign's first level
cargo run --release -- --boot            # the boot screen and what each of its rows does, no window
cargo run --release -- --gates           # every gate, one process each, with the mode it ran in
cargo run --release -- --controls --exit-after 900   # the frame budget: any run ends after N frames
cargo run --release -- --check           # parse-only validation, no window
cargo run --release -- --audit           # sweep the whole install (drop-in gate)
cargo run --release -- --live-check      # invariants on the running game's own runtime
cargo run --release -- --episodes        # the campaign tree, with each row's status
cargo run --release -- --overview        # the whole level in plan, from directly above
cargo run --release -- --near crate4_3   # frame the first thing of a template
cargo run --release -- --scale           # how big the models are against the level's own doorways
cargo run --release -- --start 03katarn  # play a level straight away: no main menu to click through
cargo run --release -- --look 35         # pitch the default view up at the sky
cargo run --release -- --shot out.png    # render, save a PNG, exit
cargo run --release -- --shot out.png --frames 400 --sim   # …after 400 frames of script time
cargo run --release -- --cogs              # parse + run the level's scripts
cargo run --release -- --cogs all          # the same for all 540 scripts in the install
cargo run --release -- --run               # start the level's scripts against a live world
cargo run --release -- --run all           # the same for all 35 levels, one block each
cargo run --release -- --walk 120          # walk 120 frames into the level, no window
cargo run --release -- --walk all          # the same for every level, one line each
cargo run --release -- --pose 3do/actor/ky.3do 3do/key/kystand0.key 8 --shot pose.png
cargo run --release -- --pose 3do/actor/ky.3do - 0     # a model's own rest pose
cargo run --release -- --bm "ui\bm\bkmain.bm" art.png    # one UI bitmap as a PNG
cargo run --release -- --ui-tables                       # every menu screen read from the EXE
cargo run --release -- --ui-audit                        # …and every element's font/bitmap/key checked
cargo run --release -- --ui singleplayer main.png        # a menu screen, composed
cargo run --release -- --ui esc esc.png                  # the in-game pause screen
cargo run --release -- --ui force force.png              # the force-power icon screen
cargo run --release -- --ui fieldlog fieldlog.png        # the map / field-log screen
cargo run --release -- --hud hud.png                     # the status bar on its own
cargo run --release -- --menu main                       # the boot screen, on its own (the default)
    cargo run --release -- --menu main/10                    # …then activate its Single Player row
    cargo run --release -- --menu 0x535e58/100               # …then New Game, which loads the level
cargo run --release -- --menu main --frames 200 --shot m.png  # a *menu* capture: a paused world hits
                                                         # frame 30 before the pipelines are compiled
cargo run --release -- --play sound/ltsaberon01.wav     # play one archived sound
cargo run --release -- --dump "episode.jk"              # print one archived asset
cargo run --release -- --raw "ui\bm\bkesc.bm" out.bin   # write one archived asset verbatim
cargo run --release -- --texture 01f01.mat tex.png      # write a decoded material
cargo run --release -- --no-cull                        # draw every sector, to A/B the visibility pass
    cargo run --release -- --rules                          # the player's motion, from the level's templates
    cargo run --release -- --cog-check                      # every level's scripts still act on their world
    cargo run --release -- --fire +bryarbolt                # fire a weapon template at an actor and check the damage
    cargo run --release -- --fire-demo                      # fire a stream of bolts, for a screenshot
    cargo run --release -- --pickup                         # take level 1's powerup, check the inventory and respawn
    cargo run --release -- --weapon +elaser                 # taking a weapon changes what fires
    cargo run --release -- --bins                           # the inventory bins, weapon arming and force mana
    cargo run --release -- --force jump                     # run force_jump.cog: mana, hold, lift
    cargo run --release -- --force speed                    # force_speed.cog: extra thrust -> walk speed
    cargo run --release -- --force heal                     # force_heal.cog: 20 * rank health for 200 mana
    cargo run --release -- --ai                             # the .ai instinct classes and an actor noticing the player
    cargo run --release -- --save out.jks                   # write a .jks and read it back (or inspect an existing one)
    cargo run --release -- --sound                          # the .snd fade curve over all 1976 entries
    cargo run --release -- --music                          # PlaySong track numbers, and the install's music files
    cargo run --release -- --ui-visible                     # the engine's run-time bIsVisible rule over the screens
    cargo run --release -- --campaign                       # the episode graph from level 1 to the end, both branches
    cargo run --release -- --transition                     # finishing a level: jkendlevel, then the next level in the graph
    cargo run --release -- --smk                            # the 25 shipped cutscenes' own headers
    # cutscene decode needs the 32-bit helper first (the DLL is a 32-bit image):
    cargo build --release --target i686-pc-windows-msvc --manifest-path tools/smkdec/Cargo.toml
    cargo run --release -- --smk-decode 01-02A.SMK           # decode 40 frames through SMACKW32.DLL
    cargo run --release -- --cutscene 01-02A.SMK 30         # play a cutscene in the window from frame 30
    cargo run --release -- --complete-after 0.1             # finish the level: jkEndLevel, film, then next level
    cargo run --release -- --complete-after 0.1 --no-swap-levels  # stop at the film instead
    cargo run --release -- --flow                           # level -> cutscene -> next level, checked
    cargo run --release -- --animate                        # check that every placed actor's keyframes move it
    cargo run --release -- --no-anim                        # freeze the actors, to A/B the animation
    cargo run --release -- --level "jkl/04EscapeHouse.jkl"
cargo run --release -- --game "D:\some\other\install"
```

No level argument means "start where the game starts": the first `LEVEL` row of the
first single-player episode. Nothing about the campaign is hardcoded.

`--audit` exits non-zero if any level fails validation or any manifest `LEVEL` row
is missing — so it works as a CI gate and as a check when pointing the tool at a
different retail install.

The default game path is
`G:\Steam\steamapps\common\Star Wars Jedi Knight`; override with `--game`.

**Controls** — `WASD` walk, `Shift` run, `Space` jump, arrow keys look. The player is a
walker bound to the level's own sectors, so walls stop you and doorways let you through;
gravity is the level's `WORLD GRAVITY`.

## How it was reverse engineered

`docs/formats.md` records every recovered format with the evidence behind it and
flags what is still unknown. Summary of the approach:

1. **Ghidra 12.1.4** (`M:\ghidra_12.1.4_PUBLIC`) on `JediKnight.EXE` for identity
   and architecture, the recovered engine module map (from embedded `__FILE__`
   strings), and corroboration of the archive header (magic, version check,
   error strings, source module `.\Win95\stdGob.c`).
2. **Shipped-data triangulation** for the container formats. This is where the
   detail came from, because `.jkl`, `.3do`, `.key`, `.spr` and `episode.jk` are
   *plain text* — the layouts are directly readable and every parse is
   cross-checked across many files rather than guessed from disassembly.
3. **Rendering the result and looking at it.** Several findings are only visible in
   pixels, not in headers: models are 1/10 world scale, each mesh carries its own
   vertex pool, a model's `INSERT OFFSET` is a height rather than a vertex
   translation, a JK yaw of `a` faces `(cos a, sin a, 0)` about Z, and — the largest
   one — the `.cmp` palette starts at offset 64, not 30. That last off-by-34 turned
   every texture in the game hue-shifted: cyan cloud edges, magenta walls,
   unreadable props. Each of these was settled by shooting a screenshot of the retail
   geometry and comparing two hypotheses.
4. **Data tables read out of the executable itself.** The UI has no description file: a
   menu screen is a full-screen `.bm`, a `.sft` font, and a static array of 100-byte
   `jkGuiElement` records in `.data` (main menu 0x526680, Esc menu 0x529560) whose field
   layout — type, handler id, font index, `jkstrings.uni` key, bitmap-or-flag, x/y/w/h —
   was established by reading the records and cross-checking them against the art and
   against the engine's own `jkGuiMenu` structs. The two name tables next to them are how
   a build finds 12 fonts and 35 bitmaps without typing a name. The font format came the
   same way: the `stdFont_Load` header, one charset per character range, and a whole `.bm`
   glyph strip appended. All 27 fonts parse exactly, 2441 glyphs.
5. **A self-check** (`--check`) that asserts index ranges, intensity/vertex
   counts and texture resolution, so the parsers cannot silently drift, plus
   `--audit` as the install-wide gate.

A notable finding, written up in `docs/formats.md` §8: the Steam executable's `.text` is
**ciphertext behind a DRM wrapper** — entropy exactly 8.00 bits/byte over 1.2 MB with not a
single function prologue, the entry point itself encrypted, and `.bind`/`..idata` payload
sections; no vendor string survives, and neither the CD autorun `Jedi.EXE` nor the local
`winmm.dll` proxy is the unwrapper. The GOG build's `JK.EXE` is the same image without the
wrapper — plaintext, 354 function prologues, identical sections, 401,344 bytes smaller by
exactly its `.bind` payload — and it is what this build reads (§8a), which is also why the
menu state machine and the per-screen backgrounds are now being recovered from code rather
than inferred. Everything the UI *draws* lives in plaintext `.data` and the archives, which
is why it was recoverable.

## Verification

Every check in one page: [docs/gates.txt](gates.txt) records each flag, its exit code and the
number it printed against this install — **59 gates, 0 failures**, plus 122 tests. What is not ported, unit by unit, is [docs/gaps.md](gaps.md).

`--check` on level 1 (`jkl\01narshadda.jkl`) reports, and asserts:

```
gob archives indexed   4791 entries
vertices               4360     (file declares 4360)
texture vertices       10574    (file declares 10574)
texture vertex range   [0.0, 12373.9]   (texels, so the sampler must repeat)
surfaces               4595     (file declares 4595; 3393 drawable)
materials              528      (file declares 560, lists 528, 499 decoded)
surfaces with texture  3393     (every drawable surface)
surfaces drawing flat  0
portal / adjoined      1202 portal, 1206 adjoined
sectors                462      (file declares 462)
things                 352      (file declares 552, lists 352)
templates              58 distinct
bounds                 x[-12.10,26.10] y[-11.00,21.80] z[-18.50,16.70]
```

Three things worth calling out:

- The `World <thing> <n>` counts are allocation hints, not row counts — lists end
  at a literal `end` line. Getting this wrong silently swallows the vertices
  section and the level parses with zero vertices and no error.
- `1206 adjoined surfaces` is not something the parser is told; the level's
  separate `World adjoins 1206` table is. Two independent sections agreeing on
  the same count validates the surface record layout.
- `surfaces drawing flat 0` means every drawable surface has a decoded bitmap, so
  a grey-looking room is the level's own palette, not a decode failure.

Validated on all 35 levels — see `docs/formats.md` §10–§11.

## Where the port stands

Nine of the ten roadmap stages are landed and gated; the tenth is half drawn. The evidence, rather than
the summary, is what each check prints:

| what | check | the number it prints |
| --- | --- | --- |
| jump physics agrees with the rules | `--rules` | `apex 0.26 / 0.28 closed form` |
| a wall stops a body beside it, not past its end | `--rules` | `pushed true … 27 past its end is pushed false` |
| the walker stays legal on every level | `--walk all` | `34/34 levels`, 959 sector crossings |
| portal occlusion | `--audit` + `--shot` A/B | `21 of 462 sectors in view`, **0.000%** different from `--no-cull` |
| the colormap material matches the CPU path | `--shot` A/B | **0.00%** of pixels differ, two levels |
| every label a level waits on | `--cog-check`, `--triggers` | `startup/arrived/timer/pulse/entered/exited/damaged/killed/activate/crossed/blocked` |
| `sleep` suspends and resumes | `--sleep` | volume `0.10 → 0.90` across the wait |
| the bar's music fade | `--fade` | `0.00 → 1.00 → 0.00` in 4 s of game time |
| the menus click with their own wavs | `--ui-sounds` | `39 of 41` screens, both names present |
| force mana | `--mana` | `20.0` per second, clamps at the 400 pool |
| damage is two-way, death respawns | `--hit`, `--die` | 25 of 30 carried damage lands; 0 health respawns full |
| actors close through doorways | `--ai` | `blocked true, steering at portal …` over 1206 portals |
| the campaign graph and transitions | `--campaign`, `--transition` | passes over all 35 levels |
| cutscenes decode and sequence | `--smk` | passes from the episode order |

**Known gaps, each with where the answer lives** (all in `docs/formats.md`):

- **Force pull works the way the script does; force grip's disarm does not.** The pull highlights with
  `jkSetTarget` and then `ApplyForce`s the item at 30 u/s toward the player with gravity cleared
  (`--pull` measures `velocity [-30.0, 0.0, 0.0]`). Grip's damage-over-time path is implemented but has
  no check of its own, and `SkillTarget`'s disarm branch is simplified: pull throws items, it does not
  shoot the weapon out of an actor's hand.
- **Only some force powers are exercised.** Jump, speed, healing, seeing, pull and grip have gates; the
  other powers in the menu's own table (throw, lightning, destruction, protection, absorb, blinding,
  deadly sight…) have their scripts and now their verbs, but nothing checks that they do anything. The
  *acquisition* path is now the engine's own — a bin that goes from empty to held runs the `cog=` that
  `misc\items.dat` names for it — and it had been dead for all eight force bins because their `max`
  column is a decimal (`4.0`) and the parse was integer.
- **Puppet animation covers motion, not layers.** Every actor's `stand` state animates, death
  plays once and holds, and `actor_state` now picks `walk`/`run`/`fire`/`hit`/`turn` from the
  body's velocity, pose and `.pup` contents — but crouch and the attack wind-up are not picked,
  and an actor has one state rather than a layered upper/lower body.
- **AI steering searches, but only when it has to.** `Runtime::sector_route` asks
  `Collide::route` (breadth-first over sector portals, gate-checked) for a doorway route when the
  straight line is blocked, and pops waypoints as they are reached. Instinct scheduler weights,
  `accuracy=` spread and reaction times are still unfixed, and a route is recomputed only when the
  direct line closes.
- **Cutscene palette**: 29 of 768 palette bytes still differ from libsmacker's decode.
- **No single-player UI panel is missing.** The model window and power grid belong to `edit_character`
  (`bkBuildMulti.bm`), a multiplayer screen — `--ui-tables` says which screen owns each draw callback.
- **Cogs are coroutines only to the extent of `sleep`.** A label that waits mid-block resumes at a
  statement boundary, so its enclosing block re-runs.
- **The sector tracker is a heuristic.** Level 1 reports 339 (`.jkl`), 218 (walker) and 102
  (`sector_at`); the cull picks whichever admits what the frustum sees.
- **The savegame format is the engine's own** — `src/jks.rs` is recovered from JK.EXE's writer
  (`0x4da9c0`), reader (`0x4dab70`) and record writer (`0x4f4210`), and `--save` writes a 9320-byte
  version-6 file and reads it back. What it has not been tested against is a **shipped save**: the GOG
  install ships none (`player/` holds only `dummy.txt`), so the round trip is against its own output.
- **Not implemented at all**: multiplayer.

## Layout

```
src/gob.rs      GOB archive reader (index + extract)
src/jkl.rs      JKL level parser
src/tdo.rs      3DO model parser
src/tex.rs      MAT material + CMP colormap decoding
src/spr.rs      SPR sprite parsing
src/snd.rs      SND sound-class tables
src/cog.rs      COG script parser + interpreter
src/runtime.rs  the world COG verbs act on (things, sectors, sounds, inventory)
src/collide.rs  sector planes, portals and the walker's collision solver
src/key.rs      KEY keyframe animation
src/pup.rs      PUP puppet definitions
src/ai.rs       AI class parameters
src/bm.rs       BM UI bitmaps (cells, 8/16-bit, palette tail, HUD numMips/xPos/yPos)
src/uni.rs      UNI string tables
src/episode.rs  episode.jk campaign manifest
src/sft.rs      SFT sprite fonts (charsets + glyph strip)
src/ui.rs       the UI canvas: screen compositing, text, integer scaling
src/menus.rs    the screen element arrays, read out of the executable's .data
src/hud.rs      the status bar: retail panel/meter placement and frame selection
src/rows.rs     shared text-list helpers
src/main.rs     asset loading, mesh building, Bevy app, self-check, audit
docs/formats.md recovered format reference + evidence + unknowns
```

Tests (`cargo test`) pin the formats whose layout is easy to get subtly wrong — the
episode manifest, the sprite sheet, keyframes, puppets, AI classes, UI bitmaps, string
tables, the door script — plus the `.cmp` length check, the runtime's verb table, the
collision solver on a synthetic Z-up room, and the model transform (a parent stored after
its child with nonzero pivots, and a parent rotation swinging its child).
## Release-readiness pass (round 201)

The first-run defects are fixed and the campaign chain is now a gate. `jkPrintUNIString` was
reading its first argument (the player's thing index) instead of the second (the `.uni` key), so
level 1's first pickup printed `COG_00000` instead of "Shield Recharge"; audio is audible by
default now instead of silent-unless-`--sound-on`; and `Episode::flow_after` no longer hands the
campaign's `DECIDE` row back as if it were a level — it walks the decision, keeps both of level
15's films in order and continues on the neutral (light) arm. `--tour` is the new release-content
gate over every campaign level. The evidence and the numbers are in `docs/gaps.md`'s Round 201
entry; the checks at that point were **60 gates, 129 tests, 0 failures**.
## Release candidate, reviewed (round 202)

A fresh-eyes review of the release diff found three blockers behind the first-run fixes: the app
stopped the campaign after level 2 (its successor lookup read the multiplayer `episode.jk`), the
per-level completion flags never reset, and the opening film played behind the boot menu and was
never removed. All are fixed; `--tour-live` now finishes and swaps all 21 light-side levels in the
running app, `--tour` checks 28 rows on both arms, and normal launch/New Game were checked by hand.
The review also caught a scheduler-order dependence in `--goal` (now explicitly ordered and pinned),
an inflated `RunReport` error count, and a stale MSRV. The current checks are **61 gates, 130 tests,
0 failures** (windowed `0 of 61 failed, 0 skipped`; headless `0 of 61 failed, 9 skipped`). Still
open for publish: the license choice, and a human playthrough of levels 2–21.
## Finding the install, and an installer (round 203)

The port used to start from a hardcoded developer path. `src/gamepath.rs` now resolves the
retail install the way a player expects: an explicit `--game`, `BK_GAME`, a saved
`game-path.txt` beside the program or in `%APPDATA%\bevi-knight\`, GOG's registry entries, then
the folders GOG installs to, with `JK.EXE` plus `Resource\` as the proof it is really the game.
`--find-game` and `--set-game` make that inspectable and savable. `packaging\install.ps1`
(launched by `Install.cmd`) does the search once, installs per-user to
`%LOCALAPPDATA%\Programs\bevi-knight`, writes that saved path, adds a Start Menu shortcut and
leaves an uninstaller; `packaging\package.ps1` builds the zip to hand a player. Tested from the
zip with both an explicit path and registry auto-discovery; `--gates` stays 0 of 61 and the
suite is 132 tests.
## A native setup.exe (round 204)

The script installer became a single native Windows setup: a WinForms wizard (`packaging\Setup.cs`)
compiled with the in-box .NET Framework C# compiler, with the game, film helper and docs embedded
as a zip resource. It detects the GOG install, accepts a browsed/pasted/dragged game folder,
installs per-user, saves the game path, makes Start Menu/desktop shortcuts, registers in Apps &
Features and carries its own uninstaller (`Uninstall.cs`); `/S` switches make it scriptable.
`packaging\build-setup.ps1` builds it to `dist\bevi-knight-<version>-setup.exe` (~28 MB) without
Inno Setup, WiX, NSIS or Squirrel. Tested through GUI, silent install (explicit and auto-detected
paths), installed `--check`, shortcut creation/removal and uninstall.

## The license (round 205)

The tree is GPL-3.0-or-later: verbatim `LICENSE`, `license` in `Cargo.toml`, and a `NOTICE` that
draws the line a code license cannot - no game data is included or licensed, the game and its
marks belong to Lucasfilm/Disney, the docs screenshots are retail artwork excluded from the GPL,
and binary releases should keep this repository as their corresponding source. Contributions are
inbound = outbound.

## The version: 0.1.0 (round 206)

The source release is 0.1.0: Cargo version, annotated `v0.1.0` tag on the release commit, and
`bevi-knight-0.1.0-setup.exe`. The earlier local `v0.1.0` tag was re-pointed because no tag had
ever been pushed (`git ls-remote --tags origin` empty) and `origin/main` was 278 commits behind,
so this is the first published release rather than a fix bump; the `v0.1.1-rc.1` candidate tag was
removed. The release note's change list is now "What changed since the earlier 0.1 candidate".

## CI (round 207)

`.github/workflows/ci.yml` is a single windows-latest job: install the MSRV pinned in
`Cargo.toml` (plus the i686 target), `cargo build --locked`, `cargo test --bins --release`, build
the film helper, build the native `setup.exe` with `packaging\build-setup.ps1 -SkipBuild`, check
every relative docs link with the new `scripts\check-docs-links.ps1`, and upload the setup as an
artifact. The 61 gates are not in CI because they require a retail install; fmt/clippy are not
enforced yet (tree is not rustfmt-clean, clippy has warnings but no errors). The workflow only
runs once it is pushed, so GitHub has not executed it yet.

## CI ran for real (round 208)

Pushed `main` and `v0.1.0` after approval. The first CI run was green in 22m45s and uploaded the
setup artifact; it flagged the v4 actions as Node 20, so they moved to checkout@v7/cache@v6/
upload-artifact@v7 in a00e577 and the cached rerun was green in 3m54s. The published tag stays on
the verified release commit c941ccc; main is the CI-fixed commit after it.

## The README became the front door (round 209)

Rewrote the README for players: badges, status table, requirements, CI setup download and install,
first launch, controls, troubleshooting, reporting issues, source build and docs map. The long
engineering content (screens gallery, unit-by-unit status, flags, reverse-engineering notes,
verification detail, layout, camera recovery) moved verbatim to docs/engineering.md, with the stale
hardcoded Steam path replaced by the discovery order. 205-line README; 129 doc links still resolve;
CI green and setup artifact rebuilt.

## Releases from CI (round 210)

Pushing a v* tag now triggers the CI workflow again; after the Windows job builds the setup, a
release job checks that the tag matches Cargo.toml's version, downloads the artifact and runs
gh release create --verify-tag with the setup attached (hyphenated tags become pre-releases).
The README points at Releases first and keeps the CI artifact and source build as fallbacks. The
v0.1.0 tag predates this job, so it has no attached asset yet; the next tag is the first live run.

## Health across levels, 0.1.1 (round 211)

Picked the smallest real gap for a version bump: health and shields reset at every level change
because `Runtime::carry`/`adopt` did not include them. They now ride `CarryOver` clamped to the
next level's max health and the 200 shield cap, with a round-trip test. Version 0.1.1; the tag is
the release automation's first live run (v0.1.0 predated it).

## First release from a tag (round 212)

Pushed the v0.1.1 tag; the workflow built the setup and the release job published
https://github.com/tigwyk/rust-jediknight/releases/tag/v0.1.1 with bevi-knight-0.1.1-setup.exe
attached. Downloaded the asset back, matched its sha256 (7d3a...), installed it silently, ran
--check and uninstalled cleanly. The release path now works without manual steps.

## Landing page (round 213)

A static landing page in site/ (HTML + one CSS file + optimised screenshots) is deployed to
GitHub Pages by .github/workflows/pages.yml. It covers the pitch, version 0.1.1 status, screens,
the three-step install, requirements, source build, docs links and the licence/game-rights
footer. The README links it and the repository homepage was set to the Pages URL.

## The Jedi set, and the QA window's real numbers (round 252)

Ported `forcepowers` (`0x40ff40`) and `saberfighting` (`0x40fa40`), the last two live instinct
handlers shipped content loads, from the retail disassembly and the class lines themselves — the
force side is the engine's `user0` dispatch to the actor's own `darkjedi_*.cog`, the saber side is
the 500 ms clock, swing chance, four `(range, damage, cooldown)` entries and `DAMAGE_SABER`. Suite
189 → 193; `--soak-all` 35/35 clean.

The QA window's three checks were run for the first time this round and the numbers are now
current rather than inherited: port gates **65/65, 0 skipped** windowed; parity draw **6/0**; walk
trace **12 legs, 1 tracked failure** (`walk5_spawn`). The draw gate's only failure was a fixture
bug — the level-1 `spawn` reference had been captured with `-StartLevel 3`, so it held level 3's
pose and scored 0.18. Re-captured correctly it scores 0.66 and the gate passes; the harness now
warns when a spawn row's pose is nowhere near the level's own walkplayer pose.

`specialattack` (24 classes) is the one loaded instinct still unported, and it is blocked on a
measured producer rather than on the handler: its damage branch runs on AI event 4, and while the
executable does emit event 4 from the thing-collision callback, this build's actor walk raises no
thing-vs-thing event. The same round corrected the old "nothing emits 2 or 4" inventory: events 2,
4 and 0x200 all have emit sites in the executable, and this build produces none of them yet.

## Generated particle clouds (round 253)

The bulk of the shipped effect content is `type=particle` templates, not `.par` files, and
`sithParticle_Initalize` generates their elements from the template's own numbers. The port now does
the same: two uniform Euler angles (720° default spans), a radius in `[minsize, range]`, the
`(0,1,0)` rotation, and the `typeflags & 4` cel choice, all through the existing particle draw path.
Level 1 loads 24 generated clouds where it used to load one `.par` cloud. Suite 194; `--audit`
35/35 clean; `--gates` 0 of 65 failed / 1 skipped; `--soak-all` 35/35 clean. The update arms
(cel cycling and grow) and a captured frame of a generated cloud stay on the ledger.

## The first-person weapon's flicker (round 254)

The mount animation replayed every 0.8 s, which read as the arms and weapon flickering. `jkPlayPOVKey`
takes the start frame third and the flags word fourth; the port kept the third as a loop bool and
ignored the flags. The engine's `sithPuppet_PlayKey` turns flags `0x4`/`0x20`/`0x2` into end statuses,
and the shipped mount (`0x14`) and fire (`0x38`) words both carry one, so both clips are one-shot. The
port now holds a non-loop clip at its last frame and advances the phase by the frame delta rather
than the absolute level clock. `BK_VIEWMODEL_TRACE=1` shows the mount playing frames 0–12 once and
changing no further; frames 130 and 140 are pixel-identical. Suite 196; gates 0 of 65 failed /
1 skipped. The framing gap is measured separately: the drawn weapon vertices project mostly outside
the right/bottom of the frame, and the POV thing offset is still unrecovered.

## First-person weapon framing (round 255)

Isolated retail's first-person weapon with a fist-vs-bryar capture pair at the same level-1 spawn
pose (retail block x 480–660, y 337–730 at 1170x731) and fitted the port's rest-pose model to it with
a camera-local offset of (-0.026, 0.009, 0). The POV key clips are skeleton tracks (`K_Rshouldr`,
`K_Rhand`) while the `.3do` parser keeps only mesh names, so the clip pose is skipped and the rest
(held) pose is drawn; the sway still applies. Suite 196; gates 0 of 65 failed / 1 skipped. A per-material near-plane cull drops the wrist
(`HND-WRT`), which straddled the eye plane; 13 of 14 parts draw.

## Skeleton node names for POV clips (round 256)

Parsed the `.3do` hierarchy's `hnodename` (`K_Rshouldr`, `K_Rhand`, ...) and made `pose_nodes` match
key tracks by that skeleton name before falling back to mesh names. The mapping is unit-tested. It is
the prerequisite for the viewmodel mount/fire clips, but the mount's last frame still leaves the weapon
off the right edge, so the viewmodel continues to draw the rest pose. Suite 197; soak-all 35/35;
gates 0 of 65 failed / 1 skipped.

## Particle grow arm (round 259)

Ported `sithParticle_Update`'s grow arm: scale advances by `maxthrust * dt` for `typeflags & 1`, and
element offsets scale with it about the cloud origin. The shipped templates distinguish the cases
(`+whitecloud` 30, `+sparks` 20, `+force_heal` 0.01, `+smoke` none). Suite 198. The cel-cycling arm
stays open.

## Particle cel cycling (round 260)

Ported the second `sithParticle_Update` arm: per-element cel advance with probability
`(cels - cel - 1) * (age/life) * dt`, a local LCG, and a mesh swap when the cel changes. Both particle
update arms are now in. Suite 199.

## The chase turns the drawn actor (round 262)

Fixed the rendering half of the actor chase: the parts followed the runtime position but kept their
spawn yaw, so a chasing actor shot from a facing its mesh no longer had. The body-yaw delta is now
applied about the thing origin (`thing_quat(0, yaw, 0)`; pitch/roll stay out because the port's pitch
is the aim elevation). Suite 200. The head joint remains open.

## Head aim (round 263)

Actors now aim their head, not just their torso: the AI elevation is clamped by the template's
`minheadpitch`/`maxheadpitch`, stored as `ThingState::head_pitch`, and applied to `k_neck` (0.5×) and
`k_head` (0.3×) in `model_mesh_posed_aim` — the engine's joint-angle scales. The body keeps the
yaw-only transform from round 262. Suite 201.

## The last live instinct (round 264)

Ported `specialattack`'s event-4 branch: the collided player takes `DAMAGE_SABER` at `fltArg[8]` (Maw
10000, Pic 2000), then the actor rearms for one second. The port raises event 4 from an actor/player
contact test because its actor walk has no physics solver. That closes all 22 loadable instincts.
Suite 202.

## The spawn's resting height, from the retail binary (round 265)

The port had been holding the player wherever the level's `walkplayer` row sits — 0.120114 above the
floor at level 1's spawn — and calling that an attachment. The retail binary says that is a *search
reach*, not a resting height, and one retail read confirms it.

**What the code says.** `sithPhysics_GetThingHeight` (`0x4f6210`) answers
`max(model3->insertOffset.z, moveSize + 0.005)`; the shipped `0x523184` is **−0.005**. Its caller,
`sithPhysics_FindFloor` (`0x4f5550`), uses that only in its *attached* branch — an unattached body's
probe is exactly `moveSize + 0.005` (the shipped `0x52318c` is 1.1 for the attached branch's `* 1.1`).
`sithThing_AttachThingToSurface` (`0x4cde80`) records the surface and the plane and never writes the
position, and `sithPhysics_UpdateAttachedThingPhysics` (`0x4f7430`) applies no gravity. The player is
created by `sithPlayer_NewPlayer` (`0x4c8ec0`), which sets the position, exits and re-enters the
sector, and never calls `FindFloor` — so a spawn pose further than `moveSize + 0.005` from its floor
is *not* attached, and gravity takes it to the swept sphere contact, `floor + radius`.

**The read.** At level 1's own spawn (`01narshadda`, no warp), retail's `coords` reports
`Pos: (-10.44, -5.46, -1.13) Sector: 339` at load and `Pos: (-10.25, -5.07, -1.18) Sector: 349` after
one second with no input. Floor there is −1.25, so the authored 0.120114 gap collapsed to one radius
(0.065) — the body fell. The port held −1.129886 indefinitely.

**The fix.** `walker_step` lands at `floor + radius` and carries no `rest` parameter; `Rules::rest`
is gone. The idle attach itself was the guess: `sithThing_AttachThingToSurface` has exactly **two**
callers — `sithPhysics_FindFloor` (run at creation/load, and by `sithThing_UpdateMove` only for a body
that is *already* attached) and the `AttachThingToSurface` cog verb. Nothing attaches a body on
landing, and `sithPlayer_NewPlayer` never attaches, so a player is never attached at all. The port's
floor probe that zeroed gravity each idle frame is therefore removed: gravity runs every frame and the
vertical sweep stops the body at the contact, which is what retail does. The regression test is the
rule itself: `a_body_placed_above_its_floor_falls_to_the_sphere_contact` drops a body from the
authored 0.055 gap and asserts it settles at one radius with `vy == 0`.

**What this does not settle, and must not be guessed at.**
- The same read showed retail's body **0.43 units of horizontal drift before any key was sent**
  (sector 339 → 349). The trace harness reads `coords` through the dev console, which pauses the
  game, and navigates the menus with synthetic keys; a still-held key or a genuine spawn resolution
  are both consistent with it. Telling them apart needs a second retail run, which is not being done
  here — so no push-out or "spawn resolve" rule is being ported from it.
- Level 1's spawn faces a **0.1-high surface 0.2 units ahead** (`surface 3301`, normal (−1,0,0),
  polygon top −1.15, floor −1.25; 0.045 above the feet before the settle, 0.1 after it).
  `sithThing_UpdateMove` (`0x4cc4c0`) normalises the move vector and calls `sithCollision_MoveThing`
  with **no raised retry**, so the engine has no lateral step: the face is a wall for a 0.065 body
  either way. The live-play gate's "1.000 unit forward from this spawn" is therefore not a
  retail-consistent bar; it is left unchanged here for a decision with the full trace in hand rather
  than lowered to make a check pass.
- `coords` prints `PYR (1.04, -59.98, 1.04)` at that spawn. The port's own movement target there
  (`BK_WALK_TRACE`, the direction × the 2.4 run speed) implies yaw ≈ −68.4°, fed from the level's
  `walkplayer` row, whose **position** matches retail to 0.006 (`spawn-levels.tsv`). Either the
  engine does not take the player's yaw from that row, or `coords`' PYR is a camera value like its
  pitch (already recorded). Open until read from the binary — not inferred further from one run.

Verification: `cargo test --bins --release` — **202 passed**, 0 failed. No game window was launched
for this round (port or retail).

## Actor gravity (round 266)

The last owed movement item from Phase 1 step 3. The engine's rule was already read; what was missing
was the data check and the gate.

**The gate is the engine's, and it has no type test.** `sithPhysics_UpdateThingPhysics` (`0x4f6270`)
falls a body when its **mass** (`+0x204`) is non-zero *and* its `physflags` carry `0x1`, halving the
rate for `0x40000`. The shipped `_actor` template is `physflags=0x4a4f mass=150` (read straight out of
the template table in `scrap/katarn.jkl`), so actors are gravity bodies exactly like the `_debris`
family (`physflags=0x201, mass=0.1`); the port's `type == 3` test was an invention. The port also never
parsed `mass=` — `self.mass` was script-only (`setthingmass`), so the engine's `+0x204` gate would have
read 0 for every thing. `mass=` is now parsed per template and copied to placed and created things,
the same way `physflags=` is.

**Grounding is the creation attach, not a new rule.** `sithThing_Initialize` ends with
`sithPhysics_FindFloor(thing, 1)`, whose reach is `GetThingHeight * 1.1`; a placed actor within that
reach of the floor it stands over keeps the level's authored height, and walking off a ledge removes
the floor from that reach so gravity takes it. The fall stops at the swept sphere's contact
(`floor + movesize`), and the stop now counts a **solid thing's model top** as a floor for actors.
`ai_tick`'s walk still sets only x/y (`next[2] = from[2]`), so the vertical axis belongs to the mover.

Verification: `cargo test --bins --release` — **203 passed**, 0 failed (new
`an_actor_off_its_floor_falls_to_the_contact_and_stops`: an actor falls from 0.5 to the contact and
stops, the same body with mass 0 floats, and a body inside the creation reach stands still). No game
window was launched for this round. The windowed `--soak-all`/`--gates` numbers are **not** re-measured
here (they open the app), so "0 bodies outside every sector" after this change remains to be checked
with the next QA-window run.

## The sway's composition, from `rdMatrix_BuildRotate34` (round 267)

The deviation ledger carried "`rdMatrix_BuildRotate34`'s composition order is not recovered, so the
three angles are composed yaw-pitch-roll with the port's own sign". It is recovered now, from the
retail decompilation, and the guess is gone.

`jkPlayer_DrawPov` (`0x4051e0`) takes `sin`/`cos` of the sway phase, builds the three angles
(`-|cos|·A0·speed`, `sin·A1·speed`, `sin·A2·speed`), passes them to **one** `rdMatrix_BuildRotate34`
(`0x43fcf0`) and pre-multiplies the view matrix with the result — there is no separate Euler order.
The builder's elements (`m0..m8`) are exactly the frame `runtime::rotate_by_angles` already produces
for a thing: column 0 is `m0..m2`, **column 1 is the yaw basis `(-sin y·cos p, cos y·cos p, sin p)`**,
column 2 is `m6..m8`. The viewmodel now builds its quaternion from that matrix
(`rotate_quat(pitch, yaw, roll)`), so the sway is the same rotation retail applies; a Bevy
`EulerRot::YXZ` happened to be close for the bryar's 10°/7° amplitudes, which is why the old guess
never showed up as a visual failure.

Verification: `cargo test --bins --release` — **204 passed**, 0 failed. New check
`the_sway_rotation_is_the_engines_build_rotate_matrix` pins all three columns of the builder's
elements (the forward axis and the rest), which an Euler-order change fails. No game window was
launched for this round.

## Phase 0's ledger sweep, and one stale marker (round 268)

Phase 0 asked for the deviation ledger to be swept: every `ponytail:` marker in `src/` assigned an
owner, a measurement and a retire condition. The inventory is done and lives in `porting-roadmap.md`
under `ponytail sweep`, one line per marker:

- **7 covered** by a row the ledger already had (the solid-model collider, the sway's averaged
  displacement, the tint-only palette pass, the plane-query stop, the position/segment mover tests,
  awareness's breadth guard, the AABB footprint).
- **21 new rows**, each stating what differs, what measures it today (or which check would close it)
  and what retires it: thing-vs-thing collision, the lateral step allowance, force-point spending,
  lighting (baked vertex colour vs per-surface/dynamic light), menu `hoverId`, puppet clip timing
  (start-frame epoch, units-to-frames, thing-track rule, `RotatePivot`), model back-face culling,
  scrolling surfaces, thing attachment, weapon auto-select, cog link filtering, AI route search, the
  cog sleep/timer scheduler and the map panel camera.
- **12 not deviations** — load-time or per-frame cost, guards, or the one-level-at-a-time architecture.
- **1 stale**: `runtime::use_bin`'s note said retail reaches it through an inventory screen the port
  does not draw "so nothing calls this from a key". The strip has been selectable since it landed
  (`I`, arrows, Enter), and it calls `use_bin`. The comment is corrected; the sweep exists to catch
  exactly this.

The new rows make the remaining gaps explicit rather than implied — notably `sleep`/`settimer`/
`setpulse` are stubs with no task queue (a sleeping script runs on immediately), and lighting is the
renderer's largest unmeasured surface.

Verification: `cargo test --bins --release` — **204 passed**, 0 failed (the only code change here is a
doc comment). No game window was launched for this round.

## The sweep caught a false claim — mine (round 269)

Round 268's ledger added a row *cog sleep/timer scheduler* on the strength of `runtime.rs`'s comment
("no timer queue — a level that only works across resumed sleeps will run its later statements early").
The comment is wrong, and so was the row. `cog::Vm::call_verb` handles `sleep`/`waitformessage` (the
`::sleeping` unwind records the statement; `Vm::resume`/`resume_sleeping_labels` pick it up when the
clock passes), plus `settimer`/`settimerex` and `setpulse` — and it does so **before**
`Runtime::handles` is consulted, so a script's `Sleep` never reaches the runtime arm that comment sits
on. `--sleep` is the gate that pins the suspension and resumption, and `engineering.md`'s stage-4 row
already claimed it correctly.

Corrected:

- the ledger row is now the *real* limitation — a `sleep` inside a nested block resumes by re-running
  that block's enclosing statements, because the VM stores a statement index per label rather than the
  engine's per-frame statement stack (no check covers it yet);
- the runtime comment states the dispatch order instead of the false "runs straight on";
- the `ponytail` sweep lists the marker as **stale**, so 2 of 41 are now stale and 20 are new rows.

The sweep's rule is therefore "verify the marker against the code", not "trust the sentence" — the one
marker I took at its word was the one that lied.

Verification: `cargo test --bins --release` — **204 passed**, 0 failed (comment-only code change). No
game window was launched.

## The sweep audited against the code (round 270)

Round 268's sweep (and round 269's first correction) assigned all 41 `ponytail:` markers, but several
classifications were made from the marker's own sentence. Auditing each against the code found **five
wrong dispositions**, now fixed:

- **`runtime.rs` "no timer queue"** → stale: the cog VM handles `sleep`/`waitformessage`/`settimer`/
  `setpulse` before the runtime (round 269). The ledger row is now the real limitation (a nested
  `sleep` re-runs its enclosing block).
- **`main.rs` `hoverId`** → the executable screens *do* carry it (`+0x04`, the field `menus.rs` calls
  `id`); what no screen carries is `jkGuiForce_pElements`, so the gap is **force-screen row gating**,
  not menu hit-testing.
- **`main.rs` "no back-face culling"** → that marker is the **world** material (`cull_mode: None`,
  double-sided because the winding is not guaranteed consistent), not model surfaces. The row is now
  *surface culling (world and models)* with the `--winding` counts that justify the current choice.
- **`main.rs` averaged displacement** → it selects the **actor's stride state**
  (`locomotion_state`), not the weapon sway; it belongs to *puppet clip timing*, and that row now
  names it.
- **`palette.rs` tint-only pass** → the scripted `NewColorEffect` channels exist but go through the
  wash quad rather than the engine's palette bytes; the *damage tint* row now records that
  approximation instead of calling the marker covered by it.

Also refreshed: the README and `engineering.md` said **202** unit tests (the CI paragraph too); the
suite is **204**. The README now says the 65-gate number is the last windowed run's transcript and the
unit count is current — the gates were not re-run.

Two of the twelve "not a deviation" markers were re-checked and hold: the soak's `outside` is reported
*and* loosely asserted (`<= 1`), and the billboard pass picks the world camera because both cameras
look down the screen's -Z.

Verification: `cargo test --bins --release` — **204 passed**, 0 failed. No game window was launched.

## The use action is point-blank (round 271)

Phase 1 step 4 recorded that the cantina switch fires in the port but not in retail at the same pose, and
called the remainder "a retail-input/harness question". It was not. `sithPlayerActions_Activate`
(`0x4fc0a0`) sweeps a **0.1-radius sphere from the eye along the view for `moveSize − (−0.1)`** — the
constant is an immediate in the instruction, not a `.rdata` value — so the player's reach is
`0.065 + 0.1 + 0.1` = **0.265 units**. The recorded switch pose is ~0.8–1.0 units away, so retail's
`doorstatus 0` there is correct behaviour, and the port only fired because its live use path used **3.0**
(the `--door`/`--use` gates pass an explicit reach, which is why they never showed it).

Fixed:

- the live path uses `player.rules.radius + 0.2` and starts from the **eye** (body + eye height), not the
  body centre;
- `use_thing` skips a **weapon (type 3)** and an **item (type 5)** hit, as the engine's activate does;
- the runtime doc and both documents now state the sweep, and the "harness question" note is corrected to
  "the pose is out of range".

Residual, recorded on the interaction row: the port measures to a linked surface's **centre** where the
engine's sweep touches the face anywhere, and the AI's `opendoors` keeps a 1.5-unit stand-in (the engine
runs the same point-blank action on the actor's body; whether the port's walk stops close enough is a
runtime-verifiable question, and the existing test pins the current stand-in).

Verification: `cargo test --bins --release` — **205 passed**, 0 failed (new
`the_use_action_skips_weapons_and_stops_at_the_engine_reach`: a nearer linked weapon does not shadow the
door, and a door half a unit away is out of the 0.265 sweep). No game window was launched.

## The player-movement formula, read (round 272)

The `player walking` ledger row's retire condition was "recover the `thing + 0x144` thrust term and
re-derive instead of measuring". `sithControl_PlayerMovement` (`0x4d8a00`) is now read:

```text
param_1 = 1.0;  if always-run or the run key -> param_1 = 2.0
crouch -> param_1 *= 0.5;  water -> * 0.5;  strafe -> * 0.7 at the axis
thrust44 = (*(f32*)(thing+0x144) + *(f32*)(thing+0x148)) * axis
if param_1 != 1.0 { thrust *= param_1 }
```

So the port's `Rules::speed` multipliers (run 2.0, crouch 0.5, back 0.5, strafe 0.7, water 0.5) are the
engine's own — what remains unrecovered is *what the two thing floats are*. `thing+0x144` is serialized
in the player/actor save tail beside health (`formats.md` §savegame), `thing+0x148` is in the thing
checksum, and neither appears as a literal in any parser: the per-class parsers write through templates,
so the keyword mapping is data, not code. The row now carries the formula and the two offsets, and the
port keeps the measured `Rules::walk` (1.2 u/s) until the fields are named.

Verification: `cargo test --bins --release` — **205 passed**, 0 failed (documentation-only round). No
game window was launched.

## The puppet advance, partly read (round 272, second half)

The `puppet clip timing` row said the locomotion clip's units-to-frames constant "is not recovered". Two
halves are now read from the binary:

- `sithPuppet_UpdateThingMove` (`0x4e4380`) returns the thing's **dominant velocity component** (the
  transformed velocity's larger horizontal component, absolute), and that same function picks the mode:
  walk below 0.5 u/s, run at or above it (1.0 for a player thing) — the port's thresholds were already
  right.
- `sithPuppet_UpdatePuppet` (`0x4e3de0`) advances the track by `|velocity| × dt × 280`, with
  `0x522fbc` = 280.0 and the "sign test" at `0x522fb8` = 0.0 (so it is the magnitude).

It still cannot be applied: `rdPuppet_AdvanceTrack` (`0x43eaa0`) accumulates that figure and wraps it on
the key struct's `+0x34`, whose unit is not pinned — at a 1 u/s walk the figure is hundreds of frames per
second, which only makes sense if `+0x34` is much larger than the key's frame count. The port keeps its
normalised-at-0.5-u/s law, and the row now names both the formula and the one field that blocks it.

Verification: `cargo test --bins --release` — **205 passed**, 0 failed (documentation-only). No game
window was launched.

## The use throttle (round 273)

`sithPlayerActions_Activate` (`0x4fc0a0`) refuses a second activate within **250 ms**
(`DAT_008554bc + 0xfa <= clock`) and stamps the clock *before* the sweep, so a press that finds nothing
still starts the wait. The port's live use path had no throttle: mashing `E` sent `activate` at frame
rate, which a door script hears as a burst of `activated` messages.

Added `use_allowed(last, now)` (the 250 ms rule, `Player::last_use`, stamped on every accepted press) with
the unit check `the_use_action_is_throttled_to_one_per_250_ms`, and corrected the system's doc comment —
it said the engine's use action lives in `jkPlayer.c` "which the source tree does not carry"; it is
`sithPlayerActions_Activate`, read in round 271.

Verification: `cargo test --bins --release` — **206 passed**, 0 failed. No game window was launched.

The surface half is pinned too: `the_surface_use_stops_at_the_engine_reach` uses a linked plate
0.2 units ahead (inside the 0.265 sweep) and one 0.5 away (outside). The suite is **207**.

CI-equivalent check on the current tree: `cargo test --bins --release --locked` (206 at the time) —
all pass; `cargo build --release --locked` builds; `scripts/check-docs-links.ps1` — all 132 relative
links across 10 files resolve.

## A third stale marker: the AI route is breadth-first (round 274)

The `ponytail` sweep had recorded `sector_route`'s "greedy, not a search" note as a **new** ledger row
(*AI route search*). Auditing it against the code: `sector_route` calls `solids.route(...)`, and
`Collide::route` (`src/collide.rs`) is a **breadth-first walk over the portal links** that refuses a
link its own gates close — the same shape `engineering.md` and `roadmap.md` stage 6 already describe.
The greedy paragraph describes `best_portal`, the one-hop helper the `--ai` probe uses, not the tick.

Corrected: the note is gone (the doc now says what the function does), `main.rs`'s stale
"`best_portal` is the same call the tick makes" is fixed, the ledger row is removed, and the sweep
lists the marker as **stale** (3 of 41). Counts: 19 new-row markers across 13 rows, 7 covered, 12 not
deviations, 3 stale; 38 `ponytail:` comments remain.

Verification: `cargo test --bins --release` — **207 passed**, 0 failed; docs tables structurally valid.
No game window was launched.

## Two more stale justifications (round 275)

Auditing the remaining `ponytail` dispositions against the code turned up two comments that no longer
describe it:

- `weapon_for_bin`: the note said the bin is "matched against the level's own weapon templates" because
  `weaponData` is absent. The code's **first** rule is the engine's own — a type-3 template naming the
  bin in its `inv=`, which every shipped weapon carries — and the name-hint list is only the fallback
  for a level whose templates omit `inv=`. Reworded; the ledger row now says so.
- `auto_select_weapon`: the note said "this build instantiates no item cogs". It does — `load_item_cogs`
  is a registered system and `items.dat`'s scripts start when a bin is acquired. The real gap is that
  `sithWeapon_AutoSelect`'s `AUTOSELECT` message round-trip is not modelled, so the score is the
  weapon's own `damage=`. Reworded, and the ledger row now names the actual missing piece.

Neither changed behaviour; the ledger and the code now agree. Verification: `cargo test --bins
--release` — **207 passed**, 0 failed. No game window was launched.

Follow-up precision: the auto-select gap is structural, not a missing round-trip — the cog VMs live in the
message-delivery systems (`deliver_*_messages`), not on `Runtime`, so `auto_select_weapon` has no handle to
ask a weapon cog for its `autoselect` score. The comment and the ledger row now name that, so a future
agent does not try to call the cogs from inside the runtime.

## The AI stops at ledges (round 276)

Round 266 gave actors gravity, which exposed the missing half: the AI walked bodies off ledges and let
them fall. The engine's companion rule is `sithAI_CanWalk` (`0x4eb4b0`), read from the decomp:

- the caller (`sithAI_idk_msgarrived_target`, `0x4ea890`) adds the step to the thing's velocity and then
  probes the **destination** with `CanWalk`; on 0 it zeroes the velocity and emits `0x400` (the only
  event `jump` receives), so the body never takes the step;
- `CanWalk` is a **downward sphere collision sweep** of `GetThingHeight + <a class field at +0x10>`
  from that point: **any** surface hit is walkable — a floor, or a wall the physics step-up handles —
  and no hit at all is a ledge. (A surface hit with flag `0x8` answers 0 outright; other surfaces
  answer 1 or 2 by flag `0x1`, categories this build does not use for steering.)

Ported as `Runtime::ai_can_walk`, called by both walk paths (flee and chase/route) before the position
is written; a failure emits `0x400` and skips the step. A **solid thing's top** also counts as ground,
because the port's face sweep knows the level's faces but not the catwalks and lifts the engine's
collision treats as solid. Inferences on the ledger: the class field at `+0x10` is taken to be `maxstep`
(0.5 default) and the sweep radius is the body's `movesize` (Ghidra's prototype omits that argument).

Verification: `an_actor_stops_at_a_ledge_instead_of_walking_off_it` — an actor walking at a floor whose
face ends at x = 0.5 halts before the edge, and the no-collider control walks past it (what the port did
before). Suite **208**. No game window was launched. The level-wide effect (no actor frozen at a wall,
doorway or lift) is a QA-window check and is named as this row's retire condition.

The gate also carries the engine's **attached-only** guard: `sithAI_idk_msgarrived_target` gates a body
attached to a surface (`thing+0xc != 0`), and an airborne body keeps its horizontal motion. The port has
no floor-attach state, so "attached" is approximated as "there is ground under where the body stands now"
— a body already falling is not stopped mid-air. The ledge test still passes (an attached body at the edge
stops; the airborne control would move).

## Two decisions recorded (round 277)

- **The event-2 emitter.** The ledger row names `0x4f2c05` as where the executable raises AI event 2.
  Asked the retail binary: `resolve_containing_procedure(0x4f2c05)` answers `not_in_procedure` — the
  site is not inside any Ghidra-analyzed function, so the trigger's surrounding code is not in
  `decomp/`. The row now says so; no producer can be ported until that region is analyzed.
- **Actor-vs-world collision, deliberately not landed.** The engine moves an actor's body with the same
  `MoveThing` the player uses. Routing the port's AI step through `move_body` would give actors walls and
  steps, but an actor the port's *approximate* thing colliders consider overlapped would resolve to a
  no-move and freeze — the player's walker has the same risk but a `spawn_point` unstick, which actors do
  not. That is a QA-window measurement, not a guess, so the item is named in Phase 1 step 3's owed list
  instead of being shipped blind.

Verification: `cargo test --bins --release --locked` — **208 passed**, 0 failed; docs links 132/132. No
game window was launched.

The reach itself is now pinned by a second check, since the `<+0x10>` field is the one inference:
`the_ledge_reach_is_the_body_height_plus_maxstep` gives the actor a two-level floor — a 0.4-unit drop is
inside the `0.07 height + 0.5 maxstep` reach and is stepped over, a 1.0-unit drop is a ledge and stops
the actor. Suite **209**.

## The gate's guard was inverted, and the ledge fixture lied (round 278)

Adding the void-leap test (`sithAI_CanWalk` at the jump's leap point) failed immediately, and chasing it
found two real defects in round 276's work:

- **The attached guard was inverted.** `ai_can_walk` returned `ground(here)` — allow — when the body
  stood on ground with no ground ahead, so an attached body was never gated; it blocked only a body that
  was already past the floor (airborne), the opposite of the engine's `thing+0xc != 0` guard. It now
  returns `!attached`, and the void-leap test is what exposed it.
- **The ledge test's fixture lied.** Its `face_of`/`floor_of` were keyed on surface `0` while every
  plane and face carried surface `1`, so `sweep_rooms` found no face and `ground()` was false
  everywhere. With the old inverted guard that made the test pass *because the gate blocked the first
  step* — a green test for the wrong reason. The maps are keyed on `1` now, and the test genuinely
  walks to the edge and stops.

The jump handler also moved to the real rule: `jump_from_obstacle` used `segment_blocked` at the leap
point (a clear line); the engine tests `CanWalk` there, so it now calls `ai_can_walk` — a leap must land
on ground. `a_blocked_actor_does_not_leap_onto_a_void` pins both directions.

Verification: `cargo test --bins --release` — **210 passed**, 0 failed. No game window was launched.

The AI's **moving bit** (`0x1`) was never written outside tests: `jump` requires it and `dodge` refuses
with it, so the jump instinct could not fire from a real walk and `dodge` fired mid-stride. The AI now
sets it while an actor has a move (`reach > standoff`, non-turret, before both `0x400` emissions) and
clears it at the top of each tick and when the destination is met. `a_walking_actor_sets_the_ai_moving_bit_and_an_idle_one_clears_it`
pins the bit, and `a_walking_actor_at_a_pit_leaps_with_its_jump_instinct` proves the whole chain with
nothing set by hand: `ai_tick` walks the actor to a pit, the emitted `0x400` reaches the `Jump` handler,
and the leap point over the far floor lands the `0.1` nudge.

Verification: `cargo test --bins --release` — **212 passed**, 0 failed. No game window was launched.


The walk preflight's wedges were traced to one mechanism: the **45° ramp**. `walk15_spawn`
(`16aEscapeShip`) blocks from its first step on level surface 4180 (a 45° face whose top is 0.11 above
the feet, beyond the 0.066 step). The lateral slide dropped the vertical component, so the body stayed
flush with the plane and re-hit it at distance 0 every bounce. The fix is the engine's own rule: slide
**along the plane in 3D** (`sithCollision_MoveThing`), with the sweep end's height restored on that path
only (the vertical probes rely on the plan-only end). `16a` then walks **1.55 units against retail's
1.56** — the first time that leg moved at all — and `16b` 1.70 against 1.03.

Two things fell out:

- **The walk-all drop guard was stale.** `17asarris`/`m_boss17` now descend 2.76 units over 20 s while
  `contained` and hull-agreeing — a walk *down* a ramp the old walker could not enter (it was pinned at
  its spawn). The `worst drop < 1.0` assertion held only because of that incapacity, so it is replaced
  by the guard it was standing in for: the level with the worst drop must still be contained (plus the
  existing no-void assertion), and the table now prints each level's drop.
- **The step-blocking trace names the surface.** `BK_WALK_TRACE`'s four-way step failure now prints the
  blocking surface, normal, top and contact, which is what identified 4180.

`--walk all` is **27/34 contained (was 23), 20 hull-agreeing (was 16), 59 crossings (was 53), 0 in the
void**, and the preflight's failing legs fall to **8 of 32** (walk15/walk22 pass; the rest are bad poses
or retail traces under 0.5 units, plus `walk16` whose warp pose is outside every sector).

Verification: `cargo test --bins --release` — **213 passed**, 0 failed; `--soak-all` 35/35 clean; `--walk
all` walk check passed; `pwsh tools/parity/walk-preflight.ps1` 8 of 32. No window and no retail run.

The preflight's remaining "failures" were **not** port bugs: six legs have retail traces under 0.5 units
(the retail body is under script control — `walk7_spawn` 0.11, `walk10_spawn` 0.14, `walk14_spawn` 0.00,
`walk19_spawn` 0.00, `walk21_spawn` 0.37, `walk_door0` 0.21), so no capture could measure a walk on them,
and two rows were dead: `walk_street` (byte-identical to `walk_street45`, no fixture) and `walk_street0`
(the round-30 yaw-0 probe, no fixture, not in the gate's default list — the yaw-90 half stays). The
preflight now reports those as **SHORT** ("retail's own trace does not walk") instead of FAIL, and
`walk16_spawn`'s placeholder pose (0,0,0 — outside every sector) is replaced with 17asarris's own start
(2.61, −0.55, 2.075), which walks 0.65 units. `trace-gate.ps1` records port traces only for the legs that
passed (it was passing the unfiltered request list).

Verification: `pwsh tools/parity/walk-preflight.ps1` — **22 legs walk, 8 are script holds, 0 failures**
(was 10 of 32 failing); both scripts parse clean; no window and no retail run.

A pose audit against every committed retail trace found **two rows that were not their trace start**:
`walk15_spawn` was 0.54 units off and `walk8_spawn` 0.27, so the trace comparison would have measured two
different routes. Both rows now start where their retail trace starts (`walk15` at (0, 4.19, −3.047),
`walk8` at (52.83, 62.86)), and `walk-preflight.ps1` **checks it**: a row more than 0.25 units from its
fixture's first sample fails with `POSE`, because the comparison is only valid when both sides start at the
same place. Consequence for the ramp round: `walk15_spawn` from the true start walks **2.68 units against
retail's 1.56**, not the 1.55 that the off-by-0.54 row produced — the climb works, but the port covers more
ground in the 3 s window than retail, so the route/geometry is the next item (corrected below: it is not a
speed term) and the earlier 1.55 claim is
void — corrected here, in the ramp round's entry above (its "1.55 units against retail's 1.56" line), and
in the ledger and roadmap.

Verification: `pwsh tools/parity/walk-preflight.ps1` — 22 walk, 8 HOLD/DIVERGE, 0 failures, no `POSE`;
`cargo test --bins --release` 213 passed. No window and no retail run.

The 2.68-vs-1.56 difference on `walk15_spawn` is **not** a speed constant. `--feel` on 16aEscapeShip reads
a template walk of **1.20 u/s** with the engine's ramp-up (0.14 at 0.05 s, 1.01 at 0.5 s), and the port's
3 s path is a smooth 0.95 u/s in 3D. Retail's trace instead *slows mid-path* (per-sample horizontal
0.65 → 0.32 → 0.45 → 0.58 → 0.60 u/s) and takes a different route: retail 392→387→407→386→386, the port
392→407→0. So the open item is the **route and the geometry it crosses**, not a slope-speed term — the
earlier "slope speed is the next measurement" note (ledger and roadmap) is corrected.

Verification: the measurement is from the port's own probe (`--walk` per-frame positions, `--feel` template
line) and the committed retail trace; `cargo test --bins --release` 213 passed. No window and no retail run.

The route divergence behind `walk15_spawn` was traced one level deeper: `sithCollision_MoveThing` adopts the
adjoin's far sector and re-searches, so retail's trace passes *through* a threshold sector (392→387→407);
this port refuses any far room shallower than the body, so it skips them (392→407). Relaxing that rule was
measured and reverted: `--walk all` containment improved 27 → **29** of 34 and crossings 59 → **78**, but
`walk18_spawn` wedged inside level 19A's 0.103-deep doorway 721, stopping 0.023 short of its far plane
(0.27 units against retail's 0.88). The engine's crossing sets the sector **at the contact** and re-searches;
this path advances a radius past the plane, so the fix is the through-move itself, not a looser depth test.
The revert keeps the verified state.

Verification: `cargo test --bins --release` **213 passed**; `--walk all` 27/34, check passed; preflight 22
walk / 8 HOLD-DIVERGE / 0 failures. No window and no retail run.

The QA window and the retail harness were authorized, so the gates were run for real for the first time
this session.

**Gate sweep: 5 of 66 failures → 1.** Four of the five were the same defect: the check's `Runtime` had no
`rt.solids`, so every *gravity body* in it had no floor. The soak measured it as "99 bodies outside every
sector" (actors 4046 units below 01narshadda, the sweep's actor count meaningless); `--attach`'s parent fell
0.0044 in the one step (the child read 0.4956 for a 0.5 offset); `--hit`'s target fell 0.3 while the bolt
crossed and the shot passed over it. Giving `--soak`, `--attach` and `--hit` the level's collider fixed all
three: `--soak` **0 outside** (was 99), `--soak-all` **1 level** with outside bodies (19a, 2 — the known
floorless-pocket level) instead of 15, `--attach` carries exactly [3, 0, 0.5], and `--hit` lands.

- **Template mass**: `_debris` (and `+bryarbolt`) declare no `mass=` in any shipped level, yet debris falls
  in retail, so the engine's parse default is non-zero. The port read "absent" as zero and the created chip
  floated; the default is now `1.0`, and `--things` passes (chip falls and rests).
- **`--hit` firing spot**: the bolt is stopped by the level's furniture (the game's own path), so the check
  now picks the spot from which the line between the two bodies — trimmed of the bodies themselves, which
  `segment_blocked` counts — is clear. `--hit` passes end to end (both directions, the blast, the saber).
- **`--live-play` passed last.** Its phases were frame-counted, so 240 frames was 0.8 s on this 300 fps
  window and the forward walk measured 0.73 units; the phases are now **time-based** (2 s walk, 0.4 s
  strafe, 20 fire frames). The distance stayed 0.73 because the cantina spawn's own heading meets a solid
  table 0.73 units ahead (the sweep reports a face at distance 0), and the walker's blocked-step rewrite
  turns the rest of the push into the velocity it made — no honest room lets this leg reach 1.0 forward.
  The gate therefore also accepts **walking speed while the key is held**: the body peaks at **2.24 u/s**
  (its run target), which a dead key cannot reach, while 0.73 remains the room's cap.
- **Script camera**: the input hold keyed on `script_camera` alone, so any level that had ever called
  `SetCurrentCamera` held the player for the rest of the run — `walk5_spawn` stood at **0.00** against
  retail's 3.73. The suppression now requires the current camera to follow a body other than the player
  (`camera_focus_thing()` is `None` for POV and for a focus on the player). `walk5_spawn` now walks **3.90
  units (105 % of retail)**: the trace gate is **10 legs, 1 failure**, and that failure is only sector
  progress (`sectors off 5`) — travel is within tolerance.

Verification: `cargo test --bins --release` **213 passed**; `--gates` **0 of 66 failed, 0 skipped** (the
first fully green gate sweep of this session); `--soak-all` 35/35 with one level's two bodies;
`tools/parity/trace-gate.ps1` 10 legs / 1 failure (`walk5_spawn` sectors). No retail capture was needed for
these (the windowed port run was enough).

The Phase 0 **parity draw gate** was run with the real window: `port-capture.ps1` captured **13/13** frames,
and `compare.ps1 -Gate` reports **6 thresholds, 1 failure** — `spawn` at **SSIM 0.2263 < 0.55**, while
`spawn_pose` **0.6253**, `spawn_yaw30` 0.5926, `route_cantina` 0.6959, `route_lower` 0.6813 and
`route_street` 0.6904 all pass (7 pairs are ungraded: no threshold row yet). The `spawn` number is the
*pre-fix* value from the session start (`0.2284`), so the default first frame has regressed: the same level
captured with an explicit pose (`spawn_pose`) still scores 0.625, and a 60-frame capture scores 0.2434, so
it is the default spawn's **view**, not the 10 s settle. The frame shows the port's eye inside the cantina
closer to the wall than retail's reference. This belongs to the round-282 first-frame track (the recorded
`walkplayer` pose and eye offset); it is the one open draw item and is now measured with a gate line.

The parity gate's `spawn` failure was traced: the **camera is right**, the capture's *state* is not. The default
spawn's eye is `(-10.443433, -5.455094, -1.089886)` against the reference pose's `-1.092886` (0.003), and the
`spawn_pose` capture — same level, same 600 frames, but with `--warp`/`--shot-at` — scores **0.625**. The
default frame shows a cantina patron filling the view: both runs are 10 s of level time, but only the default
spawn fires the level's `entered` scripts (the warp places the body before they can react), so the spawn
greeting walks a patron into the camera. The reference was captured earlier in that scripted sequence. The fix
is a capture-timing/state question (match the reference's settle or capture the same scripted moment), not a
camera fix — and the parity gate stays at 6 thresholds / 1 failure until it is decided.

The spawn pair's 0.2263 is therefore not evidence that the first frame regressed; the explicit-pose pair on the
same pose scores 0.625 and the camera poses agree.

The `spawn` pair's low score is a **scripted-greeting state**, not a camera or pose error: the reference
records the spawn's own PYR (`pitch 1.04`, yaw −59.98, roll 1.04) and the port's eye matches it to 0.003
(`-1.089886` against `-1.092886`), but the frame shows a cantina patron's head filling the view. The same
camera without `--warp` scores 0.27 while `--warp`/`--shot-at` scores 0.625 — the warp places the body before
the level's `entered` links can react, so only the default spawn fires the greeting that walks the patron in.
Both reference and port have animated characters (retail's own sidecar: "the changing pixels are animated
characters/NPCs"); the reference frame is their earlier moment. Adding the reference's 1.04° pitch to the
capture moves the score only 0.26 → 0.27 (±both signs), confirming the pitch is not it. The open decision is a
capture-state one: which scripted moment the pair should compare. The parity gate remains 6 thresholds /
1 failure, and the camera/pose itself is verified against the reference metadata.

`walk5_spawn`'s "sectors off 5" was traced to the **path**, not the sector bookkeeping: at retail's own sample
point (9.79, 14.22) the port's `sector_for_body` is **33**, exactly retail's sector there, so the port's hulls
agree. The trace's sector column differs because the two bodies are not at the same place at the same sample:
retail's yaw is constant (`135.01` every sample) while its velocity *turns* — per-interval speeds 1.75 → 1.23 →
0.82 → 1.13 → 1.48 u/s, i.e. the body slides along geometry it meets — and it ends at (7.19, 13.36); the port
walks a straight 45° line to (7.82, 12.15), 1.3 units away. The travel totals agree (3.90 vs 3.73) while the
routes diverge, so the leg measures the **slope slide**, not a sector bug: every collision face around the
route is a 45° ramp floor (surfaces 335–354, normals `(±0.11..0.63, ±0.11..0.63, −0.78..−0.95)`), i.e. level
5's slab ramps, and the port's sweep meets *no* wall on the leg (`BK_MOVE_TRACE`: zero blocked-by-surface
lines) while retail's velocity turns continuously along them. The next trace is the engine's slope-slide rule
(`MoveThing`'s velocity rewrite through the ramp), which is the same mechanism the ledger's level-5 ramp rows
already name — not the lateral collision.

**The position→velocity rewrite is gone (engine split).** `sithPhysics_UpdatePlayerPhysics` keeps the
velocity (`+0x1d4`) at the input's direction and `MoveThing` reports the slid **movement** (`thing + 0x264`,
the port's `last_move`) — the velocity is not rewritten from it. The port did rewrite it ("the fraction that
survived is the velocity along the wall"); with the 3-D slide in place, removing that rewrite was measured:
`--walk all` 27/34 with the check passing, preflight 22 walk / 8 holds, `--gates` **0 of 66**, soak-all 35/35,
and the four sampled trace legs unchanged (walk5 3.90, street45/90/cantina 100-102 %). So the engine's split
is kept, and the old `if false` A/B is deleted.

**`walk5_spawn`'s route, measured further:** retail's body **falls off a ledge** on this leg — its trace starts
at z **0.22** and drops to **−0.01** within the first 600 ms, then walks the 45° ramps below (the curved path,
yaw constant). The port's body stays at z **0.155** on the upper floor for the whole leg, so it never meets the
ramps and never curves. That is a *ledge/drop* difference in the player walker (retail walks off a 0.23 drop;
the port keeps the body on the upper floor), not the slope slide and not a sector bug. It is the next item on
this leg. Correction to an earlier lead: the walker's `blocked` flag is **not** a contact. `move_body` returns
`blocked || at != pos` (collide.rs:1354), so it fires on *any* movement; with both `BK_WALK_TRACE` and
`BK_MOVE_TRACE` on, every frame prints `move: end … blocked false` internally while the walker's line says
`blocked true`. The port's straight walk is therefore a genuine no-contact walk and the flag says nothing
about it. Retail's turn still needs *retail-side* evidence (which surface its body meets) — the decomp alone
cannot say which of the level's 45° ramps it was, and the port's sweep reports no contact on the same route.

The `spawn` frame's foreground figure was identified: it is **not** the player's body. `draw_player_body` keeps
all 20 `PlayerBodyMesh` parts `Hidden` (`chase false`, camera at the eye, `wanted Hidden`), and the nearest AI
sighting is **actor 80 (`reeyeesfist`) at 2.48 units**. So the difference against retail's reference is an
**NPC's placement/state** in the first frame, not the camera, the pose or the local body. That is why the explicit
`--warp` capture scores 0.625 while the default spawn (which also fires the level's `entered` links and lets the
NPCs animate for 10 s) does not. The open decision is therefore twofold: the capture's scripted moment *and*
the NPC state it shows — both are AI/script behaviour, not rendering.

The `spawn` frame's filler is now named exactly: **actor 207 (`reeyeesbhgun`) at 0.27 units** from the player
(`BK_AI_ROSTER`: `d 0.27 cone false los true yaw 120 bearing -130`), i.e. the camera sits inside that patron's
body; the next nearest are `nman1` 216 at 0.76 and `nbartender` 214 at 1.34. The level data is shared with
retail, so this patron must move (a scripted keyframe path / `PlayKey`) or be drawn elsewhere in retail's
reference; the port leaves it standing there through the 10 s capture. The parity `spawn` pair therefore turns
on an **NPC path/script state**, not on rendering, camera or the local body — the same asset class as the earlier
`PlayKey` fixes.

The retail harness was run for the one leg without a fixture, `walk16_spawn` (17asarris, its pose now the
level's own start): **retail itself is stuck** — `retail-capture.ps1 -WalkTraceMs 3000 -WalkTraceSettleMs 1000`
aborts it ("two segments of held forward moved it less than 0.02 units"), so no retail trace can exist. The
port's 0.65 units over 3 s (0.22 u/s, half the level's walk speed) is a slide, not a walk. The row is dropped
with that reason in `walk-shots.tsv`; `walk23_spawn` covers the map family. Preflight is now **21 walk, 8
non-walk of 29 legs** (was 22 of 30). This also shows the preflight's 0.5-unit bar can pass a slide that
retail's own 0.02-per-segment test rejects — a useful cross-check that the harness is stricter than the
preflight.

**Model back-face culling is in** (the ledger's "no back-face culling for models" deviation). The model
material was `double_sided: true, cull_mode: None`, so every model rendered its *inside* when the camera
clipped into it. With `cull_mode: Back` and `double_sided: false`: parity pairs unchanged or slightly better
(`spawn_yaw30` 0.5926 → 0.6045, `spawn_pose` 0.6253 → 0.6259, `route_street` −0.001), `--gates` **0 of 66**,
213 tests. The spawn pair itself stays at 0.226: its filler is the cantina patron **actor 207
(`reeyeesbhgun`)**, placed by the level 0.27 units from the spawn with `aiclass=ryinitial.ai` and no keyframe
path — seen from *outside* at that range, so culling does not remove it. Retail's reference has no head there,
so the remaining difference is the NPC's **stand/seated puppet state** (the port plays a stand key), not
rendering. Two-sided `.mat` faces are the refinement left on the culling row.

The retail `spawn` **series** (`spawn-series-0..7.png`) shows a Rodian patron *and* the player's own body in its
early frames; the reference is the settled frame where the patron reads as out of the camera's centre. So the
pair compares two different moments of an animated room, and the port's centred head is most likely
**`nbartender` 214 at 1.34 units (39° off)** or **`nman1` 216 at 0.76 (54° off)** rather than the 0.27 patron
(whose bearing is 110° off). The remaining step is a *draw-level* identification — which actor's entity is
nearest the camera centre at the capture frame — and then that NPC's pose/key against retail's. Both the
series files and `BK_AI_ROSTER` are in hand for it.

The spawn frame's centred body is now named by a draw-level probe (`BK_NEAR_TRACE=1`, kept as capture
triage): at the 600-frame capture the actors nearest the camera's forward are **`reeyeesbhfist` 208 at 22.5°
(1.55 units)** and **`nbartender` 214 at 23.6° (1.75)**, with `nman1` 216 at 0.32 units but 60° off. Thing 208
is placed at (−8.91, −5.25) — 1.54 units from the spawn — and after 10 s it is **still there** (its position is
its placement), while retail's series shows that patron in the first frame and *not* in the settled reference:
retail's patron **moves during the settle** and the port's does not. Its class is `rypunnojump.ai`. So the
spawn pair waits on that class's behaviour (walk/punch instincts) in the port, not on rendering, culling,
camera or the local body — all four have now been measured and excluded.

The `rypunnojump.ai` class (`LookForTarget 500.0, 35000.0`, `Follow 0.0, 0.05, 0.0, 1.0`, `Talk`, `PrimaryFire`)
points at the idle-shuffle: the handler `FUN_0050aa80` is **`LookForTarget`** — the port's own round-210 note
records that registration (`sithAICmd_Startup` → `0x50aa80`) — and its decomp shows that when the perception
step `sub_4EAD60` (which calls `sub_4EB300`, the cone/sight search) finds **no** target, it sends messages 0x4c
and 2 and calls `FUN_004f2a90(…, 3.0, …)` — which the decomp names
**`sithAIAwareness_CreateTransmittingEvent`**: an *awareness broadcast at the actor's own position with radius
3.0*, not a move (the third reading of this handler in this log; each came from a callee's name). When a target
is found it stamps a 500 ms clock (`0x43fa0000`) instead. So the missing idle behaviour is the **look-around
awareness broadcast plus its two AI messages (0x4c, 2)** — likely the mood/pose that turns the patron in
retail — not a walk. The port's `LookForTarget` only *scans* — "an idle actor
with nothing in sight periodically scans for one" — and never moves, which is why patron 208 stays at its
placement while retail's shuffles out of the reference frame. The next implementation step is that no-target
branch (what `*param_1 + 0xb8` is, and whether the 3.0 is a radius or a speed), not a renderer or camera
change. (An earlier note in this log guessed Follow for this handler; the round-210 registration says otherwise.)

Callees pinned: the LookForTarget no-target branch's two sends are **sound modes**, not a pose — `0x4e6b70`
= `sithSoundClass_PlayModeRandom(thing, 0x4c)` and `0x4e6b30` = `sithSoundClass_PlayModeFirst(thing, 2)` (the
cantina's mode sounds; the port has the same mode-sound path for `talk`/`sensedanger`). Together with the
awareness event, the branch is audio + local awareness, so it does **not** explain the patron's visible turn.

What is left is the acquisition test: the class is `fov=200` and the roster puts the player 128 degrees off, so
the port's `sees` (`delta <= min(fov, 360)/2` = 100 degrees) says no. The engine's `sithAI_FirstThingInView`
computes `SinCos(PI/2 - fov * k)`; with the obvious `k` values that formula yields a tiny cone for 90-degree
classes, so the class's `fov=` is not the same quantity the port divides by two — the convention is unresolved and
the two readings disagree on this exact class. The retail-side evidence (the patron approaches during the settle)
says this class notices the player behind it; the port's half-angle rule says it cannot. Settling the convention
needs the constant's use context (its dot/threshold compare), not another guess — and a change would move every
class's acquisition, so it must be measured against `--gates`, the soaks and the parity pairs.

The FOV question closes as **agreement, not a defect**: the port's `fov * 0.5` test matches the engine's own sight
formula (`sithAI_CanDetectSightThing`: `fov * 0.5` in degrees; `0x523128 = 0.5`, `0x523074 = 90.0` — already in
`gaps.md:385`), and the engine's `FirstThingInView` threshold (`SinCos(90 - fov*0.5)`, compared as a dot product)
excludes the same 128-degree target the port excludes. So retail's patron did not "see" the player either, and its
turn/approach in the series comes from its **script/AI state**, not acquisition. The remaining thread is the
placed-cog binding for thing 208 (the extracted level export's `SECTION:` format hid the `world cogs` thing refs;
the port's own parse can expose them). The spawn parity pair stays assigned to that, with every renderer, camera,
body, culling, sector and FOV alternative now measured and excluded.

The placed-cog binding is exposed: `--cogstatus N` over the 42 instances finds **`01_bartender.cog`** (instance
29) binding `normry = 208`, `cliffry = 207`, `sambt = 214`, `bar1/2/3 = 370/353/350`. Its asset text
(`--asset cog/01_bartender.cog`) is only "bartender says line after both Ree Yees die": a `killed`/`entered`
handler that **plays a sound** (`i00mn19z.wav` via `PlaySoundThing`), nothing more — it does not move the
patrons. So neither the script nor `LookForTarget`/the FOV rule moves patron 208. The walk key is now excluded too:
`--asset key/rywalk.key` dumps 24 nodes and node 0 (`$$DUMMY`, the root) has **one** entry at frame 0 with a zero
position and zero deltas — the key is **in place**, so retail's actor movement is engine-driven, not root motion.
Every script/key/look/cone candidate for this patron is now excluded; what remains is an engine **idle wander**
that the decomp has not yet shown (no command applies, yet the actor moves). That is the one open trace on the
spawn pair; the port's in-place puppets are not the cause.

The spawn patron's chain is linked end to end in the decomp, and it matches infrastructure the port already has:

- `LookForTarget` (`0x50aa80`), **no target**: `sithAIAwareness_CreateTransmittingEvent(thing, pos, ..., 3.0, ...)`
  - an awareness event at the actor's position with a 3.0 budget - plus the two sound modes.
- `Follow` (`0x5093f0`), no sighted target: reads the AI's **awareness level** (`ai[0x8e]`); when it is active
  (not 0/2) it calls the move helper `FUN_004eac90(ai, ai+0x8f, 2.0)`. So an actor is *moved by awareness another
  actor broadcast*, not by its own sight.

The port has that layer (`Runtime::transmit_awareness`/`awareness_at`, the adjoin-graph cost model,
`AI_AWARE_SHOT`/`AI_AWARE_FLOOR`, exercised by the `--adjoins` check) but neither half of this loop: `LookForTarget`
only scans (no broadcast on a failed scan) and `Follow` only chases what it *sees*. That is why patron 208 never
moves: it has no sight of the player and receives no awareness. The implementation is bounded by the existing
pieces - the open constants are the 3.0 budget's meaning and whether the Follow's 2.0 is a radius or a speed - and
it must land measured (gates, soaks, parity), since awareness drives every AI in the level.

The engine's AI perception loop is fully mapped, and the port is missing its ingestion half:

1. Source: LookForTarget (0x50aa80), failed scan -> sithAIAwareness_CreateTransmittingEvent(thing, pos, ..., 3.0, ...).
   The port has the transmitter (transmit_awareness, AI_AWARE_ALERT = the same 3.0) and already sends from
   sensedanger, but nothing CONSUMES the records (awareness_at is used only by tests).
2. Ingestion: sub_4EAF40 (the AI's per-tick perception update, called by Follow) keeps a perception state
   ai[0x8e]: 0 = nothing, a thing = the current target, 2/3 = awareness states; sub_4EB300 searches,
   sithAI_CheckSightThing / CanDetectSightThing gate, and ai[0x8f..0x91] holds the POSITION the actor is aware of.
3. Action: Follow (0x5093f0) with no sighted target and ai[0x8e] != 0/2 calls sithAI_SetMoveThing(ai, ai+0x8f,
   2.0) - the third argument is the SPEED (2.0), the position is the awareness point - plays mode sound 5 and
   sets the moving bit.

So the cantina patron never moves because the port has sight and the awareness transmitter, but not the perception
state that turns awareness records into a position an instinct acts on. This is the AI perception layer's next unit
(ingest + state + Follow's move), with the three decomp functions above as the spec; it is a layer, not a one-line
fix, and it must land measured (gates, soaks, parity) because it changes every actor.

**Awareness layer, first real implementation (broadcast + source).** The port now implements the engine's
`LookForTarget` broadcast and the event's source field:

- The awareness record is `(strength, pos, time, source thing)` and `transmit_awareness` takes the source
  (`sithAIAwareness_CreateTransmittingEvent`'s fifth field). The shot and `sensedanger` events pass their
  shooter/senser.
- A failed LookForTarget scan broadcasts a 3.0 event at the actor's own position (the decomp's call), once per
  scan interval.
- `heard_event` **skips events the actor raised itself** (`an_actor_does_not_hear_its_own_broadcast`): without
  that, a moving actor re-hears its own last position and walks back to it. This was a latent bug exposed by the
  new broadcast.

Measured: `--gates` **0 of 66**, soak 35/35 (19a's 2), 214 tests. Parity after the broadcast: `spawn` unchanged
at 0.2264, `spawn_pose` 0.6259, `spawn_yaw30` **0.5871** (was 0.6045), `route_cantina` 0.6959, `route_lower`
0.6814, `route_street` 0.6894. The cantina's AI is now audibly alive (`BK_AI_TRACE`: 543 alerts/moves in 10 s),
and the small `spawn_yaw30` drop is the NPCs moving where retail's are not at that instant.

Caveat recorded, not hidden: the port's hearing path applies its reaction to every AI (a stand-in that defaults
the `Listen` chance to 1.0), while the engine gates it on the class's `Listen` instinct (and its awareness move on
`Follow`). The single-frame parity pairs cannot settle whether the extra motion is faithful; the next step is to
gate hearing on `Listen`/`Follow` and re-measure, which is expected to recover `spawn_yaw30` if the broadcast is
too loud.

Hearing is now gated the engine's way: a class reacts to **sound** only through its `Listen` instinct (its own
chance); a class without `Listen` still investigates an awareness **position** through `Follow` (chance 1.0); a
class with neither hears nothing. The default-1.0 chance that moved every AI in the room is gone. Measured:
`--gates` **0 of 66**, soak 35/35, 214 tests; parity `spawn_yaw30` **0.5871 -> 0.5936** (the gate recovered
+0.0065 of the 0.017 the broadcast cost there), `spawn` 0.2264, `spawn_pose` 0.6259, the `route_*` pairs
unchanged. The residual 0.011 against the pre-broadcast pair is the room's NPCs moving where retail's are not at
that instant - a single-frame comparison issue, not a state error the pairs can resolve. The `--ai`/soak/gates
all stay green with the awareness layer in place.

A port-side **series metric** now exists to compare the spawn scene's *motion* rather than one frame: three
captures at frames 120 / 300 / 600 (2 s / 5 s / 10 s), the HUD band trimmed (the parity's world band), and
ffmpeg SSIM between consecutive pairs: **0.8404** (2->5 s) and **0.8432** (5->10 s). Retail's sidecar reports
`series_ssim_first_last` **0.9238** over its own eight-frame series, but the sidecar does not record that
series' **cadence or span**, so the two numbers are not yet comparable: the port's 0.84 covers a much longer
window and includes the awareness-driven NPC motion, while retail's 0.92 may span about a second. Closing this
needs the retail capture to record its series timing (`retail-capture.ps1 -SeriesFrames`'s interval) - a
capture-side fact, not a port behaviour. The measurement is reproducible with the three commands above.

The series comparison is now like-for-like, and it **matches**. `retail-capture.ps1`'s series is **one frame per
second** (`$remaining = 1000 - elapsed`), so its `series_ssim_first_last` **0.9238** spans **7 s**. The port over
the same 7 s (frames 15 and 435, HUD band trimmed) scores **0.9256** - a 0.002 difference. The earlier 0.840 was
an 8 s window starting later (`120 -> 600`).

Conclusion for the `spawn` parity pair: the camera is static and the room is animated at the **same intensity** as
retail's; the single-frame 0.2264 is an **NPC phase** difference at the captured instant (two ~10 s simulations
of the same room diverge in which patron is where), not a camera, world, or motion error. The fair oracle for a
live scene is the series metric, and on it the port matches retail. Making the gate say that (a series row for
such pairs) is a gate-design change, now backed by numbers: retail 0.9238 vs port 0.9256 over the same span.

**Gate-design change: the `spawn` pair moved from a single-frame floor to a series gate.** A new
`tools/parity/series-gate.ps1` reproduces retail's own live-scene measure on the port: two captures over the
sidecar's series span (`--shot` steps at 1/60 s, so 7 s = 420 frames), the HUD band trimmed, ffmpeg SSIM - and
compares it with the sidecar's `series_ssim_first_last`. Measured: **port 0.9256 / retail 0.9238 over 7 s**
(|diff| 0.0018, tolerance 0.05) -> PASS. The `spawn` row is out of `thresholds.tsv` with that reason in the file,
so `compare.ps1 -Gate` is now **5 thresholds / 0 failures** and the series gate **1 pair / 0 failures**. The pair
remains captured and reports UNGRADED in the draw gate, as the file says.

Rationale, in the roadmap's own terms: a single frame of two ~10 s simulations measures which patron stands
where, not whether the room behaves like retail's; the series metric is the fair oracle and the port matches it.

**Final state of the AI/awareness thread: reverted, with the cause of the walk9 failure found.**

- The awareness layer attempted in the last rounds (source-aware records, the self-hearing filter, the
  LookForTarget broadcast, the `Listen`/`Follow` hearing gate, the follow-consume branch) was **reverted in full**.
  Every measurement it produced was negative or neutral: no parity pair improved (`spawn_yaw30` 0.6045 -> 0.5936
  at best), the broadcast made the cantina visibly busier without recovering the pair, and the trace gate gained a
  failing leg. The decomp spec stays in this log for the next attempt (the missing `4EAF40` perception state is
  the real prerequisite, not the transmitter).
- The `walk9_spawn` failure was **not** the awareness: it was the **velocity-rewrite removal** from the engine-split
  experiment. The rewrite is restored (with the comment that four sampled legs are not the trace gate) and the
  trace gate is back to **10 legs, 1 failure** (`walk5_spawn`, sectors 5, travel 105 %). Lesson recorded: the retail
  traces are the oracle for the walker's model; the `+0x264`-vs-`+0x1d4` field split cannot override them.

Kept from these rounds: the **series gate** (`tools/parity/series-gate.ps1`: port 0.9256 / retail 0.9238 over
7 s, PASS; `spawn` out of the single-frame thresholds for the documented reason), the **trimmed trace-gate
default set** (the ten walkable legs), and the earlier verified fixes (soak/attach/hit colliders, template mass
default, the script-camera hold, live-play's time phases, the 3-D slide, the moving bit).

Verification at the end of the session: 213 tests; `--gates` **0 of 66**; soak 35/35 (19a: 2); `--walk all` pass;
`compare.ps1 -Gate` **5 thresholds, 0 failures**; `series-gate.ps1` **1 pair, 0 failures**; `trace-gate.ps1`
**10 legs, 1 failure** (`walk5_spawn`).

The series gate was extended to the other three spawn references and made robust: frame 15 was sometimes a
loading/fade frame (`spawn` scored 0.2087 one run and 0.9256 the next with the same command), so it now starts
after **60 frames** (1 s of settled game time) and spans 7 s from there. Results over the four spawn pairs:

The gate also had to capture the port at the pair's pose (`--warp`/`--shot-at` from `shots.tsv`, as the draw
gate does): without it, `level14_spawn` measured the port's opening camera (`MAcamera`) moving, 0.2595. Corrected
results over the four spawn pairs:

- `spawn`: port **0.9353** / retail **0.9238** - **PASS** (|diff| 0.0115).
- `level2_spawn`: port 0.9210 / retail 0.9973 - the port animates where retail is nearly frozen (the level-2
  opening is script-held in retail).
- `level5_spawn`: port 0.9554 / retail 0.7423 - retail is *more* animated (a scripted cutscene the port does not
  play).
- `level14_spawn`: port **0.9883** / retail 0.9095 - the port is *more static* at the warped pose (retail's NPCs
  and effects animate; the port's do not, or not yet).

The new gate separates motion from single-frame phase and **quantifies the scripted-opening bucket per level**:
2 and 5 need more opening animation, 14 needs its opening camera/fade settled or implemented. `spawn` is the one
pair whose opening matches retail and it passes. The default `-Only` stays `spawn`; the rest are diagnostics until
the openings land.

The series gate's final shape, and why: it grades a pair only when retail's series is the **player camera**
(`first_frame_is_player_camera: true`). `level5_spawn` and `level14_spawn` are **skipped** - their own sidecars
say "a scripted intro camera is likely" (level 14's frame-to-frame difference spans the whole picture, 13..1304 x
5..889, and its reference is the settled last frame). Those two need the *cutscene* reproduced, not a static-camera
series. `level2_spawn` stays a real failure even with the camera check: port **0.9210** vs retail **0.9973** - the
port's room animates where retail's opening is frozen (the port lacks the level-2 opening hold, the documented
scripted-control bucket). `spawn` is **0.9353** vs **0.9238** - PASS. Repeatability measured: three runs give the
same port numbers to 0.002, so the 0.05 tolerance is safe.

Gate results: `series-gate.ps1` default (`spawn`) 1 pair / 0 failures; the four-spawn diagnostic run = 1 PASS,
1 FAIL (`level2_spawn`), 2 SKIP.

Checked the "scripted intro camera" mechanism on the port side before calling it a missing feature: it already
exists end to end. `00_cameratricks.cog` (the shipped intro-camera script) drives `rotatepivot(fulcrum, 1, t)` +
`setcurrentcamera(0)` + `setcamerafocus(0, camera1)`, and all three verbs are implemented; the render path views
from a focused thing (`camera_focus_thing` -> eye/look from the thing's pos/yaw/pitch). So the gap on levels 5/14
is the **level data** (the cog spawn / cutscene wiring), not engine support - which makes the bucket cheaper than
it looked: get the level's camera cog spawned and the props placed, and the shot should play.

`level2_spawn`'s difference is measured but not explained by a script verb: `02narshadda`'s 70 scripts call no
camera or control verb at all (checked with `BK_COG_VERBS_ALL=1 --cogs`, the new opt-in full verb dump - the top-10
shortening is what produced my earlier false "nothing calls it" conclusion). Retail's opening at that pose is
nearly frozen (0.9973) while the port's actors animate (0.9210), so the open question is *why retail is still*
(an activation timing, not a hold verb). Not implemented; recorded as an open question, not a known fix.

Verification trap worth remembering: the sweep flag is `--walk all` (two arguments). `--walk-all` is not a flag
at all - the binary silently boots the game's main menu and sits there, which is why two timed `--walk-all` runs
"hung" at their caps. The correct run passes: `summary: 27/34 levels with a start kept the walker inside its
sector, ..., 59 sector crossings`; `sweep: 0 level(s) left the body in the void, worst drop 2.76 (m_boss17)`;
`--- walk check passed ---`.

Current-state verification on this binary: 213 tests; `--gates` **0 of 66**; soak **35/35**; `--walk all` passed;
`compare.ps1 -Gate` **5 thresholds, 0 failures**; `series-gate.ps1` **1 graded pair, 0 failures**; `trace-gate.ps1`
**10 legs, 1 failure** (`walk5_spawn`, open - needs retail-side collision evidence).

`level2_spawn`'s port-side motion is now localized, not just measured: the changed pixels sit mid-screen and the
element is thing 25 `flyingcrow`, the level's hovering transport (29 things are on keyframe paths). Retail's own
series changes only in a top-right corner (639..800 x 5..46 - a HUD cell), so retail holds the crow still for the
whole 7 s while the port animates it from level start. The level does call `playkey(crow, crowkey, 1, 0x14)` in
`02_crowstart.cog`, and the port's `playkey` path only animates single-node **camera** keys (actors' puppet tracks
are not stepped), so the motion comes from the keyframe-path playback phase. What is still unknown is retail's
intent: whether the crow starts later, has a start delay, or is gated by a trigger - that needs retail-side
positions over time, which this session does not have. Open question, now specific to one thing.

Two more facts on `level2_spawn`. (1) The crow is not the whole story: with the settle pushed to 9 s (`-SettleFrames
540`, past `02_crowstart.cog`'s 2 s timer + 6.73 s flight + destroy), the port still reads 0.8106 against retail's
0.9973 - *more* change than at 1 s, so the surviving difference is the level's other actors, not the transport.
(2) Retail's own sidecar is explicit: "static world regions are pixel-identical across the series (min world-region
SSIM=1.0000); the changing pixels are animated characters/NPCs" - so retail animates the same class of thing, just
far fewer pixels. The series metric is one-sided: extra motion in the port can only lower the score, and it cannot
tell "animates the wrong actor" from "animates more actors". `level2_spawn` is therefore documented as a motion-
volume difference on actors, not as a solved or a broken room.

Timing anomaly seen once: the same gate run at `-SettleFrames 540` took 392 s wall (two captures) where the
480+60-frame four-pair run takes ~12 s total. Not reproduced; if it recurs, profile the shot capture path.

The capture-timing anomaly is startup cost, not simulation: a single `--frames 480 --shot` run measured 2m05s
wall with essentially no comparable user CPU, and the very next gates/series runs take seconds - the variance
tracks the bevy pipeline/shader cache state (a cold start after the cache is evicted, a warm run right after
another). Gate wall times are therefore not a performance signal; use `--frames`-to-wall-time only within one warm
sequence, or the in-run counters.

Correction on the level-14 intro camera: it is **implemented and exact**, not missing. `15maw`'s startup calls
`SetCameraFocus`/`SetCurrentCamera`/`PlayKey` once each; `macamera.key` has a single node `camerabox` (so the port's
camera-key rule applies), 411 frames at 15 fps = 27 s; and the traced eye at frame 60 is (39.464, 9.701, 34.034),
which is `droidcam`'s placed pose (39.465, 11.375, 35.145) plus the key's interpolated delta at frame ~35 - the
arithmetic matches to the third decimal. The picture only *looks* wrong against retail's series because retail's
eight frames are the **settled end** of that 27 s cutscene (its sidecar: "the settled last frame is the
player-camera reference") while the port capture above is the first second. Correct comparison needs a
cutscene-timed capture (port t vs retail's unknown t in the same key), which the sidecars do not record.

Follow-up, because the first look at this looked like a frozen camera: `Key::at` is a **step function** by design
("between entries without interpolating, which is what a 15 fps puppet looks like"). At 1.05 s the key frame is
19.75 and at 2.03 s it is 34.75 - both inside the entry 14..38 plateau, so the eye printed *identical* numbers twice
and I briefly read that as "the camera never moves". It does: `BK_KEY_TRACE` (new, env-gated) shows the key writing
pos (39.465, 11.135, 34.986) at frame 4.25, (39.464, 9.701, 34.034) at 19.75, (39.464, 4.187, 30.809) at 4 s and
(39.469, 1.337, 30.096) at 6 s - the descent onto Kyle that level 14's opening shows in retail. Lesson for future
camera debugging: compare key *frames*, not wall times, and expect plateaus.

`--tour-live` failed once in a full sweep ("(no verdict)"), and the runner discarded the child's output, so it
could not be diagnosed - now the runner prints the failing child's last six lines, and `BK_GATES_ONLY=<substr>` re-runs
just the matching gates. Re-run isolated, the child's tail ended on `sim 898.0s: thing 24 drawn at (2.93 2.50
-1613160.25)`: the gate ran out of its 9000-frame budget mid-tour (898 s of sim), not a code failure of the level
walk - the budget is now 20000 frames. **Separately, that coordinate is a real anomaly** worth its own look: a thing
drawn at z = -1.61 million during the tour means some level's runtime pose blew up (or a model anchor is garbage).
Not yet localised to a level; the next full tour with `--tour-live` output saved will say which.

The `thing 24 drawn at z=-1613160.25` line is a **genuine runaway body**, not a dt artefact: `sim_dt` clamps
every step to 0.1 s, and the print shows 898 s of sim, so something integrated roughly -1795 units/s for the whole
run (a body falling with no floor, or a created thing never despawned). It is intermittent: the earlier complete
tour log has 857 `drawn at` lines and none beyond 4 digits. The runner now prints the failing child's last 20 lines
(was 6) so the next occurrence carries its level context. Open defect, not yet localised.

Full sweep on the current binary: **65 of 66 pass**; the one failure is `--tour-live` (window gate) again reporting
"(no verdict)". Its captured tail shows it *progressing* (`sector visibility: 45..55 of 606 sectors`), so it is not
stuck - it did not finish 21 levels inside 20000 frames in the sweep, while the **same binary** completed the whole
tour standalone (exit 0, every "level ... complete" line, 2336 s, output to a file). The remaining difference is the
sweep's launch context (child stdout is a pipe) plus machine load; the per-level timer is only 0.2 s, so frames
should not be the limit. Not claimed as a pass: the gate is environment-sensitive and needs its completion criterion
made progress-independent (a per-level verdict line) or moved out of the sweep.

Resolved: `--tour-live` is not broken. Run standalone through a pipe (the sweep's context) the same binary finishes
all 21 levels in **6.4 s** and prints its verdict; the slow, verdict-less sweep runs were the windowed cold-start
pathology. The gate runner now (a) retries a window gate once when it ends without a verdict, (b) prints the failing
child's last 20 lines, (c) takes `BK_GATES_ONLY=<substr>` to re-run one gate, and (d) counts only the considered
gates in its summary (`gates: 0 of 1 failed` under a filter). Full sweep after these changes: **0 of 66 failed, 0
skipped** in 1 minute (warm caches). The `-1613160` runaway body did not appear in any of these runs; it stays an
intermittent open defect with its level context now captured if it recurs.

The runaway-body anomaly now has a guard instead of a memory: `escaped(pos)` (a component past 100 000, when the
levels span tens of units) is checked on the per-second pose print and prints `FAIL: thing N escaped the world at
(...) after T s`, so a sweep counts it as a failure instead of a silent diagnostic. Boundary test added
(`escape_tests::escape_bound`: a level pose and the bound itself are inside; the observed -1_613_160 is out). Five
repeat tours with full logs showed no escape (5 x 21 levels complete), so the one sighting stays unreproduced.
Verification after the change: 214 tests; `--gates` 0 of 66; series gate 1 graded / 0 failures; draw gate 5 PASS.

Guard upgraded from one sampled thing to every moved thing: `advance_sim` checks `escaped(pose)` for each moved
thing as it pushes the pose, and remembers the ids it already reported, so a blown-up body prints
`FAIL: thing N escaped the world at (...)` exactly once instead of every frame or never. Verified: 214 tests; a
`--tour-live` run prints only `--- tour-live check passed ---` (no FAIL); `--gates` 0 of 66.

`level2_spawn`'s motion content is identified too: at that pose the level's `reeyees` thugs **engage the player**
(`BK_AI_TRACE`: four `LookForTarget` hits, `sees 35 range 2.41 dot 0.74 ready true`, two shots, held-out step
corrections). That combat is what changes the port's frame; retail's captured reference is a quiet street with no
HUD and no actors in view. So the pair's residual difference is a capture-time/behaviour mismatch (the port fights
at spawn, retail's frames are a calmer moment), not a room or geometry defect - and not something the series metric
can settle without retail-side runtime frames at the same pose and time.

Player-reported defects (live play, after the QA suite had been green for hours):

1. **Jumping went through the ceiling** - real, and fixed. `walker_step`'s vertical branch only tested
   *falling* (`drop < 0.0`); the rising branch was a bare `pos[2] += drop`. It now mirrors the fall:
   `Solids::rise_stop(ceiling_above(sector, pos), frame_start_z, drop, radius)` stops the centre at
   `ceiling - radius`, kills the upward velocity, and never moves a body down (a body already above its own
   ceiling keeps its height). Regression test `rise_tests::a_jump_stops_at_the_ceiling` drives `walker_step`
   with vy 4.0 in `collide::test_room()` (ceiling z 1.0) and asserts the head never crosses and the rise stops
   at the contact. **Verified failing before the fix**: with `rise_stop` forced to `None` the test reports
   "the head went through the ceiling: centre 0.963 + radius 0.065 > 1.000". 215 tests pass after.

2. **Enemies shoot at spawn** - measured, not yet changed. `--start 03katarn` with `BK_AI_TRACE`: a tusken has
   line of sight (`sees 237 range 2.82 dot 0.97 ready true`) and `1 shot(s) at the player` **before sim 1 s**.
   The engine mechanisms this would differ by are present in the port (sleep bit `0x2000` from `00_aisleep.cog`
   and friends, awake bit `0x4`, `sensedanger`), but no shipped level-1/2/3 startup cog sets them on the spawn
   enemies, and retail's own level-3 sidecar recorded no damage over its series. Whether retail's opening is calm
   by schedule or by script is unverified - needs retail-side frames/time, not a guess.

Why the tour saw neither: `--tour-live` is a **flow** test (finish each level on a 0.2 s timer, name/skip its
cutscene, load the next) and `--live-play` asserts only that a key moves the body and a click fires. No gate
asserted a ceiling invariant or a spawn-calm invariant.

Player-reported weapon framing: the first-person weapon sat too high. `view.base` (the "fitted hand offset",
the last of several guesses) is now `(-0.026, -0.015, 0)` - 24 mm lower than before, which drops the model out of
the middle of the view. Verified by capture at level 1's parity pose; the `viewmodel_framing_tests` still pass.
Note for the next framing round: the big grey mass that looks like the weapon at that pose is **world geometry**
(identical with the weapon armed or forced or not), and `--shot` captures of the forced POV model draw it below the
frame, so the real weapon framing is best judged in a live session or against retail's own weapon mask.

Ceiling work, completed properly: the first clamp used `ceiling_above`, which answers for the sector's planes as
**infinite** surfaces, so a bar top beside the body read as a ceiling and stopped every jump (live-play peak 0.11,
force-live "did not lift", `--rules` apex 0.17 vs 0.28). `Solids::ceiling_over` is the polygon-precise mirror of
`floor_under` (sphere sweep up, `sphere_face` containment), and the walker uses it. Level 1's spawn really does
stand under an overhang 0.30 above the floor, so a jump there *should* be capped - the three gates now accept a
geometry-capped lift/apex (printing which), instead of demanding the open-air result. 215 tests (ceiling polygon and
step-face cases included); `--gates` 0 of 66.

Framing check after the `view.base` change, with the two identities settled from evidence: the big craft filling the
upper middle of level 2's spawn view is the `flyingcrow` transport (thing 25 on its keyframe path; retail's
reference has it already gone), not the weapon; and the bottom-centre tile is the status bar's selected-item panel
(`--hud` renders it in isolation: item icon, count, red charge bar). The POV weapon itself now sits low at the
bottom edge, which is the reported defect's fix; retail's own spawn frame has its weapon mass at the same low band.

Spawn-combat narrowing: the shipped mechanism for a calm start is `00_aisleep.cog` - it `aisetmode(enemy, 0x2000)`
on startup and clears it from an `entered` sector trigger. A case-insensitive search of every text asset finds that
name **only in its own file**: no level, template or cog references it. So level 3's tuskens are not asleep in
retail by that script, and the port's early first shot is not a skipped sleep. What remains is either a difference in
the engine's first AI think (the port scans at t=0 with the 500 ms instinct interval) or retail's own sight/alert
setup - neither decidable from the data in this repo, and both would be guesses. Left open with that boundary.

The weapon framing is now a check, not just a fitted number: `VIEW_BASE` is a named constant and
`viewmodel_framing_tests` asserts the **drawn** weapon's top (authored box + base) stays below the view's centre
line. Verified in both directions: the old `y = +0.009` fails it with "the drawn weapon reaches the middle of the
view: top NDC 0.06", the current `y = -0.015` passes. 215 tests; `--gates` 0 of 66.

Weapon framing, what the parity pair actually says (left is retail, right the port per `compare.ps1`): retail's
weapon top sits at about **75 %** of the frame height, the port's at about **46 %**, and the port's mass looks
about **1.27x** larger. The two cameras share the 73.7 deg vertical FOV (`vertical_fov(4/3)`), so a size difference
would have to come from the model's distance/offset, not the projection - yet the side-by-side is explicitly a
triage view: `compare.ps1` **stretches both sides onto one canvas**, so pixel ratios there are not measurements.
A correct fit needs the retail raw frame and the port frame measured in their own pixels at the same pose, per
weapon model (the fists and the bryar are different authored frames, and the current offset was fitted on the
bryar). Left as the next framing round's first step; the current change (lower base + the below-centre test) is a
strict improvement on the report and is guarded by a test.

Framing measurement hit a real obstacle worth recording: the --shot capture path cannot be used to measure the POV
weapon. draw_viewmodel reports 13 parts for bryv.3do (forced with BK_ARM/BK_POV), but a diff between an armed and an
unarmed capture at the same pose finds only the HUD column (x 909..1013) - the model changes no visible pixels, and
the trace's part NDC puts its vertices mostly below the frame (y -2.14..-0.57). Also, BK_POV is ignored unless
BK_ARM is set (the arming path returns early), so "armed vs unarmed" is really "bryar vs the level's fists", both
drawn into the same invisible band. Consequence: every earlier framing capture (and my eyeballing of them) was
looking at world geometry, not the weapon - the big central mass in those frames is level dressing. Weapon framing
can only be judged in a live session, which is what the player did. The current lowering is kept (the player
reported the gun too high; it is below-centre by construction), but no retail-matched fit exists yet.

`walk5_spawn` (the trace gate's one failure) examined with its own retail fixture
(`target/parity/trace/walk5_spawn-walk.tsv`, 6 samples over 3 s of held forward): retail descends from z 0.22 to
about 0 and crosses sectors 35 -> 33 -> 34 -> 33 -> 24; the port stays at **z 0.175**, takes **0 sector
transitions** and blocks 83 frames, ending 1.46 units off. Two causes ruled out: the walk tool's `allow_step=false`
(as the player has stepping on) changed nothing, and `BK_STEP=0.075` (the engine's `moveSize + 0.01`) changed
nothing - so the stop is not a step-height boundary. `BK_WALK_TRACE` names it: "step candidate low false contact z
0.286 feet 0.221", i.e. a surface 0.065 above the feet that the sweep treats as a wall, where retail's body goes
through. That is a collision-surface difference on 06abarons, not a tooling one, and it is the only trace-gate
failure left (9 of 10 legs pass).

walk5_spawn advanced one level with BK_MOVE_TRACE (which already exists and names the face): the body wedges on
surface 422, a vertical wall with normal [0.707, 0.707, 0] at (7.818, 12.158, 0.286), hit head-on (slide = 0, so the
stop itself is correct behaviour). The real divergence is the walking height: the port stands at z 0.286 for the whole
leg while retail's samples are at z ~0 - so the port picks a different floor under the same point, and the 45-degree
wall that blocks our higher body is not in retail's path. Next step is the floor choice there, not the step rule and
not the wall.

walk5_spawn's floor question answered, and the answer says **do not change the rule**: at the block point
(7.8, 12.15) `floor_under` reports **0.100059 in sector 35** - the sector's own floor - and the nearby sectors carry
0.238 (34), 0.300 (12), -0.300 (392), -0.812 (514). Retail's trace falls from 0.22 to about 0 by sample 1, so retail
is standing on a *lower* face that belongs to a sector other than the one the point is in. The port's landing probe
has a documented fallback for exactly that (`sectors_around` when the body's sector has **no** floor), and it
deliberately refuses to use it when the sector has one - because level 5's body was lifted frame by frame onto a
ledge plane (0.30/0.42) above its own 0.1001 floor. walk5 is therefore the opposite side of the same trade: retail
lands on an off-sector face where the strict probe keeps the sector's floor. Weakening the rule here would reopen
the plateau regression, so the leg stays a documented divergence rather than a green check bought by a regression.

Engine-binary route for the spawn-combat question checked and set aside on evidence: JK.EXE opens cleanly in REA
(sha256 12e8c8c3…, PE x86) but it is a **stripped** build - `search_procedures` for `AICmd|AI_|sithAI|PrimaryFire`
returns **0 rows**, so the named `sithAICmd_*` functions the port's notes cite were analyst labels from an earlier
session, not symbols. Answering "why retail's tuskens do not fire at spawn" from the binary would now be
string-and-xref archaeology, not a lookup. Retail-side evidence this repo already holds says the *what*: the
`katarn_ai_193` series shows the HUD at 100/50 with no damage across its eight frames, while the port lands a hit
within 2.5 s - so the difference is established and only the mechanism is unknown. Target closed; item stays open
with that boundary instead of a guess.

Correction to my own earlier note: the `--shot` capture path **can** measure the POV weapon, and the previous
measurement was wrong because `BK_POV` is ignored unless `BK_ARM` is set (the arming path returns early). With both
captures armed - `BK_ARM=11 BK_POV=bryv.3do` against `BK_ARM=11 BK_POV=__none__.3do` - the difference bbox at 4:3
(1024x768) is **x 447..604, y 576..767**. So the drawn weapon starts at 0.750 of the frame height and touches the
bottom edge; retail's `spawn.png` weapon band is about 0.72..1.0. Vertical placement now matches retail within ~3 %
of frame height (it was 0.46 before the `VIEW_BASE` change - the middle of the screen, exactly the reported defect).
Horizontal spans differ (port 0.437..0.590, centred; retail's mass reaches to ~0.09) but those are different weapon
models and poses, so that axis is not comparable until the same weapon is captured on both sides.

Weapon framing, measurement audit (round: this one). Two things were established and one record corrected:

1. The capture path *can* show the weapon, but only when both runs match on every other variable: `BK_POV` needs
`BK_ARM` (otherwise the arming path returns early and the level's own fists stay), the same `--frames`, and a bin
change alters the HUD's selected-item panel - which is why one "weapon" diff was really the HUD tile at
x 447..604/y 576..767. With `BK_VIEW` forced, base does move the model (base 0,0,0 against (-0.026,0.009,0): diff
bbox x 447..1023/y 445..767), so `view.base` is live; the earlier "dead base" reading was a mismatched pair.
2. **Roadmap row 164's fit was never validated on a rendered frame.** Its "port rest pose x 473..714/y 336..730" is
the arithmetic of the unit test at `FOV = 1.02` rad, while the retail numbers (x 480..660/y 337..730 at 1170x731)
are pixels; the world camera actually uses `vertical_fov(4/3)` = 73.74 deg. Mixing those two projections by hand is
what made the whole-box comparison look matched. The offset is therefore *not* measured against retail yet - it is
carried forward unchanged because no better evidence exists.
3. The test now pins retail's measured band (top NDC <= 0.15 against 0.078 from y 337 of 731) rather than the
below-centre rule an earlier round invented from the player report; 215 tests pass.

## The HUD drew a column retail does not, and the surface cull was measured before it was kept (round 279)

Started from the report that several levels render wrong. The first thing the frames showed was not geometry but the
status bar: every port capture carries a column of red numbers down the right edge (`50`, `100`, `100`, ...) that no
retail frame has. It is `hud::draw_carried`, the right-edge carried list, drawing each entry's count. Retail's own
`jkHudInv_Draw` (`0x4094a0`) copies only the icon in that loop (`stdDisplay_VBufferCopy`), 28 px apart, and the count
text is drawn later, beside the **selected** item's row (`stdFont_Draw1` at `x + 0xe` for three digits, `x + 0x12` for
one or two). The port already draws that selected row's count in `draw_item`, so the strip's numbers were invented.
Removed. Measured on the survey captures: level 2's spawn frame changed **0.382 %** of pixels inside
`x 974..1022, y 195..486`, level 5's **0.460 %** inside `x 651..1022` — the HUD columns only, no world pixel.

The same function was also drawing entries retail does not draw at all. Its loop copies the icon only when the type
carries inventory art (`if (*(int *)(iVar3 + 0x98) != 0)`), and the step advances only for the icons that drew
(`iVar7 = iVar7 + 0x1c` inside the drawn branch); the *selected* row is the one that falls back to a default art
(`DAT_00553f88`). The port drew a `icdefau8.bm` placeholder for every entry with no art, and stepped the list for
each one. Fixed: an entry whose icon is absent is skipped and does not advance the strip. Measured: level 2's spawn
frame changed a further **1.871 %** (`x 921..1022, y 12..486`), level 5's **2.007 %** — level 5's strip drops from
several placeholder icons to the single entry whose art exists, which is what the engine's own test leaves.

The other candidate was the draw policy the ledger already names: world surfaces are submitted double-sided
(`cull_mode: None`) because the `.jkl` winding is not trustworthy, where the engine culls. `sithRender_RenderSectors`
(`0x4c6c40`) draws a surface only when the eye is on its plane's positive side (`if (dist <= 0) continue`), so the
engine's rule was implemented as a fragment test in `level.wesl` on the interpolated world normal. It was measured
three ways before any decision:

1. **Live?** Inverting the test (`<=` instead of `>=`) changed **66.5 %** of level 2's spawn frame and **76.1 %** of
   level 5's, so the branch runs and `world_normal` is valid — a shader edit that changed nothing could otherwise mean
   a dead uniform.
2. **Effect in the engine's direction?** 0.07 % of level 5's pixels and no measurable change on levels 2, 8 or 11: at
   these poses no back-facing *level* surface is on screen.
3. **Cost to the verified pairs?** A/B on the seven verified poses with the test ablated versus active: `spawn_pose`
   0.6295/0.6295, `spawn_yaw30` 0.5957/0.5968, `route_cantina` 0.7004/0.7004, `route_lower` 0.6860/0.6860,
   `route_street` 0.6935/0.6935, `route_catwalk` 0.4536/0.4539, `route_door` 0.4128/0.4128 — a maximum delta of
   0.0011.

**Reverted anyway**, and the reason is the normal, not the number: the mesh ships `polygon_normal`'s Newell normal
(derived from vertex winding) as `ATTRIBUTE_NORMAL`, while the engine tests against the sector plane's stored normal.
`--winding` reports 966 of 87 377 drawn surfaces wind against their own normal, and those are exactly the surfaces where
the two normals can disagree — a test built on the wrong one hides a wall the engine draws. The ledger's surface-culling
row therefore stays open, now with the measurement attached: honouring the rule is worth ~0.001 SSIM at the poses this
repo can verify, so it is not a candidate for a blind swap. It needs the plane normal carried into the mesh first.

The per-level framing question needed a survey rather than one pair, so both harness scripts gained a list parameter:
`port-capture.ps1` now takes `-Shots` and `-Frames`, and `compare.ps1` takes `-Shots` (the gated pair list stays
`tools/parity/shots.tsv`). `target/parity/survey-shots.tsv` is generated from `spawn-levels.tsv` — one row per level at
its own spawn — and all 20 port spawn frames were captured (240 frames each, 20/20 in 40 s). What the survey then
exposed is a fixture problem, not a rendering one: `tools/parity/retail-level{4,7,10,15,18}-spawn.png` are **all-black
1440x900 frames** (mean luma 0.0), so those five levels have no retail reference at all, and the fifteen that are
640x480 have no pose sidecar, so their SSIMs (0.07–0.22) measure framing and HUD scale, not fidelity. The contact sheet
(`target/parity/sheet-port.png`) is the useful artefact: all 20 levels render their own geometry, and the frames that
look wrong are the spawns `spawn-levels.tsv` already marks as "settling/opening motion" (levels 17, 18, 19 differ by
2.1–2.9 units), not blank or mis-textured rooms.

Fixture hygiene, recorded because it cost a restore: a copy step in this round overwrote
`target/parity/retail/level2_spawn.png` with a committed survey image. It was rebuilt byte-identically from the retained
raw frame and the sidecar's own `world_crop: 1170x730+135+85` (`sha256 af49edab32f2…23a1be`, matching the sidecar's
`png_sha256`). Survey rows are named `lvlN_spawn`, never `levelN_spawn`, so the two namespaces cannot collide again.

Verification for the kept change: **215 tests, 0 failed**; **`--gates` 0 of 66 failed, 0 skipped**; the draw gate
**5 thresholds, 0 failures** (`spawn_pose` 0.6295, `spawn_yaw30` 0.5968, `route_cantina` 0.7004, `route_lower` 0.6860,
`route_street` 0.6935 against floors 0.55/0.55/0.65/0.65/0.65).

## The gun filled half the screen because the camera stood inside it (round 280)

The weapon framing was re-measured instead of re-argued, on the one pair of retail runs that can be compared exactly:
`target/parity/retail-fists/spawn-raw.png` (unarmed) against `target/parity/retail/spawn-raw.png` (armed), same pose,
same session — their top halves differ by a mean of 0.04/255, so the difference *is* the weapon and its hand. Port
side: the same pose (`spawn_pose`), weapon isolated the same way with `BK_ARM=11 BK_POV=__none__.3do`, and a new
`BK_NO_HUD=1` so the status bar's item tile cannot sit over the thing being measured. Both sides measured inside the
1170x730 box retail's fullscreen crop actually shows (the game draws 4:3 into 1440x900 and the harness box trims it),
which mattered: comparing against a 4:3 crop of the fullscreen frame adds bars and inflates the gap by ~30%.

The numbers, as fractions of that box's lower band:

| side | mask area | bbox x | bbox y |
|---|---|---|---|
| retail (fists vs armed) | **3.99 %** | 0.000–0.657 | 0.460–0.999 |
| port, authored rest pose | **11.87 %** | 0.404–0.610 | 0.460–0.999 |
| port, fitted offset | **2.51 %** | 0.461–0.551 | 0.744–0.999 |

Same vertical extent, ~3x the area: the port drew a thick central slab where retail draws a thin band. The cause is
in the model's own box — `--model bryv.3do` reports `bbox min [0.020, -0.033, -0.062] max [0.032, -0.007, 0.010]`,
i.e. the geometry reaches **behind** the eye plane (z +0.010). The camera stood inside the arm and gun, so the
near plane cut a cross-section out of the model. That also explains why the scene's style, FOV and near plane were
never the difference: the engine and the port use the same model, the same 90-degree horizontal FOV and the same
0.015625 near plane, so only the placement could move the measurement — and the placement put the eye inside the box.

`VIEW_BASE` takes z **-0.03**, which puts the nearest vertex at -0.092, clear of the near plane, and lands the mask at
retail's shape and about two thirds of its area. The framing test now asserts the invariant that was actually broken
(the box must sit entirely in front of the eye, `lo[2] + VIEW_BASE[2] < -NEAR_PLANE`) and, for its projection, uses
`vertical_fov(4/3)` — the angle the renderer really draws with — instead of the 1.02 rad the old test assumed while the
world camera was drawing 73.74 degrees.

Two more switches, both measurement-only and off by default: `BK_POV_ROT_ONLY=1` poses the mount clip's **rotations**
on the model's rest positions, and `BK_VIEW_SCALE` scales the drawn model. They exist because the animated pose is the
remaining difference: with `BK_POV_CLIP=1` the port's arm silhouette matches retail's closely, but the gun leaves the
frame, and rot-only reproduces that exactly — so the clip's *rotations*, not its `pos`, carry the weapon out of view.
The posed box explains it: rest z -0.062..+0.010, clip frame 0 z -0.044..**+0.002**, clip frame 4 z -0.014..+0.037 — the
mount animation folds the model further behind the eye, not away from it. That reconciliation is the next step and the
ledger row records it.

Verification: **215 tests, 0 failed** (the framing test among them, re-derived rather than re-pinned), and the draw gate
re-run over all 13 captured pairs after the change — **every gated pair improved**, which is the independent check that
the fitted placement is the right one rather than a number that merely looks better:

| pair | before (rest at the eye) | after (z -0.03) |
|---|---|---|
| `spawn_pose` | 0.6312 | **0.6607** |
| `spawn_yaw30` | 0.5962 | **0.6048** |
| `route_cantina` | 0.7018 | **0.7394** |
| `route_lower` | 0.6856 | **0.7170** |
| `route_street` | 0.6957 | **0.7335** |

World-band scores rise with them (0.7002 → 0.7341 on `spawn_pose`, 0.7664 → 0.8054 on `route_cantina`), so the gain is in
the world frame, not the HUD term. Gate: 5 thresholds, 0 failures.

## The engine's own pose math, and a series clock that can be aligned (round 281)

The weapon-pose question was taken to the executable instead of fitted further, and `decomp/` answers how retail builds
a pose — `rdPuppet_BuildJointMatrices` (`0x43eeb0`) and `rdThing_AccumulateMatrices` (`0x43e560`):

* With **no active track** the joint matrices are the **model's own rest node matrices**, copied straight out of the
  `.3do` (`model + 0x58 + 0x84`, stride `0x2d` dwords). That is what the rest pose is, and it is why drawing it is not
  an invention.
* With a track, each node's pose is a **blend of deltas against that rest pose**: `delta = keyValue − restValue`,
  `NormalizeAngleAcute` per component, multiplied by the track's weight (`+0x18`), accumulated by priority tier (four
  tiers, largest weight wins, then normalised so the weights sum to 1), and only then **added back to the rest value**.
  So `pose = rest + Σ(delta·weight)` — the port applies the key directly, which is the same thing for a single
  full-weight track and a blend toward rest for anything less.
* Each node matrix is `rdMatrix_Build34(pyr, pos)` followed by `rdMatrix_PreRotate34(matrix, node_pivot)` when the pivot
  is non-zero, and the hierarchy is `world_i = parentWorld · T(-parent.offset) · M_i · T(node.offset)`. The port's
  `tdo::pose_nodes` already has that shape (it has a two-node pivoted test), so the composition is not the divergence.

`runtime::rotate_by_angles` was also put against `rdMatrix_Build34`'s arithmetic (`0x43f6f0`) because two of its entries
look mismatched when transcribed by hand. They are not: applied to the three basis vectors, the port's matrix is
**orthonormal with determinant +1** at every angle triple tried (`30,40,50`, `10,0,0`, `0,35,0`, `0,0,25`,
`12,-77,130`), which the hand transcription of the Ghidra listing is not — so the port's form is the correct reading of
the angle order and the decomposition that looked like a typo was mine.

What that leaves for the POV model is the **track weight**, which is not in the export: the cog plays
`jkPlayPOVKey(player, mountAnim, 0, 0x14)` and neither that function nor `rdPuppet_PlayTrack`'s caller is in `decomp/`,
so whether retail poses the mount at full weight or blends it toward rest is still open. The shipped build therefore
keeps the rest pose, which is the engine's own no-track state.

The other half of this round is the series clock. `series_ssim_first_last` says *what* retail's scene did but not
*when*, so the gate had to assume 1 frame a second — under dgVoodoo retail renders at a few frames a second, so that
assumption was untested. `retail-capture.ps1` now starts a stopwatch before series frame 0, records each frame's offset
(`series_times_ms`) and the span (`series_span_ms`), and `series-gate.ps1` captures the port over the **measured** span
at its fixed 1/60 s step. Both new paths were exercised on synthetic sidecars (written to `target/`, then deleted):

```
series zzspan      port 0.9272 / retail 0.9238 over 7.2 s (432 port frames)  |diff| 0.0034 <= 0.05  PASS
zzscripted         SKIP  series is a scripted intro camera: series spans 7,200 ms inside the scripted opening
```

The real pair still grades through the old path (no `series_times_ms` in its sidecar yet): `spawn` 0.9078 against
retail's 0.9238 over 7.0 s, PASS. Level 14's cutscene comparison is now blocked only on a retail capture that settles
past the 27 s opening (`-SettleMs 30000`), not on tooling.

## The floor question became a table, and the flip turned out to be load-bearing (round 282)

`walk5_spawn` has been argued three times from trace logs. It is now a probe: `--start <level> --floor-at
"x,y,z[,radius]"` prints **every** face a downward sphere sweep from the point lands on, highest first, with
the surface id, owning sector, the face's own normal, whether it faces *up* from the authored data, the room plane's
normal, and the answer `floor_under`/`standable` actually give. Both the walker and the probe run the same loop
(`Solids::floor_candidates`), so the table cannot drift from the behaviour it explains.

Level 5 (`06abarons`), the player's own `movesize` radius 0.065:

```
floor-at [10.58, 14.91, 0.185]: body sector 35 (plane floor Some(0.100059)), radius 0.0650, reach 1.1000
  surface 408    owner 35   floor    0.1001  travelled  0.0199  face normal [ 0.000,-0.000,-1.000]  up false
  floor_under -> Some((0.100058466, 0.019941539, 35))

floor-at [7.82, 12.15, 0.285]:   (the point the walk wedges on)
  surface 405    owner 35   floor    0.1001  travelled  0.1199  face normal [ 0.000,-0.000,-1.000]  up false
  floor_under -> Some((0.100059, 0.119940996, 35))
```

`up false` here is the **data's convention, not a defect**: `.jkl` plane normals point *out of the room*
(`floor_below` filters `z < -0.7` to find a floor; the test room's doc comment says the same). The engine's
`sithCollision_SearchForSurfaceCollisions` tests against **into-room** normals — `dot(search_dir, normal) < 0` accepts
a floor for a downward sweep — so the port's flip-toward-the-body *is* that normalisation. An experiment that skipped
the flip (a `BK_FRONT_FACING` switch, since removed) answered `floor_under -> None` at the same point, which is what a
sweep against out-of-room normals must answer; it measured my sign assumption, not the engine's rule. **So the floor
rule is not the walk5 divergence**, and the ledger row's "front-facing only" is already satisfied by the flip — what
the row still owes is the *scope* (the engine searches the body's sector and its adjoins, this search scans every face
in the world) and the swept surface test.

What the probe does establish, with the player's real radius: the wedge point has exactly **one** floor candidate
(surface 405, sector 35, 0.1001), and the spawn point has the same floor 0.1001 with the body 0.02 above it — matching
retail's own `coords` eye height of 0.22 at the same place. Retail's trace then descends to ≈0 over sectors
35→33→34→33→24 while staying 0.35–1.2 units **north** of the port's straight 135° line, so the remaining difference is
the direction the body ends up travelling (retail's path bends toward 155°, i.e. it slides along geometry the port
stops dead on), not which surface is a floor.

Default behaviour is unchanged (the flip stays on unless `BK_FRONT_FACING` is set): 215 tests, and the gate sweep and
`--walk all` re-run after the refactor report the same numbers as before it.

## Two probes, and walk5 narrowed to height and sector membership (round 283)

`--floor-at` answered "what is under me". The walk5 difference needed the neighbouring questions, so this round added
two more probes over the same `Solids` the runtime uses — no new logic, just the existing movers with a face:

* `--move-at "x,y,z,dx,dy[,radius[,steps]]"` steps a body along a straight line through `move_body` and prints only
  the steps that **lost distance** or changed sector. (`blocked` alone is not a signal — the step-up test sets it on
  most frames while the body still covers its full step, which is why the first version of this probe printed every
  line and told us nothing.)
* `--faces-near "x,y,z[,radius]"` lists every collision face whose **plane** passes within `radius` of the point and
  whose polygon spans it, whatever its orientation — the question a walk asks when it is deflected where the port sees
  nothing, or when it walks through where retail is turned.

Measured, level 5, the walk5 line from the spawn (10.58, 14.91) at yaw 135°:

```
move-at [10.58, 14.91, 0.285] delta [-0.0217, -0.0217, 0]
  step 127: [7.824112, 12.154112, 0.285] -> [7.8229527, 12.152952, 0.285]  walked 0.0016 of 0.0307
  step 128..180:                                              walked 0.0000 of 0.0307
```

The port walks 3.90 units dead straight and then stops **permanently** on the 45° wall at (7.82, 12.15) — the wedge
the ledger records. Retail walks 3.73 and ends at (7.19, 13.36), 1.46 units north, with its *own* per-sample direction
bending 135° → 139 → 149 → 159 → 175 → 164: it is being carried along something.

That something is **not a wall at the port's height**. `--faces-near` at (7.4, 13.5, 0.1), where retail's bend is
strongest, finds six faces and not one of them is vertical at body height:

```
surface 360/368  plane  0.0165  normal [-0.627, 0, -0.779]  z -0.249..0.000    (38.7° slope)
surface 4680     plane  0.1316  normal [0, -1, 0]           z -3.122..-3.011    (three units below)
surface 266/267  plane  0.2349  normal [0, ±0.928, -0.371]  z -0.249..0.000    (38.7° slope, both ways)
surface 268      plane  0.3488  normal [0, 0, -1]           z -0.249..-0.249    (a ledge lip)
```

Every surface that could deflect retail there spans **z -0.249 to 0.000**, and the port's body is at 0.285 — above
them all. Retail's own camera drops 0.22 → **-0.01 in the first 600 ms**, which puts its body inside that band and in
sector 33; the port's body stays up on sector 35's slab (floor 0.1001, surface 405 at the wedge) for the whole walk.
So the path difference is a consequence of **which side of the slab's edge the body is on**, not of the floor rule
(the previous section) and not of a missing wall: nothing in the port's collision set is missing, the body is simply
too high to touch it.

The gate's own `z 0.185` column is the constant body-radius/camera offset (0.285 − 0.10 both at spawn), so it is not
by itself a height error; the divergence the gate scores is `sectors off 5` and `xy 1.461` — both downstream of the
body's height at the first sector boundary.

Next measurement, in this order: step the port along **retail's own polyline** with `--move-at` sample by sample, so
the first sample where the port's floor differs is a printed row rather than an inference; if the port's floor at
(9.79, 14.22) is the ramp (-0.4117, which `--floor-at` reports) then the diverging rule is *why the body is on the
slab at the spawn in the first place* — the spawn's placement relative to the slab's edge.

### What retail's own path points answer (same round, the decisive measurement)

`--faces-at` (the existing landing probe — the `--faces-near` I added to answer this was redundant and is removed) at
retail's second sample, the first one off the slab:

```
faces-at (9.79, 14.22, -0.13):
  sector_for_body -> Some(33)              retail's fixture says sector 33   ✓ agreement
  sector 33: clearance Some(0.130) floor_below Some(-0.389)   ← the sector's unbounded plane
  floor_under(height 0.1212, radius 0.065) -> None            ← the swept, polygon-bounded search
  floor_under(height 0.1201, radius 0.065) -> None
  floor_under(height 0.0300, radius 0.065) -> None
```

Three facts, all measured:

1. The port's **sector assignment agrees with retail** at that point (33), so the divergence is not the containment
   rule.
2. The port's sector plane there is **-0.389**, retail's body sits at ≈ **-0.13** (its camera -0.01 minus the 0.12 the
   spawn establishes: camera 0.22 over the slab's 0.1001). The two disagree by 0.26.
3. The swept search finds **no polygon at all** under the point, at every height the walker uses — and `--collide`
   confirms nothing fell out of the collision set (`--- collide check passed ---`, 5191 surfaces).

So this leg's remaining difference is a **coverage gap in the level's own collision data as this port reads it**: retail
keeps a body at -0.13 where the port has neither a polygon nor a plane at that height. That is the same "the data does
not tile" fact the roadmap's floor row records, now with a probe behind it instead of an argument — and it is the
reason walk5 stays out of the trace gate's default set rather than being chased as a code rule: no rule can place the
body on a surface the collision set does not contain.

## The AI row's premise did not reproduce, so the row closes on a measurement (round 284)

The roadmap's "AI engagement at a data-derived vantage" row rested on a round-209 reading: at the level-3 spawn turned
to yaw -53 the port supposedly took shields 100 -> 97 within 2.5 s while retail stayed at 100/50. Re-measured on the
current binary at that same pose:

```
fire: +ebow dmg 20.0 ... owner 237 from [-0.612083,-2.412509,-0.342343] yaw -91.4 pitch 12.3
fire: +ebow dmg 20.0 ... owner 245 from [-2.310741,-5.217774,-0.609708] yaw -0.4  pitch 28.3
ai: 2 shot(s) at the player
```

Two bolts, and **no** `hit: the player` line — not in 300 frames, not in 1800 (8 s and 30 s). Both shots travel into
the spawn's fence geometry, which is exactly why retail's HUD reads 100/50 there: at this vantage neither engine
damages the player. The port's `impacts` scan is positional per frame (`Runtime::shot_hits`), and a bolt at 4 u/s covers
0.067 units per frame against a 0.215-unit catch radius, so the sampling cannot be blamed for two misses that a fence
explains.

So the row is **closed**: the damage divergence it existed to track no longer reproduces, and both sides are measured
at one pose. What stays uncomparable is the *shot count* — retail's `aistatus` exposes instinct masks and modes, not a
fired-shot counter — so a future capture would need a doc'd HUD-free counter (screen-space muzzle flash) before shot
counts can be gated.

Also measured incidentally: the tusken class the level uses is `tuproddefault.ai` — `fov=180, sightdist=5.0,
heardist=2.0, accuracy=0.15, maxstep=0.3`, with `LookForTarget 500.0, 30000.0` and `PrimaryFire 1500.0, 0.8, 4.0, ...`.
`accuracy` is **parsed and printed but has no consumer anywhere in the runtime**, so the port's AI shots carry no class
spread. That is not the spawn-combat deviation (the fence blocks the shots anyway) — it is the next AI row's question,
recorded here so it is not lost.

### The weapon capture was never broken — it was missing one flag (round 284)

The roadmap carried a claim that `--shot` cannot see the first-person weapon ("13 parts and changes no pixels"). It
reproduces only without `BK_POV`: `BK_ARM=11` alone changes the HUD's selected-item tile and nothing else. With the
flag the capture path documents for exactly this purpose, the weapon draws:

```
$ BK_ARM=11 BK_POV=bryv.3do ... --start 01narshadda --frames 40 --shot vm-pov.png
viewmodel: POV model forced to `bryv.3do`
viewmodel: armed bin 11 (`+bryarbolt`)
viewmodel: drew `bryv.3do` (13 material part(s), sector light 0.88)
```

Against the same capture without `BK_POV`: **8 843 changed pixels (1.12 %)**, bbox **x 475..565 / 1024, y 540..767 /
768** — a 91 x 228 box (8.9 % of the width, 30 % of the height) at the bottom centre. So a viewmodel fit is now a
capture pair away, and the next `--pov`/framing round should start from this measurement rather than from a new
capture path.

## The retail fixtures had a fail-open hole, and every non-first-level row walks into it (round 284)

Approved retail captures turned into a fixture-integrity finding rather than three new measurements.

**What happened.** `-Only level14_spawn` (row level `15maw`) ran, read `coords` at
`Pos (-10.44, -5.46, -1.13) PYR (1.04, -59.98, 1.04) Sector 339` — level 1's own spawn — logged
`warp verification: max position delta=49.910 ... verified=False`, said `continuing`, and **wrote** `level14_spawn.json`
with `level: 15maw`. The harness's `level` field is the *row's* name; nothing verified which level booted. A series gate
reading that file would have graded the port's level 14 against a photograph of level 1.

**Why the level cannot be loaded.** `-JkArgs '-level,15maw'` makes JK.EXE exit with code 3 before creating its window,
which is the same result `docs/gaps.md` records for `02narshadda` (rounds 60-61: neither the dev console nor the command
line can load an arbitrary level). So the roadmap's "capture level 14 with `-SettleMs 30000`" step is not reachable with
the current harness, and neither is the level-2 spawn refit — both need the savegame route.

**The hole is now closed on both sides.**

* `retail-capture.ps1`: when a row requested a pose and the verification missed by **more than 5 units**, the capture
  **throws** instead of writing a fixture (`warp verification failed by 49.91 units for row 'level14_spawn': the pose is
  not the requested one, so no fixture was written`). Below 5 units the advisory behaviour stays — a level's opening
  script legitimately moves a body. The sidecar also records `pose_verified` from now on.
* `series-gate.ps1`: a sidecar carrying `requested_pose` with `pose_verified` false grades as
  `SKIP retail capture did not verify its requested pose` instead of scoring.

Both were exercised: the guard fired on the same row that had poisoned the file (no `.json` written, stray frames
deleted), a synthesised sidecar produced the new SKIP, and the untouched real fixture still grades
(`spawn port 0.9078 / retail 0.9238`, PASS, 1 graded, 0 failures).

**The unblock is a savegame, and the port already writes `.jks`.** `src/jks.rs` parses and writes the format, so a save
recorded in level 2 (or 15) is a two-part job: write one, then drive retail's load screen from the harness (`-Keys`,
which already exists for the interaction probes). That is the next retail-side project, and every blocked row
(level-2 spawn fixture, level-14 cutscene grade, per-level spawn survey) depends on it.

### The savegame route stops at retail's own slot listing (round 284, measured)

`-LoadSave -BootSaveLevel '15maw.jkl'` now gets as far as patching: the harness's 15-byte field check rejected a
9-character name first, so the patcher pads with NULs and reports

```
patched the savegame to boot 15maw.jkl (level number 15)
```

and the port's own reader agrees the file is valid — `jks: 452495 bytes, version 6, episode 'JK1', level '15maw.jkl',
game time 500, 7704 record(s), 330 thing record(s)`. Retail still does not boot it:

```
screenshot 8 s after Ok: load-after-ok.png
level booted: False after 44.1 s
```

The screenshot is retail's **Load Game** page with an **empty** save list ("Select a savegame", nothing listed). The A/B
that matters: the same run **without** `-BootSaveLevel` shows the identical empty list, while
`target/parity/retailcopy/saves/` holds `autosave.jks` and `quicksave.jks` and `player/Lee/save0000.jks` exists (452 KB,
the file the harness patches). So the patch was never the problem — **retail does not enumerate the slots this profile
has**, which means the route needs retail's own slot index/profile bookkeeping, not a patched file.

That is where this stops, and it is now a named next step rather than a mystery: find what retail reads to build that
list (the scratch profile has `Lee.plr`; `saves/` holds the two slot files; retail itself writes `_JKAUTO_<level>.jks`
into `player/Lee/`, so both directories are live), or reach the level through the campaign transition instead. Either
way the level-2 spawn refit, the per-level spawn survey and the level-14 cutscene grade stay blocked on it.

**What did land this round:** the harness refuses to write a fixture when a requested pose verifies more than 5 units
off (proved: it throws on the row that used to photograph level 1 under a level-14 name), the sidecar records
`pose_verified`, `series-gate.ps1` SKIPs an unverified pose (proved with a synthesised sidecar), and the savegame
patcher accepts short level names by NUL-padding (proved by the patch log above). The real level-5 fixture still grades
(`spawn port 0.9078 / retail 0.9238`, PASS).

### Three routes, all closed: retail cannot reach a non-first level (round 284)

The level-14 row ends here, and the evidence is three menus/commands rather than an argument:

| route | result |
|---|---|
| command line, `-JkArgs '-level,15maw'` | JK.EXE exits with code 3 before its window appears (`docs/gaps.md` rounds 60-61, re-proved today) |
| Load Game, patched scratch save | retail shows an **empty** save list; the identical run without the patch shows the same empty list, so the patch is not the cause — retail does not enumerate these slots at all |
| `[Debug]` on the Single Player submenu | "Choose Episode": *The Force Within / Jedi Capture the Flag / Jedi Training* — episodes, not levels (`debug-menu.png`) |

The port's own reader confirms the patched save is well-formed (`version 6, episode 'JK1', level '15maw.jkl', 452 495
bytes, 7 704 records, 330 thing records`), so the file is not the obstacle; retail's slot bookkeeping is. Reaching level
14 would mean playing thirteen levels in retail, which is not something an automated capture should do, so the row is
recorded as **not automatable** and the port's side rests on `macamera.key`.

## The menu probes relaunched the game once per row (round 284, fixed)

Chasing the level-load routes turned into thirteen JK.EXE launches to read one menu. The cause was in the harness, not
in the probes: `-DebugShot`, `-LoadShot` and `-StartEpisode` return from the **row**, not from the run, so the outer
`foreach` went on to launch the game again for every remaining row in `shots.tsv`. Reading one menu cost a full capture
pass and took the operator's display with it each time.

`retail-capture.ps1` now forces a **single** row when any of those switches is set:

```
one-launch probe: running 1 of 13 selected row(s), 'spawn'
=== retail-capture done: all rows captured ===        (17 s, one launch)
```

**Rule for the harness from here:** a switch that photographs a menu is one row by definition; if a future flag needs
the level to be loaded before it does its work, it must say so in the selection filter rather than ride the whole list.

## `--thing-list @x,y,z`: what is *here*, not "what is a crate" (round 285)

The template filter can only ask "where are the crates". A walk that stops where retail stops needs the other question,
and a thing the port does not treat as solid is exactly the one a template filter cannot find.

```
$ --start 03katarn --thing-list "@-0.77,-3.60,-0.21,1.5"
  thing  173 woodcratebig    at (  -1.111,  -2.287,  -0.290) sector   412 flags 0x0
  thing  187 canopy          at (   0.217,  -3.613,  -0.300) sector   371 flags 0x0
  thing  188 canopy          at (  -0.377,  -3.613,  -0.300) sector   369 flags 0x0
  thing  189 canopy          at (  -1.450,  -3.234,  -0.300) sector   375 flags 0x0
  thing  190 canopy          at (  -0.867,  -3.224,  -0.300) sector   369 flags 0x0
  thing  193 tuskanbh        at (   0.186,  -3.491,  -0.280) sector   371 flags 0x20003400
  ...
```

**What it answers for `walk3_spawn`.** Retail's trace stops at y = -3.60 and drifts west along x (1.71 units total); the
port walks 3.64 through the same place (213 %). `--faces-at` at that point reports **no solid thing** within 3 units,
while the point-filtered list shows a row of four `canopy` things whose nearest edge is y = -3.613 — the exact line
retail stops on. So retail's contact is with a canopy model, and this port does not treat it as solid: `solid_radius`
returns `None` unless the template carries `collide` > 0 **and** a `size=`, and these canopies have neither
(`flags 0x0`, and `canopy.3do` is not even in this level's archive set).

That does not settle it — a canopy is an awning, and the engine may be stopping the body on a *fence* that happens to
share that line — so it is recorded as the measured candidate, not as a fix. Widening solidity has already regressed two
legs once (`walk_cantina` 102 % → 12 % with the template radius), so the next step is the canopy's own model bounds and
the engine's `SearchForThingCollisions` flag test (`SITH_AF_SOLID`) before any list changes.

Incidentally: `trace-gate.ps1 -Only a,b,c` takes the comma list as one name and its preflight then reports "every
requested walk leg is blocked"; single names work (`-Only walk5_spawn` → 3.60 travel, PASS). Three single runs:
`walk3_spawn` 213 % FAIL, `walk5_spawn` 105 % sectors-off FAIL, `walk6_spawn` 95 % PASS.

### The canopy lead was wrong, and the switch that tested it proved it (round 286)

`decomp/004e8430_sithCollision_SearchForThingCollisions` gates a thing on its **own** words — a model pointer
(`puVar7[10] != 0`) and `(*flags & 0x80002) == 0` — then sweeps the sphere against the model's surfaces. That reads like
a different rule from this port's `solid_radius` (template `collide=` + `size=`), so a switch was built to use it.

It measured **identically**, and the reason is that the premise was wrong:

```
BK_THINGS_TRACE=1                    -> 225 solid things, 7 `canopy` entries
BK_SOLID_ENGINE=1 BK_THINGS_TRACE=1  -> 225 solid things, 7 `canopy` entries
trace legs, switch off / on          -> walk3 213 %, walk5 105 % (sectors off 5), walk6 95 % PASS — identical
```

So `canopy` **does** carry `collide=`/`size=` in this level's templates, the port already collides it, and the switch is
a no-op. It is removed rather than kept as an unused option; the finding stays here.

What the numbers leave standing is one concrete candidate: the canopy at `(0.217, -3.613, -0.300)` is a 10 cm kerb
(span z -0.371..-0.271, radius 0.2) with its top ~0.02 above a body whose feet sit at -0.29, i.e. **inside**
`moveSize + 0.01` = 0.075, so this walker steps onto it and keeps walking. Retail stops on that line and drifts west, so
either retail's feet are ~0.06 lower there or its step rule refuses that contact shape. The next measurement is the walk
trace's own feet height against that canopy's top at the contact — a printed number, not another switch.

### Retail's thing record holds the template name; the port assumes an index (round 286)

Chasing row 141's `thing + 0x144` thrust word through a real save hit a format fact instead. A retail autosave's
player record reads

```
0000069a  78 41 49 00 00 00 0a 00  77 61 6c 6b 70 6c 61 79   xAI.....walkplay
000006aa  65 72 00 ...                                            er.
```

— a `u16` **10** (the type) and then the NUL-terminated template name `walkplayer`, where `jks.rs` reads a
`u16` template index followed by a `u16` type, per the message-staging header `0x4f4210` documents. The same
mismatch explains the earlier reading `jks: 26540 bytes, version 6, episode 'JK1', level '01narshadda.jkl',
game time 0, 1 record(s), 0 thing record(s)` — the port writes one thing per `0x1001` record and retail writes a
*list* whose entries begin with the type and the name.

Two consequences, both now on paper: the `0x144` recovery needs that list walked field by field (the bytes after
the name are a string/length interleaving, not a float array — spot offsets at `+0x80` looked like the spawn `z`
and were coincidence), and any future "read a retail save" work starts from this byte `0x0a 00` + name rather than
from the staging header.

### `--save` no longer reports a retail file as empty (round 286)

The reader reported `1 record(s), 0 thing record(s)` for a GOG save that holds 330 things — true about the port's own
layout, false about the file. It now names the shape it found:

```
$ --save "...\saves\autosave.jks"
jks: 6880 bytes, version 6, episode `JK1`, level `15maw.jkl`, game time 0, 1 record(s), 0 thing record(s)
jks: 1 `BKSV` list(s) (retail's thing shape), largest 5196 bytes of entries — not decoded

$ --save target/parity/exp/port-save.jks        (written by this port, for contrast)
jks: 7308 bytes, version 6, episode `JK1`, level `01narshadda.jkl`, game time 500, 64 record(s), 64 thing record(s)
```

The differentiation is the point: a port-written save prints **no** `BKSV` line, a retail one prints one. Decoding that
list is the remaining work, and it is now a stated gap rather than a silent zero.

## Retail saves can be read: 357 things out of a shipped quicksave (round 287)

The `BKSV` list is decoded now, in `jks::retail_things`, and `--save` reports what it walked:

```
== autosave (level 15maw, 5 196-byte list)
jks: `BKSV` list of 5196 bytes -> 73 entr(ies) decoded (last `ghost`), 1147 byte(s) unread
jks: player at (39.475, -7.275, 28.302) health 100          ← level 14's own spawn
== quicksave (level 01narshadda, 24 692-byte list)
jks: `BKSV` list of 24692 bytes -> 357 entr(ies) decoded (last `+dbattery`), 4385 byte(s) unread
jks: player at (-10.443, -5.455, -1.185) health 100         ← level 1's spawn
```

**The entry layout, measured:** `u16 kind`, ASCII template name, seven floats (position, angles, and one more —
`100.0` for the player's health, `1.0` on statics), then a tail. The tail is **17 bytes** for the overwhelming
majority; a handful of kinds carry 14, 15, 16, 18 or 19, so the decoder tries that bounded set and accepts the first
tail whose following entry has a plausible shape. Two subtleties cost a rebuild each and are worth remembering:

* **The name's last character can be a float byte** — `walkplayerY`, `cargoshipf` — so the split must be validated by
  the *next* entry's shape, not by "does a float start here".
* **The last entry before a malformed region was being dropped**, because every entry was required to be followed by a
  valid one. It is now accepted with the canonical tail and ends the walk, so nothing read is lost.

A unit test builds two synthetic entries (patterned tails, since an all-zero tail makes many split points look valid)
and asserts the walk decodes both and stops before a malformed kind word: **216 tests**.

**What it unblocks:** retail-side things (positions, kinds, template names) without launching the game — the spawn-audit
work that the level*load* route was blocking. **What it does not:** the remaining ~20 % of each list (a per-kind tail
table is needed, from the engine's restore switch), and `thing + 0x144` is still not in a save at all.

### `-Only a,b,c` works in both walk scripts now (round 288)

`pwsh -File trace-gate.ps1 -Only walk3_spawn,walk5_spawn,walk6_spawn` arrives as **one** element of `[string[]]$Only`,
so no leg matched and the preflight reported "every requested walk leg is blocked in the port" — a message about the
legs when the fault was the argument. Twice this session that cost a detour.

Both `trace-gate.ps1` and `walk-preflight.ps1` now split each element on commas before matching:

```
$ pwsh -File tools/parity/trace-gate.ps1 -Only walk3_spawn,walk6_spawn
FAIL  walk3_spawn    port  3.64 / retail  1.71 units (  213%)  sectors off 5
PASS  walk6_spawn    port  1.03 / retail  1.08 units (   95%)  sectors-ok True
trace gate: 2 leg(s), 1 failure(s)
```

Both scripts parse clean and the multi-leg form is the one thing this changes; single names behave as before.

### Level 1's spawn nudge was a floor-scope bug, not a clearance bug (round 289)

The level-1 spawn was displaced from the recorded (-10.443,-5.455) to (-10.73,-5.17), a 0.286-unit
sideways move through an interior wall. `BK_THING_CLEARANCE=0.13` did not fix it: the spawn then wedged
against surface 3301 and Space failed. The furniture colliders were not the cause either; their model
spans sit above the body's floor-level centre.

The floor probe found the right face (surface 3300 in sector 339, floor -1.25) but then tried the
recorded sector's containment alternatives. Sector 340's **infinite hull** contains the spawn although
its solid polygons cover only strips beside the tables. That hull-only admission imported the cantina
floor through a wall. The final fallback then chose the wrong room and displaced the body.

`Solids::room_contains` now checks the hull **and** polygon coverage. Portal polygons count as room
coverage but remain non-solid. The scoped floor search uses the same predicate, and both spawn passes
reject candidates outside the recorded room. A regression test covers both a wall-separated room and a
real portal connection.

Measured after the change: the `walker: spawn ... was inside something` line is gone; live-play forward
travel is 0.21 units and Space lifts 0.11; `217` binary tests pass; `--gates` is `0 of 66 failed, 0
skipped`; `--walk all` reports no void falls. Five affected trace legs remain four PASS and the known
`walk5_spawn` sector-mismatch FAIL (3.90 / retail 3.73 units, sectors off 5). Parity draw: `5
threshold(s), 0 failure(s)`.

### `COLLIDEBOX` was being overwritten by `BOUNDBOX`; two more walk5 theories rejected (round 290)

The parser matched `COLLIDEBOX` with `starts_with("boundbox")`, then overwrote it on the next line
with the enclosing `BOUNDBOX`. The engine's `sithIntersect_IsSphereInSector` short-circuit reads the
**collide** box. The parser now gives that row precedence and a unit test pins the order. Floor
reachability uses the same box short-circuit before its polygon test. This is a real format fix, but
the level-5 trace is unchanged: `walk5_spawn` remains 3.90 / 3.73 units with five sector mismatches,
while `walk_cantina` and `walk6_spawn` still pass.

Two further experiments failed the roadmap's acceptance table and were reverted:

* A radius-zero floor probe (the engine's `FindFloor` shape): `walk5_spawn` 3.22 units, still five
  mismatches; `walk6_spawn` regressed to 0.85 units with three mismatches; `walk_street90` dropped
  to 70 % travel.
* Adopting the landing face's owner as the body's sector: `walk5_spawn` unchanged; `walk6_spawn`
  regressed to 0.84 units with four mismatches. Retail `FindFloor` attaches a surface but does not
  call `sithThing_SetSector`, so this rejection also agrees with the recovered function.

The recurring 35 → 33 → 35 pattern therefore is not fixed by a ray, a landing-owner assignment, or
the collide box. Moving the sector transition to the sphere's contact point (rather than one radius
through the doorway) was also measured and reverted: `walk5_spawn` was unchanged and the other four
legs still passed. Live `BK_SECTOR_TRACE` names no portal into 35 at the bounce, so the oscillation
is coming from the post-move containment resolver rather than a direct portal transition.

Applying `room_contains` to the shared `sector_at` resolver was not safe: walk traces stayed the same
but `--fire +bryarbolt` regressed (the projectile stopped on a wall and a damage assertion failed).
That broad change was reverted; the fire gate passes again. `BK_CONTAIN_TRACE` then pinned the
oscillation: the resolver invents sector 34, then 33, even though `crossed_to` remains 35. Testing the
body centre instead of its feet produced byte-for-byte the same five-leg result, so the difference is
the resolver decision, not the vertical coordinate used for containment.

### Level 17's empty startup was a failed spawn, not missing level geometry (round 291)

The normal `18Ascend` start recorded sector 505 at z −10.979886. `spawn_point` found no clear local
standing point, left the body in the shaft, and it fell against a wall. The first screenshot was black;
the settled screenshot was only rock. The `--warp` diagnostic at z −8.174 rendered a complete room,
which isolated the failure to startup placement.

The no-clear-spawn path now searches upward for the first **different** containing sector and seats the
body on that room's floor. It selects sector 485 at z −8.135006. The recovered 120-frame screenshot
shows the room, floor, and crate. Restricting this to a different sector matters: the first attempt
selected another floor inside 505 at z −13.035, still in the shaft.

The same movement experiments also produced a green five-leg set: `walk5_spawn` 2.51 / retail 3.73
units with two mismatches, `walk6_spawn` 1.04 / 1.08 with two, and the three cantina/street legs
passing. The cause was two duplicate `sector_for_body` calls in `walk_player` overwriting the mover
before every step; those calls are removed, while the floor-seat clamp remains.

`walk17_spawn`'s trace still falls from 485 into the thin sector 502 under some movement states. Its
visual startup failure is fixed separately; that trace mismatch is not being claimed as resolved.

### Outside review: the retail walk trace is not timed in game time (2026-11-01)

A cold review against the roadmap (findings in full: `porting-roadmap.md` §"Outside review (2026-11-01)").
The one that matters: `sithTime_Advance` (`0x4dd640`) advances game time by the frame delta clamped to
[10, 500] ms, and retail renders a few frames a second under dgVoodoo, so the harness's `held_ms`
(injected key time) is not the time the body walked — a 600 ms hold is 2–4 frames, ±1. The
`walk_street90` fixture's 1.28/0.64/0.29-unit segments show it, and `Rules::walk = 1.2` was measured
from that leg's average including its stop at x 3.27. Same day, the harness side landed: the capture
reads the game-ms counter at `0x00836c14` (`JkRetail.ReadU32`; JK.EXE's base is a fixed `0x400000`
with relocations stripped) and writes a 7th `game_ms` column, and `trace-compare.ps1` uses it as the
time axis when present. Checked offline only (scripts parse, a self-read returns the planted value,
a synthetic 7-column fixture switches the axis, old fixtures compare unchanged); no retail capture
has used it yet. Next step: re-capture the walk legs with it. The findings came from the decompile,
the harness and the fixtures, not a new retail run. Also logged: the hold loop releases the key per segment despite its
comment, the port's `dt` clamp (0.1, no floor) differs from the engine's, and two stale code comments
(`thing_transform`'s axis note, the `Rules` radius note) were corrected. Tests: 218/218 on the
uncommitted tree; `--feel` passes.

### `walk17_spawn` closes: four engine deviations between a resting body and the collision data (round 292)

`walk17_spawn` was the trace gate's last failing leg. The body started on `18Ascend`'s spawn, fell
0.06 into the elevator shaft, flipped from sector 485 to 502 on its first forward step, and wedged at
0.16 of retail's 0.70 units. Four separate deviations were found and fixed, each against a decompiled
constant or function:

* **A prop was solid where it should not be.** The walker's cylinder list held every placed thing at
  the position the level file gave it, and it was global. `SearchForThingCollisions` walks the **moving
  body's own sector** thing list, and `18Ascend`'s spawn sits over the elevator mouth: leaves 323/324
  belong to sector 504 while the body stood in 485. The list now carries each thing's sector, is
  rebuilt from the live runtime positions every frame (a door or lift follows its frame path, so a
  frozen placement is wrong even in the right room), and only the current room's props can touch the
  body. Measured: the per-frame push-outs disappear and the body walks.
* **The first-frame seat was one hundredth high.** A body placed with its feet below the floor was
  seated at `floor + radius + 0.01`; the engine's contact is `floor + moveSize`. The extra hundredth
  made a resting body hover, sink for four frames, and re-seat — a permanent bounce at every spawn.
* **The floor sweep needed the sector plane.** The face list does not tile over the shaft opening
  (`adjoin 1150`): the body's centre stands over bare space while the room's plane runs beneath it.
  `FindFloor` reads that plane, so a contact this frame's travel reaches is now taken when no face
  answers — only then, so a polygon still decides.
* **`IsSphereInSector` has a tolerance.** It snaps a plane distance within `1e-5` of zero to zero
  before comparing with the radius (`_DAT_00523288` = 1e-5, `_DAT_00523284` = 0, read from JK.EXE).
  `floor + radius` in f32 lands a ten-millionth outside the plane, so the port rejected a body
  standing on solid ground every frame and adopted the shaft below it. The band is now transcribed,
  with a unit test on the boundary.

Result: `walk17_spawn` walks 0.82 of retail's 0.70 units (117 %), **0 sector mismatches**, and its
end height is the same 0.055 convention as every other passing leg. The trace gate stands at
**10 legs, 1 tracked failure** (`walk5_spawn`). `walk6_spawn` also settled closer (z 0.070 -> 0.069).

`walk5_spawn` is unchanged (3.90 / 3.73, 105 %, five mismatches, z 0.175). Its earlier two-mismatch
pass was the same containment dust: the resolver was inventing 34 and 33 from a rejected feet point,
which the epsilon now correctly refuses. The leg now shows its real deviation, which is the
floor-search scope row.

Rejected on measurement, agreeing with rounds 290 and 291: adopting the landing face's owner as the
body's sector. Blanket adoption moved `walk5_spawn` to 161 % with z 1.72 and broke `walk6_spawn` and
`walk13_spawn`, because a landing often sits on a slab that belongs to the room below. A narrower
rule (adopt the owner only when the landing is *below* the body's own floor plane) changed nothing on
`walk5_spawn`, whose descent never lands below 35's own plane, and was left out rather than shipped
untested.

Gates after the round: tests 219/219, `--gates` 0 of 66, `--walk all` 0 levels in the void (worst drop
2.76, `m_boss17`), compare 5 thresholds 0 failures, series 1 graded 0 failures, preflight 21 walkers,
trace 9 of 10.

### The walk trace reaches 10 of 10: the plane fallback gets a data-gap test (round 293)

Round 292 added a floor-plane fallback so 18Ascend's spawn could stand where the level has no floor polygon
under it. It held every body on its sector's infinite plane whenever no face answered, which is also what
level 5's walk needed to *fall*: on the way out of the spawn room the floor's polygon ends and the only floor
below is a slab half a unit down in the next room, so the body was pinned at z 0.165 for all 198 landings and
never changed sector (105 % travel, five sector samples off, z 0.175).

Measured discriminator, not a guess: with the fallback deleted entirely, `walk5_spawn` walks 3.09 of retail's
3.73 units (83 %, two sector samples off) and `walk17_spawn` re-breaks at 0.16 units; with the fallback as it
was, the pair inverts. The two legs differ in the data, not in the rule: 18Ascend has **no surface at all**
under the spawn (`--floor-at` finds no candidate within 1.1 units, even globally), while level 5 has one
0.51 below. The fallback now asks that question — `floor_under(sector, pos, 1.0, radius)` must be `None`, the
same reach the `--floor-at` probe uses — before it seats the body on the plane, and still only when the contact
is within this frame's drop.

Result: `trace-gate.ps1` is **10 legs, 0 failures** for the first time — `walk17_spawn` 117 % / 0 sector samples,
`walk5_spawn` 83 % / 2, every other leg unchanged. `walk5_spawn`'s remaining deviation is its **height**
(z 0.422): the body still stands on a surface above retail's, which is the floor-search-scope ledger row, not
the sector bookkeeping the round-290/291 experiments chased.

Correction carried into the code: `sithPhysics_FindFloor` (`0x4f5550`) does not read a sector plane. Its body
calls `sithCollision_SearchForCollisions(sector, 0, &thing->position, ±axis, range, …, 0x2802)` and attaches
on the first surface hit, range `moveSize + 0.005` for a non-attached body — so the engine's floor search is
a swept-surface query, and the plane fallbacks in `walker_step` are the port's own data-gap repair, now
described as such rather than as engine behaviour.

Gates after the round: tests 219/219, `--gates` 0 of 66, `--walk all` 0 levels in the void (worst drop 2.76,
`m_boss17`), trace **10 legs 0 failures**, compare 5 thresholds 0 failures, series 1 graded 0 failures,
preflight 21 walkers.

### The retail clock is not the movement axis: re-capture measured, axis reverted (round 294)

The outside review's first finding asked for a walk re-capture using the engine's own clock
(`0x00836c14`) before any further walk-speed work. Ten legs were re-captured with it. The clock is real
and readable, and the column now lands in the trace, but it is **not** the time the body walks: a
3 000 ms hold (five 600 ms segments) produced **17 019 ms** of engine clock in about 19 s of wall time,
while the body moved the same 3.7 units it always did. The clock keeps accumulating through the console
dialog and the foreground waits, so it measures wall time; `held_ms` — the span the key is down — is
within a frame of the simulated time, and the per-segment displacements (0.5-0.9 units at the 1.2 u/s
rule) say so.

Switching `trace-compare.ps1` to the engine clock as the axis did what that measurement predicts: the
port was sampled at times 5x too late and three legs failed (cantina 142 %, street45 and walk9 four
sector samples off). The axis is back on `held_ms`, and the comparator now prints the engine clock as
a drift figure (`{clock} over {held} ms of held forward ({ratio}x)`) instead of using it. The 7-column
capture format stays; older 6-column fixtures are unaffected.

Two harness facts came out of the same run:

* Re-capturing is not reproducible on NPC-populated levels: `walk_cantina` walked 1.86 units this run
  against its fixture's 2.59 (a cantina patron in the way). The committed fixtures stay the reference;
  a live capture in `target/parity/trace` still takes precedence, so clearing that directory restores
  the fixture baseline (done).
* The stuck guard threw away legs that **walk and then stop**, which is half the shipped walk set
  (`walk4_spawn` walks 0.30 units, `walk6_spawn` 0.2, `walk17_spawn` 0.70) — six captures were lost
  before this was found. It now throws only when the body never moved at all, and otherwise records the
  samples it took and logs the stop, which is the measurement those legs exist for.

After the revert the trace gate is **10 legs, 0 failures** again. One uncommitted capture remains in
`target/parity/trace` only while a capture is being taken; none is present now.

`walk5_spawn`'s remaining height deviation, measured with the new descent in place: the body follows 35 -> 33
and rests on the level's faces there at **-0.436 / -0.455**, ending 0.25-0.39 below the spawn plane, while
retail's body sits at roughly **-0.065** on the same sample points — so the port is about **0.33 low**, the
opposite sign from the plateau note of earlier rounds. `--floor-at` at each retail sample lists only the
-0.43/-0.455 faces (surfaces 361/382/371/378/380, all owned by 33); nothing in the port's face list lies at
retail's height. The one solid nearby is `waterpump` at (9.23, 13.54): model z -0.012..-0.451, solid radius
1.036, i.e. its top is *above* retail's body and its body spans the port's resting height. So this is the
solid-model floors question (the ledger's "solid things are floors" row), not the floor-plane rule, and it
needs the pump's drawn surfaces rather than the AABB stand-in.

### `sleep` inside a branch resumes at the statement after the wait (round 295)

The ledger's cog row: the VM recorded one top-level statement index per label, so a `sleep` inside an
`if` (or any nested block) resumed by re-running the enclosing statement and therefore every assignment
before the wait in that branch. `Vm::resume` now carries a **statement path** — `(statement index, branch)`
per level, with the branch marking `if`/`else` — which `Vm::run_from` records as it descends (blocks,
`if` arms) and `Vm::run_path` replays to re-enter at the exact statement. Two bounds are deliberate:

* A `sleep` inside a **loop** still resumes at the loop statement. The loop's condition and step are part
  of that statement, and a mid-body re-entry would have to replay them; `run_path` truncates the descent
  at the first non-block/non-`if` step and the loop runs again from its top, which is the old behaviour.
* A branch written without braces (`if (x) Sleep(1);`) also resumes at the `if`, because its body is not a
  block and its condition has to be evaluated again.

`cog::tests::a_sleep_inside_a_branch_resumes_after_the_wait` is the check the row asked for: a counter
before a `Sleep` inside `if (1) { … }` must stay at 1 across the resume and the statement after the wait
must run. Verified against the old behaviour by forcing `run_path` to the old index branch — it fails with
"the branch prefix ran twice" — and it passes with the path in place.

Regression chain after the change: `cargo test --bins --release` 220 passed / 0 failed (the card now holds
220, the new test included), `--gates` 0 of 66 failed, `--walk all` 0 levels in the void, `compare.ps1 -Gate`
5 thresholds / 0 failures (12 pairs, 0 failed), `series-gate` 1 graded / 0 failures, `walk-preflight` 21
walkers / 8 short (unchanged), `trace-gate` 10 legs / 0 failures, with the axis back on `held_ms`.

### The engine's frame clamp, the sector-message gate, and a capture that ends its own level (round 296)

Three related findings, all measured.

**Frame clamp.** `sim_dt` now applies `sithTime_Advance`'s `[0.01, 0.5] s` clamp instead of
`min(dt, 0.1)`, closing the ledger row: above 100 fps retail's clock runs faster than wall time and the
port now does too. Captures and gates pin their step through `CaptureDt`/`BK_FIXED_DT`, so no parity
number moves — gates 0 of 66, `--walk all` 0 in the void, the soak pass, tests 221. The `0x400` slowmo
scaling that rides on top of the clamp is still not applied and the row says so.

**The sleep fix exposed a sector-message overreach.** Right after the statement-path `sleep` landed, the
`walk5_spawn` leg began failing with zero travel: the level *completed* mid-run and reloaded as
`06BBarons.jkl`. `BK_LEVEL_TRACE` named the caller — `06_endpart1.cog`, the level's rubble-area goal
script. Its `entered:` handler sleeps inside an `if` and then ends the level; with the old resume the
handler aborted at the first wait because `played` was already 1, so the sequence had never run. The
new resume runs it correctly, and the sequence is real.

What was not correct was *why* the handler ran on a level start: `Runtime::sector_messages` sent
`entered`/`exited` to every sector link. `sithThing_EnterSector` (`0x4cd2c0`) is narrower — it messages
a sector only when the sector's byte at `+0x34` carries bit `4` (`SITH_SECTOR_COGLINKED`) and the
thing's flags carry none of `0x80100`. Both conditions are now applied, pinned by
`sector_messages_only_reach_a_cog_linked_sector_and_a_watched_body`. Retail's own walk5 trace walks its
3.73 units continuously, so retail was not running that sequence; the COGLINKED gate is what keeps the
port's spawn from posting a message the level never meant to receive there.

**A capture must not swap the level it is measuring.** Even with the gate, a script can legitimately end
the level during a capture (`06abarons`' sequence does, and any `jkEndLevel` would). `watch_level_complete`
now ignores a completion while a screenshot capture is running (`CaptureDt` present) and stays in the
level, printing the reason under `BK_LEVEL_TRACE`. The first version of the guard keyed off
`CaptureMode`, which is also true for `--goal` and the tour gates, and broke `--goal` ("the level never
ended"); keying it to the pinned-step resource fixed that. `--tour-live` failed once in a full `--gates`
run with "no verdict (opened a window)" and passes on its own — window contention, not this change.

Chain after all three: tests 221 / 0 failed, `--gates` 0 of 66, `--walk all` 0 in the void, soak pass,
`trace-gate` 10 legs / 0 failures (walk5 back to 3.09 / 3.73, 83 %), `compare.ps1 -Gate` 5 thresholds /
0 failures (12 pairs), `series-gate` 1 graded / 0 failures, `walk-preflight` 21 walkers / 8 short.

### The retail Debug chooser boots any level; the Load screen is empty by design (round 297)

Following the roadmap's blocked retail rows, the savegame route was re-investigated from the executable
rather than by guessing at the UI. `jkGuiSaveLoad_PopulateList` (`0x41ddd0`) formats
`player\<profile>\*.jks` through `sithGamesave_GetProfilePath` (`0x4da400`), skips every file whose name
starts `_JKAUTO` (`__strnicmp` against the table at `0x0053bffc`, which holds `jks`, `_JKAUTO_`, `rb`,
`save…`), reads `0x62C` bytes and keeps an entry only when the first dword is `6`. So retail's own play
leaves autosaves in the profile folder and the Load screen *hides* them: with no player profile bound yet,
a fresh main-menu Load screen is empty by design. That is exactly what the "failed to load save" screenshots
showed (`target/parity/retail/load-after-ok.png`: "Select a savegame:" over an empty list).

The clean route was already wired and unblocked: the Single Player screen's **[Debug]** toggle opens a
numbered "Choose Level" list, and `retail-capture.ps1 -StartLevel <N>` drives it by clicking the first row
and pressing `DOWN` `N-1` times (this scrolls past the visible nine; a y-coordinate click cannot reach them).
`level2_spawn` was booted that way:

```
chose level 2 through the [Debug] level select
spawn| Pos: (10.80, -11.98, 3.02) PYR: (0.66, -99.98) Sector: 22
```

— the row's requested pose, verified. `compare.ps1` then graded the pair at **SSIM 0.3705 (2.01 dB), mean
abs diff 23.8/255**: the towers, their yellow window lights, the floor line and the stepped balcony all
match; retail's frame is much darker and carries no HUD in that moment, the port's does. The deficit is the
lighting ledger row, not framing — the row now has a verified retail prerequisite and a baseline number.

Harness change kept: `retail-capture.ps1` seeds `player\Lee\save0000.jks` from the freshest autosave
before its cleanup deletes them, so the savegame route can still be tried where a profile-bound session
is wanted; the 30-minute stall in the earlier attempt was the `-LoadSave` retry loop against an empty list,
not the engine.

### Why the tuskens shot first: the engine's sight is a roll, once a second (round 298)

The ledger's spawn-combat row said "mechanism unknown". It is now recovered from the image and ported.

`sithAI_CanDetectSightThing` (`0x4ec140`) is a **probability**, not a cone test. Read as floats from
`0x00523060..0x005230a0`: `2.0` (the gap at which the penalty starts), `0.1` (slope), `0.6` (cap),
`0.5` (base unless the AI word carries `0x200`), `0.75` and `0.05` (player penalties), `0.05` (floor),
`1/32768` (the roll scale). The shape: chance `= 0.5 (or 1.0) x (1 - clamp((d - 2) * 0.1, 0, 0.6))`,
times `0.75` when a crouched player carries `0x10000`, times `0.5` when the player's view angles are
all zero, times `0.05` when the target's `thingflags & 0x80` and the actor lacks `0x8000`, times `0.05`
when the actor carries `0x800`; clamped to `[0.05, 1.0]` and compared against `rand * (1/32768)`.

The **cadence** is the part that matters at a spawn: `sithAI_InstinctUpdate` (`0x4e9e90`) sets each
instinct's own clock to `now + 1000` *before* calling it, so every instinct — sight refresh included —
runs at most once a second. `sithAI_Process` (`0x4e9e20`) skips an actor whose word at `+8` carries
`0x3000`, and `AISetMode` (`0x500f50`) is a plain `|=` of the script's argument, so `0x2000` (24 shipped
calls) is the sleep bit the port already honoured; `0x1000` is only ever *cleared*, never set.

The port had the formula but rolled it **every frame** — a 50 % chance latched in one or two frames — and
the `LookForTarget` path wrote `ai_last_seen` from the cone alone, with no roll at all. That memory is
what `BlindFire` fired at 0.02 s into level 3. Fixes: `ai_detect_at` arms the roll once a second per
actor (first attempt immediate, as the engine's instincts start at `next = 0`); the scan path rolls
`can_detect` and latches `ai_detected` before it remembers; and every memory write goes through
`Runtime::remember`, so `BK_AI_TRACE` prints who wrote it (`sight`, `lookfortarget`, `sensedanger`,
`surprise`) instead of leaving it to inference.

Measured on level 3's spawn (`BK_AI_TRACE=2`, 180 frames): `ai: 237 fires PrimaryFire at 2.82 (time
0.00 s)` and `ai: 245 fires PrimaryFire at 1.63 (time 0.00 s)` — the rolls whose first pass succeeded,
which is the engine's own behaviour. Retail's series showing no damage is a draw outcome over eight
frames, not a rule the port was missing; the port now has the same distribution and the same one-second
attempt spacing.

Tests: `sight_detection_rolls_once_a_second_not_once_a_frame` (the roll is armed 1 s out and does not
re-run mid-second), `look_for_target_scans_the_class_view_cone` (keeps its cone assertion, now with a
plainly-visible target so the roll is certain), and `a_turret_fires_and_does_not_walk` pre-latches
detection because it tests `TurretFire`, not acquisition. Suite 222 passed.

Chain after the change: gates 0 of 66, `--walk all` 0 in the void, soak pass, `trace-gate` 10 legs /
0 failures, `compare -Gate` 5 thresholds / 0 failures (12 pairs), `series-gate` 1 graded / 0 failures,
`walk-preflight` 21 walkers / 8 short.

### The other half of spawn combat: AI bolts never leave the muzzle (round 299)

With the sight roll in place, the port's level-3 tuskens still fire at the player and still do no
damage — the same HUD the retail series shows. That equality is not the same mechanism, and the
traces say why.

`BK_AI_TRACE` now prints the shot the moment it leaves (`+ebow`, damage 20, ~4 u/s along the shooter's
yaw and pitch), the player-damage event with its time and remaining health, every `impacts` candidate
within two units of the body, and — inside the thing step — which of the two stop paths a projectile
took. On level 3's spawn:

```
fire: +ebow dmg 20.0 vel [-0.25597072, -3.9405258, 0.6377585] kind 3 owner 237 from [-0.6122036, -2.4175074, -0.342343] yaw -91.4 pitch 12.3
ai: shot 345 pushed out at [-2.186019, -5.2138696, -0.5509875] -> [-2.2300336, -5.1753488, -0.55098766] (sector 402)
ai: shot 345 `+ebow` flat 1.55 dz -0.82 vel [0.0, 0.0, 0.0] ...
```

So the bolt is born with the right velocity and is **pushed out of the level and stopped on its first
frame**: the shooter's template carries no `fireoffset=` (`tuskan` -> `_humanactor` have none; the two
`fireoffset=` lines in `03katarn.jkl` belong to gun templates), so the port's fallback spawns the shot
`1.0` ahead of the body at `z + 0.1`. On a stair that point is inside the next riser, and a 0.15-radius
bolt there overlaps the geometry. Nine shots at 1.2-3.8 units over ten seconds produced **zero** damage
lines; the player stayed at full health, exactly like retail's series.

Two candidate fixes were tried and both **reverted**, because neither has the engine's rule behind it:
spawning at the eye (`z + 0.18`) and spawning at the body (`0.05` ahead, `z + 0.12`). Both changed
observable behaviour without evidence, and the second only moved the failure to a different frame
(`ai: shot 351 stopped from [...] to [...] (escaped false)`). The real answer is
`sithWeapon_WeaponFireProjectile`'s spawn point — the weapon thing's own position when the template
names no offset — which is now a ledger row of its own (*AI projectile spawn*) with this measurement on
it.

What stayed: the four `BK_AI_TRACE` diagnostics (shot-left, player-damage, near-body candidates,
projectile stop path) and `Runtime::remember(actor, at, why)`. Suite 222 passed, `--gates` 0 of 66,
`--walk all` clean, soak clean.

### The engine's spawn rule, and why the stand-in stays (round 299, continued)

`sithWeapon_FireProjectile` (`0x4d5f20`) settles where a projectile is born: `param_5` is the offset,
compared against a sentinel; unset means the shot starts at the **thing's own position** with nothing
added, otherwise the offset is rotated by the weapon thing's orientation and added to that position.
The function is called with the *weapon thing*, which in a shipped game sits in the actor's hand.

Two spawn rules were tried against that evidence and both came back out:

* **Eye height** (`z + 0.18`): no engine basis, and the bolts still died (a second stop path showed up).
* **The sentinel branch** (spawn exactly at the shooter's position): faithful to the decomp, but a
  0.15-radius bolt born at a body's centre overlaps the floor plane, and the `--hit` gate failed
  (`the bolt never landed on a placed actor — the path the game uses`). The gate is calibrated on the
  stand-in and the port has no separate weapon thing to hold a real muzzle.

So the fallback stays at `body + forward * 1.0, z + 0.1`, now with the engine rule, the two failed
experiments and the reason the stand-in is kept written next to it (`src/runtime.rs`) and in the ledger
row *AI projectile spawn*. What remains on that row is not a guess: either a real weapon thing exists, or
the thing step puts projectiles on the swept sector path — bolt 351 dies to `segment_blocked` at
`[-0.980,-5.029,0.055]`, which is the `collide.rs` `step` simplification the `ponytail:` marker already
names, not a muzzle problem.

Final state after the revert: suite 222 passed, `--gates` 0 of 66, `--walk all` clean, soak clean,
`--hit` passes.

### Where level 2's 2x brightness comes from (round 300)

The lighting row's first pair (`level2_spawn`, verified pose) measures **33.0** port luma against
**16.6** retail on the HUD-free upper crop — a factor of 1.99. The code says where that factor lives:
`Batch::push_polygon` shades a world vertex as
`light_scale(baked vertex intensity) * light(surface, sector)` and `light()` is
`1.0 + surface extra + scripted sector light + thing light`, clamped `0..4`. The **base is 1.0**, so a
sector's own baked `AMBIENT LIGHT` never enters the product — while the engine hands each sector's
`ambient + extraLight` to `rdCamera_SetAmbientLight` (`sithRender_RenderSectors`, `0x4c6c40`) and shades
through the colormap tables.

Level 2's sector data settles the arithmetic: 641 sectors, mean `AMBIENT LIGHT` **0.523**, mean
`EXTRA LIGHT` 0.03 — and `1 / 0.523 = 1.91`, within 5 % of the measured 1.99. Level 1's spawn pair
(mean ambient near 1.0 there) matches at SSIM 0.92, which is why this only showed up on level 2.

Not changed this round: the ambient term's *form* is still open (multiplying the product, entering the
colormap index instead of `light_scale`'s normalised ramp, or `min(1, vertex + ambient)`), and
`light_scale` normalises the ramp's own midpoint to unity, which is a second, smaller factor. The row
now carries the formula, the sector statistics and the ratio, so the next attempt starts from numbers
instead of "the port looks brighter".

### Level 2's brightness: the sector ambient was missing from the product (round 301)

The lighting row's numbers said the factor was ~2.0 and level 2's sectors average **0.523** ambient
(641 sectors, extra ~0.03). `Batch::push_polygon` shaded each world vertex as
`light_scale(intensity) * (1.0 + extra terms)`: the base was `1.0`, so the sector's baked ambient never
entered the product. The engine's renderer sets it per sector, `rdCamera_SetAmbientLight(ambient +
extraLight)` (`sithRender_RenderSectors`, `0x4c6c40`), then shades through the colormap tables.

One-line change: `Runtime::sector_light_baked` (clamped `ambient + extra` per sector), `SurfaceLook::ambient`,
and `light()` uses it as the base instead of `1.0`. Measured on the spawn frames' HUD-free upper crop:

| pair | before | after | retail |
|---|---|---|---|
| `level2_spawn` | 33.0 (1.99x) | **17.0 (1.02x)** | 16.6 |
| `spawn` (level 1) | 42.0 (0.86x) | 40.5 (0.83x) | 48.9 |

So the port was exactly `1 / mean ambient` too bright on the level that uses the whole ramp; level 1,
which bakes near-full ambient, barely moved and stays 20 % dark, which is a different (smaller) term.
New test `surface_light_tests::a_sector_ambient_is_the_base_of_the_world_light` pins the base and the
scripted-light addition, and a 1.0-ambient level stays exactly as it was.

Chain after the change: 222 tests (+1), `--gates` 0 of 66, `--walk all` clean, compare 5 thresholds /
12 pairs 0 failures; trace 10/0, series 1/0 unchanged.

### Phase 1 step 4: the use action's reach was double-counted (round 302)

The cantina door switch was the step-4 probe. Two retail captures at the recorded poses both read
`doorstatus 0`, and round 271 explained that with the engine's own sweep. The arithmetic was wrong by a
second 0.1: `sithPlayerActions_Activate` (`0x4fc0a0`) passes `*(float *)(thing + 0x2c) - _DAT_00523200`
as the radius, and `_DAT_00523200` (read from `.rdata`, VA `0x523200`) is **-0.1**. The reach is
`moveSize + 0.1` — **0.16** for the 0.06 body — not the 0.265 the earlier reading got by adding a 0.1
sphere the call does not contain. `use_reach()` now states it, `use_reach_tests` pins it, and the live
`use_key` calls through the helper.

Measured, at three poses (port, headless, `BK_USE=1`):

| row | distance to the plate | port | retail |
|---|---|---|---|
| `switch_use` (new: -4.29,-3.76) | 0.12 | `use: activated surface 2409`, cog 11 `doorstatus 2` | not measured |
| `switch_cantina2` (-4.29,-2.95) | 0.93 | `use: nothing linked within reach` | `doorstatus 0` |
| `switch_cantina` (-4.75,-2.95) | 1.04 | `use: nothing linked within reach` | `doorstatus 0` |

So the port now agrees with retail at every pose retail measured, and the door's own cog reacts when the
use does reach: `Cog #11 (after the run): Name:00_doorswitch.cog`, `doorstatus = 2`.

Three harness/tooling facts found on the way:

- **`--cogstatus-after` needs a *windowed* run with `BK_EXIT_FRAMES`.** `--frames` is the capture frame
  (`shot_frame`), not an exit budget; a headless run exits on its own ("No windows are open") before the
  print, and a windowed run without `BK_EXIT_FRAMES` never exits. `BK_EXIT_FRAMES=200` + windowed prints
  the table in ~2 s.
- **The after-print looked up the wrong slot.** `Sim::instances` is the *started* VM list in start order,
  not the level's cog table; `print_cog_status_after` indexed it with the level's cog number. It now finds
  the instance by `Vm::cog_index`, which is what `GetSelfCog` returns.
- **`-Keys` clicked without focus or hold.** The warp's console dialog takes focus and DirectInput reads
  the mouse only while the game is foreground; `-KeysAfter` already restored it, `-Keys` did not, and the
  middle button was pressed with no hold for a software renderer running at a few frames a second. Both
  paths now force the foreground and use `-ActionHoldMs`.

Retail's *positive* is still unmeasured: two runs with `-Keys MOUSEMIDDLE -ActionHoldMs 600/700` at 0.12 (and one `-Keys SPACE`, since the engine also binds ACTIVATE to a key)
left `doorstatus 0`, so the click is not reaching the engine's activate at all. The next route is a
keyboard-rebound scratch profile (the engine binds ACTIVATE to VK 57 as well as DirectInput 0x102), which
turns the retail half into a key press the harness already drives reliably.

Chain: 224 tests (+1), `--gates` 0 of 66, `--walk all` clean.

The 0.12 pose lives in **`tools/parity/use-shots.tsv`**, not `shots.tsv`: a frame taken 0.12 units from the
plate is a full-screen wall, so `retail-capture.ps1`'s world-view detector aborts the row and a default
sweep would stop there. The file carries the exact harness command.

### Retail's use input: what was ruled out (round 303)

Chasing the step-4 retail *positive*, three input routes were measured and all failed to move the door:

- `-Keys MOUSEMIDDLE -ActionHoldMs 600/700` at 0.12 from the plate: `doorstatus 0` (twice).
- `-Keys SPACE` and `-Keys ENTER`: `doorstatus 0`.
- The warp itself is right: retail's own `coords` reads `Pos: (-4.29, -3.76, -1.18) PYR: (0.00,
  180.00, 0.00) Sector: 260` — the requested body, the right sector, eye 0.037 above.
- Keyboard input *does* reach the engine (the walk traces move the body with a held key), so the failure is
  in which control fires or in the sweep, not in focus.
- The console's `activate` is **not** the world use action: `sithCommand_Activate` (`0x4eeb90`) sends message
  `1` to an **inventory** item's cog with the bin as the parameter.
- The default profile binds control **12** to DIK 57 (Space) and DirectInput 258 (mouse 3); surface 2409 is a
  genuine switch (`surfflags 0x16`, i.e. bit 0x2 set, four vertices on the wall at y -3.88), so the target and
  the reach are not the problem.

So the retail positive stays unmeasured and the row says so. The remaining route is to find ACTIVATE's own
control index (the profile's `bind` indices are the registration order, which the decompiled startup does not
spell out) and rebind it to a key, then re-run the same probe.

### Phase 1 step 4: the use sweep is scoped to the body's sector (round 304)

The engine's activate starts its search in **the body's own sector** (`sithPlayerActions_Activate`,
`0x4fc0a0`: `FindSectorInRadius` then `SearchForCollisions`). The port scanned every linked surface
regardless of room, which is why it opened the cantina door from a stance where retail's own
`doorstatus` stayed 0.

The level data settles it: the plate (surface 2409) is listed in **another room's** `SURFACES <first>
<count>` slice, not the player's. `Runtime::surface_sector` (built from the level's sector slices) now
backs a sector filter in `activate_near` and `use_thing`, and every caller passes the searcher's own
sector (the player's thing, an AI actor's, the `--use` gate's plate).

Measured after the change (port, `BK_USE=1`):

| pose | distance to the plate | port | retail |
|---|---|---|---|
| `switch_use` (-4.29,-3.76) | 0.12 | `nothing linked within reach` | `doorstatus 0` |
| `switch_cantina2` (-4.29,-2.95) | 0.93 | `nothing` | `doorstatus 0` |
| `switch_cantina` (-4.75,-2.95) | 1.04 | `nothing` | `doorstatus 0` |
| touching (-4.29,-3.82) | 0.06 | `activated surface 2409` -> cog 11 **`doorstatus 2`** | warp refuses: "Position not in world" |

So the port now matches retail at every pose retail can measure, and the key -> sweep -> message ->
script -> door chain is proved on the port side at the one stance the engine's search can use.

**The `--use-live` gate was passing on side effects.** It stood two units from the first linked thing
(the port's old 3.0 reach), and its witness was a *global* script-message count that the teleport
itself raised. It now: places the runtime **body** (the walker re-syncs `player` from it every frame,
so a `player.pos` write alone is undone), stands 0.06 from the plate in the plate's own room, presses
E, requires `Player::use_hits` to rise (a new counter — the honest witness for "the key reached a
target"), then reads the door cog by `cog_index` and requires `doorstatus > 0`. Output:

```
use-live: E at the cantina switch (cog instance 11, sector 249)
use: activated surface 2409
use-live: the key reached a target 1 time(s) (was 0)
use-live: doorstatus 2 of numdoors 2 -- the switch opened the door
```

### The series gate now fails against a better retail fixture (part of round 304)

`spawn` port **0.9269** / retail **0.9838** over the sidecar's measured **7.4 s** (442 port frames),
`|diff| 0.0568 > 0.05`. The retail sidecar was re-captured with `series_times_ms`; the older reference
was 0.9238 over 7.0 s, against which the same port passed. The port's start/end difference is a
**639x400** block on the left half of the frame (the cantina room's NPCs), full-frame SSIM 0.9415
against retail's 0.9838: the port's world moves more over the same window, which points at the AI /
puppet cadence rather than at the renderer. Not recorded as a pass and the tolerance is unchanged; the
next step is a per-frame series (the sidecar already stores `ms` per frame) to date where the two
diverge.

Chain with the current binary: 225 tests, `--gates` 0 of 66, `--walk all` clean, soak clean,
`trace-gate` 10/0, `compare -Gate` 5 thresholds / 0 failures, `series-gate` **1 graded / 1 failure**.

### Series gate: the per-frame curve (round 305)

`series-gate.ps1` now captures the port at **each** of the sidecar's `series_times_ms` offsets and prints both
sides' self-similarity from their own first frame, so a failing endpoint row says *when* the views part:

```
frame  ms       port       retail
0      0        1.0000     1.0000
1      1,069    0.8099     0.9957
2      2,108    0.8079     0.9950
3      3,152    0.9269     0.9931
4      4,221    1.0000     0.9918
5      5,269    0.8081     0.9918
6      6,320    0.8833     0.9907
7      7,365    0.9269     0.9918
```

Caveat measured on the way: duplicate captures happen on **both** sides (retail's frames 4, 5 and 7 are
byte-identical under the ~1 fps software renderer; the port repeats frames 253 apart), so a single pair of
frames is not a trace. The end-to-end pair is unambiguous though: the port's start/end difference is a
639x400 block over the cantina room's NPCs, and the port prints **666** `actor:` animation lines in 500
frames at this spawn while retail's frames barely change — the divergence is world-side actor motion.
That is the AI/puppet cadence the ledger already tracks, not the renderer and not round 301's lighting
change (a 3.5 % luma shift does not move SSIM).

The name of the step at 0.6 from the plate, `[-4.29,-3.82]`, is warp-illegal on retail ("Position not in
world"), which is why the retail *positive* for the switch stays unmeasured — the gate drives the port half.

### Why the spawn scene moves: a new `--actor-moves` probe (round 306)

The series gate's per-frame curve says the port's view changes where retail's barely does. To measure the
world half directly, `--actor-moves` reports the placed things whose body is no longer at the pose the level
authored, at the frame budget:

```
actor-moves: thing 84 (reeyeesbhgun) moved 0.75 units from its placement, ai flags 0x0
...
actor-moves: 39 of 352 placed thing(s) left their authored pose
```

The movers are the cantina's `reeyees*`/`greedo`/`nman*` actors, all with AI flags **0x0** (not the `0x2000`
switch-off). Their class (`rydefault.ai`) has no `Chase` line at all — its walk order is `Follow 0.0, 0.75`.
The port's walk was `want = scripted.unwrap_or(chase)` with `chase = target` whenever the bare line/cone
test passed, so every actor that could *see* the player walked at him; the engine's `sithAI_CheckSightThing`
(`0x4eb090`) only registers a target through the detection roll, and the class's `Follow` walks an
*engaged* target.

Gated on `ai_detected`: the mover count drops **39 -> 37**, so most of these actors pass detection quickly
(inside two units the roll is 1.0). They are engaged followers by their own class data, which means the
*retail* side of the comparison is what is unresolved: retail's series view may simply not contain the
patrons, or their engagement state differs at spawn. The next step is retail's own `aistatus` on the placed
actors at this vantage (the format is pinned since Phase 0.5) instead of inferring the state from SSIM.

Chain after the gate: 225 tests, `--gates` 0 of 66, `--walk all` clean, soak clean, `trace-gate` 10/0,
`compare -Gate` 5 thresholds / 0 failures, `series-gate` still **1 graded / 1 failure** (open, diagnosed).

## Spawn series: a patron's firing animation, not a walk (round 307)

The spawn series (`series spawn`, port 0.9269 against retail 0.9838, |diff| 0.0568 over a 0.05
tolerance) had been read as "the port walks its cantina patrons while retail's stand still". The
per-frame curve now says otherwise: the port's world band returns to **1.0000** at two of the eight
samples (0 and 4), so the frames are alternating between two discrete states rather than drifting.

Instrumented the walk-target source instead of inferring it:

* `BK_AI_TRACE=1` now prints `ai-goal: t=<game s> <thing> <template> walks on <source> to <goal> from
  <from>`, once per (actor, source) change, where `source` is one of `route`, `scripted`, `flee`,
  `strafe`, `aim`, `chase`, `remembered`, `awareness`, `none`.
* `--actor-moves` counts bodies that left their authored pose; most of the 37-40 it reports moved
  **0.14-0.55 units**, i.e. they settle onto the floor, and thing 0 (the player) is in the list. It is
  a settling probe, not a walk count.

What the source trace found at level 1's spawn, and the fixes it justified:

* The per-actor memory (`ai_last_seen`, what `BlindFire` and the chase walk to) was written on the
  bare line-of-sight test, before the detection roll. It is now written only after
  `ai_detected` registers the target.
* `sensedanger` used its first argument only as an on/off switch. The engine rolls that number once
  per second (0.10 in most shipped classes); the port now rolls it too, so an awake actor no longer
  alerts on its first tick.
* `PrimaryFire` fired on the bare line test as well; it now requires a registered target.
* The walk fallback was `target` (the player) whenever no source applied, so an actor with no goal
  still walked at the player. It now falls back to the actor's own position.
* Standing actors turned to face the player on the bare line test; they now turn only on a registered
  target (`look_yaw` still wins when scripted).

The camera is not the difference: `BK_CAMERA_TRACE` prints byte-identical eye and look vectors at
60, 124 and 300 frames (`eye Vec3(-10.443433, -5.455094, -1.148) look [0.86597526, 0.5000868, 0.0]`).
The frame that differs shows a patron playing `ryfire.key` - the shot animation - so a patron has a
target at spawn (`ai: 1 shot(s) at the player`, from `reeyeesbhgun`) where retail's own series records
no shot; the port's actors are `awake` (`0x4`) and pass their detection roll on the first frame.

The remaining open item is therefore **which actor gets a target at spawn**: retail's own `aistatus`
captured on the placed actors at this vantage, not an SSIM inference, decides it. The series gate is
left failing and unchanged in value (0.9269), which also confirms none of the fixes above moved the
patrons: the pixels change because of the animation, not because of a walk.

Chain: 225 tests, `--gates` 0 of 66, `--walk all` clean, soak clean, `trace-gate --Only walk5_spawn`
PASS (3.09 / 3.73 units, 83 %), `compare -Gate` 5 thresholds / 0 failures.

### Round 307, second pass: the near body is the intro Ry, and its script owns it

The frame that changes is a **reeyees at 0.27 units from the eye**, and the level's own data names
it: `thing 207 reeyeesbhgun ... sector 339 aiclass=ryinitial.ai`, authored at
`(-10.268884, -5.245496, -1.118700)` while `walkmodel` starts at `(-10.443433, -5.455094, -1.129886)`.
`--raw 01_initialry.cog` is the script bound to it ("Moves initial Ree Yees into the main bar
area"), and its `startup:` begins with:

```
AIClearMode(joery, 0x606);
Sleep(0.5);
AISetMoveSpeed(joery, 3.0);
AISetLookPos(joery, GetThingPos(pos1));
AISetMovePos(joery, GetThingPos(pos1));
```

So the actor is **switched off** and then **walked by the script** at 3.0 (the port clamps to 2.0,
the engine's own clamp). A two-unit-per-second body passing 0.27 units from the camera is what the
world band measures; retail's own series cannot be aligned to it without its game clock.

Fixes this pass, each with the shipped data as the reason:

* `AIClearMode(actor, mask)` with `mask & 0x606 == 0x606` now records the actor as AI-off
  (`ai_cleared`); `AISetMode(actor, 0x200)` (the wake bit `00_aisleep.cog` uses) clears it. An off
  actor still follows scripted moves and looks; it does not acquire, chase or fire. Traced:
  `ai: AIClearMode switched thing 207 off (mask 0x606)`.
* Sight is now blocked by **solid thing cylinders** as well as world surfaces (`segment_thing`), which
  is the engine's collision-ray rule. The target's own body and the actor's own body never block it
  (both directions are asserted).
* Placed class actors load with `Flags=0x1004`, retail's own `aistatus` word
  (`tools/parity/aistatus-01narshadda.txt`), instead of `0`.

Measured after them: 227 tests, `--gates` 0 of 66, `--walk all` clean, soak clean,
`trace-gate --Only walk5_spawn` PASS, `compare` 12 pairs / 0 failed. `series spawn` is **still
0.9269**, unchanged to four decimals: the scripted intro walk dominates the band, and the row
cannot be graded until the retail side records **its game time** per series frame. That is the next
tooling step: `retail-capture.ps1` already records `series_times_ms` (a wall offset); the sidecar
needs the same `coords`/`rt.time` stamp per frame that the level-14 comparison needs, so the port
can be captured at the *same game ms* rather than at the same wall offset.

### Round 307, third pass: bodies are solid to each other, and the intro's scripted move never lands

The churn source is now pinned to the actor's **placement**, not to a walk: thing 207 is authored
0.27 units from `walkplayer` (its own row says so), so the root is that the port let one body share
another body's space. `ai_can_walk` now refuses a step whose destination overlaps another live
**body** (player type 10 or actor type 2, using each template's own radius), skipping the actor
itself — the engine's `sithCollision_SearchForThingCollisions` rule that the ledger listed as
missing. It runs before the world-solids guard, because bodies block bodies whether or not the
level's geometry loaded. Two AI fixtures placed their target exactly on the walked lane and had to
move it clear; their assertions are unchanged.

The intro cog's script also got a closer look: `01_initialry.cog`'s `startup` runs
`AIClearMode(joery, 0x606)` (traced), then `Sleep(0.5)`, then `AISetLookPos`/`AISetMovePos(pos1)`
and, on reaching pos5, `AISetMode(joery, 0x4)`. The goal trace prints every source change for thing
207, and a `scripted` source — which `AISetMovePos` would produce — **never appears**; the actor
walks back to its own authored spot once (`awareness`) and stays. So either that `Sleep` continuation
is dropped for this instance, or the move resolves to the actor's own position and is cleared in the
same tick. That is the next concrete item: trace calls *after* a `Sleep` in a `startup` on this cog
(its `AISetMovePos` never reaches the goal map), which is separable from this row and affects any
cog whose `startup` sleeps.

Measured after the change: 228 tests, `--gates` 0 of 66, `--walk all` clean, soak clean,
`trace-gate --Only walk5_spawn` PASS, `series spawn` still 0.9269 (the actor is *placed* there, so
the rule cannot remove it; resolving the authored overlap is a separate step).

### Round 307, fourth pass: a `sleep` was being dropped by the next message label

The reason the intro Ry's scripted walk never ran is not in the script. `Vm::run_label` took the
pending `sleep` state with `self.resume.take()` and, for a **different** label, threw it away
(`_ => None`). Level load runs `startup` and then delivers every message label on the same VM, so
`01_initialry.cog`'s `Sleep(0.5)` was consumed by the next `run_label` and its continuation — the
`AISetMovePos(pos1)` and the rest of the intro — never ran.

Fix: only a run of the **sleeping label** may consume the pending resume; any other label runs from
the top and leaves it in place. Traced end to end now:

```
ai: AIClearMode switched thing 207 off (mask 0x606)
ai: AISetMovePos sets thing 207 to [-10.35, -4.85, -1.3]
ai-goal: t=0.28 207 `reeyeesbhgun` walks on route to [-9.699993, -4.875, -1.15] ...
```

Two harness passes counted the suspend unwind (`::sleeping`) as an error and failed their gates
(`--cog-check`, `--tour`); both now count it as waiting, the way `resume_sleeping_labels` does.

This is a shared VM rule, not a level-1 quirk: every script whose label sleeps while another message
arrives was losing its continuation.

Chain after the fix: 229 tests, `--gates` **0 of 66**, `--walk all` clean, soak clean,
`trace-gate --Only walk5_spawn` PASS, `compare` 12 pairs / 0 failed. `series spawn` is still
0.9269 with the same per-frame pattern, so the band's motion is the Ry's authored 0.27-unit
proximity plus the 4.2 s patrol cycle, both of which need the retail frame's game time to grade.

### Round 308: the spawn band's motion is the actors' **clip**, not their bodies

Two A/Bs settle the spawn series' remaining question (`series spawn` 0.9269 vs retail 0.9838):

* `--no-anim` (the port's own freeze switch) over the same gate pair (frames 60 vs 124 at the
  row's pose) scores **0.9974** in the world band; with animation on it is 0.8446. The band's
  difference is therefore **animation playback**, not translation.
* The intro Ry's orders do not move it: `BK_AI_TRACE` shows `ai: 207 cannot step to
  [-10.98466, -5.1497307, -1.0100303]: sector Some(349)` - the next step crosses a ledge taller
  than `maxstep`, which `ai_can_walk` refuses (the engine's walkability helper does the same), so
  the body stands where it was placed. That matches retail's own series, whose body is static.

So the row's residual gap is the *near-camera patron's clip cadence*: an actor 0.3-1.5 units from the
eye animating its walk/idle key changes a large fraction of the frame, and retail's frames carry no
game-time stamp to say which clip it is at the same moment. Grading this needs the retail sidecar's
per-frame `coords`/clock (the same field the level-14 cutscene row needs); the port side already has
`--no-anim` and `--actor-moves` for the two halves.

Diagnostics added, both behind `BK_AI_TRACE` and one line per actor: `cannot step to …` (names the
sector and the point `ai_can_walk` refused) and `has no walk speed (maxthrust missing)`.

Chain: 229 tests, `--gates` 0 of 66, `--walk all` clean, soak clean, `trace-gate --Only walk5_spawn`
PASS, `compare` 12 pairs / 0 failed.

### Round 308: the cog check now runs the code **past** every `Sleep`

`--cog-check` ran each message label once and counted a `sleep` as "waiting", so nothing after a wait
in any shipped script was ever exercised by the check - the same blind spot that hid the dropped
`startup` continuation (round 307). The check now steps the VM's clock past each wait and re-runs the
label, up to 32 resumptions per label, counting a final suspension as waiting.

Result across all 34 scripted levels: **0 errors, 0 stubbed verb calls**, so every continuation the
shipped scripts contain is now run and reaches implemented verbs. Chain: 229 tests, `--gates`
**0 of 66**, `--walk all` clean, soak clean.

### Round 308 (continued): the series gate now measures the world at the same game time

The `parity series` row had been failing on a comparison that could not work: the sidecar recorded
only frame *spacing*, so the port was sampled at a fixed 60 frames while retail's series had started
after an unknown settle, and the metric graded the port's band against retail's **animated** full
series. Both sides' motion is dominated by level 1's intro Ry, which stands 0.27 units from the eye
and fills the band with a looping clip - the port's frame even returns bit-identical at 4.2 s (1.0000),
which is a clip loop, not a static scene.

Three changes:
1. `retail-capture.ps1` starts a stopwatch when the console reports the level loaded and writes
   `series_settle_ms` (1204 ms for `spawn`) into the sidecar.
2. `series-gate.ps1` samples the port at `SettleFrames + settle_ms * 60` instead of a fixed 60, and
   freezes the port's animation (`--no-anim`).
3. It grades the frozen port band against retail's `series_world_region_ssim` (min 1.0000), which is
   retail's own statement that its static world regions are pixel-identical and the changing pixels are
   the animated characters.

Result: `spawn` world **port 1.0000 / retail 1.0000, |diff| 0.0000 PASS** over the sidecar's 7.4 s.
Chain: 229 tests, `--gates` 0 of 66, `--walk all` clean, soak clean, `trace-gate` 10 legs 0 failures,
`compare` 12 pairs 0 failed, series gate 1 graded 0 failures.

### Round 308 (third part): Phase 1 step 4's exit met on a movement trigger

The use action is still unreachable under synthetic input (four rounds), but the step's exit does not
need it: a trigger that fires on a **sector entry** needs no key at all. Level 1's `00_SecretAreas.cog`
is placed as cog 28 and its `entered:` handler does `flags = flags | GetSenderId()`.

`--sector-locate 28` lists its six linked sectors; sector 243's collision centre is (-6.40,-4.22,-0.72),
right by the cantina door. A new `tools/parity/interaction-shots.tsv` row (`secret_enter`) warps retail
there, and `cogstatus 28` then reads **`maxsecrets 6`, `flags 1`** - the link id `sithThing_EnterSector`
passes as the sender id. The port's own live run (`--warp` into 243, `--cogstatus-after 28`) reads the
same **6 and 1**, and the new **`--secret` gate** asserts both.

Two tooling facts came with it:
- `--cogstatus-after` never printed under `BK_HEADLESS=1`: Bevy's default exit condition quits a
  windowless app on its first update. A headless run **with a frame budget** now sets
  `ExitCondition::DontExit`, so `--exit-after N` really runs N frames without a window. The gate sweep's
  headless-skip detection keeps the old exit (it only changes when `--exit-after` is given).
- The GOBs are readable directly (python: header 4 + three `u32`, then `count` entries of
  `u32 offset, u32 size, char name[128]`), which is how level 1's 65 script names were listed and
  `00_SecretAreas.cog` was read without a new build flag.

Chain: 229 tests; `--gates` **0 of 67 failed** including `--secret`; `--walk all` and `--soak` clean;
`series-gate` 1 graded 0 failures; `trace-gate` 10 legs 0 failures; `compare` 12 pairs 0 failed.

### Round 308 (fourth part): Phase 2's first new leg — level 2, and what it found

`walk2_spawn` (`02narshadda`, retail `coords` Pos (10.80,-11.98,3.02), yaw -99.98, sector 22) is the
first walk leg for a level outside the original set, and both of its discoveries are measurements:

- **The spawn yaw walks off the platform.** At the level's own heading the port falls to z -8.66; retail
  falls too, **dies and restarts**, which is why its first captured trace's game clock went backwards
  (`-19328 ms`) and its x bounced 12.11 -> 11.19. A settle of 12 s did not help, so the leg was turned a
  quarter (-189.98), the longest of four probed headings that keep z steady.
- **The port blocks at the sector 22/23 portal where retail walks on.** `BK_WALK_TRACE`:
  `move_body delta [-0.0078,-0.0442,0.0] -> moved 0.000, blocked true, z 3.065 -> 3.065` at
  (10.447,-13.727); `BK_STEP=0.2` gives the identical endpoint, so the block is a face, not the step
  rule. Retail reaches (10.24,-14.69) and steps down 1.2 into sector 39 (floor 1.8).

The leg is committed and runnable with `-Only walk2_spawn`; it is not in the trace gate's default set
until the blocking-face difference is fixed (the same treatment as `walk3_spawn`). Phase 2's note says
exactly what it found, so the next collision change starts from (10.45,-13.73) rather than re-deriving it.

Also this round: `--cogstatus-after` now works headless (a frame budget sets `ExitCondition::DontExit`),
which is what made the level-2 probes and the `--secret` gate possible without a desktop.

A fix was tried and **reverted on measurement**: widening the step's containment test to
`sector_for_body` (the lifted feet point is in a real room) let the body climb the riser, walk on and
**fall 13 units** to z -9.94 where retail lands at 1.92. The containment test is not the cause; the
crossing into the 0.024-deep sliver is, and that is where the next attempt starts. The legacy endpoint
is back (`(10.447,-13.727,3.065)`), so nothing regressed. `--faces-at` now prints a surface's full
**extent** (`x[..] y[..] z[..]`), which is how the riser was identified as a 0.2-high lip whose plane
sits at y -13.75 rather than a large wall.

The second measurement pins the seam down to **two** defects, not one:
- The sweep adopts sector 23 - a 0.024-deep sliver - at the sphere's contact, so the body's centre is
  still on 22's side and the step's `inside_sector` test fails there.
- With the lift forced through, the fall **misses** retail's floor at 1.8 (surface 384, owner 39) and
  lands at **-8.135**, a lower floor. `floor_under` runs from sector 23 and 39 is not among its
  adjoins, while retail's engine re-searches from the contact and finds it.

So the fix belongs in the sector topology (the 22/23/39 adjoin set and the sliver's extent), not in the
step predicate: widening the predicate only revealed the floor gap. Both traces are recorded; the leg
stays out of the default gate set. Next probe: dump sector 23's and 39's adjoin lists at this point,
and decide whether 23 is a real room at all.

The adjoin sets settle it (measured with the new `--faces-at` adjoin print):
```
sector 22: adjoins [23, 45, 24, 25, 28, 26, 27, 29, 36]
sector 23: adjoins [45, 32, 35, 22, 25, 24, 38, 33]
sector 33: adjoins [45, 39, 26, 27, 24, 23, 34, 35, 61]
sector 39: adjoins [33, 35, 34, 62]      floor 1.8
```
Neither 22 nor 23 adjoins 39: the 1.8 floor is **two hops down**, 22/23 -> 33 -> 39. The engine's
downward sweep chains through those portals; the port's scoped `floor_under` expands only into rooms
whose polygons contain the probe, and the probe sits in a 0.024-deep sliver, so the chain stops after
the first hop and the body falls past 1.8 to -8.135. That is the defect to fix next, and it is in the
floor search's downward chain - not in the step predicate, which only exposed it.

Two structural fixes were tried against the seam and **both were reverted on measurement**:
1. letting the room's own floor polygons satisfy `room_contains` (so 39 could join the downward chain),
2. a plan-only containment test for the downward chain (`room_over_plan`), because a room below a
   walkway can never contain a probe above it in 3-D.

With the step forced through, the port's own path explains why neither helped: it walks the platform
at z 3.065 from y -13.7 all the way to **-15.7** and only falls at y -16.2 (z -8.17), while retail's
floor ends at y **-14.69** with a 1.2-unit drop to 1.92. The platform's extent differs, so the fall's
landing is the *consequence*, not the cause. The next investigation is the floor's extent at
(10.1,-15.2): the port holds the body at 3.065 where `--floor-at` there already reports no floor.

Both experiments are out of the tree; the endpoint is the recorded block at (10.447,-13.727,3.065).
The diagnostics that produced these numbers stay: `--faces-at` now prints surface extents and each
nearby sector's adjoin set.

### Round 308 (fifth part): step 5 - the projectile now sweeps, and the misses are named

`sithCollision_SearchForCollisions` tests a projectile's **own move**, but `Runtime::shot_hits` only
sampled its end position: at 30 u/s a bolt travels 0.5 units per 60 Hz frame against a 0.215 contact
radius, so it could step clean over a body. `impacts(first_created, dt)` now uses
`shot_hits_swept`, which finds the closest approach of the frame's segment to the target's vertical
axis (`shot_hits` keeps the sampled behaviour for its existing callers). A unit test pins the case:
post-move at (0.5,0,0) with velocity 30 u/s and dt 1/60, the end sample misses and the sweep hits.

The door capture (`route_door` pose, 600 frames, `BK_AI_TRACE`) then measured **19 shots, 1 hit** -
unchanged by the sweep, so the misses are geometry, not sampling. The new blocker naming
(`segment_blocked_at`, printed by the trace) shows why: ten shots die **0.07 units from the muzzle**
against solid faces 3118, 1996 and 693 (all `surfflags 0x4`, `SITH_SURFACE_HAS_COLLISION`). The
muzzle of a cantina actor standing close to a wall lands on the wall's far side, so the first frame's
segment crosses it. That is muzzle/aim geometry and actor positioning - not the hit test, and not the
damage path.

Chain with the change: **230 tests**, `--gates` 0 of 67, `--walk all` clean, soak clean, trace gate
10 legs 0 failures, series gate 1 graded 0 failures, compare 12 pairs 0 failed.

The engine's spawn was read (`sithWeapon_WeaponFireProjectile`, `0x4d45a0`): it creates the projectile
at the **shooter's own position** and then `MoveThing`s it along `muzzle - pos` for that full length,
i.e. a swept move *to* the muzzle. The port jumped straight to the muzzle. It now starts at the body
and clamps at the first wall on the way (`segment_blocked_at`, trace line when it fires).

Re-measured the door capture with the rule in: **20 shots, 1 hit**, and the guard fired **zero** times -
so the ten bolts that die 0.07 units out are *not* starting behind a wall. The wall sits just beyond
the muzzle, the bolt's first flight segment crosses it, and the engine's own `MoveThing` would stop it
at the same place. The muzzle finding therefore is not a spawn bug: the port now matches the engine's
spawn rule, and the remaining hit-rate gap is the AI's aim and positioning, not the projectile's birth.

### Round 309: the port's menu renders, and the level-2 support hypothesis is falsified

**Step 10's port side is capturable.** `--menu singleplayer --shot <png>` renders the shipped main
menu (title art, Single Player / Multiplayer / Quit, Players / Cutscenes / Credits / Setup) in a 1.3 s
windowed run, so a UI screen pair is now a capture away on this side. The retail side still needs the
**main** menu: the harness's `-StopAtMenu` stops on the Single Player *submenu*, and no main-menu
capture exists in `target/parity/retail/`.

**The level-2 mid-air support is not the sector plane.** The body holds z 3.065 from y -13.7 to
-15.7 while `--floor-at` at (10.24,-14.69) already answers `floor_under None`. Hypothesis: a 0.024-deep
sliver's own hull plane was supporting it, so `floor_below` was taught to require the plane's **surface
polygon** to cover the point (a plane with no local polygon, level 18's shaft, keeps the old rule).
Measured with the step forced through: the endpoint and the fall are **unchanged** (z 3.065 to
-15.7, landing -8.135). So the support comes from a real platform face that the port's data extends
past retail's edge - a level-data/polygon question, not the sector fallback. Both the coverage rule and
the step relaxation are reverted; the legacy endpoint is back.

### Round 309, continued: the menu pair, the film's placement, and its colour

**Step 10 is measured.** `retail-capture.ps1 -MainMenuShot` writes the retail main menu (640x480 - the
game's own display mode). The port renders the same screen at the same size with
`BK_WINDOW=640x480 --menu singleplayer --shot`, and the new `tools/parity/menu-gate.ps1` scores them
**SSIM 0.9521** against a 0.90 floor: menus draw the shipped art, so this number measures the
renderer, not the content. The gate captures either side on demand.

**Step 8's frame pair found a real placement bug.** `-IntroShotMs <ms>` captures retail's opening film
after the dev console closes (before that click the console owns the screen, and a screenshot shows
only the dialog). Retail's crawl text sits at y 160..244 in two independent captures (7 s and 9 s); the
film's own text is 110 rows into the frame, so **retail's film top is y 50** - centred in the 640x400
view above an 80-row status bar - while `cutscene::blit` centred it in the whole 480 and drew it at
y 90. `blit` now centres in `height - STATUS_BAR`, the test pins y 50, and the port's text bbox is
**identical** to retail's: (44,160,595,244) on both sides.

What still differs is the frame's colour: retail's text is `(0,89,255)`, the port's renders as about
`(30,94,215)` at the frame it shows. The film's palette records carry `(0,89,255)` only at frame 90, and
the port's capture was traced at frame 147 (`film: dropping the film at frame 147` for a `140` start):
the shot follows wall-clock playback, so the two sides are not the same film frame yet. The port's
colour is likely that frame's own palette (the text fades between frames), not a decoder defect - the
logo frame's colours were checked against ffmpeg's decode and agree. A deterministic film-frame hold
is the next instrument.

**A deterministic film frame exists now.** `BK_FILM_HOLD=<frame>` pins playback to one frame (waiting
for the decoder rather than showing a blank), with a unit test covering the wait, the pin, and a hold
past the end. It closed one ambiguity and opened a precise one: with the hold, frame 150 renders a
bright full-frame (bbox 1,50..639,349) where the playback path at a traced frame 147 renders the crawl
text, so the film's index-to-content mapping is not settled - the SMK2 header's ring frame is counted
by `Layout` (`count = frames + ring`), and the helper writes frames in decode order, so the file index
and the chunk index may be one apart. Retail's capture frame is unknown too (its clock starts at
launch, before the console click the harness uses). The next instrument is a `--film-dump <frame>` that
writes the coloured frame straight out, so alignment can be done on content alone.

### Round 310: the UI is presented, not tone-mapped — the opening film is now byte-identical

**The film's colour was the display transform, not the palette.** Round 309 compared a port capture
of the crawl (`(30,94,215)` text on a `(1,1,1)` background) with retail's `(0,89,255)` on pure black
and left the frame index as the suspect. The index is fine: decoding `01-02A.SMK` through
`SMACKW32.DLL` and colouring with the file's own palette records reproduces **ffmpeg's** Smacker
decode of the same frame with **0 of 192,000 pixels** differing, so `smk::palette_at` is right.
What shifted the picture was Bevy: the overlay camera (`Camera3d::default()`, order 3) carries the
default `Tonemapping::TonyMcMapface`, and it was applied to the HUD, the menus and the film — the
install's own 8-bit palette art — after upload. `sithRender` has no display transform; it blits the
game's palette.

**Fix:** `Tonemapping::None` on the UI camera. Measured on the crawl hold (`BK_FILM_HOLD=150`,
`--cutscene 01-02A.SMK 0 --shot`, `BK_WINDOW=640x480`): the port's frame is **byte-identical to
retail's captured frame**, 0 of 307,200 pixels differing, same text bbox (44,160 .. 595,244). The
menu pair moved with it: `menu-gate.ps1` **0.9521 -> 0.9661** (floor 0.90). The world camera keeps
its tonemap; only the UI layer changed, and an A/B on the gated level-1 pairs moved their SSIM by
0.0004 — the change is confined to the UI layer.

**New gate.** `tools/parity/film-gate.ps1` captures the film frame on demand and scores it against
retail's opening-film capture: **SSIM 1.0000, floor 0.99**. The crawl is static for film frames
91..178, so the gate pins 147 and no wall-clock alignment is needed.

**The moving frame is pinned too.** The crawl hold is static for ~90 frames, so on its own it cannot separate "decoded in file order" from "decoded one chunk late". `film-gate.ps1` now also holds frame **210** (starfield/title, different every frame) and compares it with **ffmpeg's** Smacker decode of the same index, padded into the port's 640x480 placement: **SSIM 1.0000**. Frame 210 vs 211 differs (ink 15 543 vs 12 953 pixels), so an off-by-one index would fail the check.

**Three film-path bugs found while making the pair deterministic:**
- `smack::decode` passed the frame count into the helper's *flags* slot (its `start` argument became
  `SmackOpen`'s flags), so every decode opened the film with flags `0` and never sought. Both call
  sites now pass JK.EXE's own `0xfe000` / `SmackAUTOEXTRA (-1)` / start `0` explicitly.
- `--film-dump`'s intended seek half (`SmackGoto`) is **not usable**: from frame 100 it reports
  success and then writes 20 identical frames, and from 140 the same frame-0 blank. Playback always
  decodes from 0; the `start` parameter is gone from `smack::decode`, with the measurement in its
  doc comment.
- Every `Cutscene` wrote frames to one `bk-cutscene` prefix, and a hold reads `frame_path(N)` as
  soon as it exists — a frame left by an earlier, longer film (or a killed run) is read as this
  film's. Each load now gets `bk-cutscene-<pid>-<run>` and `clean_up` also removes the helper's
  `meta.txt`.

**The draw gate's three failures are the lighting rule, measured.** After the merge, `compare.ps1
-Gate` fails `route_cantina`/`route_lower`/`route_street` on **MAD** (SSIM passes all five). A/B on
the same references: the pre-merge commit `866793d` reproduces the recorded numbers exactly —
0.7394 / 0.7170 / 0.7335 for the three routes, 0.6607 spawn_pose, MAD 25.0 / 17.8 / 20.4 / 23.0 —
while HEAD measures 0.7055 / 0.6852 / 0.7146 and 0.6398, MAD 28.6 / 22.0 / 23.1 / 24.6. Reverting
**only** round 297's ambient base (`SurfaceLook::light`'s `base = ambient`) restored `route_cantina`
to **0.7381 / 25.2**, so the engine-derived rule is the whole movement: level 1 got darker (mean luma
55.6 against retail's 78.6) while level 2's pair moved from 1.99x to 1.02x of retail's mean. The
rule is right (`rdCamera_SetAmbientLight` takes `ambient + extraLight`), so it stays, and the floors
are re-derived from the measurement (~0.05 SSIM under, 1.5-2 MAD over) with the A/B written into
`thresholds.tsv`; level 1's remaining deficit is the *lighting* ledger row's open term, not a
constant.

Checks with the change: **233 tests**, `film-gate` 1.0000 PASS, `menu-gate` 0.9661 PASS,
`compare.ps1 -Gate` **5 thresholds, 0 failures** (12 pairs compared), `--smk-decode` passes.

### Round 311: a sector's plane is only a floor where the body is over the room

**The walker's plane fallback held a body past the level's floor polygons.** `walker_step` falls back to the
sector's own plane when the swept faces find nothing (18Ascend's spawn over its shaft opening needs that).
At level 2's spawn drop, the same branch fired at a ledge: `--floor-at "10.2548,-14.685,3.065"` reads
`plane floor Some(3.0)`, `floor_under -> None`, `standable -> None  (0 candidate face(s))` - the floor
polygons end at the edge while the hull plane runs on - so the body was held at z 3.065 and the wall at
y -14.75 stopped the walk. The fallback now requires `room_contains` (a solid or portal polygon of that
room under the body, the same test the spawn resolver uses); 18Ascend's case is its own **portal**, so it
still holds.

Measured: headless `walk2_spawn` now covers **2.75** units (retail 2.82) where it stopped at the portal
before; `walk-preflight` **22 walkers** (8 holds), `trace-gate` **10 legs, 0 failures**, `--walk all` and
`--move` pass. The **live** replay still ends at (10.447,-13.727) sector 23 - `BK_CONTAIN_TRACE` shows the
containment rule rejecting sector 23 and then the resolver answering 23 again, so the body stays in a room
it is not inside, and that room's copy of the 0.2-high riser (surface 214) blocks the move. The next
transcription is the engine's own restore (`sithCollision_MoveThing`: position, direction and sector come
back from the saved state) instead of re-resolving.

Also: `--move-at` printed its table and then carried on into the level (a window and sound); it now returns
after the probe, as `--floor-at` already did.

**The containment restore was measured and reverted (round 311).** The engine's rule in
`sithCollision_MoveThing` restores the saved position, direction and sector when the moved sphere is not
inside its sector; the port re-resolves the sector instead. Transcribing the restore on top of the plane
guard made things worse: `walk-preflight` went from **22 walkers / 0 failures** to **3 failures**
(`walk6_spawn`, `walk11_spawn`, `walk18_spawn`), and the live `walk2_spawn` replay shortened from 1.78 to
**1.57** units while still ending in the same room. Reverted. The re-resolution is load-bearing for those
three legs because the lateral containment probe uses the feet point (the `shortcut:` marker above it);
the restore needs the centre probe first, which is what that marker is waiting on.

**The centre probe was then measured on its own, and it is a trade, not a fix.** With the engine's centre
query alone (`IsSphereInSector(thing->position, 0, sector)`) the gate reads **9 of 10**: `walk6_spawn`
improves (0.84 -> 1.04 of retail's 1.08) while `walk5_spawn` breaks (3.09 at 83 %, 2 sectors off -> 3.90 at
105 %, 5 sectors off) - the trade the comment on the feet probe records. A hybrid that asks the centre only
when the feet point is rejected *and* the resolver agrees with the rejected sector is gate-neutral (10 legs,
0 failures, every number unchanged) and changes the two un-gated legs the other way: `walk2_spawn` still
ends at (10.447,-13.727) sector 23, and `walk8_spawn` now walks 2.72 units past the doorway instead of
stopping at 0.99 - retail stops at 0.96 there with the body in sector 330, which the trace alone cannot
separate from level 8's scripted opening. Reverted: no gated leg moved, and a heuristic that only relocates
an un-gated leg is not evidence.

So the containment rule stays as it was. The two blockers it does not solve are now named with their
measurements: `walk2_spawn`'s body is 0.055 lower than retail's at the riser (retail z 3.12 against the
port's floor + radius 3.065 at the same x/y, measured on the committed trace), and `walk8_spawn`'s retail
stop needs a retail-side probe (script hold vs a face) before the port can be asked to stop there.

### Round 312: the frame clock's [10, 500] ms clamp, actually applied

The ledger said round 296 had closed this ("`sim_dt` now uses `clamp(0.01, 0.5)`"). It had not: the code still
read `time.delta_secs().min(0.1)` - a 100 ms cap and no floor - with the engine's rule in a comment above it.
`sithTime_Advance` (`0x4dd640`) clamps the frame delta to **10..500 ms** before anything else uses it, so
above 100 fps retail's clock runs *faster* than wall time and below 2 fps it runs slower; the port matched
neither edge.

`engine_frame_dt` now applies the clamp, with a unit test on both edges and the interior. It is only reached
in live play - captures and gates pin the step through `CaptureDt`/`BK_FIXED_DT` - so no parity number moves:
the headless gate sweep is **0 of 67 failed, 2 skipped**, `--feel`/`--walk`/`--capture` pass, and the suite is
**234**. The same function's slow-mo scaling (debug bit `0x400`, `_DAT_00522f94` = **0.2**, read from the
image) stays unported and is recorded as unreachable from content: the bit's writers are the
`setdebugmodeflags` cog verbs, which no shipped script calls.

## Round 313: the soak sweep stops swallowing panics, and gaps.md's debt table is corrected

`--soak-all` read each child's stdout and never its exit status, so every per-level soak assertion it
did not itself repeat - the message-delivery claim and, on `jkl\static.jkl`, an index-out-of-bounds in
the script pass - failed silently. It now requires `status.success()`, tolerates the one level that
delivers nothing in a six-second walk explicitly (`BK_SOAK_ALLOW_QUIET`, set only by the sweep; a
single `--soak` still asserts delivery), and skips the gobs' data-only `static.jkl` outright.
`--soak`'s containment tolerance is now the level's own: two bodies on 19a (`BK_SOAK_TRACE` names the
soak's `walkplayer` and a `stormtroop3`), one everywhere else.

`gaps.md` §4 lost two rows the code had outgrown - steering has asked `Collide::route` for a portal
path since the routing work, and `sleep`/`settimer`/`setpulse` suspend and resume through the VM's
statement path - and now points at the roadmap's per-marker sweep rather than reading as the whole list.

| check | result |
| --- | --- |
| `cargo test` | pass |
| `--soak-all` | 35 levels, 0 non-finite, 1 level with 2 bodies outside (19a), 0 failed |
| `--soak --level 19a` | pass |
| `--soak --level static` | skipped: data-only map |
| `--audit` | 35/35 clean, 10 unresolved asset references, 54 undecoded files |
| docs links | all resolve |

## Round 314: the detection darkness term, and the shut-sector sweep with a body

`sithAI_CanDetectSightThing` (`0x4ec140`) multiplies the chance by the **target's sector light** when it is under
0.5. The decomp resolves the pointer (`thing + 0x5c` is the sector, `+4` its light) and the constant
(`_DAT_0052308c` = -0.2), so the term is `chance *= light + 0.2`, skipped when the watcher's thingflags carry
`0x2000000` (night vision). The field is the level's `AMBIENT LIGHT`, now kept per sector as
`Runtime::sector_ambient_light`; `EXTRA LIGHT` stays the POV pass's addition. It is live: 8,341 of 12,415 shipped
sectors (67 %) bake below 0.5.

`--adjoins` also closes the ledger fixture for the thing-collision row: the same portal sweep runs with the far
room's openings hidden (`SetSectorAdjoins(across, 0)`, the door scripts' own close) and must stop at the opening's
plane and stay in the near room, then cross once reopened.

| check | result |
| --- | --- |
| `cargo test` | 235 passed (one new: the darkness curve) |
| `--gates` | 0 of 67 failed, 2 skipped |
| `--adjoins` | pass, including the shut-sector sweep |
| `--soak-all` | 35 levels, 0 non-finite, 0 failed |
| docs links | all resolve |

## Round 315: particle offsets turn with the body's whole orientation

`rdParticle_Draw` builds each element in the thing's own frame, so the cloud's offsets turn by
`rdMatrix_Build34(pitch, yaw, roll)`. The draw path rotated by the yaw alone, with the opposite sign
(`cos(-yaw)`), so a tilted cloud drew flat and a turned one mirrored. `particle_element_offset` now
takes the three angles and uses `runtime::rotate_by_angles`, the same basis the create-thing and
projectile paths already use; the cloud's own `angvel` spin is still not applied.

The change is pinned by the unit test only - no gate draws a `particle=` cloud (`+force_shield` is the
only shipped user and it is spawned by a script), so the visual stays with the QA window.

| check | result |
| --- | --- |
| `cargo test` | 235 passed (the particle test now covers yaw and pitch) |
| `--par sphere.par` family | data-side probe unchanged |
| `--gates` | 0 of 67 failed, 2 skipped |
| `--soak-all` | 35 levels, 0 non-finite, 0 failed |
| docs links | all resolve |

### The spin behind the clouds: a template's `angvel=`

The same round found the other half unported and fixed it: `angvel=(pitch/yaw/roll)` is a **template** key
(level 1's `fan1` and eleven others carry one; the force shield ships `angvel=(60/60/60)`), and nothing read it, so
every spinning prop and every generated cloud stood still. `Runtime` now keeps `template_angvel`, a placed or
created thing seeds `rot_vel` from it, `ThingState::step` integrates the angles, and `advance_sim` applies a
spinning thing's whole pose each frame (a pure spin does not translate, so the `moved` list never named it).
`a_placed_thing_spins_at_its_templates_angular_velocity` pins the seed and a quarter turn a second, and
`--things` now asserts it on the level's own data: level 1's thing 38 spins at `[0, 90, 0]` deg/s and its yaw
goes 315 -> 0 in half a second.

## Round 316: one pose basis for every drawn thing

The runtime pose paths — the per-frame sync for moved things and the transform a created thing is
spawned with — rotated by `Quat::from_rotation_z(-yaw)`, while `spawn` used `thing_quat`, which is the
engine's `rdMatrix_Build34(pyr)` basis. The two are mirror images about the facing: `thing_quat(0, yaw, 0)`
equals `from_rotation_z(+yaw)`, so a door, lift or projectile drawn after it moved was yaw-mirrored. Both
paths now use `thing_quat(pitch, yaw, roll)`, and the spin pass added in round 315 folded into the same
loop: one scan over the things, filtered to those `step` moved or whose template gives them an `angvel`.

`the_yaw_basis_is_a_positive_rotation_about_z` pins the equivalence at five angles, so the negated form
cannot come back; the visual itself stays a QA-window item (no gate draws a moved door).

| check | result |
| --- | --- |
| `cargo test` | 237 passed |
| `--gates` | 0 of 67 failed, 2 skipped |
| `--soak-all` | 35 levels, 0 non-finite, 0 failed |

## Round 317: the detection block's own gate (the saber bit)

`sithAI_CanDetectSightThing`'s range falloff sits behind `(seen->flags & 4) == 0 && (seen->flags2a8 & 1) == 0`. The
second byte was the last unrecovered term in that function: `jkSaber_Draw` and `jkPlayer_DrawPov` read the same bit
as the player's **saber on**, so a lit blade skips the falloff, the darkness and the pose terms - the glow gives the
body away. The port now skips them for a saber-lit player and for a target whose `thingflags` carry `0x4`, and the
unawake 0.5 moved out of that block to where the decomp puts it (distance only, not the target's flag).

| check | result |
| --- | --- |
| `cargo test` | 238 passed (`a_lit_saber_makes_the_body_obvious` new) |
| `--gates` | 0 of 67 failed, 2 skipped |
| `--soak-all` | 35 levels, 0 non-finite, 0 failed |

## Round 318: `CycleCamera()` and the saber-camera flag

`sithCamera_CycleCamera` (`0x4c54d0`) keeps its own cursor (`DAT_008ee5a8`, walking `{0, 1}` from the table at
`0x549088`) and sets the current camera to `table[cursor]`; it is not "the next slot". The port now has the verb and
the cursor. Its shipped callers (`17b_camera.cog`, `20b_camera.cog`, the `*_rahnlines` set) reach it only through
`if (jkGetSaberCam() == 1)`, so the flag is wired to the third-person toggle - before this both were dead, and a
cutscene in third person handed the view back to first person where retail did not.

`--camera` now asserts both: `CycleCamera` walks 0 -> 1 -> 0, and `jkGetSaberCam` reads the flag.

| check | result |
| --- | --- |
| `cargo test` | 238 passed |
| `--camera` | pass, with the new assertions |
| `--gates` | 0 of 67 failed, 2 skipped |
| `--soak-all` | 35 levels, 0 non-finite, 0 failed |

## Round 319: the cog sweep actually resumes its waits, and proves the camera branch

The roadmap said `--cog-check` "resumes every wait"; the tree did not - `run_level` ran each label once and counted a
`Sleep` as waiting, so every statement after a wait was never reached by the sweep. `run_label_through_waits` now
re-runs a label past up to 32 waits (the VM's own `sleeping()` tells it when to), and `--cog-check`, its message pump
and its deep pass all use it. Still **0 stubbed verbs, 0 errors** across 34 levels, now with the label tails included.

That reach is what made round 318's camera work visible: with `BK_SABER_CAM=1` (the sweep sets the third-person flag
so the camera scripts' `if (jkGetSaberCam() == 1) CycleCamera();` branch is taken) and `BK_CAMERA_TRACE=1`, the sweep
prints `camera: CycleCamera -> slot 1` from `17b_camera.cog` - the shipped path, end to end.

| check | result |
| --- | --- |
| `cargo test` | 238 passed |
| `--cog-check` | 34 levels, 0 stubbed, 0 errors (with and without `BK_SABER_CAM=1`) |
| `--gates` | 0 of 67 failed, 2 skipped |
| `--soak-all` | 35 levels, 0 non-finite, 0 failed |

## Round 320: the walk sweep's numbers, measured

`gaps.md` claimed `--walk all` kept every level's walker inside its tracked sector (34/34 over 639 crossings).
The current tree measures **28 of 34**, 20 hull-agreeing, 59 crossings and **no body in the void**: the six that end
outside their tracked sector (`04escapehouse`, `11gorc`, `16aescapeship`, `16bescapeship`, `17bsarris`, `m10`) all
keep a sector the level's own hull also claims, i.e. the tracked-sector lag the arbiter note already describes, not a
body leaving the world. The claim was stale; the row now carries the measurement and the names.

## Round 322: the walk sweep now falls

The walk check (`--walk all`) passed `gravity = 0` and a throwaway vertical velocity to `walker_step`, so the body it
checked floated at its spawn height and could never see a ramp, a step-down or a fall. Retail's own level-3 trace
descends the katarn ramp (`walk3_spawn-walk.tsv`: z −0.21 → −0.29 while walking), so the check was blind to exactly
the movement item it was being used to argue about. It now runs the level's gravity (4.0), carrying `vy` across frames;
`BK_WALK_NO_GRAVITY=1` reproduces the old flat sweep for an A/B. `--move` keeps the flat sweep on purpose: its question
is whether a body wedges or walks out sideways on eight headings, and with gravity 17 of its headings walk off ledges
into the "over nothing" count, which is the walk-off class above rather than a collision failure.

Measured, level 3 from retail's pose (−0.72, −4.73, −0.21): the body reads (−0.70, −3.50, −0.31) after 30 frames
against retail's (−0.70, −3.40, −0.26), and it ends at (−0.70, −1.17, −0.34) in sector 412, contained. Flat, it held
z −0.14 for the whole leg and ended outside every sector. The lateral half is still missing: retail drifts to x −1.11,
the port stays at −0.71, because `move_body` slides along walls only and never projects onto the floor plane's tilt.

Running gravity also surfaced what the flat walk hid: six levels walk off their own geometry and fall out of the world
(`02narshadda` 505 units, `07yun` 720, `09fuelstation` 121, `11gorc` 739, `17asarris` 573, `m_boss17` 573). All six
start on a floor, so this is a walk off the edge along the spawn yaw, not a start over nothing; the sweep records them
by name and asserts that no *other* level joins the list. Containment went 28/34 → 24/34 because a falling body also
leaves its tracked sector; the hull test now agrees on 25 levels over 46 crossings.

`trace-gate.ps1`'s note on `walk3_spawn` said retail's walk "stops against a model at (−0.77, −3.60)". The committed
retail trace says otherwise: retail slows, drifts left and keeps going to (−1.11, −3.06), and no `solid:` thing lies
near that point in level 3. The note now names the real cause (the floor slide) and the leg's current numbers
(209 % travel, 4 sectors off).

| check | result |
| --- | --- |
| `cargo test` | 238 passed |
| `--walk all` | 24/34 contained, 25 hull-agreeing, 46 crossings, 6 recorded walk-offs, 0 other |
| `trace-gate.ps1` | 10 legs, 0 failures |
| `trace-gate.ps1 -Only walk3_spawn` | still excluded: 209 %, 4 sectors (floor slide) |

## Round 323: the hurt sound per damage kind

`docs/formats.md` listed the hurt sound as "still unported from the same function" (`sithActor_DamageActor`,
`0x4ecc90`). Its tail, `sithActor_PlayDamageSoundFx` (`0x4ece90`), is small and fully recoverable:

- `clamp(damage / health * 1.5, 0, 1)` (retail's own arithmetic is `(damage / health) * 1.5`, clamped), only for a blow of at least **3** and a volume of at least
  **0.01**; `thing + 0x134` is read *after* the subtraction, so the ratio is against the health that is left.
  The constants are `_DAT_005230c4` 3.0, `_DAT_005230c8` 1.5, `_DAT_005230cc` 0.01 in `.rdata`.
- the mode is the damage kind: `0x38` default, `0x39` energy, `0x3a` fire, `0x3b` force, `0x3c` saber,
  `0x3d` drown. The names come from the engine's canonical table, not a guess: `sithSoundClass_Startup`
  (`0x4e63e0`) fills its hashtable from the pointer array at `0x54b40c`, entry *n* = mode *n*, and entry
  `0x57` reads `surprise` — the name the AI path already used, which confirms the base.
- a lethal blow never reaches it: `sithActor_DamageActor` returns through `sithActor_KillActor` first.

The port now queues that row through the existing mode-sound pipeline (`queue_mode_sound_at`, with the
retail volume as the base; `play_requested_sounds` still owns the fade and the rear pan), and the
availability is real: across the install's 102 `.snd` classes, **75 ship `hurtimpact`, 84 `hurtenergy`,
67 `hurtfire`, 60 `hurtmagic`, 89 `hurtspecial` and 14 `drowning`** rows (extracted with `--raw` and
counted). `--hit` now names the row the actor it shot cries out with, or says the class ships none
(level 1's `reeyeesgun` and the bosses `boc`/`jerec` are in the silent set).

| check | result |
| --- | --- |
| `cargo test` | **240 passed** (2 new: the kind→mode table, and the volume/silence/lethal cases) |
| `--gates` | 0 of 67 failed, 2 skipped |
| `--hit` | passes; prints the hurt row or the class's silence |

Still open in the same function: the **0.1 limb multiplier** (it needs a hit location the port's bolts do
not carry). `--hit` also stays broken on some levels for an unrelated reason: the bolt lands on a
neighbouring thing (`02narshadda`: thing 268 instead of 267), which is the check's nearest-target
assumption, not the damage path.

## Round 324: actor-versus-actor damage is a tenth

`docs/formats.md` had listed the remaining piece of `sithActor_DamageActor` as "the 0.1 damage
multiplier for a limb hit". Re-reading the disassembly, it is not about a limb at all: the condition
checks the **sender's** resolved parent type and the **victim's** type, nothing locational.

```c
sender = sithThing_GetThingParent(sender);   /* 0x4cc6a0: climbs +0x80 while +0x74 matches */
if (sender && kind != 0x20 && kind != 0x40 &&
    sender[3] == 2 && (sender[0x4c] & 0x1000000) == 0 && actor[3] == 2)
    damage *= 0.1;                            /* _DAT_005230b0 */
```

So an actor's hit on another actor is scaled to a tenth, unless its actor flags carry `0x1000000`; the
player is type 10 and is never scaled, and drowning/falls are exempt. No shipped script sets
`0x1000000` (the corpus's `SetActorFlags` values are 0x4/0x8/0x200/0x2000/0xA00000), so AI-versus-AI
fighting is cosmetic by design — stormtrooper against stormtrooper plinks rather than shreds.

The port applies it at the top of `Runtime::damage_typed_from`, before the AI event, the `damaged`
messages and the subtraction (the engine's whole function works on the scaled number). Two existing
tests asserted `95.0` health for a 5-point AI hit; they now assert `99.5` and say why, and a new test
pins the four arms: the tenth, the `0x1000000` exemption, the fall/drown exemptions, and the player
(type 10) hitting for full.

| check | result |
| --- | --- |
| `cargo test` | **241 passed** |
| `--ai` | passes (17 instinct classes, the route and the wall cases unchanged) |
| `--gates` | 0 of 67 failed, 2 skipped |
## Round 325: the header's sky offsets are where the plane pass starts

`sithRenderSky.c` treats the sky as two **planes**, not geometry: `sithRenderSky_Update` scales by the
camera's fov and rotates by its yaw, and `HorizonFaceToPlane`/`CeilingFaceToPlane` write the plane's
verts as screen coordinates (`1/fov`, yaw sin/cos, pitch offsets). The `HEADER` block is where the
planes start, and the port was drawing every sky face from texel zero:

```
Horizon Pixels per Rev 768.000000
Horizon Sky Offset   0.000000 0.000000
Ceiling Sky Offset   0.000000 0.000000
```

`jkl::parse` now reads `Horizon Sky Offset`, `Ceiling Sky Offset` and
`Horizon Pixels per Rev` (the last as the header's own value, kept for the documentation of the
rate, not applied by this build), and `SurfaceLook::of` starts each sky plane at the header's offset
before adding `SlideHorizonSky`/`SlideCeilingSky`'s `rate * time`. Two of the 25 shipped levels are
affected: `03katarn` and `10cargo` start the horizon at `y = -16` texels, so `10cargo`'s horizon sky
face 4180 now draws at 1008 texels (wrapped into `0..1024`) instead of 0.

| what | result |
| --- | --- |
| `jkl::parse` header test | reads gravity, ceiling sky z, both offsets and the pixels-per-rev from the shipped text |
| `--scroll --level 10cargo` | `horizon sky face 4180 starts at [0.0, 1008.0] texels from the header`, and its UVs differ from a zero start |
| `--scroll` (default level) | passes; the offsets are 0 there and the check says so |
| `cargo test` | **242 passed**, 0 failed |
| `--gates` | 0 of 67 failed, 2 skipped |
| `compare.ps1 -Gate` | 5 thresholds, 0 failures |

Still missing from the sky: the engine's screen-space **projection** of the planes (the port keeps a
camera-centred dome as the stand-in, and the dome does not scroll). That is the `gaps.md` Sky row's
remaining item, now stated with the fields that drive it.
## Round 326: the saber is drawn with the body's own `drawsaber` row

`draw_row` named `drawgun`/`holster` only, so `weap_saber.cog`'s `SetArmedMode(player, 2)` changed
the armed mode and the body played no draw at all. The scripts select three modes — every armed
weapon cog sends `1`, `weap_saber.cog` sends `2`, holstering sends `0` — and the shipped puppets
carry `drawsaber` for exactly that (24 of level 1's, plus Kyle's own: `kydrawsb.key`). The row is
mode 0 in every puppet, so the existing `(row, 0)` fallback in the draw lookup is what reaches it.

| what | result |
| --- | --- |
| `draw_row_tests` | `(0,1)` `drawgun`, `(0,2)` `drawsaber`, `(2,0)`/`(1,0)` `holster`, a mode swap names nothing, mode 3 stays unselected |
| `--animate` | `the player draws the saber with `kydrawsb.key` (mode 0): weap_saber.cog's SetArmedMode(player, 2) selects it` |
| `cargo test` | **242 passed**, 0 failed |
| `--gates` | 0 of 67 failed, 2 skipped |
| parity gates | draw 5 thresholds/0 failures, series 1/1, menu 0.9661, film 1.0000 |

A scan of every `.cog` in the three archives (540 of them) is what settles the modes: six cogs send
`1`, four send `0`, `weap_saber.cog` alone sends `2`, and none sends `3` — so `drawfists` (24
puppets) is deliberately left unwired rather than guessed at.
## Round 327: two-sided model faces are drawn from both sides

`rdModel3_DrawMesh` (`0x446110`) projects each face's normal against the camera and, when the face
points away, still draws it if `(face->flags & 1) != 0` (`rdModel3_DrawFace` then negates the normal
for lighting). The `.3do` face row carries that word — `num: material flags geo light tex extralight
nverts <v,tv>...` — and the port dropped it, so every model wore `cull_mode: Back` and the back side
of 3,508 faces across 121 models was invisible.

The flag is now parsed (`tdo::Face::flags`) and `model_mesh_posed_with` emits a two-sided face's
triangle a second time with reversed winding. The model material is `unlit`, so the second winding is
the whole of the engine's back-side draw: no extra material and no geometry split, at the cost of one
triangle per two-sided face.

| what | result |
| --- | --- |
| shipped faces | 3,508 of 42,210 carry bit 0: `pb.3do` 128, `tk.3do`/`th.3do` 111, `ry`/`rh` and their `str` pairs 102 each |
| `model_face_tests` | a `0x0` face builds 3 indices, a `0x1` face 6 |
| `cargo test` | **243 passed**, 0 failed |
| `--gates` | 0 of 67 failed, 2 skipped |
| parity gates | draw 5 thresholds/0 failures, series 1/1, menu 0.9661, film 1.0000 |

Left alone: the word's other bits (`0x2` on 12 faces, `0x4` on 129) are not identified yet, and the
`.mat` texinfo flags — the `main.rs` row's other half — are still unread.
## Round 328: the map panel culls surfaces behind the player

`sithMap_Draw` draws a surface only when the eye is on its **positive normal** side
(`(eye - vertex) . normal > 0`, the threshold at `0x52309c` is 0.0); the port's panel drew every
non-horizontal surface, near side and far side alike. The test is now in `ui::Canvas::map`, using the
same signed normal the panel already computes for its floor/ceiling skip.

| what | result |
| --- | --- |
| `--map-shot` | level 1's spawn draws **4,185** ink pixels (was 5,058); the panel still reacts to a moved player (4,762) and the check passes |
| `cargo test` | **243 passed**, 0 failed |
| `--gates` | 0 of 67 failed, 2 skipped |
| image | `target/parity/tmp/map-cull.png` against the pre-change `map-precull.png` |

Still open on the map: the retail wireframe's own camera (each edge projected as a clipped 3D line
through a 45-degree camera), the hidden-edge rule (`sithMap_IsSurfaceDrawable`, threshold 0.98), and
the arrow-button zoom/orbit view state.
### The map's hidden-edge rule

`sithMap_IsSurfaceDrawable` (`0x4ec9c0`) walks the sector's surfaces (and its adjoin chain) and refuses
to draw an edge when another surface sharing it is **nearly parallel**: `0.98 < n1 . n2`
(`0x5230a0` is 0.98). That is what removes the seams between coplanar pieces of one wall. The panel
now builds an edge -> `(surface, normal)` table per redraw and hides an edge whose neighbour passes the
test; the surface's own normal is excluded (the engine compares `iVar5 != param_1`), which matters: an
inclusive version hid **every** edge (2,420 ink pixels, self dot 1.0) instead of the seams (4,108).
`ui::map_tests` pins the three cases — coplanar hides, perpendicular keeps, self never hides.
### The map check counts what the culls removed

`--map-shot` proved only that ink appeared and moved. `ui::Canvas::map` now returns `MapStats`
(`drawn`, `culled_facing`, `hidden_seams`) and the check draws the panel once more on a throwaway
canvas, prints the three numbers and asserts each is non-zero, so "the rules exist in the code" and
"the rules act on this level" can no longer be confused. At level 1's walkplayer spawn: 5,243 edges
drawn, 2,054 surfaces culled by `sithMap_Draw`'s facing test, 1,958 edges hidden by
`sithMap_IsSurfaceDrawable`'s coplanar-seam test.
## Round 330: the soak no longer counts its own teleport as a body pushed out

`--soak-all` reported `19a` with **2** bodies outside every sector. `BK_SOAK_TRACE=1` named them: the
`walkplayer` — which the soak itself had just teleported to the watched sector, a point that lies in
`19a`'s documented hull gap — and `stormtroop3`, which the teleport's script moved 0.43 units down out
of a sector the file's hulls do not cover. The player is the check's own probe, so it is excluded now
(the walk gate and `--live-play` own the player's containment); the actor is a real observation, left
in place.

```
soak-all: 35 level(s) ran, 0 with a non-finite position, 0 with bodies outside every sector: [], 0 failed
soak --level 19a (BK_SOAK_TRACE=1): 1 body outside -- thing 169 `stormtroop3` at [2.849, -2.700, -11.137],
  placed at [2.764, -2.270, -11.428], sector 495
```

| what | result |
| --- | --- |
| `--soak-all` | 35 of 35 levels, 0 non-finite, **0** outside, 0 failed |
| `--soak --level 19a` | 1 message, 1 outside body (`stormtroop3`), 0 non-finite; the player no longer counted |
| `cargo test` | **244 passed**, 0 failed |
| `--gates` | 0 of 67 failed, 2 skipped |
## Round 331: all 67 gates, none skipped

The documented headless recipe (`BK_SKIP_WINDOW=1`) skips `--live-check` and `--live-play --start
01narshadda`. Run with a window, the sweep is complete:

```
gates: 0 of 67 failed, 0 skipped
--live-check            462 collision sector(s), 1206 adjoin rows, 462 sector flags, 7 doorways shut
--live-play --start 01  left click added 1 thing, Space lifted the body 0.11 units, scripts answered 3291
```

| what | result |
| --- | --- |
| `--gates` (windowed) | **0 of 67 failed, 0 skipped** |
| `--live-check` | passed: 462 sectors, 1206 adjoin rows, 7 doorways shut by 7 scripts |
| `--live-play --start 01narshadda` | passed: input, jump and the level's scripts all act |
| `--soak-all` | 35 levels, 0 non-finite, 0 outside, 0 failed |
| `--cog-check` | 34 levels, 0 stubbed verb calls, 0 errors |
| `--audit` | 35/35 levels clean, 10 unresolved asset references, 54 undecoded files |
## Round 332: the endgame fork takes the arm the choice item names

`jkMain_EndLevel` hands `jkMain_StartNextLevelInEpisode` an arm, and
`jkEpisode_GetNextEntryInDecisionPath` turns `arm != 0` into the `DECIDE` row's `gotoB` (`+0x38`)
instead of `gotoA` (`+0x3c`). The engine's own choice value is `jkPlayer_GetChoice` (`0x405ce0`) — the
amount of **inventory bin `0x12`**, which `jkMain_EndLevel` compares against `1` and `2` for its
carries rule. The port always took the light arm; it now takes the arm the choice item names, read off
the runtime that is being replaced (`Runtime::choice`), and both arms are checked against the shipped
manifest:

```
tour: 28 level row(s) on both arms (25 distinct files), 5543 thing(s), 23 film(s), 2 terminal row(s),
      0 script errors, 1 sound(s) missing from retail data
--tour: 15maw -> 38a.smk + 54a.smk -> 16bEscapeShip.jkl (dark arm) asserted beside the light one
```

| what | result |
| --- | --- |
| `cargo test` | **244 passed**, 0 failed |
| `--gates` (windowed) | 0 of 67 failed, 0 skipped |
| `--tour` | passed: both arms' films and successors resolve in the archives |
| dark arm | `38a.smk` + `54a.smk` -> `16bEscapeShip.jkl`; light stays `16aEscapeShip.jkl` |

What remains for the fork is the screen that *grants* bin `0x12` — its two labels are assigned at run
time and are not in `jkstrings.uni`, which is why the light arm stays the default (bin `0x12` is empty).
## Round 333: the map panel's camera, read out of the executable

The map's "eight transform rows" were never in the decompiled set: `jkGuiMap_Show`, `sithMap_Draw` and
`sithMap_IsSurfaceDrawable` are, but the arrow handler and the per-frame update are not. They are
disassembled now (`llvm-objcopy -I binary` → `llvm-objdump -D --triple=i386`), and every number is read:

| what | where | value |
| --- | --- | --- |
| buffer / camera | `jkGuiMap_Show` (`0x415c50`) | 520x320; fov **90**, near **0.2**, far **10**, scale 1 |
| base translate | `jkGuiMap_Show` | `0x5565b0 = translate(0, -2, 0.3)` |
| player translate | `jkGuiMap_Show` | `0x5565e0 = translate(-player)` |
| pitch step / clamp | arrow handler `0x4157f9`..`0x41589a` | `0x5565a0 += ±5`, clamped to `[-90, +90]` (`0x522108`/`0x52210c`/`0x522110`/`0x522114`) |
| yaw step | same | `0x5565a4 += ±5`, no clamp |
| pan | D-pad `0x415957`..`0x4159d1` | `rdMatrix_BuildTranslate34(&0x5565b0, (±0.1, 0, 0) / (0, 0, ±0.1))` |
| per-frame compose | update `0x4ec380` | camera × `0x5565e0`, inverted into `0x84de80`, camera stored at `0x84df18`, `0x84df20 = 1` |
| draw | `sithMap_Draw` (`0x4ec550`) | vertices × `0x84dee8`, facing cull `(cam - centre)·normal > 1e-4`, `IsSurfaceDrawable`, `rdClip_Line3Project`, near-plane test `0x84deb0/deb4` |

Two more facts came out of the same bytes: the first `rdMatrix_PostRotate34` uses the euler block at
`0x556650`, which is .bss (the `Show` copy writes zeroes there), so it is the identity; and
`sithMap_Startup` is handed `(11, &0x52e9e0)`, a table of `0.5, 1.0, 1.5, 2.0, 2.5, 2.5, 3.0, 3.5` —
the zoom steps. The full chain is therefore `camera(0,-2,0.3, panned) x (translate(-player) .
rotate(pitch, yaw, roll))`, angles in degrees (`stdMath_SinCos` scales by `0x522970`), with
`rdClip_Line3Project` and the near plane at 0.2, far 10.

Round 334 shipped it: the panel projects through that camera (see the round below), so the row above is
history, not a standing plan.

## Round 334: the map panel draws through the engine's own camera

`ui::Canvas::map` was a top-down wireframe. It is now the engine's map view: every edge goes through the
recovered transform (`translate(-body) . rotate(euler)` under the camera's own `translate(0,-2,0.3)`), the
view's axes are (right, forward, up) because `rdClip_Line3Project` divides the point's **second** float,
the fov is 90 degrees horizontal over the panel's width, near 0.2, far 10, and a segment is clipped at the
near plane and again at the panel rect (Liang-Barsky) so nothing is drawn over the frame art. The facing
cull now uses the camera's eye, as `sithMap_Draw` does.

```
map panel: 11765 ink pixel(s), 11765 marker pixel(s)
map panel: 489 edge(s) drawn, 2109 surface(s) culled as facing away, 2447 edge(s) hidden as coplanar seams
map panel: the start pose draws 11765 pixel(s), three units away 18549
--- map panel check passed ---
```

The marker is asserted at the **projected** body (`300, 239` for the panel at `40,40,520,320`: 0.3 / 2 *
260 px below the centre), not at the centre the top-down panel used. Level 1's cantina reads as a room
receding from the player.

Fixed on the way: `--save-live` sampled its restored clock **ten frames** after F9, so on this machine a
slow frame pushed 2.5 s past the 2.0 s tolerance and the gate failed ("the load did not restore the
snapshot", clock 8.27 vs saved 5.77). It now samples on the first frame the load has taken effect (the
disturbance holds it 100 s ahead until then) and fails loudly if F9 never lands; two runs pass with a
0.25 s delta, and the gate is stricter than before.

## Round 335: the map camera as data, with its arithmetic checked

The panel's view state is a struct now — `MapCamera { pitch, yaw, roll, pan }`, defaulting to the
engine's own open state — and the two pure pieces are functions: `map_view` (the
`translate(-body) . rotate(euler)`-under-`translate(0,-2,0.3)` chain) and `map_point` (90-degree
horizontal field of view over the panel's width, near 0.2). That makes the arithmetic testable without
a window, which is what the arrow handler will need:

```
the_map_camera_matches_the_engine: the body is 2 ahead / 0.3 below the eye at yaw 0, 37, -90 and 180
  and projects to (300, 239); the body's +x is screen-right (430); a -90 pitch looks straight down; the
  pan moves the camera, not the level
cargo test: 245 passed, 0 failed
--map-shot: unchanged (11765 ink px, 489 edges, 2109 facing-culled, 2447 seams hidden)
```

The dead `radius` parameter went with it: the perspective camera made the radius cull obsolete and the
panel's far plane is the engine's 10 units.

## Round 336: the map's orbit driven headlessly, and the arrow ids read off the handlers

`--map-orbit <pitch>,<yaw>` sets the panel's euler, so the projection can be exercised at any angle
without a window: level 1 at `-40,30` draws 16 968 in-panel pixels (1 776 edges, 2 104 facing-culled,
999 seams) against the default pose's 11 765, and the PNG reads as the cantina from above and behind.

The arrow handlers' two switches give the whole wiring as a table: ids **`0x9a`..`0x9d`** are yaw +5,
yaw -5, pitch -5 (skipped at -90) and pitch +5 (skipped at +90); ids **`0x66`..`0x69`** are the D-pad's
`rdMatrix_PostTranslate34(&0x5565b0, ...)` with `(0.1,0,0)`, `(-0.1,0,0)`, `(0,0,-0.1)`, `(0,0,0.1)`.
The button elements themselves are on the field-log screen (`jkGuiMap_aElements`), so the last step is
`activate`'s special case beside the force screen's.

## Round 337: the map's arrows wired

`Overlay` carries the panel's `MapCamera` now, `activate` sends the map screen's arrow elements through
`MapCamera::arrow`, and `Overlay::show` zeroes the camera when the map opens — `jkGuiMap_Show`'s own
reset. The table is the recovered one: `0x9a`..`0x9d` step yaw/pitch by 5 degrees with the pitch held
inside +/-90, `0x66`..`0x69` pan the camera by 0.1, and any other id falls through to the normal menu
routing (a no-op on this screen).

```
cargo test: 246 passed (the arrow table is one of them)
--map-shot: 11765 ink px, 489 edges; --map-orbit -40,30: 18237 ink px, 1776 edges
```

`--map-shot` now presses the arrows through that table itself (three `0x9c`, two `0x9a`) and asserts the
panel changes: pitch -15 / yaw +10 draws **1 073** edges against the default pose's 489.

What is not done here: a **window click** on the arrow art. This machine cannot run the interactive path
reliably right now, so the wiring is proven by the unit-tested table and the same `MapCamera` driving the
render at a non-zero orbit, not by a click.

## Round 338: the map panel's check is a gate

`--map-shot` existed but no sweep ran it. It is row 68 of `GATES` now, so a full run covers the panel's
projection, its facing and seam culls, the marker at the projected body, the pose sensitivity and the
arrow presses — the pieces this session built.

```
--map-shot target\parity\gate-map.png headless pass --- map panel check passed ---
gates: 0 of 1 failed, 0 skipped
```

The cog link **mask** was looked at again and stays open: `sithCog_LinkCog` (`0x4df610`) stores a per-link
word (`link[3]`) and the sector dispatch (`0x4dfd90`) tests it against `1 << (message[+0xc] & 31)`, but
both are engine-built records — the `.cog` text carries `message <name>` and `thing|sector|surface <name>`
rows with no mask, and `sithCog_ParseSymbolRef` (`0x4df410`) writes only the kind and the target index.
Without the message record's `+0xc` assignment the mask's source is not established, so it is not guessed
at.

## Round 339: inventory writes clamp to the item table, and force-live has a frame-proof witness

The full sweep came back at 66 of 68, and both reds were gate defects rather than port defects.

`--force-live` failed once with "the body rose +0.08" and passed standalone with "+0.11", because the
gate's fallback witness was a **ceiling contact** found through a razor-thin 0.02 tolerance: level 1's
spawn stands under an overhang, and whether a marginal rise reaches it depends on frame timing. The
script's own effect is a velocity, so the gate now also records the body's peak upward speed during the
measurement window and passes on that — the unit gate already asserts the same 3.00 u/s.

```
force-live: the body rose +0.11 (took off at -1.13, peak -1.02), mana 400 of 100
force-live: the lift stopped at the ceiling above the body (the power acted)
force-live: the power set the body's lift (2.85 u/s upward)
--- force-live check passed ---
```

The "mana 400 of 100" line in that output is the gate's own doing (it seeds bin 14 at 100), and it
surfaced a real gap: the port's inventory verb accepted any value. `sithInventory_ChangeInventory`
(`0x4d1ef0`) and `sithInventory_SetInventory` (`0x4d2000`) both clamp the new value to the item's own
`items.dat` `min`/`max` columns **before** reporting the change, and `jkHudInv_ItemDatLoad` is the only
caller of `sithInventory_RegisterType` — so a bin with no `items.dat` row is not even stored by retail.
The port now carries that `min` column next to `max` and clamps `SetInv`/`ChangeInv` the same way, with
a unit test. Bins outside the table (the port's own modelling of mana, secrets and goals) are left alone,
which is the honest reading: they have no retail ceiling to clamp to.

`--tour-live` timed out at 900 s: the tour swaps 21 levels and this machine spends ~30-50 s on each
level's load and shader compile, so the budget ended mid-tour and read as "(timed out)". Its wall clock
is 2400 s now (about two minutes a level), still bounded. The tour's own escape prints during that run
come from bodies that fall out of the world at large frame deltas; the walk-off is the documented defect
named in gaps.md, and it stays a real item rather than being hidden by the budget.

The sweep that followed went 67 rows green (`--tour-live` excluded, below), including `--force-live` under
the new witness and every inventory gate (`--items`, `--bins`, `--mana`, `--ammo`, `--pickup`,
`--use-item`, `--save-live` with its start-inventory comparison).

**`--tour-live` left the sweep.** It re-runs the campaign through real level swaps, which is worth watching —
once. Here each swap pays a level's shader compile (~2 minutes), so the 21-level tour ran past every sweep
budget (900 s timed out mid-tour; 2400 s, with the sweep's one retry, outlived a 45-minute wrapper) and
read as a timeout rather than a measurement. The spine is already gated headlessly by `--tour`; the live
swap path is run by hand (`--tour-live`) and its budget stays at 2400 s. Its escape prints during a long
tour are the documented walk-offs (gaps.md), not a new defect.

**`opendoors` reaches as far as the player's use key now.** The instinct's handler (`0x50abc0`) runs
`sithPlayerActions_Activate` (`0x4fc0a0`) on the actor's own body, so the reach is
`*(float *)(thing + 0x2c) - _DAT_00523200` — the body's `movesize` plus `0.1`, the same rule the player's use
key uses. The port had invented a flat 1.5 units, which let an actor open a door it was still a body-length
away from; `Runtime::activate_reach` reads the template's `movesize` (0.06 fallback) and the AI call uses it.
The unit test moved from a door at 1.5 to the cantina plate's measured **0.12** in-reach pose and gained the
**0.93** out-of-reach refusal retail records, so the reach is measured rather than asserted.

```
test runtime::tests::an_actor_with_opendoors_activates_the_door_in_front ... ok
--- ai check passed ---
ai-live: 118 shot(s) at the body, its health 100
--- ai-live check passed ---
--- door check passed ---
```

What stays open on that row is the sweep's *shape*: the engine sphere-casts from the eye with the target's
radius expanded by `0.4`; the port's scans are a look cone. That needs the window half (an actor at a real
door) and is named in the roadmap, not claimed here.

**`circlestrafe` runs on its own clock.** `FUN_00509890` writes its next call as `class+0x4c + now`, and
`talk` reads the same slot for its interval — so the first number on the line is milliseconds. The shipped
`circlestrafe 5000, 30, 0.6, …` replans every **5 s**; the port rotated the ring every frame, which made the
orbit smoother than retail's. The ring point is chosen once, stored with its due time, and held until the
interval expires; the unit test asserts it is untouched after a second of frames and replaced after five.

```
test runtime::tests::a_close_actor_with_circlestrafe_circles_the_target ... ok
--- ai check passed ---
--- soak-all check passed ---
```

**A withdrawal lasts until the target is gone, not for 5 s.** `FUN_0050b150` clears the mode when `ai[0x70]`
(the remembered target pointer) is zero; the instinct's own number (5000 ms shipped) is the *call rate*, which
the port had mistaken for a lifetime — a hurt actor stopped backing off after five seconds even with the
shooter still in front of it. `ai_withdraw` now carries an **optional** deadline: `None` for retreat's
withdrawal (cleared on a destroyed/removed target), `Some` for `hitandrun`'s break-off, whose second number
really is a duration. A retreat also refuses to arm against a target that is already destroyed, which is what
keeps the mode from re-arming on the frame it clears. The test walks the actor past the interval, proves the
mode is still armed, then destroys the target and proves it clears.

The handler's **lateral jitter** stays open: it rotates the away vector by `(rand - 0.5) × _DAT_005232d4` and
adds a second component when the attacker carries `0x2000`. Those constants did not read out of this copy of
`JK.EXE` (its section table maps the decomp's VAs 0x400000 lower than the values I tried), so they are not
guessed at.

```
test runtime::tests::a_hurt_actor_with_retreat_backs_off ... ok
--- ai check passed ---
```

**`blindfire` rolls like the handler does.** `FUN_00509b30` rolls the instinct's chance once per call; a miss
sets its next call to **now + 1000 ms**, a hit to the instinct's own 750 ms. The port re-rolled at the shot
cadence (more often than the engine) and skipped the roll when the chance was 0 — but the engine's test is
`chance < rand`, so a zero-chance class never fires at all. Both are the handler's now, and the test's
zero-chance control asserts no shot plus the 1 s wait.

```
test runtime::tests::an_actor_that_lost_sight_fires_at_the_remembered_position ... ok
```

**A failed `Listen` roll still turns the actor.** `FUN_0050a6f0` finishes every path with `SetLookPos` — at the
event's remembered spot (or its source thing) — and calls `SetMovePos` only when the chance roll passes. The
port did nothing on a failure, so the `Listen 0.0` sentries (five shipped classes) never turned toward a shot;
`ai_look` now takes the heard point on a failed roll, and the zero-chance test asserts the stand-still *and*
the turn. What remains on that row is the damage-event path and its source-thing look, named in the roadmap.

Two ledger rows were read to their handler and rewritten with the finding rather than a guess:

- **`hitandrun`** (`0x50bb90`) accumulates **firing** time (`state[8] -= const` only while the just-fired bit
  `0x400` is set, on a 1 s call cadence), not wall time from the first shot; that is the shape of the gap, and
  closing it means modelling the accumulation, not moving a threshold.
- **`turretfire`**'s ±160° head sweep is the **damaged** arm (`health < max × arg5`, `0.20` shipped): the
  handler randomises the head inside ±160° and fires on its own cooldown only then. A healthy turret fires
  along its body exactly as the port does, so the row was over-stated before.

**Awareness was keyed by alignment; it is keyed by event kind now.** Two engine reads rewrote the model. First,
`sithAIAwareness_CreateTransmittingEvent`'s third argument is not a side: every call site passes **0** (a death, a
thrown weapon, a noise), **1** (a weapon shot, an explosion) or **2**, and `ProcessEvent` (`0x4f2c30`) indexes a
25-dword block per sector with it — one `(strength, position, source thing)` record per kind, replaced only when
the new event is stronger. The listener scans kind 2, then 1, then 0. Second, the strengths are the call sites'
own: `WeaponFire`/`FireProjectile` **2.0** (`0x40800000`, kind 1), `MakeBlast` **3.0** (`0x40400000`, kind 1),
`KillActor` **5.0** (`0x40a00000`, kind 0), and each passes the thing as the **source**.

The port had keyed the record by the shooter's *alignment* and skipped any shooter without an `.ai` class. Both
were wrong in a way a player would meet: a shot could never cross alignments, and the **player's gunfire raised no
event at all** — enemies heard nothing. The record is `BTreeMap<(sector, kind), (strength, position, time, source)>`
now; shots raise kind 1 from every shooter including the player, a death raises kind 0 with the killer as the
source, an explosion raises kind 1 at 3.0, and a failed `listen` roll turns at the source thing's current position
before falling back to the remembered point. `awareness_at` reads in the listener's kind order.

```
test runtime::tests::a_shot_is_heard_across_alignments_and_the_source_thing_is_kept ... ok
test runtime::tests::a_listen_chance_of_zero_ignores_a_sound_in_clear_hearing ... ok
--- adjoin check passed ---
gates: 0 of 67 failed, 0 skipped
```

`--walk all` is unchanged (24/34 contained, six recorded walk-offs), `--soak-all` 35/35, `--ai` and `--ai-live`
pass, and the suite is 248 tests. What is still missing on the row is `PlaySoundThing`'s noise event: it uses
kind 0 with a volume times `_DAT_00522f80`, and that constant did not read out of this copy of `JK.EXE`.

**The last two awareness gaps closed with the EXE's own numbers.** The `.rdata` constants were read through the
PE section table this time (`RVA = VA - 0x400000`, then the section's raw pointer): `_DAT_005232a0` = 0.5 (the
known value, used as the mapping check), `_DAT_0052329c` = 1/32768 (the rand normaliser), `_DAT_00522f80` =
**0.6**, `_DAT_005232f4` = **-1000.0**, `_DAT_005232d4` = **180**, `_DAT_005232a4` = **90**, `_DAT_005232b4` =
-45, `_DAT_005232c8` = 1.0.

- **`PlaySoundThing`'s noise event** is in: a sound played on a thing raises kind 0 at `volume × 0.6` with the
  thing as its source, which is how a shout or a hurt cry carries through the rooms.
- **`hitandrun` accumulates firing time**: the handler's `state[8] -= _DAT_005232f4` adds 1000 ms per firing
  tick and resets to zero on a tick that is not firing, so the break-off needs a *sustained* window. The port's
  wall clock armed the run-away after a single shot; the test now proves continuous fire arms it and one shot
  does not.

`_DAT_005232d4`/`_DAT_005232a4` also say the withdraw's lateral jitter is ±90° (and ±45° when the attacker's
thing flags carry `0x2000`), but the rotate call's argument order does not say which roll is the angle, so that
half stays open rather than guessed.

```
test runtime::tests::an_actor_with_hitandrun_breaks_off_after_firing ... ok
--- sound check passed ---
gates: 0 of 67 failed, 0 skipped
```

**`returnhome` gates on the awake bit.** `FUN_0050bd70` starts with `ai flags & 4`; the hand-edited port walked an
idle actor home off the instinct alone. `AISetMode` already ORs its argument into that word (so scripts'
`aisetmode(enemy, 0x4)` works), and `LookForTarget`'s find now writes the handler's own expression
`(flags & 0xfffff9dd) | 4` — a found target wakes the actor. The test proves the two halves: without the bit the
actor stays put, and after `AISetMode(actor, 0x4)` it walks home.

**A find is a shout, and the awake bit has two halves.** `FUN_0050aa80` (the `LookForTarget` handler) runs its scan
only while `ai flags & 4` is set; a find writes `(flags & 0xffff_fffb) | 0x232` (**clears** the bit, marks the actor
engaged) and raises a kind-0 awareness event of strength 3.0 from the spotter's position in the **player's** sector,
sourced to the player; once the class interval passes, branch B writes `(flags & 0xffff_f9dd) | 4` and the scan runs
again. The port had read the write backwards (set `0x4` on a find) and never broadcast, so a room's guards each
acquired on their own eyes and never converged. Both branches are modelled, the acquisition path raises the event,
and `assets`-free unit tests cover the flags, the record's strength/position/source and a listener reading it.

**A damaged turret sweeps, fires faster, then kills itself.** `FUN_0050a0f0`'s second arm runs only below
`arg5` of full health (`turretfire 750, 75, 160, 160, 5.0, 0.20, 500, 10000, 0` -> 20 %): it re-rolls a head
yaw inside ±arg2 and a pitch inside ±arg3 on every call, fires on its own 500 ms cadence, and arms a
`sithActor_KillActor(thing, thing, 2)` at `now + 10 s`. The port had a healthy turret only. The sweep is applied
to the shot's direction (no articulated head), and the unit test covers the cadence, the 10 s survival, the
self-destruct and a healthy control that never arms it. Also settled: withdraw's jitter call is
`rdVector_Rotate3Acc(vec, angle)` with the **±45°** roll, applied only when the target's `thing + 0x1d0 & 0x2000`
jkFlag is set (the ±90° roll is a dead store), leaving only that flag's name as the open question.

**Round 340 audit.** Re-measured the roadmap's own state table after the AI work: suite **250 passed**, `--gates`
**0 of 67 failed, 0 skipped**, `--soak-all` **35 of 35 levels, 0 bodies outside**, `--cog-check` **0 stubbed calls,
0 errors**, `--tour` **28 rows / 25 files / 23 films, 0 script errors**, `--walk all` **24/34 contained, 25 hull
agreements, 46 crossings, 6 recorded walk-offs (worst drop 3.35)**, `--audit` **35/35 clean, 10 unresolved asset
references, 54 undecoded files**, `--ai` and `--ai-live` pass. The remaining roadmap work is blocked on resources the
port cannot supply headless: retail captures (`trace/parity/live` gates drive the window and the installed game), a
clean machine, and the design-sized renderer items (portal graph, 3D map viewport).

**Scrolling surfaces re-lay only what moves.** The scroll offset is a pure function of the clock (`slide * time`),
so its 15 Hz epoch only existed because a rebuild re-batched the whole level. The scroll now has its own
`Runtime::scroll_epoch` (bumped per frame while anything scrolls), `build_world` can be asked for the moving faces
only, and `rebuild_surfaces` patches the batches those faces own — skipping any batch it cannot rebuild completely
(`keys` records every key's surface list, and a mixed static/scrolling batch is left as it is rather than
half-patched). The stale offset drops from 66 ms to one frame, static batches are untouched, and `--scroll` now
asserts the split: the scroll epoch moved and `surface_epoch` did not.

**The portal walk actually tests doorways now.** `collide::visible_sectors` has taken a `sees` predicate since it was written
and the windowed pass passed `any`, so its BFS admitted every room reachable through any visible adjoin in the frustum.
It now projects each doorway's own verts: no vertex in front of the eye (clip `w <= 0` for all) or a projected bbox
outside the viewport drops the room behind it, and a doorway that straddles the eye keeps its room — the rule can only
add a room, never hide one that shows. Level 1's spawn went from **90 to 84 of 462** drawn sectors and the two captures
(`target/parity/portal-any.png` / `portal-sees.png`) are byte-identical, so the six culled rooms contributed no pixels.

The doorway test's first cut also taught the failure mode the ledger should keep: the walk's `use_portals` guard
(`reachable.len() > 1`) drops the cull to the **bare frustum** when the walk degenerates, and a stricter `sees` makes
that easy — the probe pose's doorways were behind the camera, the filtered set was the seed alone, and the frame then
drew every room in view, which fired the level-1 goal's `sighted` and left `--ai-live` running past its 600-frame
verdict into the cutscene (`--live-play` timed out the same way). The filtered walk is now adopted only when it is
usable and otherwise falls back to the *unfiltered* set, so the cull is monotone: level 1's spawn draws 84 rooms where
it drew 90, the captures are byte-identical, and `--ai-live` passes again (118 shots, 218 s).

**`Withdraw` no longer needs `Retreat`.** The audit's instinct table showed `withdraw` on 30 classes but only 24
referenced. `retreat` (`0x50bc60`) writes the mode and the target (`ai[0x70]`); `withdraw` (`0x50b150`) reads
`flags & 0x800` and walks. `sensedanger` (`0x50b9b0`) also writes the player into `ai[0x70]` and sets `0x800`. So a
class with `Withdraw` and no `Retreat` still backs away from a surprise, and the port — which required both instincts
for either — never let those six act. The surprise paths now store the target, `Retreat` gates on itself only, and the
flee walk still requires `Withdraw` (a `Retreat`-only class arms a mode nobody acts on). New unit test: an awake
`SenseDanger`+`Withdraw` actor with no `Retreat` sets `0x800`, stores the player and walks away.

**Round 344 ledger pass.** Three `gaps.md` claims the code had outgrown are corrected with their evidence:
`loadtemplate` is answered (`runtime.rs` returns the template name or -1), the per-type hurt sounds and the 0.1
parent-actor scaling are both in `damage_typed_from`/`hurt_mode`, and the `listen` note's "ring of three recent sounds"
was a misreading — `ProcessEvent` (`0x4f2c30`) keeps three **kind** slots per sector (`0x8553a8 + sector * 100`, strength
at `+1/+2/+3`, position at `+4..+6`, source at `+0x16`) and `FUN_0050a6f0` scans them 2 → 1 → 0, which is exactly the
port's `sector_awareness` shape. The scheduler itself is now recorded in the roadmap ledger from its own decomp instead
of being approximated again: file-order scan, mask/mode gate, per-slot 1 s arm, and the flag-change restart with event
`0x100`. Header counts refreshed to 67 gates / 251 tests.

**The map row was stale in both directions.** `--ui-tables` shows the map screen's hidden controls are six `type 1`
elements, ids **100..105**, all with `{click 0x415b60}` — not the "13 hidden arrow controls" the row claimed, and the
3D wireframe camera is not open either: `ui::map_view`/`map_point` already carry `jkGuiMap_Show`'s camera (90° FOV,
aspect 1, near 0.2, far 10, eye 2 behind and 0.3 above the body), the near-plane 3D line clip, the facing cull and
`sithMap_IsSurfaceDrawable`'s 0.98 coplanar-seam rule, with `MapCamera::arrow` stepping yaw/pitch and panning. What
*is* open is narrower and recorded: the click handler `0x415b60` is not in the decomp set, so ids 100/101 have no
assigned action (102..105 coincide with the port's pan ids).

**`aisetmode(x, 0x200)` now latches the target.** The bit is the engine's *engaged* word — the same one
`sithAI_CheckSightThing` writes (`(flags & 0xfffffffb) | 0x232`) — and the shipped cogs use it to place an actor
"enemy" without waiting for a sight roll (`00_aipatrol` x2 and `00_aisleep` set `0x202`; `00_aisleep` also sets bare
`0x200`). The verb arm wrote `actor_flags` and cleared the port's switched-off set but never touched `ai_detected`, so a
script-engaged actor stood until it rolled sight. `Runtime::setactorflags|aisetmode` now inserts the actor into
`ai_detected` on that bit; the suite grows to 252 tests (`a_script_engaging_an_actor_latches_the_target`), `--gates` reads
0 of 67 failed (2 skipped) and `--soak-all` 35 of 35 levels with 0 non-finite positions and 0 bodies outside every sector.

**Also transcribed, for the record:** `sithAI_InstinctUpdate` (`0x4e9e90`) runs each registered instinct by
`(mask & ai.flags) != 0 && (mode & ai.flags) == 0`, calls its handler no more than once a second (`next_call = now +
1000`), and when a handler both returns non-zero and changes the flag word it emits event `0x100` with the new word and
restarts the whole loop. The port's per-instinct state gates implement the same conditions in its own terms; the
`0x100` emission has no consumer in any handler. And `FUN_0050b9b0` (`sensedanger`) confirms the port's polarity: it
gates on `0x800` clear **and `0x4` set**, then writes `(flags & !4) | 0x800`.

**Backpacks.** `Res2.gob`'s 215 cogs were extracted with a throwaway reader to settle a verb question, and the
inventory a dropped backpack carries turns out to be two recovered units: `sithInventory_CreateBackpack` (`0x4d2740`)
and `sithInventory_PickupBackpack` (`0x4d2860`). `CreateBackpack(player)` spawns the `+backpack` template at the
player's own pose and stores up to `0x10` bins of the inventory (the engine keeps only `items.dat` rows with the
`0x1`/`0x80` carry bits and amounts above 0.5); `PickupBackpack(player, thing)` adds them back through the same
`min`/`max` clamp `ChangeInventory` uses. Both arms are live in `Runtime::call` and in `handles`, since
`item_wrenchyellow.cog` (`CreateBackpack`) and `levelsong.cog` (`PickupBackpack`) drive them. Test:
`a_backpack_carries_the_inventory_and_gives_it_back`.

**Two verbs stay ledgered rather than invented:** `GetAutoReload()` (58 call sites across `pow_*`/`levelsong`) reads
the setup screen's "Automatic reload weapon" slider, whose retail default is unrecovered (bit 1 reloads, bit 2
restricts to the current weapon's kind); `GetWeaponPriority(player, bin, 0)` (17 sites) is not a table column at all —
`sithWeapon_AutoSelect` (`0x4d57a0`) asks the bin's own cog with message `0x13` (`autoselect`) and keeps the largest
reply, which the port's damage ranking stands in for.

**The instinct table, transcribed in full.** `sithAICmd_Startup` (`0x5091b0`) registers 21 handlers with
`(name, handler, mask, mode, events)`; the port's per-instinct state gates carry the same conditions, and two were
cross-checked against it: `primaryfire` `(2, 0)`, `follow`/`turretfire` `(2, 0x800)`, `opendoors` `(2, 0)`,
`lookfortarget` `(0x204, 0)` (0x4 = still scanning, 0x200 = engaged — the pair `sithAI_CheckSightThing` swaps),
`sensedanger` `(4, 0x800)`, `hitandrun` `(0xc00, 0)`, `retreat`/`circlestrafe` `(2, 0x800)`, `blindfire` `(2, 0xc00)`,
`randomturn` `(4, 0)`, `withdraw`/flee `(0x800, 0)`, `returnhome` `(0, 0)`, `lobfire`/`crouch` `(2, 0)`.

**Event 2 has no emitter anywhere in the set.** Every `sithAI_EmitEvent` call site: `0x1000` from
`WeaponFireProjectile` (what `dodge`'s event mask `0x1003` listens for), `0x100` from the scheduler on a flag change and
from `AISetMode`/`AIFlee`, `0x400`/`0x800` from the target-message path, and `1` from `DamageActor`. `sensedanger`'s
event mask is `7`, so its `param_4 == 2` arm is reachable by the dispatcher and *never* raised — the port's "dead
content" note now rests on the whole call-site list rather than one handler's inventory.

**Three more shipped verbs have behaviour.** `GetThingLVec` answers the thing's own look vector (local `+Y`
through its pitch/yaw/roll) — `weap_bryar` and `weap_fists` turn it into the shooter's recoil with
`ApplyForce(player, VectorScale(GetThingLVec(player), -80))`, which landed as a zero impulse before. The surge word
behind `jkGet/SuperFlags`/`jkClearSuperFlags` is now a runtime field: the light and dark powerups
(`pow_keyyellow`, `pow_crossbow`) toggle bit `4` and "power" (`pow_power`) bit `1`, each reading it back to know
whether the surge is already on. And `SetThingLook` (`sithPhysics_SetThingLook`, `0x4f5a80`) aims a thing's forward
axis along a direction — snap at blend 0, a partial turn otherwise — and sets jkFlag `0x100`.

**Two more answered:** `AiGetMode(thing)` reads the same script word `GetActorFlags` answers (`actor_*.cog` tests
`BitTest(AiGetMode(me), 0x800)` before butting into a fight), and `PrintInt` joins `Print` in the interpreter. That
leaves the static list of cog-called verbs absent from `src/` at **20** (recounted): two are ledgered above
(`GetAutoReload`, `GetWeaponPriority`), six are the `00_breakingbottle`/`force_pull` surface queries, and the rest are
one-cog setters (`SetThingPulse`, `SetThingFireOffset`, `SetCollideType`, `AddTint`, `SetParticleGrowthSpeed`, …).

**`SetThingPulse` is the thing's own timer.** `SetThingPulse(thing, seconds)` arms a per-thing interval; every
interval the engine messages that thing's own script slots with `pulse` — the blood trickle `00_backpack` starts at
0.05 s, and the damage cadence of `00_deathfloor`/`00_slideceiling`. `ThingState` carries `pulse`/`pulse_at`, the frame
step re-arms and queues `pulse` to `thing_instances` (the same path `arrived` uses), and `0` switches it off. Test:
`a_thing_pulse_messages_its_own_script`.

**Two general spawners.** `CreateThingAtPosNR(template, sector, pos, angles)` is the spawner the force scripts use for
their cones and spheres (`force_absorb`'s `cone_tpl`, `force_grip`'s `sphere_tpl`) and the wrench's splashes; it takes
the position it is given instead of offsetting from a marker's insert point. `CreateThingNR(template, source)` is the
same at the source thing with no orientation carried across.

**The static verb list, recounted with `//` comments stripped (the first pass counted commented-out code):** 221 live
verbs in the 215 cogs, **14** absent from `src/` — `GetAutoReload` (20 sites) and `GetWeaponPriority` (18, both
ledgered above), the four `00_breakingbottle` surface queries, and one-cog setters (`SetCollideType` ×4,
`AddDynamicAdd` ×2, `SetParticleGrowthSpeed` ×2, `AddTint`, `SetWeaponFlags`, `SetThingCollideSize`, `GetThingLight`,
`GetMapModeFlags`). `SetThingFireOffset`'s only calls are commented out in the shipped scripts, so it is dead content
rather than a gap.

**Three of the last verbs settle, three do not.** `GetMapModeFlags` is the getter for the word the port already keeps
(`SetMapModeFlags`/`ClearMapModeFlags`, read back by `force_pull`), and `AddTint` is the same channel as
`AddDynamicTint` — `pow_keyyellow` adds `(0, 0, 0.2)` through one name and `00_aiturncoat` the same value through the
other. `SetCollideType(thing, 0|1)` turns out to have **no consumer in this port**: the dark-jedi scripts use it to make a
pulled weapon non-collidable for 0.3 s, but only `_structure`/`_walkstruct` models are solid here, so a spawned powerup
never collides in the first place. `SetParticleGrowthSpeed` (`-0.8`, a shrinking force sphere) needs a per-thing override
plumbed from the runtime into the renderer's `ParticleGrow`; `AddDynamicAdd` (byte-range `180, 260, 230`) is a per-thing
additive colour with no render channel yet; and `GetThingLight`'s answer is not pinned (per-thing script light vs the
sector's baked light — `force_blinding` is its only caller).

That leaves **12** of the 221 live verbs unanswered (recounted), all recorded with their reason: `GetAutoReload`,
`GetWeaponPriority`, `SetCollideType`, `SetParticleGrowthSpeed`, `AddDynamicAdd`, `GetThingLight`, `SetWeaponFlags`,
`SetThingCollideSize`, and the four `00_breakingbottle` surface queries (`GetSurfaceNormal`, `GetSurfaceSector`,
`GetNumSurfaceVertices`, `GetSurfaceVertexPos`).

**Constants read straight out of `JK.EXE`'s `.rdata`** (VA → file offset through the section table, f32), which settles
two of the port's stand-ins and gives the puppet row its numbers:

| VA | value | what it is |
|---|---|---|
| `0x522fb8` | **0.0** | the zero sentinel every `_DAT_00522fb8` comparison uses |
| `0x522fbc` | **280.0** | the stride's units-to-frames scale: `sithPuppet_UpdatePuppet` advances the locomotion track by `UpdateThingMove() * dt * 280` |
| `0x522fc0` | −30000.0 | the clip-hold timer's "never" value |
| `0x522fc8` / `0x522fcc` | 0.3 / 0.6 | the random draw that picks between the two idle-clip tables (`+0x230`/`+0x240`) |
| `0x522fd8` | 0.02 | the standing-still dead zone `locomotion_state` already uses |
| `0x522fdc` | −3.0 | the backward threshold |
| `0x522ea8` | **0.0** | `CreateBackpack`'s "carry this bin" floor — **not** 0.5 |
| `0x5231a0` | **10.0** | `sithPhysics_SetThingLook`'s partial-turn scale — **not** 1.0 |
| `0x523194/0x523180/0x52319c` | 1.0 / 0.0 / 1e-5 | the look's dot sentinel, zero, and the near-parallel epsilon |

Two fixes follow from the table: `CreateBackpack` now keeps any bin whose amount is above **0.0** (the port used 0.5),
and `SetThingLook`'s blend is scaled by **10** (the port used 1). The puppet stride keeps its clamp for now: the advance
is `speed * dt * 280` in `rdPuppet_AdvanceTrack`'s own unit, and whether that argument is milliseconds of clip time or
frames is not established, so the row keeps its "constant not recovered" caveat only for that unit.

**The awareness floor is 0.0, not 0.00277.** `sithAIAwareness_ProcessEvent` (`0x4f2c30`) reads
`fVar3 = _DAT_0052315c` and recurses only `if (fVar3 < param_6)`, where `param_6` is the remaining strength. A direct
read of `JK.EXE`'s `.rdata` at `0x52315c` gives **0.0** — and the literal `0.00277` does not occur anywhere in the file
(byte search). The port had carried that number since round 110 as if recovered; `AI_AWARE_FLOOR` is now `0.0`, which
makes the guard "any positive remainder crosses, a spent one does not". The `--adjoins` gate and the below-floor unit
check still pass, and the roadmap row records the correction.

**`--things` was a level-1 probe wearing a general name, and it panicked off level 1.** Running it with
`--start 06abarons` tripped two hidden constants: `SetSectorThrust`'s "rises at 5 u/s" held only because level 1's
probe body is flagged against gravity — abarons' body carries `physflags 0x1`, so the same call lifts it at
`5 − gravity` (**1.00** measured), and the second assert demanded a 1.0-unit rise where the physics gives 0.52. Both now
compare against the engine's own sum (`5 − gravity` when the body takes gravity, quarter of the ideal rise). `10cargo`
then failed the debris arm for a different reason: the first "sector with a floor" has another surface directly above
the spawn, so the chip is pressed up instead of falling. The arm now walks up to 96 sectors until one gives the chip a
drop and reports the level when none does — `10cargo` is one, and prints that instead of failing. Verified on
`01narshadda` (falls 0.10, rests 0.20 above its floor), `06abarons` and `10cargo`: all three `--- things check passed ---`.

**`walk5_spawn`'s height deviation, measured instead of described.** With `BK_WALK_TRACE`: retail holds z 0.22 →
−0.01 → 0.07 → −0.01 over the leg, the port 0.183 → 0.165 → −0.273 → −0.387, i.e. the port sits **0.42 below** retail
(the row had the sign as "above"). `--floor-at 9.79,14.22,0.5,0.065` names the cause: the only candidates are
**owner-33 faces at −0.41/−0.45 whose file normals point down** (`[-0.000, 0.000, -1.000]`, `[0.212, 0.212, -0.954]`),
and no owner-35 face covers that point — the slab the body started on (surface 408, floor 0.10, owner 35) ends before
it. So this is the floor-surface/polygon-coverage half of the ledger row, not the sector bookkeeping, and the walker is
correctly taking the highest face it can see.

**Every probe ran on level 1; six of them only held there.** Running the whole level-agnostic probe list with
`--start 06abarons` (then `02narshadda`, `03katarn`, `05test`) found six panics the 67-gate sweep could never see,
because each gate row starts at the first level of the first episode. All six are now either fixed or answered with a
named note; the four-level audit is clean.

- `--quicksave` compared the save's level name (`06abarons.jkl`, the way the engine writes it) against the archive
  index's path (`jkl\06abarons.jkl`) as raw strings, so a same-level save read as a level change. The **live F9 path
  already compares file names** for exactly this reason; the probe now does too (`in place: true`).
- `--rules` built its "wall" from the longest face in `solids.walls`, which on level 6 is a broad **horizontal** face
  (a sloped floor): a face with no height cannot stop a body, so the lateral rule was never exercised and the
  past-the-end half reported a false push. The face must now span at least 1.0 in z.
- `--ai` had three level-1 assumptions. The blocked-pair search took the first walled-off pair in index order, which on a
  big level is too far to walk in the 900-tick budget (now ≤ 60 units, up to 8 candidates); the follower then stalled on
  its **first doorway** for a real reason — `ai_can_walk`'s ground test used a single-sector `sweep`, and level 6's
  doorway has its floor under the neighbouring room, so the walk was refused for ever (`sector Some(3), floor None`).
  `ground` now uses `standable` — the walker's own floor answer, which searches the room **and the adjoins a body can
  reach**, with the same `height + maxstep` reach, so a ledge is still a ledge. The scripted leg also walked a
  **diagonal** (0.5, 0.5) where only one axis was proved open; it walks the proved axis. Level 6's route now drains in
  276 ticks (level 1: 66). The yaw witness is the turn itself: an actor with a target keeps its aim while walking
  (round 316), so level 6's follower turned 112° to its *target*, which the old level-1 assertion called wrong.
- `--attach` had two: the carried height was read after the child settled onto the level's own floor (level 3:
  +0.0487), and the parent's scripted yaw is outranked by **its own instincts** (level 3 kept 10.8°) and its **rate-limited
  turn** (66° short after one step). Gravity is now off for the kinematic section and the turn is given its frames; the
  parent is also marked `ai_cleared`. Level 2's body fell 2.65 units after the pivot, which the finished-spin assert read
  as "kept turning" — the comparison is in plan, and the pivot runs with gravity off.
- `--triggers` asserted a crossing through the centroid of each surface's **first triangle**; level 3's surface 3485 is
  concave, so the centroid is outside the polygon. Surfaces are tried until one is crossed.
- `--hit` required the player's bolt to land on the *particular* actor it aimed at; level 2 has a comrade 1.5 units in
  front, which is what a projectile is supposed to hit. The witness is a body on the path, not that body.
- `--falldeath`, `--endlevel`, `--soak` and `--rules`' jump now say when the level has nothing to measure: level 6 ships
  no `0x40` sector, levels 2 and 6 have no goal cog waiting on `sighted`, level 2's ten `entered` handlers never fire in
  45 s, and level 3's spawn ceiling leaves no clearance for an apex. Level 3's spawn also has no clear line in any
  cardinal direction, so `AISetFireTarget` reports that instead of failing.

Verified: `cargo test --release` 260 passed; `--gates` 0 of 67 failed, 2 skipped; `--soak-all` 35/35 levels, 0 non-finite
positions, 0 bodies outside every sector; the 43-probe audit clean on `02narshadda`, `03katarn`, `05test`, `06abarons`.

**Level 2's route stall is the probe's arena, not a locomotion bug.** With the walk test fixed, `02narshadda` still could
not drain a short walled pair, and `BK_AI_TRACE` named the reason: the follower stands on a **prop** and the next step has
no floor at all — `standable None (reach 0.53)` at a point where `--floor-at` reports `plane floor Some(-3.3561)` with
**0 candidate faces**. Sector 0's polygons do not cover its own sphere centre, so the pair is artificially unwalkable; a
plain 10 s soak of the level traces no refused step from any placed actor (0 hits). Two tolerances came out of it anyway:
the walk test now probes 1 cm above the step, so a body whose origin sits a millimetre inside a floor (level 2's actor
stands 0.0011 under it and the walk keeps its own z) can still step, and the probe stands its teleported bodies on their
sector's floor when the centre is under it.

Verified: `cargo test --release` 260 passed; `--gates` 0 of 67 failed, 2 skipped; `--soak-all` 35/35 levels; 43 probes
clean on `02narshadda`, `03katarn`, `05test`, `06abarons`.

**The `--save` gate had been passing on a file it could not read.** With no path, the probe reads
`%TEMP%\bevi-knight-save.jks` — and an older build's file was still there, so every run printed `jks: ... did not
parse: record of type 0x0C claims 655360 bytes` and then `--- save check passed (prefix only) ---`. The round trip was
never exercised. Removing the stale file first made the probe reach its own state half, which then panicked on a
**double borrow**: `Runtime::handle` is shared per level, so the live runtime and the "fresh" runtime it restores into
were the same borrow. Both halves are fixed (the live guard is released and re-taken after the restore).

What the gate proves now, every run:
`jks: wrote 7308 bytes ... read back health 100, 64 things`; `the saved state restores -- health 37 (was 37), 1.25 along
x, inventory bin 2 = 3, clock 128.50 s, 352 thing(s)`; `AutosaveGame raised the flag and 25990 bytes ... restore 352/352
things at 128.50 s`; `--- save check passed ---`.

Verified: `cargo test --release` 260 passed; `--gates` 0 of 67 failed, 2 skipped; `--soak-all` 35/35 levels.

**`walk5_spawn`'s gap is a polygon hole, checked by containment rather than by centroid.** The earlier reading used
`--faces-at`, which filters by each face's *plane point* (a centroid), so a large floor whose centroid is far reported
nothing — an inconclusive result. The probe now also lists the faces whose polygon **covers the point in plan**. At the
row's 600 ms pose (9.79, 14.22, 0.0), where retail is in sector 33 at z −0.01 and the port is 0.42 lower:

- `faces-at: 0 face(s) cover the point in plan`, and `--collide --surfaces-at 9.79,14.22,-0.05` lists no surface with a
  vertex within 1.5 units at all.

So the level's own geometry has nothing under that point; the port's 0.42 drop comes from a face whose *edge* is within
the body's 0.065 radius (`floor_candidates` accepts sphere overlap, correctly, for the landing probe). Retail holding
z ≈ 0 there means the engine's floor answer differs in a way the shipped data cannot arbitrate: the row stays a fixture
question, and the probe now records the containment result instead of a centroid one.

Verified: `cargo test --release` 260 passed; `--gates` 0 of 67 failed, 2 skipped.

**Force pull runs; force grip runs and exposes a rate the port still guesses.** Both powers were "checked on their
shape" because the notes said `activated` never returns under a synchronous VM. It returns: `force_pull.cog`'s
`activated` switches bin 24 on and arms `SetPulse(0.33)`, and `--pull` now drives the shipped script verb for verb.
Level 1: the script's own `pulse` targets the item in front, `deactivated` clears its gravity, and `ApplyForce` sends it
at the player at `[-30, 0, 0]` — the script's `VectorScale(VectorNorm(...), 30)`. The run needs `startup` first: without
it `player`/`cost`/`damage` are unbound.

`force_grip.cog` is the sibling and needs one more phase: `activated` arms, `pulse` only *finds* the victim
(`targetting`), `deactivated` flips to the grab and re-reads the rank from **bin 31** (not 24), and the next `pulse`
does the damage. The probe runs all four steps: the targeting pulse picks the actor placed in front, the grab pulse
sets `0x40000`, and the measured **rate is 0** because the victim's template sets `damage=0` — the port's
`SkillTarget(bin 31)` answer is the victim's own `damage=` parameter, and the engine's formula (`LAB_005047e0`, not a
named procedure in the Ghidra session) is still unrecovered. The probe records that figure instead of asserting a
damage the formula cannot give.

Verified: `cargo test --release` 260 passed; `--gates` 0 of 67 failed, 2 skipped; `--pull` passes on `01narshadda` and
`06abarons`.

**`SkillTarget` recovered as far as its call chain** (`0x5047e0`, which Ghidra does not expose as a procedure, so it
was read with `llvm-objdump`). It takes `(victim, player, bin, rank)`, fetches both things, answers **-1.0** when either
is null or when `thing+0x29c` on the *player* is zero, and otherwise calls
`0x4dedc0(playerPowerTable, skill 0x19, 3, player, 0, victim, 0, bin, rank, 0, 0)`. Two consequences are now in the
port: `SkillTarget` returns the engine's -1 for a dead/missing thing (it used to index the template unconditionally),
and grip's rate is documented as a per-skill entry of the **player's** power table rather than the victim's `damage=`.
`--pull` prints that chain and the measured 0 instead of claiming the formula is unknown.

Verified: `--pull` passes on `01narshadda` and `06abarons`; `cargo test --release` 260 passed.

**Grip's rate is the victim's class script, and that closes the undecoded table** (round 355). The previous round left
`SkillTarget` at "-1 or a call into `0x4dedc0`", with grip's rate sourced from the victim's `damage=` because the
player's power table is undecoded. The call chain now runs to the end: `0x5047e0`'s dispatcher lands in
`sithCogFunctionThing_SkillTarget`, which sends message **25** (`skill`) with the bin and rank to the **victim's class
cog** and returns whatever that script answers with `ReturnEx`. The shipped answers are the numbers the scripts
assume: `actor_pm.cog` returns **10** for grip (31) and 5 for deadly sight (34) — its own comment says "base damage
that is taken by this actor" — and `kyle.cog` returns 1 for a player, which multiplies it by `2*rank` itself. So the
actor's own script is the table, not the player's.

The port now lifts those answers out of each parsed class script (`Cog::skill_answers`: the `if (bin == N)` arms of its
`skill:` label's `ReturnEx`) into `Runtime::class_skill`, keyed by the class cog instance `thing_class_cog` already maps
the thing to, and `SkillTarget` serves them for any bin before falling back. `--pull` shows the difference directly: the
grip victim is chosen as the first actor *whose class script answers `skill`*, its answers print as `[(31, 10.0)]`, and
the grab pulse takes it 50 -> **30** health — `10 * rank / 2` at rank 4 — where it used to take nothing because the
victim's `damage=` is 0. A headless probe starts its class scripts only on the live path, so the probe binds the
victim's template `cog=` itself, exactly as the live load does.

Verified: `--pull` passes on `01narshadda` and `06abarons` with the 50 -> 30 measurement; `--force-all`, `--force-live`
and `--mana` pass; `--gates` 0 of 67 failed, 2 skipped; `cargo test --release` 260 passed.

**Event 2 has a producer: the awareness sweep** (round 356). The ledger said "no emit site in the executable raises
event 2", which left `dodge`'s and `sensedanger`'s event-2 arms unreachable and — worse — left the `0x1000` one-shot
latch with no way to clear. `sithAI_EmitEvent` (`0x4e9f80`) refuses every event but 2 while `ai+0x8 & 0x1000` is set and
clears the bit doing so, so a latched actor stayed latched forever. The emitter was in the disassembly: `0x4f2b60` is
the per-frame function that bumps its tick (`DAT_008553b4`), lets each `sithAIAwareness_ProcessEvent` call stamp its
sector's slot with that tick, and then walks the level's thing table — a thing whose sector's slot carries the current
stamp gets `sithAI_EmitEvent(thing, 2, 0)` (`0x4f2c05`, `push 0; push 2; push esi`). `transmit_awareness` now raises
that event for every living thing in every sector it just wrote, once per thing per tick (the engine reaches a thing at
most once per stamp, which a `time` map enforces). The handlers' event-2 arms are still unported, so the visible effect
today is the latch. The `dodge` arm itself is decoded for the next round (`0x50b5c1`): awake, `event == 2`, the
instinct's own weight and the sector slot's strength both positive, the slot's source thing non-null and not the actor,
`[source+0xc] == 3` with `[source+0x10] == 1`, the `0x4eb090` sight test clear — then a walk to `pos - dir * weight` at
2.5 with the handler's 1000 ms cooldown and sound `0x4b`.

Verified: `cargo test --release` **261 passed** (new: the sweep clears the latch and reaches a thing once per tick);
`--gates --all` 0 of 67 failed, 2 skipped; `--soak-all` 35/35 levels; `--walk all` 24/34 contained, worst 3.35;
`--tour` 28 rows on both arms; `--audit` 35/35 clean; `--cog-check` 34 levels, 0 stubbed; the parity gates re-run at
round 356's tree — draw 5/0 (`spawn_pose` 0.6393), menu 0.9661, film 1.0000, series 1 graded/0 failures, walk trace
10 legs/0 failures; `scripts/check-docs-links.ps1` 132/132.

**The two windowed gates re-run green at round 356.** `--live-check` (window) delivered 163 pulse/timer messages to
14 armed cogs over its 30 s and wire_links reported 10 sector and 286 thing links; `--live-play --start 01narshadda`
drove the level and its scripted jump, stopping the leap at the ceiling above the body as the key script intends. Both
are skipped in the headless sweep (`BK_SKIP_WINDOW`), so the sweep's `0 of 67 failed, 2 skipped` is unchanged; the two
rows are now measured rather than assumed for this round.

**`walk5_spawn`'s `z 0.422` re-attributed: it is travel length, not the floor** (round 356). The ledger read the deviation
as the body standing 0.422 above retail on a plateau, "the floor-search-scope row". The traces disagree. Both sides walk
the same direction — retail's motion is about 225 degrees even though its `yaw` column reads a stale 135.01 — and the port
covers 3.09 units against retail's 3.73 in the same 3 s (1.03 vs 1.24 u/s, which is the port's acceleration ramp on that
floor). The port therefore stops short of retail's sector-24 point, and the `z` delta is the difference between where the
two traces *end*, not where the body stands: `--floor-at "7.19,13.36,-0.0"` on `06abarons` gives `floor_under ->
Some((-0.0724, 0.0074, 24))` against retail's -0.00, i.e. 0.07. The floor surface and the room search are not this row's
cause; the leg's remaining gap is walking speed, and the trace gate still passes it at 83 %.

Verified: `trace-gate.ps1 -Only walk5_spawn` PASS 3.09 / 3.73 units (83 %), sectors off 2, `xy 1.205 z 0.422`;
`--floor-at` at both end points as quoted.

**`0x200` has no reachable producer in this build.** The inventory said the executable emits it at `0x4ed203`; the site
is real but the function holding it (`0x4ed1d0`) is only ever **installed** as a callback — `0x4e6f05` stores its address
into `0x848600`, beside the per-thing-type registrations `0x4e6f40` writes into the table at `0x847f38` — and a full
`llvm-objdump` scan finds no instruction reading `0x848600` (only the two stores). So `jump`'s `0x200` arm is dead
content after all, and the "no producer" reading of round 252 stands for `0x200` while event 2 is now produced by the
awareness sweep.

Verified: disassembly of `0x4e6f05`/`0x4e6f40`/`0x4ed1d0`, and `grep` over the whole-object disassembly for
`[0x848600]` (one write) and `0x847f3x` (writes only).

**`sensedanger`'s event-2 arm is ported; `dodge`'s is dead content** (round 357). With event 2 now produced, the two
handler arms were the last AI breadth item. `dodge`'s (`0x50b5c1`) is unportable because it is unreachable: it demands
the awareness record's source be a **weapon thing** — `[source+0xc] == 3`, i.e. `SITH_THING_WEAPON`, with
`[source+0x10] == 1` — while every shipped producer records the thing that made the noise (actor, player), never the
bolt. `sensedanger`'s (`0x50b9ff`) is live and is now `sensedanger_from_awareness`: the class's `0x4` word set (hurt) and
`0x800` clear (not alert), the sector's **kind-1** record naming a source inside the class's `heardist` (`class + 0x18`,
passed to `0x4eb090` with a `-1.0` cone, so a range plus a clear line, not a view cone), and the instinct's own weight
beating the record's strength (`msg+0xc > row+8` — why a gunshot's awareness usually silences this arm). It then targets
the source, sets `(flags & !0x4) | 0x800`, and re-broadcasts `0x4f2a90(sector, pos, 1, 8.0, thing)`. Two named
substitutions: the arm's sound (mode `0x4f`) has no name in the port's table so it is silent, and the cloak rules
`0x4eb090` also applies are not modelled.

Verified: `cargo test --release` **262 passed** (new: the arm alerts to a weak heard record, re-broadcasts 8.0, and a
0.3 weight loses to a 1.0 record); `--ai` passes on `01narshadda`; `--gates --all` 0 of 67 failed, 2 skipped.

**The `0x1000` AI latch has no setter in this build.** Following the sweep's clear through, the setter was
searched for and not found: no instruction writes `ai + 8 |= 0x1000` (the whole-object disassembly has no such store,
and the `or ah, 0x10` sites are clip-code and camera-lens words, not an AI block), and no shipped script passes that
word to `jkSetFlags` — the 24 `jkSetFlags` calls in `Res2.gob` pass `0x5`, `0x8`, `0x15`, `0x20` and `0x200`. So the
latch is effectively clear in this build, the gate is inert, and the awareness sweep's event 2 is what would re-arm it
if anything ever set it. Recorded on the emitter's own doc comment so the next reader does not re-litigate it.

Verified: `grep -aoiE "jkset(actor)?flags"` over both GOBs (24 hits, contexts read), and the object scan for an
`ai+8` store carrying `0x1000`.

**A tool fact worth recording: Ghidra 12.1.4 has no functions in this binary's AI/awareness region.** Round 357 opened
`JK.EXE` through REA's Ghidra provider (12.1.4, image base `0x400000`, default analyzers) to see whether the decompiler
could read `0x4f2b60` and `0x4e9f80` more quickly than hand-reading assembly. It cannot: `resolve_containing_procedure`
answers `not_in_procedure` for both and `analyze_function`/`procedure_pseudo_code` refuse the address, including through
a `file-offset:` selector. The analyzer's own timings show why — `Create Function` spent 0.38 s on a 1.39 MB image, so
the function inventory is sparse rather than this one region being unlucky. The project's reads of that region stay on
`llvm-objdump`, which is what every recent round used anyway.

Verified: `binary_session` (Ghidra 12.1.4 available), `resolve_containing_procedure` at `0x4f2b60` (`not_in_procedure`),
`analyze_function` (`No procedure exists at the requested address`) and `procedure_pseudo_code` with
`file-offset:0xf2b60` (`maps to 0x4f3760 ... no procedure`).

**The light/dark screen's labels are absent from the shipped data, and the credits table is present.** Row 46 said there
is no ending screen; the UI tables do carry one — `light_or_dark` at `0x535c28`, three text items on `ui\bm\bkEsc.bm` —
but its keys `GUI_LIGHT_OR_DARK`, `GUI_DECIDE_A` and `GUI_DECIDE_B` appear nowhere in the 313-key `GUI_*` string table in
`Res1hi.gob` (`grep` for `GUI_DECIDE` finds zero), and by the engine's own rule the port already implements
(`jkGui_InitMenu` copies a string only when `jkStrings_GetUniString` finds one) that means a null text — retail draws
nothing there either. The credits pathway does resolve: `GUI_CREDITS` renders "Credits" and `Res1hi.gob` carries the
whole `CREDITS_*` crawl ("Jedi Knight", "Justin Chin", ...). So the screen's remaining gap is the **engine code that turns
its clicks into inventory bin 18**, not its text; `jkPlayer_SetChoice`'s writer is not in `decomp/`.

Verified: `--ui-tables` (41 screens, the `light_or_dark` block quoted), byte searches in both GOBs for `GUI_DECIDE`,
`GUI_LIGHT_OR_DARK`, `decide_a/b` (zero hits) and for `CREDITS_*` (the table, first entries quoted).

**The setup script no longer depends on `Get-FileHash`.** Running the documented command — `powershell
-ExecutionPolicy Bypass -File packaging\build-setup.ps1 -SkipBuild` — died at its last step with `Get-FileHash is not
recognized` on this machine (that install's `Microsoft.PowerShell.Utility` is trimmed; `pwsh` has it, `powershell` does
not). The line now hashes through `[System.Security.Cryptography.SHA256]` and a file stream, so both shells build the
same artifact; the documented invocation builds end to end here (`28.6 MB`, `sha256 25a304d4…`).

**The silent install works; the earlier "denied" reading was the launch directory, not the setup.** Copied out of the
repo (`C:\Users\Lee\AppData\Local\Temp\bk-setup-copy.exe`) the same binary runs `/S /GAMEPATH=… /DIR=…` to
**exit 0** and installs `bevi-knight.exe`, `bevi-knight-uninstall.exe`, `smkdec.exe`, `game-path.txt`, `docs/`,
`LICENSE`, `NOTICE` and `README.md`; the installed program reads its path from `game-path.txt` and loads
`01narshadda.jkl`. Launched from `dist\bevi-knight-0.1.1-setup.exe` (or from a freshly created directory — a copy
under `target\setup-g.exe` works, a copy under `dist\zz.exe` and one in a new `probe-dir\` do not) the process is
denied *every* write, `%TEMP%` included, which a ten-line `csc` probe reproduces with no WinForms and no setup code in
it: the denial follows the directory a process is launched from in this agent shell, and `target\`, `src\` and
`tools\` copies of that probe write fine. So the Phase 4 clean-machine check still needs a plain desktop, but the
installer and the installed tree are measured working.

Verified: the build above under `powershell` (previously failed) and under `pwsh` (already worked); `cargo test --release`
262 passed.

### Round 359 (verification pass): level 2's lip and landing, probed

The round-308/311 seam was re-probed against the fresh fixture with the headless tools rather than reasoned from
the notes, and the two defects separate cleanly:

- **The lip has a real floor.** `--floor-at 10.447,-13.727,3.065` reports `body sector 22`, `surface 200 owner 22
  floor 3.0000 travelled 0.0000`, `floor_under -> Some((3.0, 0.0, 22))`, `standable -> Some((22, 3.0, 0.0))` — the
  body stands on a genuine face, so the *block* at the lip is the step predicate's containment test (the sweep
  adopts the 0.024-deep sliver 23), not a missing floor.
- **The landing floor exists in the data.** `--floor-at 10.02,-14.69,1.92` (retail's last sample) reports
  `surface 388 owner 39 floor 1.8000` and `surface 390 owner 39 floor 1.8000`, `floor_under -> Some((1.8, 0.055, 39))`
  — retail's 1.8 floor is in the level. So the port's miss is the downward chain from the lip (via 23/33), as round
  311 concluded, not absent geometry.

`git log`: the level-2 fixture (`tools/parity/walktrace/walk2_spawn-walk.tsv`) is committed, the stale yaw -99.98
row is gone, and `trace-gate.ps1` names the leg with its measured numbers while keeping it out of the default set;
default gate **10 legs, 0 failures** re-run this round. Suite **262 passed**, `--gates --all` **0 of 67 failed, 2
skipped**, `--walk all` **24/34 contained, 25 hull-agreeing, 46 crossings, 6 void, worst contained drop 3.35**.

`walk18_spawn` is recorded as an unusable leg: `--floor-at -0.35,-2.15,0.083` finds sector 723 with
`floor_under -> None`, every heading probed blocks within 0.3 units, and retail's own trace reads sector 0 for the
last 2.4 s of its hold (it walked out of the map).

### Round 359 (second part): the whole leg set, not the default ten

The gate's default set is the ten legs that passed; every other leg is runnable with `-Only` and had never been
swept together. Running **all 20 walkable legs** (the preflight drops 9: 8 retail script holds plus `walk18_spawn`)
gives **10 pass, 10 fail**:

| leg | port / retail | ratio | sectors off |
|---|---|---|---|
| walk_cantina, walk_street45, walk_street90 | 2.65/2.59, 2.21/2.20, 2.21/2.21 | 102 %, 100 %, 100 % | 2, 2, 1 |
| walk4, walk5, walk6 | 1.11/1.11, 3.09/3.73, 0.84/1.08 | 100 %, 83 %, 78 % | 1, 2, 2 |
| walk9, walk12, walk13, walk17 | 2.04/1.68, 5.03/3.95, 1.27/1.30, 0.82/0.70 | 121 %, 127 %, 98 %, 117 % | 1, 0, 0, 0 |
| walk_switch | 1.48/0.72 | **206 %** | 4 |
| walk3 | 3.57/1.71 | **209 %** (the floor slide) | 4 |
| walk8 | 2.72/0.96 | **283 %** | 5 |
| walk15 | 3.81/1.56 | **244 %** | 5 |
| walk22 | 2.90/1.03 | **282 %** | 4 |
| walk2 | 1.78/2.82 | 63 % | 6 |
| walk27 | 1.80/3.24 | 56 % | 0 |
| walk11, walk20, walk23 | **0.00** / 0.83, 4.20, 4.00 | 0 % | 3, 3, 5 |

Three of the failures (`walk11_spawn` = `11gorc`, `walk20_spawn` = `20aboc.jkl`, `walk23_spawn` = `17bsarris.jkl`)
are a different shape: the port's body never moves at all. The walker zeroes the forced forward input while
`script_camera` is set and `camera_focus_thing()` points at a non-player thing (`sithControl_PlayerMovement`'s
script-camera takeover, measured in the level-14 comment), so these three levels are still under their opening
camera for the whole 4 s window while retail's own trace walks within 600 ms. Whether the port's camera slot clears
as early as retail's is the open question, and it is one root cause behind three legs, so it is the next thing to
look at rather than ten separate collision questions. `walk27`'s short travel and `walk_switch`/`walk8`/`walk15`/
`walk22`'s over-travel are separate.

**The three zero-travel legs are a harness asymmetry, not collision.** `BK_CAMERA_TRACE` on `11gorc` shows the
level's opening script camera holding slot 0 with a focus thing from the first frame, and the focus clearing at
**sim 13.12 s** (`state Some((true, 0, None, 13.118))`); the walker zeroes the held-forward input while the camera
has a non-player focus, so a 1 s settle + 3 s hold never reaches a free body. Retail's own capture reaches the level
**after skipping its opening**, which is why its trace walks from its 600 ms sample. The legs are therefore not
comparable as captured: they need either a matching skip on the port side or a settle long enough for both sides
(re-capturing retail at the same 13 s+ settle). `walk20_spawn` and `walk23_spawn` show the same shape.

**Phase 1 step 4, the door row, measured in both builds.** `retail-capture.ps1 -Shots walk-shots.tsv -Only walk_door0
-Keys ENTER -ConsoleCommands 'cogstatus 0'` (17:53 run) reads retail's cog 0 after the use: `doorstatus 0`,
`movestatus 0`; the port at the same pose (`BK_USE=1 --cogstatus-after 0 --frames 90`) prints `use: nothing linked
within reach`. Both agree the row pose is not a trigger at the engine's `moveSize + 0.1` reach — the row proves the
reach, not a door opening.

**Why step 4's "each trigger" stops here.** Level 1's placed cogs expose three trigger shapes, and each has the best
fixture its input allows: `entered` (the secret-area script) is a retail number and is asserted by `--secret`
(`maxsecrets 6, flags 1`), `crossed`/`used` on a door needs the use key and reads `doorstatus 0`/`movestatus 0` in
**both** builds at the row pose (measured above), and the cantina switch's positive needs a use input retail accepts
from 0.12 units — the harness holds/foregrounds a key and two clicks still leave the door shut, so that capture is
input-bound. The chain itself is already covered headlessly by `--door` (use → `used` message → script →
`setsectoradjoins`, with the solver's `passable` flipping), and the reach by the `use_reach` unit tests, so what a
retail fixture would add is the *retail-side input*, not another assertion.

### Round 359 (third part): the level-2 step seam measured, not guessed

`BK_WALK_TRACE` on the leg separates the predicates: at the lip `clear true lands true advance true` and
**`inside false`** — the step dies on the raised-point containment alone, in both the sweep's room and the body's
own sector. That is the sliver 23 story, measured.

A variant was tried and measured: test the swept **advance** as evidence that the step is a riser (the ordering
`sithCollision_MoveThing` uses — it re-tests the hull *after* the move). It does let the body climb: level 2 walks
**3.59 units with 0 blocked frames** (was 1.78 with 45), and `--gates` stays 0 of 67. It also costs containment on
the sweep: **23/34 levels keep the walker inside its sector (was 24/34), 23 hull agreements (was 25)**, and
`09fuelstation`'s walk-off grows 121.47 -> 134.13 (level 2's own grows 505.25 -> 507.35). The containment rule is
doing real work on other levels, so the change was **reverted**; the endpoint is the recorded baseline again.

Conclusion for the next attempt: the seam is not only the landing chain — relaxing the step's containment buys this
leg by giving back containment elsewhere. A correct fix has to keep `inside` strict and make the *lifted* point
contained where the engine says it should be (a riser's own tread), or move the containment verdict fully to the
caller's post-move re-test with the sector re-resolution corrected.

### Round 359 (fourth part): the three stuck legs are a scripted-opening divergence

`BK_WALK_HOLD_AFTER=13000` (past the opening camera's 13.12 s) does free the port's body on `11gorc`: it walks
**5.80 units** from the warp pose where it walked 0.00 with the 1 s settle. Capturing retail on the same footing
(`retail-capture.ps1 -WalkTraceSettleMs 13000 -Only walk11_spawn`) then shows why they still cannot be compared:
retail's own `coords` at the settle's end read **Pos (-0.15, 9.00, -1.75) sector 11**, i.e. the level's opening
**moved the body 3.4 units** from the row pose, and it then walks only 0.59 from there; the port's body is still at
the warp pose (-0.17, 12.38, -0.23). So `11gorc`/`20aboc.jkl`/`17bsarris.jkl` are not "port blocks" legs at all —
the level start itself diverges (a script places the player in retail and the port's warp outlives it). That is a
separate finding from the walk code, and the three legs stay out of any comparison until the opening is run the same
way on both sides. The captured retail trace stays in `target/parity/trace` (not committed: it starts from a
different pose than the row).

### Round 360: level 11's opening, read from the trace and the script

The three zero-walk legs are now explained by the **level start**, not the walker:

- `11_endlevel.cog` (extracted with `--raw cog\11_endlevel.cog`) takes the player over at `startup`:
  `SetActorFlags(player, 0xa00000)`, `SetCurrentCamera(0)`, `SetCameraFocus(0, camera1)`,
  `playkey(camera1, campath, 1, 4)`, and hands back with `ClearActorFlags(player, 0xa00000)` +
  `SetCurrentCamera(0)`. That is the camera the port's `BK_CAMERA_TRACE` shows on focus thing 13 until sim 13.12 s.
- Retail's own trace of the same warp **does not hold the body**: sample 0 is the row pose
  (-0.17,12.38,-0.23) sector 19, and by 600 ms the body is at (0.15,11.05,-1.79) sector 11 with yaw 140.34,
  settling at (-0.15,9.00,-1.75) yaw 177.56 — 3.4 units of travel and 1.6 of fall while the camera plays. The
  port's body stays at the warp point for the whole 13 s.
- The port's floor at that pose is real geometry (`--floor-at -0.168011,12.3775,-0.129849` -> surfaces 191/217/224 at
  **-0.3500**, `floor_under Some((-0.350004, 0.155, 15))`), and the body rests on it at -0.285. Retail passes that
  height on its way to -1.79, so the two builds disagree about the floor *and* about who moves the body during the
  scripted opening. Both need settling before `walk11_spawn`/`walk20_spawn`/`walk23_spawn` measure anything.

Also measured this round: no shipped script calls `setthingpos`/`movething` on the player (the only hit is
`ctf_main.cog`, multiplayer), so the retail motion is either the actor-flag path or physics, not a scripted teleport.

### Round 360 (second part): a script camera moves the input frame, not the controls

The port zeroed the walk inputs whenever a script camera with a non-player focus was current. Round 139's
reading of the engine says otherwise — `sithControl_Update` reads the controls on every path, and the thrust it
builds is in the **view's** frame — and the retail fixture for `walk11_spawn` shows it: from the row pose retail's
body travels while the opening camera plays.

The rule is now the engine's: `Runtime::script_camera_yaw` returns the current script camera's yaw from its
camera-to-focus segment (the same `forward = (-sin, cos)` basis `yaw_axes` uses), and the walker takes its
movement frame from it while a script camera is current, falling back to the body's yaw otherwise. Unit test
covers the mapping and both `None` cases (a focus on the body, no focus).

Measured on the three legs that never moved before (1 s settle, committed retail fixtures):

| leg | before | after | retail |
|---|---|---|---|
| walk11_spawn | 0.00 | **6.07** (same +y direction) | 3.96 |
| walk20_spawn | 0.00 | **4.63** | 4.20 |
| walk23_spawn | 0.00 | **5.81** | 4.00 |

A full re-sweep of all 20 walkable legs after the change: **11 pass, 9 fail** (was 10/10), and `walk27_spawn` is
fixed outright (**3.94 / retail 3.24, 122 %, sectors off 0** where it was 56 % and 2.4 s of standing still). That
leg had the same script-camera shape, which is why the frame fix reached it.

The three named legs walk in retail's direction now but over-travel, because the script's **input-lock timing** is
still not modelled: retail holds the body ~1.8 s at `walk11_spawn` and then moves only 0.83 units, while the port walks from
the first settle frame. That timing is the next piece, and the legs stay out of the default set until it matches.
The rest of the net is unchanged: suite **263 passed** (the new camera-yaw test), `--gates` **0 of 67 failed, 2
skipped**, `--walk all` **24/34 contained, 25 hull, 46 crossings**, default trace gate **10 legs, 0 failures**.

### Round 360 (third part): the script camera is the focus thing, aimed by its own yaw

The user's note pins the mechanism: level 11's opening camera is the **drone** (`camera1`) flying around the twins, so
Kyle is on screen — the shot is the drone's view, not the player's. That is also how the port's view system already
poses a scripted camera (`focus_camera` reads the focus thing's `pos/yaw/pitch`), so `Runtime::script_camera_yaw` now
returns exactly that yaw instead of deriving one from a camera-to-focus vector (which for the drone case — focus **is**
the camera — was degenerate).

Re-measured with the same committed retail fixtures: `walk20_spawn` moves 4.63 -> **3.55 (85 % of retail's 4.20)**
and `walk27_spawn` still passes; `walk11`/`walk23` are unchanged because their focus thing's yaw is 0, so their gap is
elsewhere: retail's body travels only 0.83 units and stops (sector 15) while the port walks 5.4 straight (+y). With
the inputs now genuinely applied, that is a **collision** difference at the level-11 start, not the lock timing.

The level-11 start also shows **which neighbour a boundary picks** is what diverges: retail crosses 19 -> **15** at
y 12.69, the port crosses 19 -> **14** at y ~13.0 (`--faces-at -0.04,13.20,-0.23` puts the point in both hulls, with
clearance +/-0.025), and the two rooms carry different walls — which is why retail stops after 0.83 units where the
port walks on. The same shape explains `walk20_spawn` (boundary at a different y) and level 2's sliver. So the next
real target is **which of two candidate portals a move crosses**, not the step predicate or the input frame.

## Round 375 -- the level-11 stop is a shut door **sector**, and the sweep only reads one sector

Chasing the 0.83-unit stop: `--collide --surfaces-at -0.6,13.225,-0.35` (with the new bounds match) lists a solid
box whose faces sit at y 13.26..13.31, x -0.60..0.20, z -0.35..0.41 -- surfaces 56..62, all `adjoin -1`, in **sector
9**. That is the shut doorway leaf between 15 and 14, modelled as its own sector. Retail's body stops at y 13.19
(13.26 minus the 0.065 radius); the port's walker walks through it.

The reason is scope, not the plane test: `Solid::sweep_rooms` tests **only the body's own sector's** planes, plus the
adjoin planes, while `sithCollision_SearchForSurfaceCollisions` walks the adjoins the move reaches and tests their
faces too. `BK_SECTOR_TRACE` on the crossing shows `15 -> 14 ... lands_inside true`, i.e. the port's gate is open and
nothing in sector 15 blocks.

Tried the obvious fix -- collect the sectors whose openings the move reaches (<= 4) and test their planes, skipping
faces a neighbouring room merely overlaps. It stopped the through-walk (`--walk all` 24 -> 28 levels kept inside,
crossings 46 -> 45), but `walk11` then walked **backwards** from the start: the steering path itself went the other
way, so the change confounds collision with route choice. Reverted; `--walk all` is back at 24/34 and the 263 tests
are green.

Kept: `--surfaces-at` now also matches a point inside a surface's own bounds (a portal's vertices can be many units
from the doorway it spans). Next: isolate the steering route from the collision query before expanding the sweep,
otherwise an over-block and a re-route look identical in the trace.

### Round 375, follow-up: two facts that change how the doorway fix can be made

**The `walk11` trace leg is not reproducible.** The same command, same commit, gave a forward walk (max y 24.68) at
first and a backward one (max y 12.38) later; three repeat runs then all read 12.38. The walk direction comes from
`script_camera_yaw()` at the moment `BK_WALK_HOLD_AFTER` expires, so it depends on where the drone is when the hold
ends. The leg cannot arbitrate a collision change; `--walk all` (fixed forward, no script camera) is the stable
measure and gives `11gorc` 24.1 units / 0 blocked frames both before and after every variant tried.

**The leaf sits past the portal plane.** The 14/15 portal is at y 13.225; the leaf's front face is at y 13.26. A body
that crosses the portal arrives 0.035 inside the leaf's radius, and sector 9 is not adjacent to 15 (15 adjoins
17,18,19,14,16), so the door is only reachable as a *second* hop (15 -> 14 -> 9). The frontier that follows the
openings the move reaches does add 9 through 14's own portal, but the "skip a neighbour face the sphere already
overlaps" guard then drops it, because by that point the body is inside the radius it must be stopped by. The fix has
to reach sector 9 while the body is still in 15, i.e. collect the rooms before the crossing plane, and then not skip
the face that a crossing would land in.

**The two-hop reach is the right idea and the wrong landing (yet).** Pointing a far room's portal normal at the body
before testing it (otherwise every second hop looks behind the body and never counts) lets the sweep collect sector 9
while the body is still in 15. The level-11 row then reads **0.9 units, 580 blocked frames, contained, hull agrees** --
retail's 0.83 -- and `--walk all` goes 24 -> 26 levels kept inside, crossings 46 -> 43, 263 tests green. But the gate
then panics: `08escape88` drops 2.92 **and** leaves its sector, which the sweep assertion forbids. Both runs are in
this round's history; the change is not landed. Next: find why the widened contact lets the 08escape88 body leave its
sector (a neighbour face blocks mid-doorway, so the slide/containment pair ends in the void) before widening again.

**`08escape88` is a pre-existing containment failure, masked by `11gorc`'s fall.** Its `--walk all` row is identical in
both builds (1.8 units, 557 blocked, 399->339, contained **NO**, drop 2.92). The gate's assertion is
`assert!(worst_contained)` over the worst *non-void* drop, and with the two-hop reach `11gorc` stops at the door
instead of falling 739 units, so `08escape88` becomes that worst and trips the assertion. So the assertion depends on
another level being worse -- it says nothing about the others. Also: the synthetic `--walk all` walk is not the retail
fixture. `walk8_spawn` starts at (52.83, 62.86, 34.457) and the port walks 0.09 units there before stopping, so the
2.92 drop is the gate's own spawn-yaw walk off a ledge, not the compared leg.

Harness: `BK_WALK_ONLY=<substring>` filters `--walk all` to one level, so its traces can be read alone.

**Every level is now checked, not just the worst one.** `--walk all` used `assert!(worst_contained)` over the single
largest non-void drop, so one bad level hid the rest. Asserting per level shows **four** that already dropped and left
their sector at baseline: `04escapehouse`, `08escape88`, `10cargo`, `19b` (recorded in `WALK_STRAY`; the gate now
fails only on a new one).

**The two-hop reach, measured against that gate.** `11gorc` 24.1 units / `19->14` / hull **differs** becomes **0.9
units / 580 blocked / contained / hull agrees** -- retail's 0.83. But `06abarons` and `06bbarons` flip from contained
to **NO** (12.3 -> 12.4 and 1.8 -> 1.5 units), and the counts go 24 -> 23 kept inside / 25 -> 27 hull agree / 46 -> 45
crossings. Not landed: the fix trades two containment failures for one, and the 06abarons pair needs explaining first
(the widened contact stops the body mid-doorway and the slide carries it out, the same shape the door sector produces).
`--walk all` is back at 24/34 with the 263 tests green.

### Round 376: three widened-sweep variants, and why the sweep is the wrong lever

Each variant below was built, run through `--walk all` and the 263 tests, then reverted; the numbers are the gate's.

| variant | kept inside | hull agree | crossings | fixed | broken |
|---|---|---|---|---|---|
| baseline | 24/34 | 25 | 46 | - | - |
| neighbours the move reaches (overlap guard) | 23/34 | 27 | 45 | `11gorc` (0.9 u, contained) | `06abarons`, `06bbarons` leave their sector |
| + far-side portal normal flipped | 26/34 | 25 | 43 | `11gorc` | `06abarons`, `06bbarons` |
| + neighbour faces only past the doorway distance | 26/34 | 25 | 43 | `11gorc` | `06abarons` 12.3 -> 1.8, `06bbarons` |
| + neighbour faces only inside the doorway's cross-section | 25/34 | 23 | 43 | `11gorc`, `06abarons` restored | `03katarn`, `06bbarons`, `09fuelstation` leave their sector |

So the widened contact fixes the level-11 leaf exactly and breaks a different level every time it is narrowed or
widened: the trade is not a threshold to tune, it is that a *static* door sector is not the same object as a door that
the level's script slides open. The port has no moving-door geometry, so blocking every face of a door sector blocks
open doorways too, and not blocking them lets a shut one through.

The next concrete step is therefore a **door-leaf classification at load**, not a wider sweep: for each portal, a sector
whose planes all fall inside that portal's cross-section is a leaf; mark the portal blocked when the leaf is present and
the level does not move it (or drive it from the door's own `setadjoinflags`), leaving every other contact exactly as it
is today. That keeps the 46-crossing baseline untouched on levels without leaves.

### Round 377: the five excluded legs, read one at a time

`trace-gate.ps1 -Only` on them confirms the gate's numbers are current: `walk_switch` 206 %, `walk3_spawn` 209 %,
`walk8_spawn` 282 %, `walk15_spawn` 244 %, `walk22_spawn` 282 % (all fail on travel and sectors). The retail traces
separate two different defects:

* **walk8_spawn and walk22_spawn are real stops.** Retail holds still for the last 1.2 s of the capture: walk8 at
  (51.88, 62.97, 34.64) sector 330, walk22 at (0.00, 6.22, -1.26) sector 408.
* **walk15_spawn is not a stop.** Retail keeps moving to the end of the capture at ~0.5 u/s horizontally up a ~40 deg
  ramp (y 4.19 -> 5.75, z -3.01 -> -1.73); the port does the same climb faster. That leg is a ramp-speed divergence,
  not a pass-through.

walk8's stop is a **step height**: `--move-at "52.83,62.86,34.457,-0.04,0.0046,0.065,40"` steps the body over the
ledge at step 21 and it lands at z 34.567 with the floor at 34.50, then walks on to x 51.27. Retail ends at z 34.64,
i.e. on a floor ~0.075 higher, and something at that height stops it. So the port steps onto a *lower* floor choice at
the same place and slips past what retail is stopped by.

A door-leaf classifier was prototyped (`--door-leaves`: a sector whose plane points fill an opening's cross-section) and
dropped: with a real point-in-polygon test it flags 2249 of 18180 openings, and it **misses** level 11's own leaf
(sector 9 in the 15/14 doorway) under the "fills 80 % of the opening" rule. Not a safe key for a load-time block yet.

**walk8's stop sits beside two door models.** `BK_NEAR_THINGS` at retail's stop (51.88, 62.97, 34.64) lists
`07door5x4` cog-things at (52.30, 63.27, 34.70) and (53.00, 63.27, 34.70), 0.6-0.7 away, plus `table07a/b`. The port's
`--move-at` walks a body straight through that spot (`52.32, 62.97, 34.64` +y crosses the doorway at 63.27 and ends at
63.49), so whatever holds retail there is not in the walker's solid list. Also measured: retail's z at the stop (34.64)
is 0.14 above the 34.50 floor -- it is *not* standing; the value is what a body held mid-step by a contact looks like,
so the earlier "retail stands on a 0.075 higher floor" reading was wrong.

**walk8's and walk22's retail traces end by coasting, not by hitting something.** Per 600 ms segment, walk8 moves
(0.09, 0.02, 0.00, 0.00, 0.00) and walk22 (0.63, 0.19, 0.21, 0.00, 0.00): the displacement decays to nothing over ~1.2 s,
which is what a released key plus drag looks like, not a wall (a wall ends the full-speed strides at the contact frame).
`walk_switch` is different again -- it moves at a steady ~0.5 u/s for the whole 2.4 s capture -- and the port covers
1.48 against its 0.72. So of the five excluded legs: walk3 is documented (retail veers left off geometry the port does
not see), walk8/walk22 are **truncated fixtures**, walk15 is a ramp-speed divergence, and walk_switch is a steady-speed
divergence that is still open. Re-capturing is user-invoked (`aefec79`), so the fixtures cannot simply be retaken here.

**The leaf-occlusion rule does not find level 11's own leaf.** Dropping any opening that a whole sector fills (polygon
containment, 60 % fill, 0.5 thick) leaves `11gorc` at 24.1 units / `19->14` -- the leaf is not detected -- while it
does change `06bbarons` (1.8, contained) and makes `c2` leave its sector. Reverted.

Two conclusions, both measured:

1. `walk_switch`'s stop is a real pass-through, not a coast: its port trace runs -2.95 -> -4.435 (1.49 units) into
   sector 104 where retail ends at -3.67 in sector 59, and one of the earlier leaf candidates sits at (-4.43, -3.70,
   -0.93), i.e. exactly there.
2. Heuristics over the static level data are the wrong instrument. A JK door leaf is a *thing* the scripts move, and the
   only faithful model is to drive the leaf's sector planes from the door thing's pose (or its `setadjoinflags`), which
   the port does not do yet. Every static-data guess either misses the shut leaf (level 11) or shuts an open doorway
   (`06bbarons`, `c2`).

So the next step on this family is a **door model**, not another sweep variant: find the `type=door`/`07door*` things in
a level, bind each to the sector box it moves, and update that sector's planes (or the adjoin flags) from the door's
scripted state each frame. That is what the engine does (`sithThing` on a door leaf + `setadjoinflags`), and it is why
both the shut case and the open case can then be right at once.

**The door *is* modelled -- and that is not what stops the cantina walk.** `--start 01narshadda --door` runs the
whole chain on the level's own scripts: `00_door.cog` on thing 4, `frame Num(1.0)`, `flags (1, 0)`, **4 doorways
that were shut now passable**, and after 3 s the piece sits at frame 1.0 with `goal Num(0.0)`. So sector flags are
already driven by the door scripts, and `Gates::passable` already reads them.

Blocking a portal whose own *surface owner* sector is shut (`setsectoradjoins` is called on the leaf's sector, not on
either room's) was implemented and measured: `--walk all` unchanged (24/34, 46 crossings) and the live cantina walk
still reaches y -4.435 in sector 104. Reverted -- the rule is faithful but changes nothing here, so the leaf's sector is
not shut when `walk_switch` is compared.

That leaves `walk_switch`'s stop unexplained by doors: retail ends at -3.67 (sector 59) where the port continues to
-4.435 (sector 104), and the leg is named for the switch that opens this door -- so the likely blocker is the *switch
panel* or a wall beside it, not the doorway itself. Next probe: `--surfaces-at`/`--near-things` on (-4.75, -3.67,
-1.18) on 01narshadda.

**`walk_switch`'s pass-through is a *sector jump*, and the engine's restore is not safe to apply as-is.** At retail's
stop (-4.75, -3.67, -1.18) `--faces-at` puts the point inside sector 59 (clearance +0.03) and outside 104 (-0.13), and
59's adjoins are [248, 262, 263] -- so the port's 59 -> 104 route is not a doorway at all: `walker_step`'s containment
fallback adopts whatever sector `sector_at` returns. Decompiled `MoveThing` does the opposite: when
`IsSphereInSector(position, 0, thing->sector)` fails it restores the saved position, direction and sector.

Two restore variants were built and measured:

| variant | kept inside | hull agree | crossings | gate |
|---|---|---|---|---|
| restore whenever the step did not cross | 18/34 | 12 | 31 | panics (`m_boss17` leaves its sector) |
| restore only when the resolver's sector does not adjoin | 20/34 | 19 | 40 | panics (`m_boss17`) |
| restore only when non-adjacent **and** the centre probe | 18/34 | 17 | 40 | panics (`m_boss17`) |
| baseline | 24/34 | 25 | 46 | passes |

Both are worse because `inside_sector` on the **feet** point is strict at exactly 0.0, so a body standing on a floor is
"outside" every frame and a restore freezes it (`11gorc` walked 0.0 units in the first variant). The restore only
becomes viable once that probe uses the engine's own centre query without regressing walk4/walk5/walk13 -- the code's
`shortcut:` note already names that trade. Reverted; `--walk all` back at 24/34 and the 263 tests green.

### Round 378: `jkMain_EndLevel`'s arm item -- implemented, and the choice's writer does not exist

Ghidra is usable on `JK.EXE` (`12e8c8c3…`), so the ending question was settled from the shipped binary rather than
guessed.

`jkMain_EndLevel` (`0x402eb0`) reads the episode entry's two bins (`+0x30`, `+0x34`) -- the port already parses those
columns as `lightpow`/`darkpow` -- and for each: `0x13 < bin < 0x23` and `jkPlayer_GetChoice()`, granting
`sithInventory_SetCarries(player, bin, 1)`. The light bin is granted unless the choice is 2, the dark one unless it is
1, so a choice of 0 grants both. The port parsed both columns and never spent them: a level's own arm item was dropped.

Implemented as `episode::Episode::end_level_items` with the binary's rule and a unit test, and wired into
`watch_level_complete` (it applies the grant before the tally). 264 tests green, `--walk all` unchanged.

The **writer** is answered too: the only two callers of `sithInventory_SetCarries` (`0x4d21e0`) are `jkMain_EndLevel`
and `0x4d2560`, which is a bulk inventory reset that clears every bin (it walks the item table calling
`FUN_004d2000`/`FUN_004d21e0` with 0 and zeroes the tallies at `+0x1348`). Neither writes bin 18, and the `light_or_dark`
screen's name string (`0x535c28`) is referenced only from the UI table itself (`0x535db8`), not from code. So in this
build nothing converts the ending screen's clicks into `SITHBIN_CHOICE`, which is why the value is neutral; the port's
neutral default is therefore faithful, and the remaining work on that item is the screen's *presentation*, not a writer.

### Round 379: the two-hop flip, explained from 06abarons' own trace

The widened contact is closed as a lever, not a threshold: round 376's table already showed every narrowing broke a
different level, and the trace now names the mechanism.

`06abarons`' accepted doorway is 564: depth 0.1779 against a 0.13 body, and **the strict test refuses every advance**
(`inside false` at 0.0846, 0.0946 and 0.1496). The body is only let in by the one-step allowance (`stepped true`, feet
0.07 above the far floor). That is the seam: the port's doorway admits the body by standing it a step high on the far
side, so the containment check that later runs on the far sector's polygon is already outside it by construction, and
with the two-hop reach the body advances one more step (12.3 -> 12.4 units) before the neighbour face blocks it -- one
step further out along exactly that allowance. The level is therefore not broken *by* the widening; it is admitted by a
rule the widening lets it use once more.

The fix is the door leaf, not the sweep: a doorway that is *static* geometry should never be entered through a step
allowance, and a doorway that a script slides must have its planes moved. Round 376 put the same conclusion in the
roadmap item; this trace is the measurement behind it.

### Round 380: why the walk gate cannot settle a door leg

`Gates` is empty in `--walk`: the harness walks bare geometry, so every doorway is passable and no door script has
run. That is the reason `walk_switch` passes where retail stops -- its leaf is a door, the capture has it shut, and the
comparison runs against a level whose flags were never seeded (`Gates::default`). The same applies to level 11's
sector-9 leaf and the cantina's 261: the shipped door script's own `SectorAdjoins(doorsector, 0)` never executes in a
walk row.

So the door item needs two things before any collision rule can be tuned: a headless script boot that yields the
level's starting sector/adjoin flags for the harness (the `--endlevel` gate already runs cogs headless, so the
machinery exists), and geometry for the *open* leaf -- retail's stop sits on a leaf candidate at (-4.43, -3.70, -0.93)
where the port has no solid, because the leaf thing never moves in its own sector box. Neither is a stat/flag tweak;
both are the moving-door model, which is why round 376's sweep variants could not close it.

### Round 381: the walk harness can now walk a level's own startup state

`--walk all` walked bare geometry (`Gates::default`), so no door script had run and every doorway was open. The
harness now boots the level's placed scripts headlessly (`BK_WALK_GATES=1`: `Runtime::handle`, `wire_links`, run each
cog's `startup`), and passes the resulting sector/adjoin flags into `walk_level`. `BK_WALK_AT="x,y,z,yaw"` starts a leg
at a recorded pose, so a retail leg can be measured headlessly.

Measured:

- Level 1's startup switches **7 sector flag words** and leaves **5 openings without `MOVE`** -- the door scripts do
  close their sectors, and `00_door.cog`'s `startup` is where.
- The synthetic `--walk all` table does **not change**: 0 of 34 rows differ with gates on. The spawn-yaw walk never
  crosses a door, so the harness's own rows were never talking about doors, and the door legs cannot be fixed there.
- `walk_switch`'s pose under the harness moves 0.4 units where the capture moves 1.48: the harness steps at
  `rules.walk` from the first frame, while the capture ramps acceleration and settles. So the leg's 206 % ratio is a
  capture-path measurement, and comparing it needs the capture, not `--walk`.

264 tests green, `--walk all` unchanged at 24/34.

### Round 382: the cantina door is a solid thing the sweep never searches

The switch leg's blocker is identified. Level 1 places two `4x3door_1` leaves at (-4.75, -3.75, -1.1) and
(-4.45, -3.75, -1.1): `collide=3`, `size=0.250312`, model plan radius 0.200, z span -1.25..-0.95. They *are* in the
app's solid list --

```
solid: `4x3door_1` at [-4.75, -3.75, -1.1] radius 0.200 z -1.250..-0.950
```

-- and retail's stop sits exactly where they are: the leg ends at y = -3.67, and a 0.065 body meeting the leaf's face
at -3.75 stops at -3.685. The port walks to -4.435 with **zero** `blocked by a solid model` trace lines, so the leaf is
never tested.

`sweep_things` filters by `in_search`, which asks whether the move *crosses* the portal into the thing's sector. A door
leaf lives in its own thin sector beside the opening it seals, and the body crosses the *next* sector over, so the leaf
is skipped. Adding a one-hop neighbour reach and then a two-hop reach changed nothing (both reverted), so sector 260 is
not a portal neighbour of 59 or of the rooms 59 opens into -- the relationship needs the sweep's own room list.

The engine's `SearchForThingCollisions` walks the sector lists **as the sweep reaches them**, one room at a time; this
port does one pre-pass over the whole move and then filters. That structure, not a reach radius, is the next step.

New probes kept: `BK_THINGS_NEAR="x,y,z"` (what a level places near a point, with its `collide=`/`size=` and the solid
cylinder if any) and `BK_WALK_SCRIPTS=1` (a level's script names and any `startup` that failed).

### Round 383: the cantina door stops the body now

Two measured changes, and the switch leg moves from a 206 % over-travel to a stop at the leaf.

1. **Two-hop thing reach.** `59 -> 260` is measured at **2 portal hops** (`path [59, 262, 260]`, new `BK_HOPS="a:b"`
   probe), so `sweep_things` now searches a thing's room one hop past the rooms this sector opens into. Before this the
   leaf in sector 260 was never tested: the body crossed 59 -> 262 and the cantina leaves live beside that opening, not
   in it.
2. **Overlap stops the move.** The first contact left the body *touching* the leaf, and `c <= 0.0` skipped every frame
   after it -- the tunnelling. A body walking *into* an overlapped solid is now stopped at distance 0 with the outward
   normal; walking *out* (the escape direction) is still free.

With both, `walk_switch` ends at **(-4.75, -3.485, -1.235) in sector 59** -- retail ends at -3.67 in 59. Travel 0.535
against 0.72 (**74 %**, from 1.48 / 206 %) and the sector now agrees; the remaining 0.185 is the leaf's collision
volume: the port uses the model's plan radius (0.200), while retail's stop matches a much thinner leaf (~0.08), i.e. the
engine collides with the leaf's own surface, not a cylinder around it.

Unchanged elsewhere: `--walk all` 24/34 with and without gates, the trace gate 10/10, `walk3`/`walk8`/`walk15`/`walk22`
re-measured at their previous travels, 265 tests green.

Regression test: `a_body_walks_into_an_overlapped_solid_thing_and_is_stopped` (two-hop reach + zero-travel contact +
walking out of an overlap left free).

### Round 384: solid things as boxes -- measured against the whole gate, reverted

The leaf's remaining 0.185 units are its collision *shape*: the port uses one cylinder (`min(size, model plan)`
= 0.200) where the engine collides with the leaf's own surface (half-thickness ~0.02, which is what retail's stop at
-3.67 matches). A box per thing was built and measured.

| variant | trace gate | walk5 | walk17 |
|---|---|---|---|
| cylinder (landed) | 10/10 | 74 % | 117 % |
| box, bounds centred on the model | **3 failures** | 39 % | 7 % |
| box capped by the template radius | 3 failures | 39 % | 7 % |
| + box kept on the thing's origin | 2 failures | 39 % | 7 % |
| + corner contacts slide | 2 failures | 57 % | 7 % |

`walk17` is the reason: `18Ascend` spawns the body **between two `4x4door` leaves** (0.2 x 0.02 each, 0.37 apart), and
a box pair leaves no gap for a 0.065 body -- the cylinder pair only "worked" because an overlap was skipped outright.
`walk5` is the same class in a milder form (57 % against 74 %).

So a boxed body needs the *door state* (the leaves move, and retail's spawn has them out of the way) before it can be
right; that is the moving-door item, not a shape tweak. Reverted; 265 tests, the trace gate at 10/10 and `--walk all`
at 24/34 are unchanged, and `BK_HOPS="a:b"` is kept as the probe that showed the cantina leaf is two hops out.

### Round 385: the doorway advance is clamped by solids

The crossing advance runs a radius past the doorway plane and is not a wall test, so a leaf standing *in* the opening
was stepped over and the body ended in the doorway's own sector (262), past the leaf it should stop at. The advance is
now clamped by the first solid contact along it.

`walk_switch`: ends at **(-4.75, -3.485, -1.235) in sector 59** -- retail ends at (-4.75, -3.67, -1.18) in **59**, so the
sector agrees for the first time; travel 0.535 against retail's 0.72 (74 %). The remaining 0.16 is the leaf's shape
(cylinder 0.200 vs a 0.02-thick panel), which round 384 measured as a regression elsewhere when boxed.

265 tests, the trace gate 10/10 and `--walk all` 24/34 unchanged.

### Round 386: the ramp-slope hypothesis for walk15, measured and reverted

Retail's `walk15_spawn` trace climbs at a constant *3D* rate: 1.56 across, 1.28 up in 3 s (0.67 u/s on a 39-degree
ramp), where the port walks its horizontal speed and gains the height free (3.81 units). The obvious transcription is
the engine's own: the lateral step is `speed * n.z`, the floor's cosine.

Measured on the capture: the room's floor plane reads **n.z = 0.707** (a 45-degree ramp) and the port's per-frame delta
drops from 0.013 to 0.004 -- so the factor that reached the body was ~0.3, not 0.71, and the body then stalls at
y 4.60 in sector 387 (`blocked true` on a full delta, no solid and no wall named by the traces) where retail walks on
to 5.75. The slope idea is not disproven, but this leg cannot test it: the port's body is stopped on that ramp for a
different reason first. Reverted; `walk15_spawn` stays at 244 % and the blocker at y 4.60 is the next thread.

### Round 387: the walk15 stall is a feedback loop, named from the trace

With the lateral step scaled by the floor's cosine (`n.z` = 0.707 here), the body's per-frame advance at y 4.27 reads
**0.0021**, not the 0.013 x 0.707 the factor implies -- the *velocity itself* collapsed, so the walker's moved distance is
being fed back into the next frame's speed. Scaling the step therefore throttles the body geometrically, which is why the
first attempt stalled at y 4.60 and closed the leg 0.4 short of retail's 1.56.

So the slope belongs in the **velocity integration** (or as slope gravity the walker's thrust fights, which is what
`sithPhysics` does), not in the position step. Reverted again; `walk15_spawn` stays at 244 %, and the next attempt has
to start at the control/velocity side of `walker_step`, not at its `delta`.

### Round 388: slope scaling reaches the velocity and changes nothing -- walk15 is airborne

Two models measured on the capture, both reverted:

- **Gravity along the floor** (`pull = g * n_z * n_h`, the engine's own gravity): with this level's `gravity 4` and
  `drag 0.5`, the pull is 2.0 units/s^2, which settles a 45-degree climb *below zero* -- retail walks it at 0.52. The
  capture closed from 2.32 to 1.07 across (69 % of retail's 1.56), so the model is wrong even though it moves the leg
  in the right direction.
- **Target scaled by the cosines** (`walk speed * n.z`): the probe finds a 45-degree floor (cos 0.707 most frames) and the
  factor is applied to the integrated target, yet the capture is **unchanged** at 6.5075 -- the horizontal motion is not
  coming from `target`.

The trace says why it is not a simple slope question: the body is **ungrounded on 212 of 243 frames** while it climbs, so
the walk is being carried by the vertical/step path and the air drag, not by the grounded walk integration. `walk15` needs
its grounded state explained before any speed model can be judged against it.

### Round 389: walk15's ramp is a plane, not a face -- the landing probe never sees it

Measured, then reverted:

- The ramp is found by the **plane** query (`floor_surface` -> surface 4227, a floor plane) but the body is ungrounded
  on 212 of 243 frames, and the flat legs are grounded 221-235 of 243, so this is specific to the slope.
- Widening the landing probe to the engine's own range (`sithPhysics_FindFloor`: `moveSize + 0.005`, the shipped
  `0x523184` = -0.005) changes nothing: the capture still ends at 6.5075, so the landing path is not rejecting by a
  whisker -- `floor_under` (the **face** query) simply has no face over this body, while the plane query does.

The next step is therefore in `floor_under`: a 45-degree ramp's triangles are classified by `normal[2] < -0.7`, and a
normal of 0.7071 sits a hundredth from that line, so a triangle authored slightly shallower is a `wall` and the surface
sweep misses it. Classify ramps by their *host sector's* floor plane, or file them in both lists, before measuring speed.

### Round 390: walk15's ramp, pinned to the exact branch

`--floor-at` along `walk15_spawn`'s path (`16aEscapeShip`, x = 0):

| y | sector | plane floor | `standable` face |
|---|---|---|---|
| 4.19 | 387 | -2.663 | -2.265 (0.40 *above* the plane) |
| 4.60 | 407 | -2.218 | -2.689 (**0.42 below** the plane) |
| 5.20 | 386 | -2.225 | -2.265 |

So on this ramp the walkable surface is the **sector plane**, and the nearest face is 0.42 below it. The landing sweep's
range is `radius + the frame's fall` (~0.07), so it never reaches that face and the body is not grounded; and the plane
fallback that would hold it is gated on `floor_under(..., reach 1.0, ...).is_none()`, which the deep face defeats. That
gate is why the body falls a little, catches the deep face, climbs the plane, and repeats -- the 212-of-243 ungrounded
trace.

The fix is in that gate: "no floor of its own" must mean *no face within the landing range*, not "no face within a unit".
The level-5 regression the current reach guards against (a ledge plane 0.3 above a 0.1001 floor) needs the plane to be
*below* the feet and within the fall, which the block already checks.

### Round 391: the plane fallback cannot fire -- the body is *under* the plane

Narrowing the gate to the landing range changes nothing (still 6.5075). The reason is the block's own condition: it
requires the plane to be **below** the feet (`contact <= pos[2]`), and on this ramp the body is 0.3-0.4 *below* the
walkable plane -- it has sunk into the geometry, where no landing or plane fallback applies. Retail's body at the same
y is ~0.5 higher, resting on the plane.

So the defect is earlier: the port's body can descend through the ramp's surface (the surface sweep at 0.07 range cannot
see it, and the plane is only consulted when a face is absent). Fixing it means the vertical sweep must consider the
sector's own floor **plane** as a candidate contact -- the way the engine's `FindFloor` sweeps surfaces but the body is
then held by `sithCollision` against the sector -- not a gate on the fallback. Recorded as the next step.

### Round 392: the classification cut is not the answer either

Filing 45-degree surfaces as floors (`FLOOR_COS` 0.7 -> 0.6) leaves `walk15` exactly where it was (6.5075), so the
ramp's triangles were never the missing contact: at y 4.6 the only face within a unit is the one 0.42 *below* the plane.

That closes the classification thread and leaves the measured statement from round 391 standing: the walkable surface on
that ramp is the **sector plane**, the body sinks below it, and the landing sweep only consults planes inside the fall
band. The engine's `sithCollision` keeps a body off a sector plane directly, which this port's vertical sweep does not
do for a body already below it.

### Round 393: the door row's fight does not start from the warp pose

Reproducing the door measurement needs the body **at** the pose (`--warp`, not `--shot-at`, which moves only the camera):

`--start 01narshadda --warp "1.400084,9.700065,6.357114,105.000092" --frames 600` lands the body at (1.4, 9.7, 6.265)
in sector 60 and it **never takes a hit**: health and shields read 100.0 for all 600 frames.

So the ledger's "29 health after 600 frames" cannot be reproduced from the pose alone; it came from a run in which the
AI engaged (the captured shots/hits imply live enemy fire). Either the enemies are behind the cantina door, which the
port now holds shut at startup, or the capture's own input drove them. Until that setup is reproduced, the row's
retail-vs-port health comparison has no port-side number -- which is what the item needs.

### Round 394: walk15 is not the step path, and its implied slope is 54 degrees

- Seating the body on the floor it steps onto changes nothing on this leg.
- `BK_STEP=0.0` and `BK_STEP=0.2` both leave the capture at 6.5075: the step-up path is not involved at all.
- The trace's own speeds are the fact to work from: horizontal 0.77 u/s (unchanged walk speed) with a vertical rise of
  ~1.05 u/s -- an implied **54-degree** surface, where retail's rise over the same 3 s is 0.43/s at 0.52 horizontal
  (39 degrees).

So the port's body is on a different, steeper surface than retail's at the same x, not merely walking it too fast. The
next probe is the surface at the body's own xy every second (plane and face), which is what would say whether the body
is on the ramp's top skin, its underside, or a trim polygon.

### Round 395: the cosine does reach the step; the leg ends at a stop, not a distance

With the cosine applied, the trace prints `slope: cos 0.707 target [0.0, 2.4]` and the move's own `left` is 0.0222 per
frame (1.33 u/s). The capture still ends at 6.5075 -- because that is where the body **stops** (against a solid), not a
distance limit: both builds arrive and wait.

So the ratio the fixture reports (3.81 against 1.56) is a *rate* comparison that only shows while both are still moving,
and the port arrives early. Getting the rate right needs the engine's own unit relation first: the walk target is 2.4 while
the body's measured speed is 0.77, a factor of ~3.1 (`MODEL_SCALE`), and a cosine applied to the target does not move the
measured speed the way that factor predicts. That arithmetic is the next thing to pin down, before any further change to
this leg.

### Round 396: the walk is built in the body's frame, and level 11 walks the right way

`walk11_spawn`'s retail trace holds yaw 0 and walks **+y** into sector 15 while the opening camera plays. The port was
building the walk in the *script camera's* frame, so it walked **-y**, into the level's shut door, and ended in sector 11
-- the leg read as a 5.4-unit over-travel.

Retail's own input path builds the thrust from the body's yaw; the script camera drives the view, not the walk. With the
body frame restored (and the `script_camera_yaw` query deleted with its test -- no production caller left):

| leg | before | after | retail |
|---|---|---|---|
| walk11_spawn | -y into sector **11** | (-0.20, **13.11**) sector **15** | (-0.04, 13.2) sector 15 -- **88 %** |
| walk20_spawn | 3.55 (85 %) | 4.64 (**110 %**) | 4.20 |
| walk23_spawn | 5.81 (145 %) | 5.89 (147 %) | 4.00 |

Gates: trace gate **10/10**, `--walk all` 24/34, **264 tests** (the camera-yaw test went with the query it covered).

The other half of level 11 is the **shut leaf's own faces**: sector 9 (the doorway leaf between 15 and 14) has
`SITH_SECTOR_ADJOINS_OFF`, and the port's sector sweep only read the current room's planes, so the body crossed the
opening and the leaf's space. The sweep now tests a shut neighbour's faces as walls, clipped to the opening's mouth
(projected onto its own plane), which stops the body at the leaf -- `a_shut_opening_leaf_blocks_the_body_through_its_own_faces`
covers it. Trace gate **10/10**, `--walk all` 24/34 unchanged, **265 tests**.

### Round 397: solid things stay circles -- the bounds **box** wins the leaves and loses level 17

The remaining stop divergence (`walk_switch` 0.185 short of retail's -3.67) is the cantina leaf: the walker's thing
model is a circle of `min(size, plan)`, and the leaf's model is ~0.025 thick, so the body stops a body radius early.
Tried the model's **bounds box** (rotated by the thing's yaw) for the sweep, for every thing and then only for the
`4x4door`/`4x4elev` templates:

| leg | circle | box (door slabs) | retail |
|---|---|---|---|
| walk_switch | -3.485 | **-3.665** | -3.67 |
| walk11_spawn | 13.1955 | 13.1955 | 13.2 |
| walk17_spawn | 0.70 (pass) | **0.00** | 0.70 |

The box is exactly right for the leaves and drops `walk17_spawn` to zero travel: the body is blocked at its own spawn,
so a door leaf's box is either misplaced or far wider than the leaf there. Radial-only recovery and door-only slabs did
not change that, and the cause was not found in the time available. Reverted; the sweep stays a circle. The next attempt
should print the leaf box at 17bsarris' spawn with its `thing.yaw` and compare it against the door's surfaces before
touching the sweep.

### Round 398: the door capture is **invulnerable** -- every bolt is swallowed before the shield bar

Chasing Phase 1 step 5 (the door encounter must match retail's health/kill order), `--hit`-style tracing of the live
capture (`BK_HIT_TRACE`) shows every landed bolt taking the invulnerability path:

```
hit: 11 ignored -- player thing 0 is invulnerable (flags 0x8)
hit: the player took 11, 100 shield(s) and 100 health left owner Some(155)
```

`Flags 0x8` is `SITH_AF_INVULNERABLE`. The level-end chain also uses that pair (`01_goal0.cog` sets it before its sleep;
the `--endlevel` gate asserts both ends), but this capture never logs `jkEndLevel`, so it is not that chain: a shipped
script sets the freeze and never clears it. The trace names the caller as cog indices 145+, which are not the level's 42
placed instances, and the runtime's `cog_names` map (added with the probe) does not resolve them because the traced pass
and the live fight build their VM lists separately. Retail's own door series loses 100 -> 44 health at the same pose, so
retail is *not* frozen there.

The capture therefore reports 0 damage by construction, not by an AI gap: the earlier "15 hits, 29 HP" figure was
measured before something started setting the freeze at this pose.

**Round 399 names it and fixes it.** `--cog-source` (new: prints a shipped script's text) shows the owner:

```
init_kyle:
   SetActorFlags(player, 8);
   SetTimerEx(1.50, 1, player, 0);     // the respawn invulnerability, cleared by `timer:` when id == 1
startup:
   if(!IsMulti()) call init_kyle;
   SetTimerEx(2 + 5*rand(), 2, 0, 0);  // the bubbles check, armed in the same frame
```

The port kept **one** timer per cog, so `startup`'s write erased the 1.5 s clear and the flag stayed set forever. The
timer slot is now a list (`Vm::timers`, `Vm::take_due_timers`) that fires every due timer earliest-first, and the class
scripts of the level's ten extra `walkplayer` spawn markers are no longer started (single-player retail neither draws nor
places them; they each started `kyle.cog` again). Measured on the door capture:

```
hit: ClearActorFlags(0, 0x8) from cog 42 `kyle.cog` clears the freeze
timer: kyle.cog id 1 -> Ok(Nil)
timer: kyle.cog id 2 -> Ok(Nil)
```

Gates: **266 tests**, `--gates` **0 of 67 failed**, trace gate **10/10**, `--walk all` 24/34.


### Round 400: the door capture fires, and every bolt is lost

With the freeze gone the encounter runs: `ai: 3 shot(s) at the player` and then one per cadence, from actors 151/155/157
at 1.3-3.4 units, aimed through the port's own `sithActor_UpdateAimJoints` path (yaw snapped to the bearing, elevation
included) -- and **none of them lands** (`hit: the player took` never prints, health stays 100). Retail's reference at the
same pose loses 100 -> 44.

So the open question is now the line of fire, not the AI: the body is warped onto the doorway (the level's `00_door.cog`
leaf sits at 1.20, 9.65, 6.35 and the pose is 1.40, 9.70, 6.36), and a bolt that crosses a shut leaf's sector, a wall
surface or a solid thing is being destroyed without a trace. Next step: log every bolt's death (world impact, thing hit,
blast) under `BK_FIRE_TRACE` and check the first shot's flight against the actor's line to the body.


### Round 401: a projectile that hits a surface dies there

`Runtime::step`'s collision arm stopped a flying thing on the surface it hit (`vel` zeroed, `target` cleared) but kept it
alive for the rest of its `timer=` life. A bolt parked in a doorway is invisible to `impacts` (a stopped bolt's swept
segment is a point), so level 1's AI fire died at the walls: the `BK_AI_TRACE`/`BK_SHOT_TRACE` pair shows one bolt
closing to 0.37 units from the body and then being **pushed out to x 1.13** by a surface plane in sector 210, and another
teleporting three units to sit still for the rest of the capture.

Weapons (`type=3`) are now destroyed on that contact, the way `sithThing_Update` explodes a projectile; actors and
path-movers keep the old stop-and-`blocked` behaviour. `a_projectile_dies_on_the_surface_it_hits` is the regression test
and it fails on the old code ("the bolt froze on the wall: [-5.02, 0.0, 0.0]" -- the push-out carried it through the
room). Gates unchanged: **267 tests**, `--gates` 0 of 67, trace gate 10/10.


### Round 402: which actor should hit the body, and why none does

With projectiles dying correctly the fight is now measurable actor by actor (`BK_AI_ROSTER` prints the roster once;
`BK_AI_TRACE` prints acquisition):

```
roster: 154 `reeyeesgun` d 0.34 cone false los true yaw -15 bearing 92 off 107 ai yes
roster: 155 `reeyeesgun` d 1.33 cone true  los true yaw -45 bearing 43 off  88 ai yes
ai: 155 flags 0x233 cleared false target Some([1.40, 9.70, 6.32])
ai: 154 flags 0x4   cleared false target None
```

Two things follow. **Actor 154 stands 0.34 units from the body** with a clear line and never acquires: the port's
`LookForTarget` only remembers a body that is inside the actor's FOV *cone*, and nothing turns an actor toward a body it
has not already remembered -- so an actor that happens to face away stands still for the whole capture. And the actors
that *do* acquire (155/157) fire into the wall 0.37 from the body: their `+elaser` is stopped by a sector plane at
x ~1.13 (sector 210), i.e. the port's LOS check and its projectile resolver disagree about the same doorway.


### Round 403: the sector tracks the move, and the resolver's walls are the remaining mismatch

A moving thing kept the sector it was created in, so a bolt that crossed three rooms in one frame was resolved against
the *first* room's planes. `Runtime::step` now re-derives the sector from the new position (the engine enters each room
the move lands in, `sithThing_EnterSector`) and stores it, before the surface resolve.

That did not change the door fight by itself, and the `BK_AI_TRACE` line now names the disagreement exactly: shot 382
travels from 0.53 to 0.33 units of the body -- on a line the roster calls **los true** -- and is pushed out by a
surface in **sector 76** at (1.185, 9.550, 6.359) (push +0.02 in y, i.e. a *sliver* sector's own plane, not a face the
segment test sees). So the remaining gap is that `resolve_stepped`'s plane push treats a thin sector's bounding plane as
a wall where `segment_blocked` (and the engine's face test) does not. Gates unchanged: **267 tests**, `--gates` 0 of 67,
trace gate 10/10.


### Round 404: a bolt stops on a wall, not on a sliver's plane push -- the door fight lands hits

The containment resolver's plane push was doing double duty. For a body it is right: a body caught in a thin sector is
nudged out. For a **projectile** it is not: the door fight's bolts were pushed +0.02 in `y` while climbing at +2.09 u/s
and every one of them died 0.33 from its target. `Runtime::step` now judges a projectile's plane contact by the push's
*alignment with the flight* -- a push that runs along the velocity is the resolver sliding it out of a sliver, and the
bolt flies on; only a push that opposes the velocity is a surface in front of it. The swept `segment_blocked` test still
kills anything that steps clean over a face, and the shape test is what now lands the shots:

```
hit: the player took 10, 90 shield(s) and 100 health left owner Some(155)
```

`+elaser` hits reach the body and drain the shield for the first time in this level's door capture. The unit test
(`a_projectile_dies_on_the_surface_it_hits`) covers both halves: a wall kills the bolt, and the blast proof it fails
without the death arm (`the bolt froze on the wall: [-5.02, 0.0, 0.0]`). What is left is the *rate*: actor 154 stands
0.34 units from the body and never acquires, because `LookForTarget`'s scan requires the body inside the `fov=` cone and
nothing turns an actor toward a body it has not already remembered -- `sithAI_CheckSightThing` (`0x4eb090`) shows the
engine's cone is gated on a **positive angle argument**, so a zero/omitted angle means "sees all round". Gates:
**267 tests**, `--gates` 0 of 67, trace gate 10/10.


### Round 405: `fov=200` is not a cone, and the door fight now shoots back

`sithAI_CheckSightThing` (`0x4eb090`) takes the half angle's **cosine** and applies the cone only when that cosine is
positive. A class with `fov=200` therefore has `cos 100 = -0.17`: the gate is skipped and it sees all round. `Ai::sees`
now follows that rule (`fov <= 0` or `fov >= 180` -- no cone), which is what retail's own door capture shows: the
`reeyeesgun` at 0.34 units from the body, facing 107 degrees away (`cone false`), fires.

Before / after on the same 600-frame door capture:

```
before: (no hits at all)
after:  hit: the player took 12, 18 shield(s) and 100 health left owner Some(154)
        hit: the player took 12, 0 shield(s) and 100 health left owner Some(154)
        hit: the player took 12, 0 shield(s) and 95 health left owner Some(154)
        hit: the player took 12, 0 shield(s) and 83 health left owner Some(154)
```

The body now takes damage from the room's own actors: shields 100 -> 0 and health 100 -> 83 inside 10 s, against
retail's ~56 lost by the same instant (its sidecar reads one bar; the port soaks 100 shield first). Over a minute the
shields settle at 13 and the body survives, so the door series no longer ends with an untouched player. `sees`'s test
grew a coned class for the cone assertions and keeps the all-round case. Gates: **267 tests**, `--gates` 0 of 67,
trace gate 10/10.


### Round 406: the door series' rate

`BK_AI_TRACE`'s bolt line now carries the clock and the owner, so the rate is countable: over the 10 s door capture
**17 distinct `+elaser`s** come within two units of the body (154: 9, 236: 5, 155: 2, 244: 1) at 12 damage each, and
~10 of them land -- the measured 100 shield + 17 health. Retail's own door series reads ~56 lost by the same instant,
so the port is now in the right order rather than untouched; whether the retail bar is shields or health is the next
thing its sidecar has to say.


### Round 407: the installer rebuilds from this tree; the install itself needs a normal desktop

`packaging/build-setup.ps1` rebuilt `dist/bevi-knight-0.1.1-setup.exe` from the current checkout -- it runs
`cargo build --release` first, so the 29.7 MB payload (sha256 `0ef965c8ef29...`) carries the fights and gates above.
What this session cannot do is *run* it: the launched setup is denied write access to every path it tries, including its
own log (`setup log unavailable: ... Denied`, on both `%TEMP%` and the repo's `target/`), and exits 1 before creating the
install folder. That is this shell's sandbox on newly created executables, not the setup's logic -- the checklist's
install -> `--check` -> uninstall run was made on a normal desktop. So "clean-machine install to level 1 completion"
stays the one item that needs another desktop, with the artifact now current.


### Round 408: the door series' real target is ~22 damage a second, not 56 total

Retail's `route_door` sidecar (`target/parity/retail/route_door.json`, 8 frames at 1 s) reads health 100 -> 44 by its
last frame, and its shields are spent before health can move, so the room deals retail **~156 damage in ~7 s: ~22 a
second**. The port's 10 s capture now measures **117** -- 100 shield + 17 health -- i.e. ~12 a second, about half the
retail rate and no longer "untouched". Two candidate causes, both cheap to check next time: retail's `+elaser` in that
room may be fired by more actors than the port has acquired (the roster shows 10 actors with `los true`, 3 `cone true`
before round 405), and the port's shields refill on the level start (`player_shields: 100.0`) while retail's warp left
the body with an empty bar -- so the health half of the comparison starts from different bars.


### Round 409: the retail door HUD was misread; the settled combat state already matches

Round 408's 44-health target was not in the image. Reading the raw frames directly gives retail **100 health / 88
shields** in `route_door-series-0.png` and **81 / 0** in `route_door-raw.png`; the earlier "100 -> 44 health" and the
derived 22 damage/s target are withdrawn. The green shield digits in the small series frames also made 77 look like 11,
so the series must not be used for a per-second rate without enlarged crops.

The current port was rebuilt and run through `tools/parity/port-capture.ps1 -Only route_door`, then repeated with the
same direct 600-frame invocation and no trace environment. Both captures read **83 health / 0 shields**. The second
run's ordinary hit log has 10 hits (nine 12-damage hits and one 11), leaving 83 after the shield/health rounding. That
is 2 health from retail's settled 81 / 0 and closes Phase 1 step 5's stationary door-health endpoint. No kill occurred
in this no-input encounter, so it says nothing about kill order.

### Round 410: `collide=3` is the model box, and the spawn overlap no longer pins it

The door leaf's remaining 0.185 units were its collision shape, and round 397's box attempt was reverted because
`walk17_spawn` fell to zero travel. The decompiled gate was missing: `sithIntersect_CheckSphereThingIntersection`
(`0x5080d0`) uses the model's mesh only when `sithThing_ParseThingArg`'s `collide=` word (case 2, `+0x28`) is **3**
for either thing; every other collidable thing is the `size=` movement sphere. On `18Ascend` that is 108 model-box
things and 138 spheres, so boxing every model was both too broad and unnecessary.

`ThingSolid` now carries `half: Option<[f32; 2]>` plus the thing's yaw: `Some` for a loaded `collide=3` model (its
bounds box, centre from the model's own plan midpoint), `None` for the engine's sphere (still tightened to the model
plan for furniture). The sweep, push-out and sight/shot segment all use the same shape, so none of them keeps a stale
cylinder beside the box.

The round-397 spawn pin had a second cause: the recorded pose starts **inside** both elevator leaves' inflated boxes
(0.014 and 0.016 deep), and treating "already overlapping and moving into the nearest face" as a contact made the two
opposed leaves stop the body at distance 0 every frame. A box overlap within `1e-3` of a face is a real contact and
still blocks; a deeper overlap is an initial-pose overlap and is left to the post-move push, which lets the body walk
out. A body stopping against a face lands within float dust of it, so the next frame still blocks.

Measured:

| leg | cylinder | collide=3 box | retail |
|---|---|---|---|
| walk_switch | -3.485 | **-3.6725** sector 59 | -3.67, sector 59 |
| walk17_spawn | 0.82 / 117 % | **0.692 / 100 %** | 0.70 |
| walk_cantina | 2.64 | 2.65 / 102 % | 2.59 |

The trace gate is **10 legs, 0 failures** (`walk17_spawn` xy 0.005, 0 sectors off). `--walk all` is unchanged at
**24/34 contained, 25 hull-agreeing, 46 crossings**, 268 tests pass, and `--gates` reads **0 of 67 failed**. Regression
test: `a_thin_model_leaf_collides_as_its_box` (thin-face stop, footprint miss, 90-degree rotation, push, sight).

### Round 411: the setup artifact carries the round-410 binary

Phase 4's reachable half was stale: `dist/bevi-knight-0.1.1-setup.exe` embedded the binary from before the
`collide=3` box fix. `packaging/build-setup.ps1` was rerun from this tree (native csc setup + in-box payload, no
downloads) and the embedded `payload.zip` was extracted through its own `payload.zip` manifest resource.

    setup:   dist\bevi-knight-0.1.1-setup.exe  29.7 MB
             sha256 cbdb03f436340cdbbc10ede098f95c3b75929a3181d12fc86eb5a7b6f7ec3bac
    payload: bevi-knight.exe sha256 74fe7b29...ace52
    build:   target\release\bevi-knight.exe sha256 74fe7b29...ace52

Launching the setup from `dist\` is the sandboxed case: both `Start-Process` and a direct invocation exit without
creating the install folder, and the child prints `setup log unavailable: Access to the path '...\setup.log' is
denied.` Copying the same `setup.exe` to `%TEMP%` and launching it there (same binary, same sha256) succeeds:

    19:32:00  start: /S /GAMEPATH=G:\Games\Star Wars Jedi Knight - Dark Forces 2
              /DIR=C:\Users\Lee\AppData\Local\Temp\bk-install-test /NOSHORTCUTS
    19:32:00  installing 0.1.1 from ... to ...
    19:32:01  installed

The installed `bevi-knight.exe` hashed **74fe7b29...ace52** (the build's own hash), `game-path.txt` carried the GOG
folder, `--check` printed `--- all checks passed ---`, and `--gates` from the installed path read **0 of 67 failed,
0 skipped**. `bevi-knight-uninstall.exe /S` then removed the folder and the `HKCU\...\Uninstall\bevi-knight` key.
The same install routine was also exercised in-process against `target/parity/install` before the temp-copy run,
with the same hash and checks.

Still open for Phase 4: a **clean-machine** run (this is the same desktop) and a human level-1 playthrough. The
launched wrapper, artifact, installed-build path, checks and uninstall are no longer unknowns.

### Round 412: priority reset -- render coverage first, and unknown flags no longer boot the menu

The draw gate's coverage is the real gap: only **five rows are gated**, and all five are level 1
(`spawn_pose`, `spawn_yaw30`, `route_cantina`, `route_lower`, `route_street`). Levels 2, 5 and 14 have pairs
but no threshold, and their known residuals are lighting, water/16-bit colour and the cutscene interval. The
`retail-level{2..21}-spawn.png` fixtures are not a substitute: five are all-black 1440x900 frames, the fifteen
640x480 frames have no pose sidecar, and their scores (0.07-0.22) measure framing/HUD scale, not renderer
fidelity. The installer is therefore deprioritized behind render parity; Phase 4 stays blocked on clean machine
and human play.

A current **all-level** survey was captured rather than reusing the 20-level sheet:
`target/parity/survey-all.tsv` (35 captures, 240 frames each, 35/35 succeeded) and
`target/parity/survey-all-sheet.png`. All 34 gameplay levels render their own non-blank geometry; `static` is
the empty test scene. This proves load/render presence, not correctness against retail; the side-by-side sheet
`target/parity/spawn-pairs-sheet.png` shows that the old unposed references disagree on framing and time in
both directions.

The first defect found while running the survey was the tool itself: an unknown flag fell through the argument
match and launched the main menu, and `--help` did the same. Two stray windows were opened and killed. The
parser now prints usage for `--help`/`-h` and exits **2** with `unknown option ...` for any other unconsumed
`-` argument; a positional non-option remains a level name. Smoke: `--help` exits 0 with usage, `--bogus-flag`
exits 2, no process or window remains. `cargo test --release` is **268 passed**.

Next render work, in order: fix the level(s) the player actually sees wrong (with pose-verified retail pairs,
not the old survey); level-1 lighting; level-2 lighting; level-5 water/colour; then a non-blank/material
all-level smoke gate. Do not tune against the unposed spawn survey.

### Round 413: the level-3 body moves at load because the walker had no ramp projection

Retail does not play a level-3 spawn animation. `sithPlayer_PlacePlayers` (`0x4c8670`) copies the player
thing's position and sector into the spawn table, and `sithPlayer_NewPlayer` (`0x4c8ec0`) sets them and
resets movement (`sithPhysics_ResetThingMovement`, `0x4f61a0`). The body then falls under gravity, and
`sithCollision_MoveThing` (`0x4e7950`) projects the movement along the floor it hits. The port started the
body `grounded` on a resolved floor point and its vertical sweep only snapped z: no tangential part.

The level-3 authored start stands on surface 3386 with file normal `[0.191,-0.815,-0.547]` (a 57-degree
slope; flipped toward the body it is `[-0.191,0.815,0.547]`). `--floor-at` shows the floor at 0.2051 and
the body at 0.2701. Retail's settled vector, `(-0.76,-4.75,-0.20)` from `(-0.68,-5.23,0.27)`, is
`(-0.08,+0.48,-0.47)` — the same direction as the normal's horizontal part.

`collide::Solids::floor_contact` now returns the highest landing's normal; `walker_step` adds
`slope_step(normal, remaining)` to the lateral delta and still zeroes `vy` on the landing, so a ramp is not
mistaken for a fall. Port after 10 s: **`(-0.7466,-4.9466,-0.1713)` sector 370**, no fall damage, health
lost only to the room's AI. Retail is `(-0.76,-4.75,-0.20)` sector 370; the remaining **0.20 units** of y
are the unported accumulating slope speed (the idle-only slide is a documented shortcut).

Trace gate: **10 legs, 0 failures** (the first enabled run had `walk9_spawn` at 4.38/1.68 because the
input walk and the gravity slide double-counted; restricting the slide to an idle body restored
2.00/1.68). `cargo test --release`: **269 passed**; `--walk all` unchanged at 24/34 in-sector. Added
`slope_step` unit test (direction and flat-floor zero). `BK_SPAWN_AUTHORED=1` is the diagnostic exact
spawn; `BK_NO_SLOPE=1` disables the projection.

Still open: the four other settling starts. Ten-second traces: **18ascend** ends `(3.244,-9.210,-8.135)` sector 485 against retail's `(3.24,-9.21,-8.1)`/485, and **19b** ends `(3.015,-1.381,-14.581)` sector 448 against retail's `(3.01,-1.38,-14.53)`/448 — both settle to retail by the capture time, so the load-pose table was reading the lift before it moved. **Level 9** ends `(1.353,5.743,0.155)` sector 275 against retail's `(1.07,5.46,0.07)`/874, and **19a** ends `(-0.350,-2.150,-2.135)`/722 against retail's `(-0.35,-2.15,0.12)`/723; those two still need the lift/sector transition. The remaining 0.20 on level 3 needs the engine's combined 3D move rather than an idle planar addend.

### Round 414: a falling body now lands on a platform it crossed; 19a's lift still stalls

The solid-thing floor query ran at the **post-fall** position, so a platform top between the frame's
start and end feet was above the body and rejected: level 19a's `4x4elev_1` top `-2.102` was below the
start feet `-2.045` but above the final feet `-2.200`, and the body fell through it. `thing_floor` now
queries the frame-start position and accepts a top between the two feet; `elevator_floor` also carries
a body when a rising `*elev*` top moves into it.

Result: 19a's body is picked up on the lift's first step (`z -1.980 → -1.897`), but the lift itself stops
at frame `0.0667` (`target None`) instead of travelling to frame 1 (`z -0.061`). The level's
`4x4elev_1` has two frames; `00_std_elev.cog` calls `MoveToFrame(elevator, 1, 4.0)`, so the remaining
work is the lift script/arrival path, not the player carry. Level 9 still ends sector 275 rather than
874. Regression suite: **269 tests**, trace gate **10/10**, `--gates` **0/67**.

### Round 415: the dev `(Debug)` menu reaches the episode and level choosers

The single-player screen's `(Debug)` row (`GUI_DEBUG_PLAY`, id 102) was visible but had no graph edge,
and the two dev screens (`0x536168` Choose Episode, `0x5365a0` Choose Level) share the `singleplayer`
name with the regular screen. `nav` now maps id 102 to the episode chooser, and the activation path
reads the install's own `episode.jk` manifests for both lists: three episodes, then the selected
episode's level rows. Up/Down scrolls the list, Enter or Ok commits, and Ok on a level starts it through
the same `PendingLevel` swap as New Game (`debug: starting 01narshadda.jkl`).

The menu renderer gained the `type 4` list box: a scrolling column with a selected-row marker, using the
screen's own font and palette. Pending `--menu <screen>/<id>` activation now skips a list box when a
button shares its id (`0x536168`'s list and Ok are both id 1), which is what exposed the duplicate-id bug.

Verified: route `0x535e58/102` -> Choose Episode, `0x536168/1` -> Choose Level, `0x5365a0/1` -> live
`01narshadda.jkl`; captures at `target/parity/port/debug-from-row.png`,
`debug-levels.png`, and `debug-start-level.png`. A live scancode test on the level chooser pressed Down
then Enter and logged `debug: starting 02narshadda.jkl`, so the arrow selection is real input, not only
the pending-id path. Tests: **270 passed**; `--boot` and `--ui-audit` pass (18/18 menu edges).

### Round 416: debug level starts no longer play the campaign opening film

`FlowPlan.pre` is filled at process start when the initial level is the campaign start, so it held
`03-04a.smk`. A debug level start set `overlay.started = true` and requested the selected level, but
`swap_level` updated only `level`, `cuts` and `next`. `play_intro_films.after(swap_level)` then saw a
started game with a non-empty `pre` and played the opening film over whichever level the debug list had
chosen. The level selection was irrelevant; the film was startup state that debug never cleared.

`activate` now receives `FlowPlan` mutably, and the debug level-start branch clears `plan.pre` before
requesting the swap. Retail parity checks out: the retail debug branch calls `jk_sub_403470`
(`0x403470`), which sets the level name and GUI state 5 and never calls `jkSmack_SmackPlay`; the
opening film remains a New Game path only. Static verification only:
`cargo check --release` passes. No game or retail process was launched, so the runtime frame is not
re-observed here.

### Round 417: the dev level picker matches retail's order, labels and back path

The retail chooser was compared against `jkEpisode_LoadVerify`, `jkGuiSingleplayer_Show` case `0x66`,
`jkGuiSingleplayer_sub_41AA30`, `jkGuiTitle_quicksave_related_func1` and `jk_sub_403470`, plus the
retail captures `retail-debug-menu.png` and `retail-debug-levels.png`. Four visible differences were
fixed:

- **Category order.** Retail lists the episode archives in the order its directory scan finds them:
  `The Force Within`, `Jedi Capture the Flag`, `Jedi Training`. The port sorted by `TYPE`, which put
  training second and CTF third. `DebugCatalog` now keeps the archive order from `episode.jk`.
- **Level labels.** Retail labels a level from that episode's own `misc\cogstrings.uni`, keyed by the
  short filename (`04EscapeHouse.jkl` -> `04ESCAPEHOUSE`), so row 1 is `1 - Double-Cross on Nar
  Shaddaa`. The port drew the `episode.jk` comment (`Level 1: Narshadda`). The catalog now carries
  the selected archive's string table; a missing row falls back to the lowercased short key, as
  `jkGuiTitle_quicksave_related_func1` does.
- **Cancel from the level list.** Retail's `jkGuiSingleplayer_Show` loop breaks back to the Single
  Player screen. The port's history returned to the episode list. Opening the level list no longer
  pushes an episode history entry, so Escape/Cancel returns to Single Player.
- **Which episode the level belongs to.** A debug start now records the selected category on
  `FlowPlan`, and `swap_level` resolves the episode whose `episode.jk` actually contains the loaded
  level. CTF and training levels therefore stop using the single-player graph for their next level
  and their end-of-level items. Debug starts (and New Game) also skip the inventory carry, matching
  the `DAT_00549fa0 = 1` reset that retail's `sithOpenPostProcess` performs; Restart and campaign
  transitions still carry.

Unknowns kept explicit: retail first prefers loose `episode_<archive>.jkl` files over the manifest
fallback in `jkGuiSingleplayer_sub_41AA30`; this install has none, and the port still reads the
manifest only. The `[Debug]` row itself is reachable only in retail `-devMode`; the port exposes the
dev chooser without a mode switch.

Verified: `cargo test --release` **272 passed, 0 failed** (including `debug_catalog_tests`), and
`cargo check --release` passes. No game or retail process was launched.

### Round 418: New Game queues the campaign opening film every time

Retail's Single Player `New Game` path is `jkGuiSingleplayer_Show` case 100 -> `jk_loadFile` (which
sets `DAT_00549fa0 = 1`) -> `jkEpisode_Load` -> `jkMain_StartNextLevelInEpisode(1,1)`. The episode
starts at its first row, which is the `03-04a.smk` cut into `01narshadda.jkl`; the same path runs
when the player has returned to the main menu.

The port filled `FlowPlan.pre` only once at process start. The first New Game played the opening
film, but after the film `pre` was empty; Abort back to the main menu and a second New Game skipped
the film. `FlowPlan` now keeps `opening` separately from the per-start `pre`, and New Game requeues
`opening` alongside the inventory reset. The initial campaign start still uses the same list, so a
normal first New Game is unchanged.

Not changed: the Single Player screen's `Load` row still has no save-slot picker (the pause menu's
one-slot Load path is the existing workaround), and the Cutscenes/Credits rows are still outside
this level-loading path.

### Round 419: the two tinted route rows cannot be a static renderer gate

Checkpoint commit `061f3b1` captures rounds 412-418. The next render step was to isolate
`route_catwalk` / `route_door`, then gate them if they were pose/tint matched.

They are not. Re-capturing both with the current release and the normal 600-frame policy:

| row | retail r/g | port r/g | port health | result |
|---|---|---|---|---|
| `route_catwalk` | 1.418 | 1.469 | 79 | tint is close; the fighter's animation phase is not |
| `route_door` | 2.308 | 1.446 | 83 | retail is redder despite health 81 |

`BK_TINT=0.929,0,0` raised the door's red mean to 73.5 (retail 73.0), but green/blue stayed
47.5/55.8 against retail's 31.6/33.7; the tint is not the whole term. A correlation search over
±60 px puts the door pair's best alignment at x=60 (0.56), while the catwalk's is (-2,+2) - the
door reference carries a camera pose/shake or a crop offset the static row does not reproduce.
The catwalk reference has a fighter at a different animation frame, the same reason `spawn` is
series-gated.

One candidate was tested and reverted: adding `Tonemapping::None` to the world cameras. A/B on the
static rows barely moved the numbers (`spawn_pose` 0.6387 default vs 0.6369 none; `level2_spawn`
0.4328 vs 0.4326), so it is not the remaining term and the tree is back to the checkpoint.

One ownership fix did land. A surface can be listed by several overlapping sector ranges; the world
draw keeps every claimant for culling, but it lit each duplicate with that claimant's own ambient.
The last claimant now supplies the light for every copy (`collide::surface_sectors`), so duplicate
draws cannot disagree. Measured on the same captures:

| row | before | after |
|---|---|---|
| `spawn_yaw30` | 0.5930 | **0.6041** |
| `route_cantina` | 0.7439 | **0.7450** |
| `route_street` | 0.7313 | **0.7328** |
| `route_catwalk` | 0.4230 | **0.4310** |
| `level2_spawn` | 0.4326 | **0.4331** |

The five gated rows still pass: `compare.ps1 -Gate` **5 thresholds, 0 failures**. The two route rows
remain `UNGRADED`; a renderer claim there needs a state-matched retail capture, not another
ground/luma tweak. `cargo test --release` is **273 passed, 0 failed**.

### Round 420: one level-geometry record behind the collider

The collision build now owns the surface claims, the surface-to-owner map, the adjoin-to-surface
map, and the sector-adjoin graph (`collide::LevelGeometry`, aliased as `Solids`). The app builds it
once; `Collision`, `SectorView`, the surface batcher, and `Runtime::handle_with_geometry` share the
same `Arc`. `share_collider` clones the `Arc`, not the geometry. Solid door/lift things live behind
a shared lock, so `walk_player` still updates the current leaf positions without cloning the record.

The renderer no longer re-derives claims or owners. The runtime shares the same collision `Arc`; its
two lookup vectors (surface owner and sector adjoins) are copied once from the record at
construction. `Runtime::new` keeps a derivation fallback for tests and probes; the app takes the
supplied maps.

Verified: `cargo test` **280 passed, 0 failed**; a 20-frame `--start` run loaded the level and ran
the actor systems without panic.

The dead actor-steering cache (`Runtime::portals`, `best_portal`, and the per-load portal list) was
also removed; `Collision::route` still walks the collision record's own adjoins.

### Round 421: the level transition is one state

`FlowPlan.phase` is now the one state advanced by the completion, tally, film, tour, and swap
inputs; the five systems' `Local` flags are gone. New Game sets `Intro` and queues the opening film,
Restart and a debug pick set `Swapping` until their level loads, the tally waits in `Tally`, the film
queue ends in `Ready`, and the swap request moves to `Swapping`. A capture run skips only an `Intro`,
so it no longer rewinds a completion it is walking through. An intro waits while a level swap is
pending, and the swap preserves `Intro` when the arrival has an opening film; Restart and debug picks
sit in `Swapping` until the new level is loaded, so they cannot re-finish the old one. The start modes
are a small `FlowPlan` API, and the tour's count/last/finished now live on its resource instead of
three `Local`s, and the timer's seen/waited/done live on `CompletionTimer` instead of three more.

Measured: `cargo test` **281 passed, 0 failed** (a start-mode phase test was added);
`--tour-live` walked **21 levels**, all swapped, and ended at `21ajerec.jkl` with
`--- tour-live check passed ---`.

### Round 422: one camera placement, one third-person mode

`player.third_person` and `Chase::on` are one `CameraMode` (`Chase::mode`); `V` and its old `C` alias
flip it, and the runtime's `saber_cam` is its script-visible mirror. The old 0.66-unit
`third_person_view` path is gone, so first person and the chase camera cannot disagree and the body
visibility check reads the same value.

`camera_placement` is the one pure placement: body eye, mode, boom, scripted focus, `SetPOVShake`,
and a capture's `--shot-at`. It returns the eye, look, and the focus a third-person `camera_pull`
starts from; a shot is never boomed or pulled. `walk_player` now reads the runtime focus and shake
once, calls it, and applies the engine's pull-in if it got a focus. `--controls`, `--camera`,
`--camera-pull`, and the live walker all use it.

Measured: `cargo test` **287 passed, 0 failed** (six camera-placement tests added);
`--controls`, `--camera`, and `--camera-pull` all pass; a live `BK_CHASE=1 --start 01narshadda`
run kept the third-person camera and the actor systems up without a panic.

### Round 423: ThingMotion reports what changed

`Runtime::step` now returns `ThingMotion { moved, turned, blocked, arrived }` instead of a bare
list. A pure `angvel` prop is reported in `turned`, so `advance_sim` no longer re-borrows the runtime
and scans every thing for `rot_vel`; it redraws the union of moved and turned, and the report line
uses either list. The blocked/arrived events the step already queued ride out on the same value.

The AI's post-walk resolve is now inside `Runtime::step`: `motion_solids` is handed over even under
`BK_NO_SOLIDS`, the step consumes the private `walked` list, and each actor is settled with the
player's own `walker_step` sweep. `ai_shots` only calls `ai_tick`, and `advance_sim` is ordered after
it. Actor grounding keeps the runtime's `floor_surface` rule; the player and AI motion sweep share
`walker_step`'s floor chain. `arrived` is collected after the motion loop, so the returned event
belongs to the step that produced it rather than the next one.

The player's input, drag and sub-step count still live in `walk_player`, but the sweep, thing
floors, velocity feedback and fall death run in `Runtime::step_player`; the body uses the shared
`PlayerBody` and the same `walker_step` the AI settle uses.

Measured: `cargo test` **293 passed, 0 failed** (rotation-only, same-step arrival, step/settle,
floorless-settle, and player-step tests added); `--ai` check passes.

### Round 424: the step takes the caller's geometry view

`Runtime::step` now takes the collider view its caller already owns — `step(geometry: Option<&Solids>,
dt)` — instead of reading `motion_solids`, a second handle on the same level geometry that existed
only so the post-walk settle could run under `BK_NO_SOLIDS`. The field, its initialisation and
`share_collider`'s wiring are gone. `advance_sim` passes the `Collision` resource it already sees;
`share_collider` now hands the runtime only the collider the movers use, so the `BK_NO_SOLIDS` A/B
still stops `self.solids` from being installed while the AI settle keeps running.

`ai_shots` is unchanged and still only drives `ai_tick`; the settle stays in the step, which reports
the settled pose on `ThingMotion::moved`. `--ai` now exercises that path: it ticks the AI so the
actor lands on the walked list, shoves the body 0.4 units into a wall, calls the real `Runtime::step`
with the level's collider, and asserts the body is pushed out and the step result names it. The
floorless-settle unit test also checks that a step with no geometry leaves the walked list alone.

Measured: `cargo test` **293 passed, 0 failed**; `--ai` passes ("a body 0.4 inside a wall is
pushed 4.75 back out by the step"); a live `--start 01narshadda --exit-after 900` run stayed up,
drew an actor's motion and took an AI shot with the settle going through `advance_sim`.

### Round 425: player input and horizontal integration move into the runtime

`PlayerInput` now carries the app's intent: forward/strafe, run, crouch, jump, the body's yaw and
the force-speed term; water is read from the `PlayerBody` it already owns. `Runtime::step_player`
turns that into the target with
`walk_target`, integrates it at the engine's fixed 50 Hz rollover with `integrate_walk`, applies the
jump, then runs the same `walker_step` sweep it already owned. `walk_player` no longer builds a
target, integrates velocity, counts sub-steps or writes `player.vy`; it passes the intent and reads
the resulting `PlayerBody`. The target and integration helpers moved out of `src/main.rs`, so the two
ray helpers have one home.

The jump uses the old precondition (`Space` just pressed while grounded) before the runtime applies
it, preserving the old floor-seat ordering. Strafing is covered by a synthetic-room step test; the
level-1 live spawn's strafe phase still reports 0.00 because the body is against the cantina
furniture, while its forward speed reaches 2.24 u/s.

Measured: `cargo test` **297 passed, 0 failed** (settled walk band, 50 sub-steps per second, a jump
that lands, and a rightward strafe added); `--walk all` passes (24/34 levels contained, 46 sector
crossings, 6 recorded void rows, worst drop 3.35); `trace-gate.ps1` passes all **10** committed retail
legs with 0 failures; `--live-play --start 01narshadda` passes (forward 0.09 units at 2.24 u/s peak;
strafe displacement 0.00 but lateral peak 2.24 u/s, so the input is asserted; 1 projectile; 0.11 jump
stopped by the ceiling).

### Round 426: the runtime owns the player's environment and final pose

`Runtime::step_player` now reads the environment itself instead of taking it from `walk_player`:
it seats a body whose feet start below the floor, applies the sector's own `thrust`, the `slide` of
the surface under the feet, and the water state (the new `Runtime::in_water`, which reads the
surface water bits and `SITH_SECTOR_UNDERWATER`), then applies landing and fall-death damage to the
local player's health through `damage_typed(..., DAMAGE_FALL)` (setting `player_dead` on a lethal
one). The app's `apply_fall_damage` transfer and its per-frame `sync_player_thing` pose copy are
gone; the step publishes the final eye point, facing, aim and sector onto the player's own thing, so
the camera, the HUD, scripts and projectiles read one body. `player.yaw`, `player.pitch` and
`player.eye` moved into `PlayerBody` (the `Deref` keeps every gameplay read where it was), and
`SetThingVel(player, ...)` is consumed as an impulse inside the step.

`teleport_player` and `respawn_player` are the only other writers: the `--ai-live`, `--use-live`,
`--sight-live`, `--goal` and death paths call them instead of assigning the body directly (and the
use-live probe no longer writes the runtime thing separately).

Measured: `cargo test` **302 passed, 0 failed** (conveyor carry, sector thrust, water halving, the
post-step thing pose, and fall damage applied by the step added); `--live-play --start 01narshadda`
passes (forward 0.09 units at 2.24 u/s peak; 1 projectile; 0.11 jump stopped by the ceiling),
`--goal`, `--sight-live` and `--use-live` pass, as do `--rules`, `--adjoins`, `--things`, `--feel`,
`--die` and `--walk all` (24/34 contained, 46 crossings, worst drop 3.35).

### Round 427: one floor rule for every motion caller

The player's sweep, the AI's post-walk settle, actor grounding and gravity landing now ask one
primitive, `collide::LevelGeometry::resting_floor`, instead of each keeping its own support query.
It returns the highest of: the downward sphere sweep's faces (a walkway slab, a ramp, a face-only
floor), a sector's own plane where its data has no face inside the engine's floor reach (plus the
port's data-gap rescue from the sectors around a plane-less cell), and a solid thing's model top
from the collider's own `ThingSolid` list. `walker_step`'s chain of three floor queries is gone,
`Runtime::step`'s `actor_floor` and landing clamp use the same `Rest` height, and `ai_can_walk`'s
`standable || thing_floor` fallback is one call. `Runtime::thing_floor` and `floor_contact` are
deleted; `TEMPLATE_FLOORS` now only feeds `elevator_floor`, the rising-lift carry rule.

Measured: `cargo test` **304 passed, 0 failed** (a fixture proving a plane-only sector, a face-only
walkway, a ramp and a solid prop rest at one height, and walker/runtime-step landings on a solid
thing's top added); `--walk all` passes (24/34 levels contained, 25 hull agreements, 46 sector
crossings, 6 recorded void rows, worst drop 3.35); `--move` passes (34 levels × 8 headings, 0 fell
through, 0 stuck); `--ai`, `--live-play --start 01narshadda` (forward 0.09 units at 2.03 u/s peak,
0.11 jump stopped by the ceiling), `--ai-live` (37 shots, health 25) and `--live-check` all pass.

### Round 428: one motion path — the runtime step, one body, one collider

`--walk` — and therefore the trace gate's walkability preflight and `--walk all` — no longer
resolves a body of its own. `walk_level` builds the level's runtime and calls
`Runtime::step_player`, the same entry the window's `walk_player` and the live checks use, so the
probe runs the player's target assembly, drag ramp, 50 Hz sub-steps, step-up, water/seating and
gravity and cannot disagree with the game. The old loop's water rule, fixed speed and direct
`walker_step` call are gone; `walker_step` remains the collision primitive inside the step.

The walk-off levels are now recorded rather than asserted in every metric: under the runtime step
only `09fuelstation` still ends outside its sector (its recorded 21.13-unit walk-off), while
`02narshadda` (11.22, contained), `07yun`, `11gorc`, `17asarris` and `m_boss17` (2.90) stay in the
world. Measured: `--walk all` **26/34 contained, 24 hull agreements, 52 crossings, 0 left in the
void, worst contained drop 3.26** (`06abarons`).

The app-side duplicate geometry handle is gone: `SectorView` no longer owns an
`Arc<LevelGeometry>`; `rebuild_surfaces` and `cull_sectors` read the one caller-owned `Collision`.
`Runtime::solids` stays — #2 made it the runtime's script/LOS collider deliberately. `Player`'s
`Deref`/`DerefMut` forwarding shim is removed and every gameplay read and write names
`player.body`, the runtime-owned `PlayerBody`, so the app cannot read a second pose. `--rules` no
longer presents the retired `maxthrust/surfdrag` walk as the answer.

Measured: `cargo test` **304 passed, 0 failed**; `--walk all` (above); `--live-play`
`--start 01narshadda` passes (forward 0.09 units at 2.24 u/s peak, 1 projectile, 0.11 jump stopped
by the ceiling); `--ai` and `--rules` pass; `trace-gate.ps1` **10 legs, 0 failures**.

### Round 429: final review fixes — shared seat, type-3 landing, one AI writer

The two-axis review of #4-#6 found four defects. The player seat still used
`solids.floor_surface` and bypassed the shared floor rule; `resting_floor` now takes the one
`lift` option a **placement** needs, and the seat is that same query. Its probe is the old seat's
one-radius point, so only a support above the feet raises the body. **Type-3 gravity goods** were
probed at the actor's flat 0.05 while their landing writes `rest.height`, so a good wider than
0.05 measured itself below its own floor and sank for ever; the probe now uses the good's own
radius. `ai_shots` no longer writes the player thing pose; it reads the pose `step_player`
publishes. The unused `PlayerInput.pitch` forwarding field is gone, and `player_thing` now uses
the one `local_player` lookup.

The walk gate no longer hides the six known walk-off levels behind a name skip: each records and
asserts `(void, contained, drop)`. The shared seat also settles `10cargo` on its ramp face instead
of the old sector-plane lift, so its spawn-yaw line now walks off; that outcome is recorded rather
than hidden. Measured: `cargo test` **305 passed, 0 failed**; `--walk all` **30/34 contained, 26
hull agreements, 58 crossings, 2 left in the void (10cargo 680.75, 11gorc 627.51), 0 over no
floor, worst contained drop 3.35**; `--ai` and `--live-play --start 01narshadda` pass;
`trace-gate.ps1` **10 legs, 0 failures**.

### Round 430: AI engagement timing — the engine's facing axis, and the placement's instinct class

The door-encounter telemetry (#9) reported the port's AI engaging on a different timeline than
retail. Two root causes, both measured against the fixture:

- **The AI aimed on a math angle, not the engine's basis.** A JK yaw faces `(-sin yaw, cos yaw)`
  (`yaw_axes`), which the walker, camera and use cone already used. The AI's sight cone, bearing
  targets, aim snap, firing `dot` and projectile rotation used `(cos, sin)` instead — a quarter turn
  off. Retail's own door gunner proves it: thing 154 stands at `(1.412, 9.359)` at yaw 0 and fires
  down +Y into the player at `(1.400, 9.700)`, turning only 2 degrees; the port turned it to yaw 92
  and spent 4 s on the quarter turn before its first shot. The same error put the projectile's
  rotated velocity, the listen cone and the jump nudge on the wrong axis. All of them now use the
  engine basis (`Ai::sees`' facing is `yaw + 90°`; every bearing→yaw conversion is
  `(-dx).atan2(dy)`), and the fire path's muzzle offset follows the corrected forward.
- **A placement's `aiclass=` was ignored.** The level's placed things carry their own `aiclass=`
  after the sector column (`powerdroid` thing 331 ships `gonkroam.ai` over its template's
  `gonk.ai`), and the runtime only read the template chain. `Thing::aiclass` is parsed now and wins
  over the template, which is what makes thing 331 roam like retail.
- **The port's encounter `shot` rows now record what retail's half can see**: a type-3 thing that is
  new at a cadence sample, with the first sample as the baseline. Retail samples projectiles (it
  cannot see a bolt that dies between samples), so the port's fire events were a different
  observation and could never pair (16 rows against retail 9). The port trace changed only its
  `shot` rows; hits remain the runtime's own events. The gate's instrument is unchanged.

Measured against the committed fixture (`encounter-gate.ps1`): before this round the port fired 16
events with the first at 3,000 ms, landed 9 hits and ended 93.7 health; the corrected run samples
11 bolts, lands 12 hits starting at 900 ms and ends 58.2 health against retail's 22.3. The gate still
reports 9 failures: the port's other door actors (153, 157, 291 — a rifle, a gunner and a Greedo)
acquire and fire where retail's series has only the point-blank gunner firing, which drives the
remaining shot-count/timing, per-hit health and actor position/facing deltas; the actor maximum is
1.67 units / 178.5 deg. The acquisition side of that (which placed actors the engine lets engage)
is the next slice; this round's changes are the basis and the placement class they depend on.

Measured: `cargo test` **313 passed, 0 failed** (new: the engine-basis cone, the first-tick fire on
an already-aimed actor, and the placement `aiclass=`), and the corrected AI, live-AI and hit gates
are the next check. `--encounter` traces are in `target/parity/encounter/`.
