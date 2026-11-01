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
| T1 Implemented | Runs in the live game | the existing 64 gates |
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
| `walk_player`, idle-only attachment | a body keeps the level's recorded spawn z while idle, and the same rule is now used by the **walking** path (`walker_step` lands at `floor + GetThingHeight`, round 12), so a moving player's eye no longer drops 0.055 below retail's | walk trace: z delta 0.055 -> 0.034 on `walk_cantina`; spawn `--shot`/`--camera-trace` now agree | the attached-surface half of the movement port (`sithPhysics_UpdateAttachedThingPhysics`, Phase 1 step 3) |
| floor search scope (`floor_under`) | **The search is global, not sector-scoped.** `sithPhysics_FindFloor` (`decomp/004f5550`) searches the body's sector and the sectors its adjoins lead to; this port's data does not tile (a walkway slab can sit in a sector nothing below it links to), so `floor_under` scans every face in the world within reach. That finds floors the body's room does not reach: level 5's body lands on a **downward-facing slope of the sector above** (surface 342, owner sector 33, normal (0.44,0.44,−0.78)) below its own sector 35 floor (0.1001), and walks a plateau at z 0.1703 while retail descends to the room across sectors 33/34/24. The engine's own rule is front-facing only (`sithCollision_SearchForSurfaceCollisions`: `dot(search_dir, normal) < 0`) | `BK_FLOOR_TRACE` names every landing below the body's own sector floor (walk5: surface 342/owner 33 every frame; walk6: surface 2837/owner 316 once). Four candidate fixes were measured and reverted — excluding downward normals (`walk5` 3.70→3.39, `walk6` 1.06→0.85 **broken**, 2 tests failing), the same exclusion at −0.5 (identical), orienting faces from the sector's hull plane (`room_normals`: identical to the exclusion), rejecting a landing below the own sector floor (`walk5` 3.70→3.39, walk6 kept), and preferring the engine's one-hop sector scope with the global scan as fallback (`walk5` 3.43, walk6 kept, all other legs unchanged). The shipped gate is **14 of 15** with `walk5_spawn` the failure | **retired when the non-tiling fallback can be removed**: either the level data's sector tiling is verified for the affected levels (so the search can be sector-scoped outright), or the landing probe distinguishes a *reachable* floor from a neighbouring room's slope by polygon/sector reach rather than by normal — the four measurements above are the acceptance table, and `walk5_spawn` must pass with `walk6_spawn` still passing |
| `spawn_point` | a recorded pose the port's body cannot stand in is nudged sideways and floor-seated; the engine places at the recorded pose and relies on its own attachment | the `walker: spawn ... was inside something` log line; `--walk all` | the movement port models the engine's body width and attachment |
| `compare.ps1 -Gate` | the port's HUD is inside its frame while the retail world crop has none | round 103: the world-band metric measures the HUD term at 0.07–0.09 of a level-1 pair's score (spawn 0.6246 → 0.7128, route_cantina 0.7279 → 0.7877); the full-frame floors stay as they were and all six pass | **retired round 103**: the HUD-masked metric exists (Phase 0's open bullet) and `thresholds.tsv` accepts a world-band floor as its optional fourth column, so the asymmetry is measured and separated instead of capping the gate |
| `shots.tsv` / `--shot-at` | retail's rendered camera pitch is ~0 while its `coords` reports PYR pitch 1.04, and `warp` cannot set pitch, so a pair cannot reproduce a pitched retail view; rows request pitch 0 | port pitch 0.25° already drops the spawn pair 0.63 → 0.59; `coords` PYR is in every sidecar | a retail-side way to set or read the rendered view pitch (none known); the port side already takes pitch |
| camera pinned vs retail camera on the body | a port capture puts the **player** at the recorded pose (`--warp`) but keeps the camera pinned to the row's eye (`--shot-at`), while retail's camera rides the body; where the body is moved by a script or the floor the two cameras diverge | same framing at the warp instant (sector and yaw match retail: 8/8 rows); no row shows a moved body yet | a capture that lets the port's camera ride the warped body once bodies stay put (attachment state and damage tint below) |
| damage tint | the engine tints the palette red as the player is damaged: `sithActor_DamageActor` adds `damage * 0.04` to the local player's red tint, `sithPlayer_Update` decays 0.4/s, `stdPalEffects_UpdatePalette` applies `byte += round(byte * tint * 0.5)`. Both halves are ported: the state (adds with the engine's clamps, decay, damage flash) and the draw (`src/palette.rs`, a `FullscreenMaterial` pass on the world camera that scales the finished picture's bytes by `1 + tint/2` — measured ×1.52 for a 1.0 red tint, identity byte-identical; the gated cantina pair rose 0.7315 → 0.7497 with it) | `route_catwalk`/`route_door` still score 0.45/0.42 against 0.72–0.74 for the untinted rows, because retail's own series turns red at ~5–6 s and the reference at ~10 s is saturated (r/g 2.0) while the port's 10 s capture lands **0 hits** at the harness invocation (health 100; the 15-hit figure this row used to cite is not reproducible — see the `sithAIAwareness` row) so the frame stays un-tinted where retail's saturation is what the pair sees, and the tint **state** is now proven exact against the EXE's own constants (`0x522e48` = 0 floor, `0x522e4c` = 1.0 red/green ceiling, `0x522e58` = 0.5 blue ceiling, `0x5230b4` = 0.04 per damage point, decay 0.4/s) - so the two rows' remaining difference is **content**, not tint | the AI's engagement timing (Phase 1 step 5); no port-side renderer or tint gap remains |
| `sithAIAwareness` propagation | **ported (round 110)**: firing raises an event at the shooter (`CreateTransmittingEvent`, `0x4f2a90`, strength 4.0), and `ProcessEvent` (`0x4f2c30`) records it per **sector and side**, recursing through the adjoin graph with `next = strength − link->attn` and stopping under the floor `0x52315c` = 0.00277. `Runtime::sector_adjoins` pairs each opening through its adjoin `mirror` rows and takes the mirror row's `dist` as the attenuation; `transmit_awareness` walks it, `awareness_at` is the reader, and an actor whose room and side hold a fresh record walks to the shooter — reinforcements converge on gunfire, as the engine's do | `--adjoins` checks level 1's own graph: sector 0 → 13 costs **1.13**, a shot's 4.0 crosses it, a below-floor event crosses nothing and a side's record stays its own. In the door capture 16.5k actor-reads happen, but awareness only *walks* an actor toward the fight; firing needs sight | — (implemented; the remaining acquisition gap moved to the `LookForTarget` row) |
| AI turning | a body turns at `maxrotthrust / surfdrag` — 23 deg/s for a Ree-Yees gunner, 60 for a human — rather than the template value read directly (a rate) or a snap. The engine's `ApplyDrag` decays angular velocity the way it decays linear velocity, which is why the division is the reading; the angular integration itself is not recovered. **Round 111 adds the acquisition half**: an idle actor with nothing in sight runs its `LookForTarget` instinct (interval = the shipped first argument, 500 ms) and takes the player as last-seen when it is inside its own `sightdist` with a clear line, so the body turns at the same clamp instead of snapping | door capture at the deterministic 10 s clock: **46 shots, 15 hits with the scan; 31 shots, 2 hits with `BK_NO_SCAN=1`** — the room's engagement is reproduced, and the control shows what a facing-blind idle actor does. The five gated draw pairs improved when the capture clock was fixed (spawn 0.6246 → 0.6602, route_cantina 0.7279 → 0.7381 …) | `sithAICmd_LookForTarget` recovered (round 210: handler 0x50aa80, the acquisition chain is `sithAI_FirstThingInView`'s horizontal+vertical FOV cone → `sithAI_CheckSightThing`'s range−radius/facing/collision test, with an awareness event of strength 3.0 on acquisition), and the port's scan now uses the class's own view cone (round 211: `Ai::sees` in the stand-in instead of `range <= sight && clear`; the new headless test fails on the old condition and passes now). Still approximated: one `fov` number instead of the engine's separate horizontal/vertical limits, the thing origin instead of the model-insert eye point, no target-radius term, and no `sithAI_CanDetectSightThing` re-detect for a lost target; the class layout is recovered (round 214: defaults `maxstep 0.5`, `sightdist 20`, `heardist 10`, `accuracy 0.5`, `fov` stored as `0.5·fov`'s trig, and `wakeupdist` is a key this port keeps but never reads) — and `sithThing_UpdateMove`'s angular branch for the turn itself |
| non-player thing gravity | **ported for the shipped gravity goods (round 246)**: a created thing starts with its template's `physflags=` word (the engine copies it; `spawn_at`/`fire_with_spread`/`explode` all set `0` before), the mover applies the level's gravity to `type=weapon` bodies whose `0x1` bit is set — the `_debris` family (`0x201`; `0x40000` halves it, attached bodies excluded) — and the fall stops on the highest floor plane under the body across `sectors_around`, accepting only planes at or under where the step started so a lip it fell past is not a floor. **Actor gravity is not ported** (the walker owns the player and `ai_tick` the actors, so an actor walking off a ledge does not fall), the stop is a plane query rather than the engine's swept `SearchForSurfaceCollisions`, and the body rests with its **origin** on the plane instead of `GetThingHeight` above it (zero for the shipped shrapnel, `shrp_1.3do` insert `[0,0,0]`) | `--things` drops a `CreateThing`'d `shrapnel_1` chip over a real sector centre: it falls **0.10** to a resting plane with velocity 0 and a sector still around it; with the gravity arm disabled the chip reports `falls 0.00` and the check fails (`the debris chip did not fall: z 6.299998 -> 6.299998`). `--soak-all` 35/35 clean (0 bodies outside), gate sweep 0 of 65 failed | actors get the same fall through `ai_tick`'s walker, and the stop becomes the walker's swept floor query |
| player walking | the base speed is **measured from retail**, not derived: the engine's `sithControl_PlayerMovement` thrust carries a `thing + 0x144` term no recovered template parser produces, so `maxthrust / surfdrag` = 0.667 was 40% under. `Rules::walk` is 1.2 u/s (the clean `walk_street90` leg measures 1.07-1.23), `Rules::speed` keeps the engine ratios (run 2.0, crouch 0.5, back 0.5, strafe 0.7, water 0.5) and plain forward runs by default (shift walks), as retail does | **`walk_street45` now matches retail**: 2.21 units over 2.4 s of held forward against retail's 2.20 (0.92 vs 0.92 u/s), 0 of 5 sector mismatches, 0.60-unit max delta (sample timing); `walk_street90`/`walk_street` stop at 3.265 = retail's 3.27, sector 155 | the `thing + 0x144` term is recovered and the derivation replaces the measured constant |
| first-person weapon sway | `jkPlayer_DrawPov`'s sway is now drawn: `jkSetWaggle`'s three amplitudes (degrees) and its phase rate, the phase advanced by the distance the body actually moved (`thing + 0x264`, capped at 0.02 a frame) and the angles scaled by the body's speed over `maxvel` (`thing + 0x21c`; `_actor` ships `maxvel=1.0`), applied as a rotation of the weapon about the eye. What still differs: **`rdMatrix_BuildRotate34`'s composition order is not recovered**, so the three angles are composed yaw-pitch-roll with the port's own sign, and the `thing + 0x168` eye offset under the rotation is still zero (nothing in the dump writes it) | the formula is pinned by unit checks on the bryar's own firing waggle (`10.0 7.0 0.0`, rate 350): 10°/7° at full speed, phase +7° a frame at the 0.02 cap, and a full waggle moves an authored corner at most **0.011** level units — the old translation moved it by the amplitude itself (10 units). The bryar's clip and model are measured by `--weapons` (`bryvpst1.key`, 13 frames; `bryv.3do`, 15 materials) | the composition order is read from `rdMatrix_BuildRotate34`'s disassembly, or a QA-window frame pairs a swayed weapon at a known phase |
| AI chatter (`talk`, 198 classes) | the handler's timing, chance and mood table are retail's (round 235), but the mood sound goes out through the port's **one-shot 2D request pipeline** with the actor's position for distance/facing attenuation — not the engine's mixer channels and 3D buffers, the same ceiling every script sound already has — and the handler's upstream one-shot bit is not modelled | `--sound` proves the data (5 of 5 mood events across 102 `.snd` classes; 72 of 73 `soundclass=` classes on level 1 archived, the 73rd a retail gap it prints); unit checks pin the interval/chance and all seven mood branches, and the tick check asserts the queued wav, its position and the interval | the mixer grows channel/3D-buffer management and the AI one-shot state is read from `sithAI.c`'s handler preconditions |
| `dodge` | both mechanical paths are ported (rounds 239–240) through the engine's own event dispatcher (round 241): the **damage** path and the **shot-passing** path (the player's `typeflags & 0x200000` projectile pre-sweep, the actor's own sight test of the shot, the random 45–90° sideways step). Its **event-2 branch is unreachable**: the six `sithAI_EmitEvent` call sites raise only 1, `0x100`, `0x400`, `0x800` and `0x1000`, so nothing can reach it — the mask table keeps the bit because the handler does. The port walks both steps at the actor's own walk speed instead of `SetMoveThing(..., 2.5)`, and the five-unit pre-sweep uses the actor's `movesize` against the shot's line rather than the engine's full collision search | damage path: gate (walking bit `0x1`), step (`arg0`), direction and 1000 ms cooldown unit-checked; shot path: the shot's thingflags, the actor's sight/`sightdist`/AI-bit gates, the class step and the 90–135° sideways direction unit-checked; either call removed -> its checks fail | `SetMoveThing`'s speed parameter is carried by the port's AI walk, and the sweep uses the collision search instead of a line test |
| `sensedanger` | both paths are ported: the **instinct-update path** (round 236: on sight, awake -> alert, `fear`, the 3.0 awareness burst, the player target) and the **damage path** (round 241, through `sithAI_EmitEvent`: `surprise` `0x57`, the attacker's parent as the target, `flags & !0x4 \| 0x800`, no awareness event). Its event-2/event-4/`0x400` branches are unreachable or outside the mask: nothing emits 2 or 4, and `0x400` is not in its `0x7` mask. The one approximation left is the target refresh — `sithAI_sub_4EAD60`'s `CheckSightThing` state is the port's `sighted` test plus the last-seen slot | the awake-bit/alert-bit transition, the `fear` row, the player target and the **3.0** awareness burst are unit-checked; the damage path's `surprise` row, flag swap and attacker-as-target are unit-checked; gates removed -> the checks fail | `CheckSightThing`'s own state is modelled where it differs from the port's sight test |
| AI detection (`CanDetectSightThing`) | the engine's detection **chance** is ported exactly from the `.rdata` constants (round 242: range falloff, awake bit, still/crouched physics targets, hidden flags), but the port rolls it only when an actor **first registers** the player and then keeps the deterministic line test, so an engaged target is never lost to a bad roll; the engine re-rolls on every target refresh. The engine's darkness term needs the target's sector light, which this build does not keep, and its second falloff gate reads a byte at `thing + 0x2a8` whose meaning is unrecovered | the whole chance curve is unit-checked against the retail constants, and acquisition is checked at a 0.2 roll over 200 ticks; gate removed -> the acquisition check fails | sector light is parsed and the roll moves to the refresh path, or a QA-window pair shows the port's acquisition timing matches retail at the recorded vantage |
| sector thrust arm | the engine gates the acceleration on the body's `mass != 0` as well as the `0x400000` physflag (round 237); the port's global thing loop now honours the flag, and `walk_player` applies the arm to the player (x/y through the walk direction, z through the vertical velocity) without the flag gate — the shipped player templates do not carry `0x400000`, so that half shows only if a script sets it | m5 soak `outside` 4 -> 0 with the flag gate; `a_thrust_sector_lifts_bodies_except_the_ones_flagged_against_it` | the physflags/mass pair is read off the thing (`+0x1d0`/`+0x204`) for every body, which is the same prerequisite as the gravity arm |
| particle clouds (`particle=` things) | the `PAR 1.0` clouds are loaded and **drawn** (round 244: one camera-facing `Sprite3d` quad per element at `element + INSERT OFFSET`, yaw-rotated, `SIZE` across, billboarded; `--par` probes all three shipped files). Still open: the shipped **`type=particle` templates are not `.par` clouds** — `+force_blind`/`+dustcloud`/`+sspks_saber` carry `material=`, `rate=`, `elementsize=`, `range=` and the engine's `sithParticle_Initialize` generates their elements, which nothing draws yet; the `thingflags` cel-cycling (`0x20`) and grow (`0x1`) arms; `sithParticle_New`'s spawn path; the cloud's pitch/roll and `angvel` spin (built from yaw at spawn); and the material's own effect state — `00teleport.mat` is **texture-less** (5 cels, 0 bitmaps), so the port draws the quads untextured while retail draws the material's effect | `--par` asserts each shipped cloud against `rdParticle_LoadEntry` (completeness, every element cel inside the material's cel count; `test.par`'s missing `test.mat` reported as a retail gap), `--audit` reports `par 3 3`, and the element-offset/yaw check is unit-tested | a QA-window frame shows the Force shield's cloud at a known pose, and the generated (`type=particle`) systems draw their elements |
| non-level-1 draw pairs | level 5's pair scores 0.2982 full-frame / 0.3648 world-band (round 103: the HUD band is *not* its problem) with a **verified pose**: the reference is a player-camera frame and, at the retail view size, the port's sweeps peak at the row's own pitch 1.12 (0.3054 vs ≤0.28 either side) and yaw 134.99 (0.3057 vs 0.23–0.26 for twelve other yaws), so the camera is not the gap. What is: a sharp 24-bit image against retail's blocky 16-bit one, animated water over half the frame, and renderer colour — layout-normalised SSIM (HUD strip cropped, blur, 234×112) is 0.5925 against 0.8344 for the known-good level-1 spawn pair. Level 2 was **mis-diagnosed** in round 101: its reference was not a cutscene but a player-camera frame the harness had cropped to a 1094×355 strip (the world-box detector's "lit" test could not see a dark plaza), and re-captured it reads 0.3359 / 0.3489 with the masked score above the full-frame one, as a coherent pair should | both pairs stay **ungraded**; level-1 thresholds hold 6/6 and the gated pairs pass the world-band check (0.69–0.80); `retail-capture.ps1 -ProbeBox` re-derives any retained raw frame's box | level 2 and level 5 alike: a metric that survives content/renderer differences on dark and sky-and-water vistas (edge or landmark based) |
| interaction triggers (retail side) | a door/switch fires only on the engine's own **use action**: `00_doorswitch.cog` opens on the `activated` message from **sender id 2** and `00_door.cog` on `activate`, while the `blocked` message is only ever sent by a *moving* thing whose animation is obstructed - so walking into a closed door can never trigger it. The profile binds ACTIVATE to DirectInput **0x102 (middle mouse)** plus VK 57 | no retail trigger captured yet: the exact switch poses are known (`--links` surface centroids: 2409/2403 at -4.29,-3.88/-3.62,-1.15; things 56/57 at -4.75/-4.45,-3.75), the middle-click and `9` key are injected with the cursor inside the game window, and cog 11 stays `doorstatus 0` (`cogstatus11-middledclick-01narshadda.txt`); the port side is verified instead by the **`--door` gate**, which runs the whole chain on the level's own scripts (`Runtime::use_thing` -> `used` message -> `MoveToFrame`/`SetSectorAdjoins` -> `collide::Gates`): cog 0's piece reaches frame 1 after 3 s and **4 doorways the solver refused become passable** | synthetic mouse input reaches the switch (works with a retail-side trigger) or a keyboard-only trigger is found; retire the deviation then |
| thing collision (solid models) | `sithCollision` treats a collidable thing's model as solid surfaces: a moving body is **blocked** by it and slides. The port models each solid thing as a cylinder: `solid_things` now sizes it from the **model's own plan bounds** (min of the template `size=` and the model's plan extent), not the template radius, and `move_body` sweeps the body's circle against it (`sweep_things`) so a walk into a crate stops and slides round it. Round 124's blocking with the *template* radius was measured and reverted (it sealed gaps retail walks through: `walk_cantina` 102 % → 12 %, `walk4_spawn` 100 % → 50 %); with the model footprint those legs hold and the block works | gate: `walk_cantina` **102 %** (1 sector), `walk4_spawn` **100 %** (1 sector), `walk5_spawn` **81 %** (2 sectors) — all three pass, so the trace gate is **6 legs, 0 failures**. Still outside: `walk3_spawn` 266 % and `walk6_spawn` 190 %, because retail's stops there are *not* props (level 3's is scripted; level 6's is stair-climb speed) | the model's real surfaces (not its plan bound) are swept, and the stair/step path is read against retail's climb rate |
| slanted-floor sliding | the engine's slope slide is `sithPhysics_UpdateThingPhysics` subtracting the **world's** gravity from the thing's **z velocity** (`world->gravity * dt`, `DAT_008339c8 + 0xd4` = the `.jkl`'s `world gravity`, 4.0 in the shipped levels; halved by thing flag `0x40000`) and the floor collision sliding that downward motion along the surface. This port's walker keeps a body *on* its floor (`pos[2] = floor + rest`), so it has no z velocity to slide, and on level 3's slanted ground retail drifts downhill (−x 0.38, first window 1.35 units at 2.25 u/s) while the port drifts +x 1.2 and crosses a different set of ground triangles (3 of 6 sector samples). A **planar** version of the force was built and reverted: it double-counts in a floor-locked walker — level 3's drift moved slightly toward retail's but its mismatches stayed, and level 5's passing leg regressed (sectors 2 → 3) on its 45° ramp | `walk3_spawn`: 2.82 of retail's 2.26 (125 %), 3 of 6 sectors; the planar experiment: level 5 68 % with 3 sectors (gate failure) | the **vertical half** of `MoveThing` is ported (the body can leave its floor, fall under world gravity, and the sweep redirects it), after which `walk3_spawn` should join the gate |
| cutscenes (camera + player lock) | **The premise is contradicted by measurement (round 173): the engine's control lock is the *video-cutscene* camera, not a `SetCurrentCamera` focus.** `sithControl_Update` (`decomp/004d6c70`) skips `stdControl_ReadControls` only while `DAT_0055d5d4` is set, which is the camera object `jkCutscene` installs; a `SetCameraFocus(slot, thing)` + `SetCurrentCamera(slot)` focus (level 14's shape) does **not** lock the controls. Measured: level **20**'s opening holds a script camera on thing 44 and retail's body walks **4.20 units**; level **21**'s holds one on thing 23 and retail walks **0.33**. The port suppresses input whenever a script camera is current, which is right for 14/7 by accident: their bodies are motionless for other reasons (14's Kyle is **prone** after the knockdown cutscene, 7's pin is its own mechanism, 10's has no camera verb at all). Removing the suppression was measured and reverted: `walk21` **passes** (0.34 vs retail 0.33, 0 sectors), `walk20` improves to 2.85/4.20 = **68 %** (3 sector mismatches), but `walk14` 5.21 vs 0.00 and `walk7` 2.54 vs 0.11 break their legs. Level 8 walks 0.96 and stops | the four measurements above; `walk14`/`walk7` are gate legs today (forced-pass by suppression) | **retired when the player-immobility state is ported** — the prone/knockdown body (level 14) and level 7's own pin — so the walker can drop the blanket suppression and levels 20/21 join the gate; the ledger row then becomes "player animation/knockdown state", not "script camera" |
| solid things are floors | a solid template's model **axis-aligned bounding box** (top + footprint, `set_template_floors`) stands in for the engine's drawn collision surfaces, so a thing rotated off the axes is covered a little generously and an L-shaped model offers its whole box as floor | the catwalk pose holds at retail's z; `BK_WARP_TRACE` prints every solid thing's model top around a pose; a unit test pins the probe | thing-model collision surfaces (`sithCollision` against `_walkstruct` models) are ported |
| rank and force-point crediting | **retired (round 108)**: `jkMain_EndLevel` grants nothing. The port used to raise the rank (bin 20) by one and credit three stars (bin 16) per level, but retail's own saves prove the engine does not: level 1's load-time autosave and level 2's save (written after level 1 was finished and its tally shown) carry **no** `16`/`20` at all, and the decomp reaches `jkPlayer_SetRank`/`jkPlayer_FixStars` only from multiplayer's new-character screen — nothing recovered writes the point bins | the port's level-2 carry now matches retail's `jkauto-02narshadda.jks` bin for bin except the route-dependent population bin (`73`): `1, 2, 11, 13, 42, 60, 63, 71=8, 75=20, 99=2000, 100=1, 102=1` | — (`finish_level` still lands the end-of-level state and the tally still reads the rank and bin `0x10`; single-player force powers come from the episode table, not from rank) |
| thing activation | **retired (round 107)**: a thing's class cog is sent `created` when its sector is first drawn, which is what retail's two autosaves prove — the level-1 load-time save carries no population bin (73 = 0), the level-2 save carries 10.5 after the level was played. `Runtime::awaken_sector` tracks the woken set (idempotent) and `cull_sectors` calls it for each visible sector; the `--activation` gate holds the load state to 0 and requires the first wake to credit the bin and the second to do nothing | the port's load state now matches retail's load-time fixture; the after-play number is route-dependent on both sides | — |
| AI engagement at a data-derived vantage | **open (round 209)**: at the level-3 spawn pose turned to face thing 193 (`katarn_ai_193`: eye (-0.76,-4.74,-0.20), retail yaw −53, `--thing-list` puts the `tuskanbh` at (0.186,-3.491,-0.280), 1.6 units away), the port's AI sees the actor and **lands a hit** (HUD shields 100 → 97 within 2.5 s; `BK_AI_TRACE`: `sees 193 … ready true`, thing 193 walking to (−0.27,−4.09)), while retail's HUD reads **100/50 with no damage** across its 8-frame series and its rendered frame at that pose shows no actor in the fence gap. Retail's `aistatus` index is **not a thing id** (0 → thing 24, 1 → thing 25, 3 → thing 29), so the nearby actor's instinct words were not yet read | the pose row, both HUD readings and both frames are committed (`tools/parity/interaction-shots.tsv`, `tools/parity/aistatus-03katarn-193.txt`); the port's live actor positions come from `BK_AI_TRACE` each tick | **retired when a same-pose, same-time pair agrees** — either the port stops hitting where retail's HUD is untouched (the divergence is the port's acquisition, e.g. a `sithAICmd_LookForTarget`/`sightdist` condition — **round 211: the engine's scan is a view cone and the port's was range-only; the port's scan now uses the class's `sees` with a headless test, so this half is addressed and only the same-pose re-measurement remains, which needs a QA window**), or retail's actor turns out to be elsewhere/occluded (the divergence is the vantage) — and the comparison then moves to the engagement-timing measurement Phase 1 step 5 wants |

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
- **Done (round 103):** the HUD-masked metric. `compare.ps1` reports a **world-band SSIM** beside the
  full-frame number — the same SSIM over the world above the port's HUD band (bottom 22 % of the crop,
  dropped from both sides because retail's weapon and HUD corners are inside its raw frame too). It
  measures the HUD term at **0.07–0.09** of the level-1 pairs' score (spawn 0.6246 → 0.7128,
  route_cantina 0.7279 → 0.7877), shows `route_catwalk`/`route_door` barely moving (+0.04) so their gap
  is content, and `level2_spawn` *falling* (0.3041 → 0.2734) so its reference, not its HUD, is wrong.
  `thresholds.tsv` accepts an optional fourth column (world-band floor) for pairs whose HUD term
  dominates; no row uses it yet because none was rescued by the number.
- **Open:** the status table (next bullet), and a HUD-free capture is no longer needed.
- **Trimmed:** do not relabel every `roadmap.md`/`gaps.md` row. Keep one status table for the ten
  roadmap stages plus the deviation ledger, and correct the stale rows; the long prose stays appendix.
  The tier relabel below is the remaining piece.
- Tier the ten roadmap rows (T0/T1/T2/T3) and merge the overlapping `roadmap.md` / `gaps.md` /
  `engineering.md` status summaries into that one table.
- Build the **deviation ledger** (started above): sweep the `ponytail:` markers and the gaps §2
  partials into it with owner, measurement and retire condition.
- Freeze: installer, site, new UI screens, version bumps. Releases pause until Phase 1 exits.

### Phase 0.5 — Retail trace v0 (unblocks Phase 1 steps 3–6)

**Exit criteria met (v0, rounds 100–213).** T3 always worked because a screenshot is a complete observation;
T2/T4 needed a replayable trace and pinned console formats, and both exist now: `port-trace.ps1` replays a
leg at a fixed 1/60 s step, the walk-trace gate replays 15 retail-derived legs, and the `cogstatus`/`aistatus`
formats are pinned (criterion 2) with the shipped capture fixtures behind them. "The same inputs produce the
same position/sector/health" is therefore measurable, and it is what Phase 1 steps 3–6 use. The criteria are
kept below with their evidence; the dense Frida hook (criterion 4) is still not needed.

Exit criteria:

1. **Done (v0).** A scripted walk on retail yields a parseable position + sector trace (poll `coords` on a
   fixed cadence, with `slowmo` if the console cannot keep up), and the format is documented. The format is
   in `tools/parity/README.md` §"Movement traces (Phase 0.5 trace v0)": retail writes
   `held_ms x y z sector yaw` per hold segment (held-key game time, because the console pauses the game),
   the port writes `tick t_s x y z sector vy health shields` per 1/60 s frame, and the fixtures are the
   committed `tools/parity/walktrace/*.tsv` legs that `trace-gate.ps1` replays (15 legs today).
2. **Done (v0).** The `cogstatus`/`aistatus` handlers are decompiled and their output formats pinned; one
   level's cog variables are captured through them. The `aistatus` side: usage is `aistatus <index>`,
   where the index is a position in the level's **AI actor list in thing-id order** (`0` is the first
   actor; bare `aistatus` dumps level-load timings instead, and a thing id is rejected). The dump is
   `AI Status dump for thing N (template)`, then `Class 'x.ai', Flags=0x…, Moods a/b/c, NextUpdate=n`,
   then one row per instinct in the **class `.ai` file's own line order** — verified by count (eleven
   lines in `ai\tuproddefault.ai` ↔ eleven rows for a `tuskan`) and by the registration defaults: mask and
   mode are the two bit words `sithAI_RegisterInstinct` stored (`sithAICmd_Startup` registers
   `lookfortarget` with 0x204, and row 1 — `LookForTarget` in that file — shows it). For a brown Tusken
   (`tuproddefault.ai`, fov 180, sightdist 5.0) the rows read: Jump `mask 0x604`, LookForTarget
   `mode 0x204`, Listen `mask 0x7`, PrimaryFire `mask 0x100 mode 2`, BlindFire `mode 2`, Follow
   `mask 0xe00 mode 2`, OpenDoors `mode 2`, ReturnHome `mask 0x900`, Retreat `mode 2`, Flee
   `mask 0xf05 mode 0x800`, Talk `mode 0xffff`. `Params` are the four per-instance numbers (the class
   defaults are 0 on an idle actor). The shipped class files themselves are readable without the game:
   `--asset ai\tuproddefault.ai` prints them (round 210).
3. **Done (v0).** `port-trace.ps1` replays a leg with `--warp` + `BK_WALK_FORWARD=1` at a fixed
   1/60 s step (capture-mode `--shot` pins `dt`) and writes one line per frame
   (`tick t_s x y z sector vy health shields`); `trace-compare.ps1` interpolates the port path to the
   retail sample times and reports per-sample `d_xy`/`d_z`, travel distance and the sector sequence,
   with `-Epsilon` turning it into a verdict. First leg (`walk_cantina`): start poses agree exactly;
   **z is 0.055 low** at every moving sample (the walking-settle gap, now measured against retail);
   xy diverges with distance - retail 2.17 units in 2.4 s (0.9 u/s) vs the port's 1.06 (0.44 u/s) -
   which names the next movement fix: the port's `maxthrust / surfdrag` walking speed is under
   retail's, and `walkplayer maxvel` 1.0 is the likely law.
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
   level-1 route. *Partly done:* a **solid thing's model is a floor** now (the loader registers every
   solid template's model top and footprint from the model it draws, a falling body lands on it, and
   the idle attachment holds a warped body at retail's pose over one — level 1's catwalk no longer
   drops the body out of the level; `route_catwalk` 0.4493 → 0.4603, `BK_WARP_TRACE`/`BK_WALK_TRACE`
   are the probes, and the box approximation is on the ledger). The engine's height comes from the
   model's insert offset (`sithPhysics_GetThingHeight`), which is also the authored stand height of the
   level's `walkplayer` placements (0.120114 at the spawn), so the resting/attach rules are pinned;
   **non-player thing physics now has gravity** (round 246): a created thing starts with its template's
   `physflags=` (`spawn_at`/`fire_with_spread`/`explode` all set `0` before), the mover applies the
   level's gravity to the `_debris` family's `0x201` bodies (`0x40000` halves it) and stops the fall on
   the highest floor plane under the body, which is the round-120 note's missing ground test — the
   `--things` gate drops a created chip and measures the fall, and `--soak-all` stays clean. Still owed:
   **actor gravity** (the walker owns the player and `ai_tick` the actors, so an actor walking off a
   ledge does not fall), the engine's exact `SearchForSurfaceCollisions` sweep and `GetThingHeight`
   resting rule, the *walking* settle (`top + radius`, 0.08 below retail's idle pose on the catwalk), and
   the recorded trace itself.
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
   capture landed **6 real hits** and the catwalk took its first hit; the pair scores still waited on
   retail's saturated tint because the port's fire was bursty — the engagement rate was the remaining
   gap. Two AI mechanisms then closed most of it: `sithAIAwareness` (a side-mate's shot tells the others
   where the target is; flat radius and lifetime stand in for the un-recovered sector propagation, see
   the ledger) and `maxrotthrust / surfdrag` turning (23°/s for a Ree-Yees gunner, 60 for a human,
   instead of a snap — the instant turn is what made the port's fights faster than retail's). The door
   capture now fires 41–47 shots and lands **15 hits** in 10 s (≈165 damage, the body left at 29 health
   where retail's own HUD at ~10 s reads 56), and the gate run exposed a real bug: `AIFlee` was ignored
   by the "nothing to do" early return, so a fleeing actor that had lost the player stood still.
*Progress (rounds 209–239).* Combat's AI side is now measured off the shipped `.ai` files rather than guessed. The
instinct registry is complete (25 registrations across `sithAICmd_Startup` and the JK-side startup; the four unnamed
entries resolved from the EXE's string data), and `--audit` tallies which names shipped content actually uses: **24**.
Of those, **19 of the 22 instincts shipped content can actually load** are implemented — `follow`, `lookfortarget`,
`primaryfire`, `opendoors`, `blindfire`, `returnhome`, `retreat`, `withdraw`, `circlestrafe`, `hitandrun`,
`randomturn`, `roam`, `turretfire`, `flee`, `listen`, `talk`, `sensedanger`, `dodge`, `lobfire`, `jump` (20 handlers, but
`randomturn`'s nine class files are named by no shipped content — round 243) — with `sabermove` unused by any class and
`crouch` likewise dead content. The remaining gameplay gap is the **Jedi set** (`forcepowers` and `saberfighting` 45
class-uses each, `specialattack` 24 — all loadable) plus `jump` (85 of 92 loadable, and its handler only runs on the
track-target event); 18 of 22 closed in rounds 209–247. Each instinct landed with a
headless test and a measured before/after (suite 149 → 189). Reading the handlers also corrected behaviour underneath
them: the view cone of `LookForTarget`, the engine's class defaults for omitted `.ai` keys, the script-set AI bits from
`AISetMode` (`0x2000` switches an actor off and `0x4` is the awake bit `listen`/`sensedanger` gate on), turrets firing
from `TurretFire` (a turret class has no `PrimaryFire`), `listen`'s `heardist` gate, its 3-slot sound ring, its
line-of-sight/1.0-unit/60° hearing gates and its per-sound reaction chance, `talk`'s 95-name sound-mode table with its
flag/health mood selection, `sensedanger`'s alert bit `0x800` (which `talk` reads for `fear`/`gloat`) and its
strength-3.0 awareness burst, `dodge`'s damage event and 1000 ms cooldown plus its player-side shot pre-sweep (which also
put the `typeflags=` word on the runtime and showed it is not the `thingflags=` word the sweep tests), the engine's **AI
event dispatcher** with its complete emit-site inventory (round 241 — which proved two handler branches dead), the
**detection chance** that sits under every sight test (`sithAI_CanDetectSightThing`, round 242, constants from the
`.rdata`), and the **loadable-class accounting** that retired `crouch`/`randomturn` from the work list (round 243), plus
most of the movement constants from the EXE's `.rdata`. The per-instinct class counts and the port's status are the table
in `gaps.md`.

6. **Exit.** End-of-level tally, transition, carried state (health, shields, bins). Exit: level 2 starts
   in retail's state. *Partly done:* the savegame **container** now matches the real files — the port
   reads both engine-written autosaves end to end (`version 6, episode JK1, level 02narshadda.jkl, game
   time 500, 7704 record(s), 330 thing record(s)`, and level 1's 5856), with each save's first thing
   record decoding to the **player at that level's spawn** (level 2: 10.798, −11.983, 3.020 against the
   `level2_spawn` row's 10.80/−11.98/3.02), pinned by a test on the committed fixtures. The container had
   been modelled from the in-memory message header; on disk a record is `u32 type, u32 length, payload`
   and the prefix carries a 0x24-byte game-side block before the level name (see `formats.md`). The
   engine's own bin table rides in the save as records of type 0x10 (`sithDSS_Inventory`), and the two
   autosaves differ in exactly **five** bins — 71 6→8, 73 0→10.5, 75 0→20, 99 1000→2000, 102 0→1 — so
   retail's level-1→2 inventory delta is measured and pinned by a test; 100 is `SITHBIN_GOAL00`, 71 is
   the level's secret-area count (`00_secretareas.cog`), and 73/75 are the alignment pair
   (`jkPlayer_CalcAlignment` reads 72/73 and writes 75). The port's own **level-start inventory now
   matches level 1's autosave bin for bin** (`1=1, 2=1, 11=50, 13=100, 42=1, 60=100, 63=1, 71=6,
   99=1000, 100=1`), asserted by the `--save-live` gate: it needed the player's **template** cog
   (`walkplayer` carries `cog=kyle.cog` and the placement table has no row for the player) plus fixes to
   `GetInvMax` (a flat 100 stub), the scripted pickup's `taken` label, pickup reach (contact, not a
   unit) and `array[index]` resolution (`linkid` is a message bit, not an ordinal) — see `gaps.md`. The
   level change's carry now also applies **before** the incoming level's scripts (the engine's order:
   `jkPlayer_*` outlives the level and the new level's cogs adjust it), so the port's level-2 state
   carries `71=8, 99=2000, 100=1, 102=1` like retail's own level-2 autosave instead of keeping level 1's
   secret count. The start state's other half is now retail-asserted too: the player's **health** is
   decoded from the thing record's type-specific tail (`thing + 0x134`, payload `0x6F`, derived from
   `GetThingHealth` at `0x502570` and the serializer at `0x4f4210`; both autosaves hold 100), `--save`
   prints it, and the `--save-live` gate asserts the port restores the same 100. Shields needed no new
   field — they are bin 60, already in the bin comparison. The carry's **alignment** bin is now written
   the way the engine writes it: `jkPlayer_CalcAlignment` stores its answer in bin 75
   (`sithPlayer_SetInvItemAmount(0x4b, …)`) and the tally is its single-player caller, so the level-end
   path calls `Runtime::calc_alignment` and the `--tally` gate asserts the store; the port's level-2
   carry is therefore retail's autosave **bin for bin except bin 73**: `71=8, 75=20, 99=2000, 100=1,
   102=1` all match. Bin 73 is `CalcAlignment`'s ratio denominator (the actor scripts' `created:`
   timer credits it per woken civilian; 72 is the kills numerator). That timer path is now live: every
   thing whose template names a `cog=` gets its own VM (level 1: 249), `SetTimerEx` keeps the id its
   handler branches on, the timer bus delivers it, and — the **dormancy** half — a thing wakes with its
   sector instead of at load. Retail's own two autosaves are the fixture: the level-1 load-time save
   carries no population bin (73 = 0) while the level-2 save carries 10.5 after the level was played,
   so `Runtime::awaken_sector` wakes a sector's things once when `cull_sectors` first draws it and the
   `--activation` gate holds the load state to 0 and the first wake to a credited bin. The port's load
   state therefore matches retail's, and the after-play number is route-dependent on both sides; the
   ledger's thing-activation row is retired. `72/73 = 0/x` on both sides, so `CalcAlignment` answers
   20 and bin 75 still matches. The **rank/star stand-in is gone** (round 108): `jkMain_EndLevel` grants
   nothing, because retail's own saves carry no `16`/`20` at level start and the decomp reaches
   `jkPlayer_SetRank`/`jkPlayer_FixStars` only from multiplayer's new-character screen — single-player
   force powers come from the episode table. With that and the mana pool (a rank-0 Jedi has none:
   `kyle.cog` tops it to `rank * 50`) the port's level-2 carry is retail's autosave **bin for bin except
   the route-dependent population bin 73**: `1, 2, 11, 13, 42, 60, 63, 71=8, 75=20, 99=2000, 100=1,
   102=1`. Step 6's carry comparison is therefore closed; the alignment chain is complete and gated.

### Phase 2 — Generalise

Run the Phase 1 harness on levels 2–5, then all 34. Replace "the walker reaches the end" with "the walker
trace matches a retail trace" for each. Fix by bucket, not by level. Add the retail-trace fixtures to the
repo so regressions are caught without a retail run.

*Progress (rounds 153–231).* The walk-trace gate stands at **15 legs, 1 tracked failure**: `walk5_spawn` at 3.43 of
retail's 3.73 units (92 %, 5 sector mismatches), the "floor-search scope" deviation the ledger names with its retire
condition; the other 14 legs pass. At the last windowed run `--gates` was **65/65, 0 skipped** and the parity draw gate
**6 thresholds / 0 failures**. Everything still unmeasurable is listed in "The QA window" below — it is the single list
of what a real-game session would settle, in the order a release candidate needs it.

*Progress (round 245).* Both all-content checks were re-run **headless**: `--soak-all` is **35 of 35 levels clean**
(`0 with a non-finite position, 0 with bodies outside every sector`), and `--gates` is **0 of 65 failed, 1 skipped**
(`--live-check`, the one row that needs the camera entity; 55 rows print `headless pass` and 9 window-labelled rows run
their deterministic frame mode). The gate sweep is therefore **not window-bound** any more: the checks that still
genuinely need the QA window are the walk-trace replay (its port half records frames), the parity draw captures, and
the per-behaviour fixtures listed below.

*Progress (round 112).* The trace fixtures are in the repo and wired: `trace-gate.ps1` seeds a missing
`target/parity/trace/<leg>-walk.tsv` from `tools/parity/walktrace/<leg>-walk.tsv`, so the four level-1
legs gate from a clean checkout with no retail install (proved by deleting the live traces and passing
4/4). The first non-level-1 leg (`walk2_spawn`, level 2's spawn pose) was captured and **is not yet
gateable**: level 2's opening scripts move the body during the 3 s window (retail goes forward, back,
forward over sectors 22/45) while the port walks straight -- the leg needs a pose the opening has settled.

Level 5 (`06abarons`) has a calm spawn, so `walk5_spawn` is the working non-level-1 leg: retail's trace is
a clean 3.73-unit walk over six samples (sectors 35 → 33 → 34 → 33 → 33 → 24, committed as
`tools/parity/walktrace/walk5_spawn-walk.tsv`), and the port wedges after **0.90 units** at
(9.78,14.49) with the body 0.34 units below retail's — it is standing in a gutter beside a raised
surface retail's walker treats as ground. That is Phase 2's first measured bucket: **walker floor
support for solid things** (`--warp` attachment already reads them; the walking support probe does not).
The leg joins the gate as soon as it passes.

*Progress (round 119).* Phase 2's first non-level-1 leg is **in the gate**: `walk3_spawn` (level 3,
`03katarn`) passes at travel 1.13 of retail's 1.71 units (66 %) with 2 sector mismatches, and its retail
trace is committed. The retail harness's unused `-SettleMs` knob now feeds a `coords` read after the
`-StartLevel` boot (`spawn| Pos: …`), which gives any level's real spawn pose without a warp — the input
every later level's leg needs. Level 3's trace also records two known differences: the port has no
opening-script hold (retail freezes for 600 ms), and the port stops at (−0.77, −3.60) after 1.2 s.

*Progress (round 120).* Levels 4 and 6 measured with the same recipe. Both fail and stay outside the
gate: level 4 travels 0.56 of retail's 1.11 units (50 %) and stops at (−4.41, 1.885) where retail walks
on; level 6 travels 2.11 of 1.08 (195 %) and ends in sector 78 against retail's 315 — the port does not
stop where retail halts. Their retail traces are committed, and the debug list's numbering was
corrected: entry 4 is `04escapehouse`, entry 6 is `06bbarons` (not the file numbers they looked like).

*Progress (rounds 121–129).* Five levels measured end to end with the same recipe, and **two of them are**
**in the gate**: level 4 `walk4_spawn` (100 % of retail's 1.11 units, 1 sector) and level 5 `walk5_spawn`
(81 % of 3.73, 2 sectors), alongside level 1's three legs — `trace-gate.ps1` is **6 legs, 0 failures**, with
retail traces committed for levels 3, 4, 5, 6 and 7. The measured remainders are each named against a
deviation instead of "walks too far": level 3 drifts **1.41** in x on slanted ground against retail's 0.38
and passes the crate retail stops at (ledger: slanted-floor sliding); level 6 stops against face 788
(`top 1.150`, twice the step allowance) where retail walks the treads; levels 2, 6 and 7 open under script
control (level 7 moves 0.11 units in the whole window, so it is not a walk at all until scripted player
control lands). Three buckets were fixed on the way and are on the ledger: the **thing-collision push**
(never against the motion, wall-aware, and blocking on cylinders sized by the model's plan bounds — this
took level 4 from 50 % to 100 % and level 5 from 1.41 to 3.02 units), the **step-up test** (judged by the
face's top edge, so stairs climb), and the **walk-leg start pose** (retail's own trace start, not the
spawn `coords`). The next item in Phase 2's own order is the rest of levels 2–5, then all 34: the recipe is

*Progress (rounds 130–132).* The sweep reached levels 7, 8 and 9, and the **leg-quality check** the
harness now runs (`camera is scripted`, `the body travelled … not a plain walk`) separates the two shapes
it found: **script-owned openings** on levels 2, 6, 7 and 8 (level 7: body pinned to 0.11 units while the
yaw spans 240.1°; level 8: 0.02 units with a steady yaw), and **real walks** on level 9, where retail moves
1.68 units on a steady yaw and the port matches the distance (104 %) but climbs 0.34 and ends airborne on an
edge against a 0.55-high ledge — one more instance of "the port climbs where retail walks flat", now with
the pose and surface ids recorded. Retail traces for levels 7, 8 and 9 are committed; the cutscene bucket

*Progress (round 133).* The "walker climbs where retail walks flat" bucket was root-caused and fixed: a
landing could raise a body by `rest − radius` (0.055) per frame, so a body blocked at a ledge compounded
that into a climb. With the landing unable to leave a body higher than it was, **level 9 `walk9_spawn`
passes at 100 % with 0 sector mismatches** and joins the gate — now **7 legs, 0 failures** — and the two
held-out walkable levels improved sharply (level 3: 208 % → 125 %, sectors 3 of 6; level 6: 190 % → 155 %).

*Progress (rounds 134–152).* **Twelve gated walk legs**: level 1's four, level 4 (100 %), level 5 (70 %), level 6
(98 %), level 9 (100 %), level 12 (127 %), level 13 (98 %), and levels 7/14 as **stop-pose** legs after the
script-camera hand-over (0.00 units against retail's 0.11/0.00). Remaining levels each carry a measured reason:
level 3 (slanted-floor drift, 2.75 vs 1.71), level 8 (walks 0.96 then stops; port 2.14), level 10 (0.12-unit
pin with no camera verb), level 11 (genuine 1200 ms scripted start), and levels 2/6-sill closed. Walker fixes in
this stretch: landing may not raise a body (level 9), the steep-face push is trimmed of its backwards component
(level 6), a near-head-on hit stops instead of creeping (level 13), actors are no longer walker obstacles
(level 11), a doorway landing may stand a step higher, a script camera suppresses movement (levels 7/14), plus
the harness settle fix on both sides (rounds 148–149). The script camera's **view** (posing the camera at the
focus thing) is the next slice.

### The QA window — the gated checks that need the real game

Every "needs a windowed fixture" note in this document lands here. Both the retail harness and the port's own window
take over the mouse and keyboard, so this is **one session, run deliberately** — at a release candidate, or when the
accumulated behaviour changes are worth a regression — not a per-question tool. Everything that can be checked from
files (tests, `--audit`, `--asset`, `--grep-assets`, `--model`, the data-derived probes) has already been checked that way; this
list is only what is left.

Run, in order, from the repo root with `pwsh`:

| check | command | last measured | expected |
| --- | --- | --- | --- |
| port gates | `cargo build --release` then `target/release/bevi-knight.exe --gates` | **0 of 65 failed, 1 skipped** in a `BK_SKIP_WINDOW=1` sweep (round 245: 55 rows `headless pass`, 9 `window pass`, `--live-check` skipped); last windowed run 65/65, 0 skipped | all pass |
| walk trace | `powershell -NoProfile -File tools/parity/trace-gate.ps1` | 15 legs, 1 failure | 14 pass, `walk5_spawn` tracked |
| parity draw | `tools/parity/retail-capture.ps1`, `port-capture.ps1`, then `compare.ps1 -Gate` | 6 thresholds, 0 failures | 0 failures |

What the session settles, all of it already on the deviation ledger with its retire condition:

- **`walk5_spawn`** — the floor-search scope on level 5's gutter; the gate leg is the fixture (14 of 15 pass).
- **`opendoors`** — the port uses the 1.5-unit use reach and a look cone where the engine sweeps the actor's body
  radius; walk an actor at a closed door and compare which door it opens.
- **`blindfire`** — the chance is rolled per shot slot, not on the engine's own tick.
- **`returnhome`** — the port gates on the instinct existing and distance > 0.5, not the engine's awake flag.
- **`retreat`/`withdraw`** — the mode ends on the withdraw interval instead of on sight of the target, and the away
  direction has no lateral jitter.
- **`circlestrafe`** — the port replans every frame where the engine uses 300–500 ms.
- **`hitandrun`** — break-off lands on the exact threshold instead of within the next AI update.
- **`turretfire`** — the ±160° aim-head sweep is not modelled (turret fire is modelled as accurate).
- **`listen`** — the hearing gates and the class's reaction chance are ported (round 233, measured headless); the
  look-only turn on a failed roll and the damage-event path are still one remembered position.
- **`AISetMode`** — only the `0x2000` off switch is honoured; the wake bit (`0x4`) needs the engine's own wake logic
  first.
- **AI engagement timing** — the level-3 same-pose pair: a port run and a retail capture at the recorded vantage, to
  confirm the view-cone fix moved acquisition to where retail acquires. The detection **chance** underneath it is
  retail's now (round 242, unit-checked against the `.rdata` constants) and acquisition is a roll; what a window can
  add is the *timing* of acquisition against retail's at the same pose.
- **viewmodel materials** — the data side is measured headless (`--model bryv.3do`: 14 materials over 76 faces, all
  present), but whether the bryar's body and hand both draw correctly can only be judged in a frame; round 231 drew
  material 0 alone (`HDBACK`, the hand's back).
- **POV sway** — the amplitudes, phase rate and speed scaling are retail's now (round 234, unit-checked), but
  `rdMatrix_BuildRotate34`'s composition order and sign are not recovered; a frame at a known phase would settle it.

Safety and recovery: the retail harness never touches the real install — it copies it to `target/parity/retailcopy/`
and records the desktop display modes to `target/parity/display-pre.json` before it starts. Keep hands off the mouse and
keyboard while a capture runs; both harnesses drive synthetic input. The port side opens a window per shot and closes it
itself.

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
