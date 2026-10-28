# Porting roadmap — the north star

`roadmap.md` lists *subsystems*. `gaps.md` lists *missing units*. Neither says **how to decide that
something is ported**, or **what to work on next when the build is not yet fun to play**. This file does.

## Why this exists

As of 0.1.1 the port reports 10 of 10 roadmap stages "done", 61 gates and 141 tests green, a signed-off
release, an installer, CI and a landing page — and level 1 still had fundamental playability problems.
The first retail reference frame (`tools/parity/`, gaps §5.9) looked like it found the cause at once:
the spawn view appeared to sit inside a cantina table that retail does not draw through the eye. The
harness then showed that was two other bugs, not a draw rule: the thing at the eye was one decor
prop drawn frozen (a `PlayKey` keyfile the loader skipped, now loaded) and the port's spawn camera sat
0.055 units below retail's because setup replaced the level's recorded pose with a floor-seated one
(fixed and now gated; the `spawn` pair went 0.23 → 0.63 SSIM). The lesson stands even though the first
suspect did not: the port's own gates had been passing.

The cause is structural, not a missing feature:

1. **Our checks compare the port with the port.** A gate asserts what we *believe* the engine does
   (`--feel`, `--walk`, `--hit`…). If the belief is wrong, the gate passes and the game is wrong. Only
   the retail binary can say "right".
2. **"Done" means "implemented and self-consistent"**, not "matches retail". The table in `roadmap.md`
   cannot distinguish the two, so it reads finished while the first room is not.
3. **Work went breadth-first.** Cutscenes, saves, installer, CI, releases and a site were built before
   one level was faithful from spawn to exit. Polish and packaging made the gap more visible, not smaller.
4. **Approximations live in the core loop.** Marked `ponytail` or listed in gaps §2: sector seed chosen
   by heuristic, sweep not sector-scoped, non-player things with no physics, a stand-in sky dome, no
   model back-face culling, no transparency sorting. Each is defensible alone; together they are what
   the player feels.

## North star

> **Level 1 played from the main menu to the exit, with the same sectors, positions, health, sounds
> and frames as retail for the same inputs — then every other level the same way.**

Everything else (installer, site, extra screens, multiplayer) is downstream of that sentence.

## Principles

1. **Retail is the oracle.** A claim is true when the retail `JK.EXE` produces the same observable
   result, not when our gate prints PASS. `decomp/` and `tools/ghidra/` give the code;
   `tools/parity/` gives the running game.
2. **Port the function, not the feature.** Name the engine function (`sithPhysics_FindFloor`,
   `jkPlayer_DrawPov`, …), cite its address, keep its inputs, constants and *order of operations*. A
   re-derived behaviour that looks right is how spawn, camera and sweep bugs got in.
3. **Depth before breadth.** Finish a vertical slice (one level, whole loop) to retail parity before
   widening. A new subsystem does not start while a slice-blocking discrepancy is open.
4. **Every deviation is on a ledger.** An approximation is allowed only with an entry saying what differs,
   how it was measured, and what retires it. No entry, no merge.
5. **Fix at the layer that owns the bug.** Classify a discrepancy first (below); do not patch the
   symptom in `main.rs`.
6. **Releases follow parity, not the calendar.** Tag when the slice's exit criteria hold, and say
   plainly in the note which levels are faithful.

## Definition of done: four tiers

"Done" is a tier, written on the row. A subsystem is never just "done".

| Tier | Meaning | Evidence |
|---|---|---|
| T0 Parsed | The data loads, nothing is dropped | `--audit`, unit tests |
| T1 Implemented | Runs in the live game | the existing 61 gates |
| T2 **State parity** | Numbers match retail for the same inputs: position, sector, velocity, health, bins, cog variables | a retail-recorded trace (`coords` and friends, or an instrumented run) diffed against the port |
| T3 **Visual parity** | Same pose gives the same picture within a stated tolerance | `tools/parity` shot pairs with a per-shot SSIM/diff threshold |
| T4 **Play parity** | A recorded input script reaches the same end state in both builds | replay script plus end-state diff |

Today almost everything is T1. As of the parity round, level 1's spawn pose and spawn frame are the
first T2/T3 rows: the pose comes from retail's own `coords` recording, and the frame is a thresholded
shot pair under `tools/parity/compare.ps1 -Gate`. Everything else is still T1/T0 on the roadmap rows,
and each row should name the tier it actually reached; most will read T1.

## How to port one thing (the repeatable loop)

1. **Observe retail first.** Capture the behaviour before writing code: shot pair, `coords` trace,
   console output, or an instrumented run. Save it under the repo as a fixture (derived numbers and
   screenshots, never retail assets).
2. **Find the function.** Locate it in `decomp/`; read callers and callees; note address and
   constants in `docs/formats.md`.
3. **Port it with its order.** Same call sequence per tick (the engine ticks at 50 Hz; thing update
   order matters). Same constants, same data flow.
4. **Compare, then gate on the retail fixture.** The gate loads the retail-derived number. A gate that
   computes both sides from our own code does not count.
5. **Record what still differs** on the ledger with a tier. Update the table.

## Triage: classifying a retail/port discrepancy

Put every discrepancy in one bucket before touching code; the bucket names the layer and the fix.

| Bucket | Typical sign | Owner |
|---|---|---|
| Data | wrong thing/surface/sector parsed or dropped | parsers (`jkl.rs`, `tdo.rs`, `key.rs`…) |
| Constant / formula | right shape, wrong magnitude | the ported function, cite its address |
| Order of operations | right at rest, wrong in motion or after a script | tick sequence in the runtime |
| Draw policy | world is right, picture is not (cull, sort, near plane, which things draw) | renderer |
| Approximation | a ledger entry exists for it | retire the entry |
| Retail quirk | retail itself looks odd | keep it, document it |

## Deviation ledger

Deliberate differences from retail that are allowed for now. Every entry names what differs, how it
is measured, and what retires it. The full sweep of the 38 `ponytail:` markers plus the gaps §2
partials is still to be written; these are the entries this round created or measured.

| where | what differs | measured | retires when |
|---|---|---|---|
| `walk_player`, idle-only attachment | a body keeps the level's recorded spawn z while idle, but once it walks the port settles it onto the floor; the engine keeps the surface attachment through movement, so a moving player's eye would be 0.055 lower at level 1's spawn than retail's | spawn `--shot`/`--camera-trace`: eye −1.0929 idle vs −1.148 walking (retail: −1.0929 both) | `sithPhysics_UpdateAttachedThingPhysics` — the attached-surface half of the movement port, Phase 1 step 3 |
| `spawn_point` | a recorded pose the port's body cannot stand in is nudged sideways and floor-seated; the engine places at the recorded pose and relies on its own attachment | the `walker: spawn ... was inside something` log line; `--walk all` | the movement port models the engine's body width and attachment |
| `compare.ps1 -Gate` | the port's HUD is inside its frame while the retail world crop has none, so a pair's ceiling is renderer/HUD-limited | verified pairs cap at 0.62–0.65 (spawn) and 0.72–0.74 (route); thresholds sit at 0.55 / 0.65 | HUD-free capture, or a HUD-masked metric |
| `shots.tsv` / `--shot-at` | retail's rendered camera pitch is ~0 while its `coords` reports PYR pitch 1.04, and `warp` cannot set pitch, so a pair cannot reproduce a pitched retail view; rows request pitch 0 | port pitch 0.25° already drops the spawn pair 0.63 → 0.59; `coords` PYR is in every sidecar | a retail-side way to set or read the rendered view pitch (none known); the port side already takes pitch |
| camera pinned vs retail camera on the body | a port capture puts the **player** at the recorded pose (`--warp`) but keeps the camera pinned to the row's eye (`--shot-at`), while retail's camera rides the body; where the body is moved by a script or the floor the two cameras diverge | same framing at the warp instant (sector and yaw match retail: 8/8 rows); no row shows a moved body yet | a capture that lets the port's camera ride the warped body once bodies stay put (attachment state and damage tint below) |
| damage tint | the engine tints the palette red as the player is damaged: `sithActor_DamageActor` adds `damage * 0.04` to the local player's red tint, `sithPlayer_Update` decays 0.4/s, `stdPalEffects_UpdatePalette` applies `byte += round(byte * tint * 0.5)`. Both halves are ported: the state (adds with the engine's clamps, decay, damage flash) and the draw (`src/palette.rs`, a `FullscreenMaterial` pass on the world camera that scales the finished picture's bytes by `1 + tint/2` — measured ×1.52 for a 1.0 red tint, identity byte-identical; the gated cantina pair rose 0.7315 → 0.7497 with it) | `route_catwalk`/`route_door` still score 0.45/0.42 against 0.72–0.74 for the untinted rows, because retail's own series turns red at ~5–6 s and the reference at ~10 s is saturated (r/g 2.0) while the port's 10 s capture lands 6 hits (≈66 damage, bursty) and draws a softer red — same direction, less than half the rate | the AI's engagement timing (Phase 1 step 5); no port-side renderer gap remains |
| warp attachment | after a warp the port's idle attach still needs a floor within one radius; at the catwalk pose there is no sector floor in that reach (and the catwalk itself is a solid *model* 0.145 below the body), so the body falls — while retail's `coords` holds z 2.80 for the whole capture. Whether retail's placement step keeps the pose or its collider has a surface the port's does not is not settled | `--warp` catwalk: support `None`, body falls and dies within 3 s; retail's `coords` stays at z 2.80 across 12 s | the floor/attachment step (`sithPhysics_UpdateAttachedThingPhysics`, `sithThing_AttachThingToSurface`) and solid-thing floors, Phase 1 step 3 |

## Phases

### Phase 0 — Reset the definition (do first, small)

- **Done:** `tools/parity/` is committed, the staged retail build takes `warp` (the dev-command patch
  is applied to the scratch copy), and `pwsh -File tools/parity/compare.ps1 -Gate` scores every pair
  against `tools/parity/thresholds.tsv` and exits non-zero. The gate is verified in both directions:
  the pre-fix spawn frame fails at 0.2284, the fixed one passes at 0.62–0.65. The gate now covers six
  pairs: the three spawn rows and three level-1 route rows (0.72–0.74).
- **Partly done:** `--shot-at` now takes a pitch. Measured: level 1's rendered camera pitch is ~0 even
  though `coords` reports PYR pitch 1.04, and the retail `warp` cannot set pitch — so pairs request 0
  and a retail-side way to set view pitch is still unknown.
- **Open:** HUD-free capture or a HUD-masked metric, so a pair is not mostly the port's HUD difference;
  and the status table (next bullet).
- **Trimmed:** do not relabel every `roadmap.md`/`gaps.md` row. Keep one status table for the ten
  roadmap stages plus the deviation ledger, and correct the stale rows; the long prose stays appendix.
  The tier relabel below is the remaining piece.
- Tier the ten roadmap rows (T0/T1/T2/T3) and merge the overlapping `roadmap.md` / `gaps.md` /
  `engineering.md` status summaries into that one table.
- Build the **deviation ledger** (started above): sweep the `ponytail:` markers and the gaps §2
  partials into it with owner, measurement and retire condition.
- Freeze: installer, site, new UI screens, version bumps. Releases pause until Phase 1 exits.

### Phase 0.5 — Retail trace v0 (unblocks Phase 1 steps 3–6)

T3 works today because a screenshot is a complete observation. T2/T4 do not: `coords` is a manual
console poll, `cogstatus`/`aistatus` output formats are not pinned, and no movement replay exists, so
"the same inputs produce the same position/sector/health" cannot be measured yet. Steps 3–6 all need
this, so it is its own deliverable rather than a hidden dependency.

Exit criteria:

1. A scripted walk on retail yields a parseable position + sector trace (poll `coords` on a fixed
   cadence, with `slowmo` if the console cannot keep up), and the format is documented.
2. The `cogstatus`/`aistatus` handlers are decompiled and their output formats pinned; one level's cog
   variables are captured through them.
3. The port replays the same scripted inputs and produces a comparable trace, with the fixed-step
   policy stated, so step 3's "within a stated epsilon" is measurable.
4. If console polling cannot carry health/velocity, hook `sithThing_SetPositionAndOrient` and the
   damage function with Frida and capture a per-tick CSV; the hook script lives under `tools/parity/`.
5. **Port-side pose warp — done.** `--warp "x,y,z,yaw"` puts the *player* (not just the capture
   camera) at a recorded pose: the pose is the camera eye, the body goes one template eye offset
   below it, and the sector is re-derived from the position, so the room's `entered` links, AI sight
   and hazards react to a body the way they do in the retail reference. `port-capture.ps1` passes it
   for every numeric row; the port-derived sector matched retail's `coords` sector on all eight
   level-1 rows (339 ×3, 254, 137, 156, 175, 60). It did **not** unblock the two upper route rows:
   retail's red there is the engine's damage tint, not a scripted effect (see Phase 1 step 2). It did
   expose a combat bug on the way: live projectiles were *collected as weapon pickups*
   (`collect_items` took any weapon-type thing within a unit), so bolts heading for the player vanished
   and the player was immune to close-range fire. `is_pickup` now excludes any thing carrying damage,
   and the port's body takes hits at the door.

Do not start Phase 1 step 3 without this.

### Phase 1 — Level 1 vertical slice (the real work)

Ordered so each step is observable and unblocks the next. Each item ends with a retail fixture and a
tier, not a checkbox.

1. **First frame — level 1 done.** The spawn pose is the level's recorded `walkplayer` pose
   (`sithPlayer_NewPlayer`, no floor re-seed), the eye offset is the template's 0.037 in level units,
   and an idle body stays attached to its floor instead of settling the 0.055 authored gap. Evidence:
   the `spawn` pair went 0.2284 → 0.62–0.65 SSIM (MAD 31.3 → 23.2), the equal of the warp-verified pairs,
   and `compare.ps1 -Gate` fails on the pre-fix frame. Remaining: the same spawn audit for the other
   levels, and the idle-only ceiling in the deviation ledger.
2. **The room as retail draws it — route set in place, draw fixes not started.** A first route set is
   captured: five level-1 `walkplayer` positions (cantina, three along the way up, the upper door) plus
   the three spawn rows. The player is warped to each pose, matching how retail's reference was taken.
   Three route pairs are gated at 0.72–0.74 SSIM; `route_catwalk`/`route_door` stay triage-only: their
   retail frames are red from **damage tint** (the body is being hurt where it stands — door health
   100 → 44 over the series). The tint's state is ported and tested; its world draw is not, because the
   palette multiply has to run after the camera's tone map (see the deviation ledger), and the catwalk
   pose also needs standing on a solid model. The renderer differences this step exists for — model
   back-face behaviour, transparency order, sky, distance light ramp, sprites — are still T1 and
   unfixed. Exit: the fixed pose set within threshold, including the two upper rows once the
   AI's engagement timing and the attachment state land.
3. **Movement.** Replace the unscoped sweep with the engine's sector-scoped collision; non-player
   thing physics (props, corpses, dropped items); step, slope and ledge behaviour. Exit: a recorded
   input script yields the same position/sector trace as retail, within a stated epsilon, across the
   level-1 route.
4. **Interaction.** Doors, lifts, switches, pickups through cogs; thing parenting; timing (`Sleep`,
   pulses) at engine granularity. Exit: cog variable/state trace matches at each trigger.
5. **Combat.** Weapon fire, hit detection, damage, AI sight and routing, puppet states. Exit: the same
   encounter plays out with matching health and kill order from the same script. *Partly done:* shots
   leave the shooter's own `fireoffset=` in level units (the port scaled it by ten, which put a
   Ree-Yees' muzzle 0.8 units ahead of a 0.07-radius body) and the `.ai` aim error jitters the aim
   vector the way `sithAI_RandomFireVector` does (the port used a yaw cone ±`error` right angles wide),
   both pinned by the `--muzzle` check and a unit test; projectiles are no longer collectable as
   pickups. Measured at level 1's door with the shot's owner traced: 6 hits on the body (72 damage in
   8 s, ~9 damage/s against retail's ~22) — all from one actor, while the rest of the room's fire lands
   on a Greedo standing in the lines. `BK_AI_TRACE` shows only a handful of actors ever see the body
   there. So the damage rate is now the same order as retail's and the gap is AI acquisition and
   positioning, not a missing firing rule — the engine's `FirstThingInView` is the cog-verb snapshot
   behind force pull/grip (`firstthinginview` here), and a segment-clearance firing gate modelled on it
   was tried, cut the room to one shot in 6 s, and was reverted. `BK_AI_ROSTER` measures the room: 56
   armed actors within 8 units of the body at the door, all with an AI class and a clear line, most
   facing >100° away. Two engine-backed corrections followed: an actor with a target keeps its **aim**
   on it while walking (`sithActor_UpdateAimJoints`) instead of facing its walk heading, and the chase
   standoff is the class's `Follow` second argument (`rydefault` 0.75, `grdefault` 2.0) instead of a
   fixed 0.25 that crowded friends onto the body. The same 10 s door capture now fires **26 shots and
   lands 6 hits** (72 damage, ~7 damage/s against retail's ~15.6) where it had 8 shots and none, and
   the damage tint appears in the captured frame; the pair still scores 0.412 because retail's
   reference is far more saturated (r/g 2.0 at 10 s) and the doorway geometry dominates. The gated rows
   moved with it: `spawn_pose` 0.6296 → 0.6543, `route_cantina` 0.7315 → 0.7697, `route_street`
   0.7369 → 0.7574, `route_lower` 0.7229 → 0.7174, `spawn_yaw30` 0.6231 → 0.6117. Two body-size bugs at
   the projectile layer then fell out of chasing the same red: a bolt was resolved as a 0.4-radius body
   and stopped on doorway jambs (now its template `movesize`), and a hit used a 0.6-radius body so the
   crowd absorbed every shot (now `movesize`, the engine's own radius, with the model height). The door
   capture now lands **6 real hits** and the catwalk takes its first hit; the pair scores still wait on
   retail's saturated tint because the port's fire is bursty — the engagement rate is the remaining gap.
6. **Exit.** End-of-level tally, transition, carried state (health, shields, bins). Exit: level 2 starts
   in retail's state.

### Phase 2 — Generalise

Run the Phase 1 harness on levels 2–5, then all 34. Replace "the walker reaches the end" with "the walker
trace matches a retail trace" for each. Fix by bucket, not by level. Add the retail-trace fixtures to the
repo so regressions are caught without a retail run.

### Phase 3 — Breadth, by shipped-content weight

Only now take the remaining units from `gaps.md`, ordered by how many shipped levels and scripts need
them (its §5 already ranks them): particles and effects, saber and force-power fidelity, aim joints,
track things, scope, ending screens. Each follows the porting loop above and enters at a tier.

### Phase 4 — Ship

Cut a release when: the exit criteria of every phase so far hold, the release note lists the tier per
level, and a clean-machine install plays level 1 to completion. Multiplayer stays out of scope, as now.

## Rules for the next agent or contributor

- Do not mark a row done without a tier and the fixture that earns it.
- Do not add a gate that derives its expected value from the port. Take it from retail.
- Do not widen scope (new screens, packaging, docs sites) while a Phase 1 discrepancy is open.
- When retail and our reading of the code disagree, retail wins and the reading is corrected.

## What the retail harness already answers

Reviewed `tools/parity/` (retail-capture, retail-lib, port-capture, compare, thresholds, shots.tsv).
It drives the GOG `JK.EXE -devMode` on a scratch copy, so it is the oracle this roadmap needs. It has
eight level-1 pairs: the spawn row and two warp-verified spawn poses (0.62–0.65), and five route poses
(things 167/169/170/171/174), of which three are gated at 0.72–0.74 and two are triage-only because
retail's reference there is red from the engine's **damage tint**: the port draws that tint faithfully
now, but its body takes no damage in those captures' window (retail turns red at ~5–6 s and is saturated
by ~10 s; the port's first hits land at ~15–20 s — AI engagement, Phase 1 step 5).

| Question | Answer from the harness |
|---|---|
| Can we read retail state without a debugger? | **Partly.** The dev console's `coords` prints position, PYR and **sector**, and the console list box is scraped for engine output (e.g. level-parse lines). That is enough for T2 on position/sector. Health, velocity, bins and cog variables are not exposed yet; polling is manual, not a per-tick trace (Phase 0.5). |
| Can we drive retail without a human? | **Yes.** Menus take synthetic mouse clicks at fixed coordinates; in gameplay synthetic keys work (keys held 300 ms or more, or the engine drops them). Walking scripts look feasible but are unproven. |
| Can we put retail at a chosen pose? | **Yes.** The staged `JK.EXE` is patched so the gated dev verbs register (`Set-JkDevCommandsEnabled`), and `warp x y z p y r` reproduces a pose that `coords` verifies (position delta ≤0.02 on all eight rows). Pitch/roll are not settable, and the rendered camera pitch is not the `coords` PYR pitch (level 1 prints 1.04 but renders at ~0). |
| Can we put the **port** at the same pose? | **Yes.** `--warp "x,y,z,yaw"` moves the player (body one eye offset below the recorded eye), and `--shot-at` pins the camera; the sector the port derives from the position matched the sector retail's `coords` reports on all eight rows. Body movement after the warp is still where the two engines differ (attachment state, and the damage the room deals). |
| Is there a retail frame to compare with? | **Yes.** F12 is the engine's own screenshot. The default 16-bit D3D path through dgVoodoo2 writes a 24-bit BMP; `-Software` writes a palettised PCX (palette-index comparison would be possible there). Frames are cropped to the world box (1170x731 today) and scored by SSIM/mean difference — triage, with per-shot floors in `thresholds.tsv`. |
| Can the comparison fail a build? | **Yes.** `pwsh -File tools/parity/compare.ps1 -Gate` exits non-zero on a missing or below-threshold pair. Verified: the pre-fix spawn frame fails at 0.2284, the fixed one passes at 0.62–0.65; a 0.03-unit eye-height shift drops a pair to ~0.24; the three gated route pairs sit at 0.72–0.74. |
| Is the harness safe to run routinely? | **With care.** It takes over the display (fullscreen, mouse captured; currently 1440x900 with `viewSize=6`), needs hands off, and has restore scripts for an interrupted run. Not CI-able: needs a retail install and a desktop session. |

## Getting the rest from Ghidra and REA

The exported decompilation in `decomp/` already settles part of this. Plan for the remainder:

**Found in `decomp/` (read, and used where noted):**

- `sithCommand_Startup` (0x4edc10) registers `players`, `coords`, `trackshots` and a few others always, and
  the rest **only if `(DAT_008ee66c & 0x100) != 0`**. That is why `-devMode` gives `coords` but not
  `warp`. The gated set is the useful one: `warp` (`sithCommand_Warp` 0x4eea40: `warp x y z [p y r]`,
  finds the sector by `sithIntersect_IsSphereInSector`, then detach, exit sector, set position, enter
  sector), plus `cogtrace`, `coglist`, `cogstatus`, `aistatus`, `ailist`, `invul`, `slowmo`, `fly`,
  `jump`, `activate`, `disableai`, `notarget`, `disablepuppet`, `matlist`, `memdump`.
- So retail can already print **cog and AI state** (`cogstatus`, `aistatus`) and run in slow motion
  (`slowmo`, bit `0x400` read in `sithTime_Advance`), which goes a long way toward the per-tick state and
  replay-determinism questions without instrumenting anything.
- `jkDev_TryCommand` (0x41fd60) looks commands up in a hash table after an XOR-`0xc5` decode of the name,
  a second, obfuscated command set. Worth dumping its table with REA.
- The same `0x100` bit also gates the debug key bindings (`sithControl_RegisterControlFunctions`,
  `RegisterKeyboardBindings`) and changes death behaviour in `sithPlayer_KillPlayer`.

**Resolved with REA + Ghidra on a scratch copy of `JK.EXE` (sha256 `12e8c8c3...efe44`, x86, Ghidra 12.1.4):**

1. **What sets `0x100`: nothing at startup, only cog verbs.** `0x8ee66c` has 30 references. The writers
   are two tiny functions Ghidra did not turn into procedures (hence absent from `decomp/`): `0x506a90`
   (OR a mask in) and `0x506ab0` (AND it out). A registration table at `0x5057c0` binds them to the cog
   verbs `setdebugmodeflags` / `cleardebugmodeflags` (string table at `0x54dcfc`, which also holds
   `getdebugmodeflags`, `bitset`, `bittest`). There is no command-line switch, ini or registry path.
2. **Why a cog cannot help.** `sithCommand_Startup` (0x4edc10) has one caller, `0x4d9e14` inside
   `sithConsole_Startup`, so command registration runs once at engine start, long before any level cog.
   Setting the flag later does not register `warp`.
3. **The enabling patch (scratch copy only).** At VA `0x4edc99` the engine does
   `mov eax,[0x8ee66c]; add esp,0xc; test ah,1; jz <skip registration>`. The six bytes of the `jz` are
   `0f 84 b8 01 00 00`. Replace them with six `90` and every gated command registers. The byte pattern
   `a1 6c e6 8e 00 83 c4 0c f6 c4 01 0f 84 b8 01 00 00` occurs **exactly once** in the file, at file
   offset `0xed099` (jz at `0xed0a4`). `Set-JkDevCommandsEnabled` (`retail-lib.ps1`) restages the
   install into `target/parity/retailcopy/`, verifies that unique site and NOPs it; the real install is
   never touched. **Confirmed:** the staged build now takes `warp`, and `coords` verifies the posed
   camera to a 0.005-unit position delta.
4. **Screenshots.** `jkGame_Screenshot` (0x4025e0): on the software path it converts the framebuffer to a
   bitmap, swaps in `stdDisplay_GetPalette()` and calls `stdPcx_Write`. So the F12 PCX is **8-bit indexed
   with the display palette**: palette-index comparison is possible, not just SSIM. The 16-bit/3D path
   (the harness default through dgVoodoo2) writes a 24-bit BMP instead; `-Software` still produces the
   indexed PCX for a future palette-index metric.

**Not yet done (needs a retail run, available but hands-off and screen-taking):**

- Confirm `cogstatus`/`aistatus`/`slowmo` print through the patched build, and record their formats.
- Decompile the `LAB_` handlers for `coords` (0x4ee960), `cogtrace` (0x4ee170), `coglist` (0x4ee2f0) to
  pin their output formats for parsing.
- Frida/`observe_native_calls` hooks on `sithThing_SetPositionAndOrient` and the damage function for a
  per-tick CSV (position, velocity, health). `slowmo` plus a fixed key script may make this unnecessary.
- Palette-index diff: have the port emit an indexed frame, or quantise through the retail palette.
- Route poses beyond the first set: five `walkplayer` eyes are captured (three gated, two triage).
  More poses are cheap now — the port warps its player too (Phase 0.5 item 5, done) — but the two
  triage rows stay ungraded until the AI's engagement timing and the attachment state land.

Still open after the review:

- **Per-tick state.** `coords` is a manual poll. The unlocked `cogstatus`/`aistatus` commands may cover
  script and AI state; health, velocity and bins likely still need a hooked trace. Phase 0.5 owns this.
- **Pitch.** `--shot-at` takes a pitch now, but retail's rendered camera pitch is not its `coords` PYR
  pitch (level 1 prints 1.04 and renders at ~0) and `warp` cannot set pitch, so pairs request 0.
  Setting a genuinely pitched retail view is still unsolved.
- **Player pose.** `--warp` puts the port's player at the row's pose (Phase 0.5 item 5, done), so
  scripts and AI react to a body as they do in retail's reference. Two engine gaps still differ after
  the warp: the port re-probes the floor each idle frame instead of keeping the attachment state
  (retail's catwalk body stays 0.145 above the catwalk model; the port's falls), and the damage that
  reddens `route_catwalk`/`route_door` in retail does not land inside the port's
  10 s capture — the AI engages late. The tint itself is ported on both halves now (state and the
  fullscreen palette draw); the timing gap is Phase 1 step 5. Both are on the deviation ledger with
  their measured symptoms.
- **Input replay determinism.** Retail frame timing is not ours; T4 needs fixed-step or a tolerance, and
  the harness has no replay of movement yet.
- **Better than SSIM.** Confirmed the PCX is palette-indexed on the software path (above); what remains
  is the port emitting indices. The gate's SSIM floors are the triage version.
- **Renderer tolerance.** Retail's renderer and Bevy will never match pixel for pixel. Per-shot floors
  now live in `tools/parity/thresholds.tsv` (0.55 for spawn against 0.62–0.65, 0.65 for route against
  0.72–0.74); tighten them as renderer parity improves, never below the verified-pair noise.
- **Fixture hygiene.** Keep traces and numbers in the repo; keep retail screenshots out of any
  GPL-licensed distribution unless the existing NOTICE terms cover them.
