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

## Where the ten stages stand

One table, per stage, with the **tier** the tier definitions above demand and the retail-derived
evidence that earns it. Everything not marked T2 is T1 (implemented and self-consistent, with a gate
that fails on drift) — which is the point of this table: an internal check is not parity.

| stage (`roadmap.md`) | tier | what earns it | next rung |
|---|---|---|---|
| 1 sector/portal visibility | **T1** | portal occlusion on, and the **doorway test runs now (round 342)**: `visible_sectors`' `sees` hook was passed `any`, so the walk admitted every room through every visible adjoin; it now projects the doorway's verts and drops a room whose only opening has no vertex in front of the eye (the `w` test) or whose projected corners miss the viewport. The filtered walk runs on the **same seed** the unfiltered one chose and is adopted only when it is usable (a seed and at least one on-screen room), so the drawn set can only shrink — the first cut let a degenerate filtered set trip `use_portals` and draw the whole frustum, which ended level 1 through the renderer's `sighted` link (`--ai-live` timed out; the guard fixed it). Level 1's spawn: **90 of 462 sectors drawn → 84**, and the two captures are **byte-identical** (526,617 bytes), so nothing visible was lost. The earlier A/B (`--no-cull`, 0.000 % over 230,400 samples) still stands | a retail spawn pair that isolates a room drawn through a portal |
| 2 player parity | **T2 (partial)** | the spawn pose: retail `coords` agrees with the port's body on all eight level-1 rows (position and sector); `walk_street45` replays retail's 2.20 units as **2.21** with 0 sector mismatches, `walk_street90` at 83%; **issue #10 lands the cantina and door legs** — the route replay holds retail's sectors on all four legs and its vertical trace (a warped body rests at `GetThingHeight` 0.120114, the spawn falls to the `floor + radius` contact, the walk rate is the engine's 0.875 run, and a low solid model is a step the body climbs) within the fixture's stated epsilon | per-sample retail timing (the harness's 600 ms samples advance a variable amount of engine time) |
| 3 `.key`/`.pup` animation | **T1** | `--animate`: 1684 of 1697 actors move over the loop (internal) | a retail frame pair at an animated actor |
| 4 COG verbs | the level's own logic runs and changes the world (`--door`, `--goal`, `--sleep`, `--triggers`, `--secret`); `sleep` suspends and resumes, and **round 308 makes `--cog-check` resume every wait**: each label is run again past each `Sleep` (32 resumptions max), so the code after every wait in all 34 scripted levels is exercised - **0 errors, 0 stubbed verbs**. Round 307 also fixed `Vm::run_label` dropping a pending resume when a different label ran, which was silently killing `startup` continuations | the level's own logic runs and changes the world (`--door`, `--goal`, `--sleep`, `--triggers`); `sleep` suspends and resumes | **movement triggers matched (round 308)**: `00_SecretAreas.cog` at cog 28 reads `maxsecrets 6`, `flags 1` on both sides after entering sector 243, and `--secret` gates it. Open: a *use-action* trigger (retail's ACTIVATE does not fire under synthetic input) |
| 5 combat | **T2 (partial)** | `--hit`/`--fire`/`--pull` internal; at level 1's door after the same 10 s capture, retail's HUD reads **81 health / 0 shields** and the port reads **83 / 0** in two uninstrumented runs (round 409) | per-hit timing and the rest of the combat route |
| 6 AI | **T1** | `--ai` internal; the door encounter now reaches retail's settled health within 2 points, but its per-hit timing is not yet compared | the same encounter, same timing |
| 7 audio | **T1** | `--sound`/`--music`/`--fade` internal (375 clips, the script's own fade chain) | a retail level's audio state compared — no fixture exists yet |
| 8 `.smk` cutscenes | **T3** | it plays; **round 309** captured retail's opening film (`retail-capture.ps1 -IntroShotMs`) and fixed the film's placement (retail draws it at y 50 in a 640x480 screen; the port had centred it at 90); **round 310** closed the frame and the colour: with `BK_FILM_HOLD=147` the port's crawl frame is **byte-identical to retail's capture** (0 of 307,200 pixels differ, bbox 44,160..595,244), and the palette is independently verified against ffmpeg's Smacker decode of the same frame (0 of 192,000 pixels). The difference was Bevy's default `Tonemapping::TonyMcMapface` on the UI camera — the engine blits its palette untransformed. `tools/parity/film-gate.ps1` gates the pair at **SSIM 1.0000** (floor 0.99), and a second check pins the decode order where content changes every frame: the port's hold-210 frame against **ffmpeg's** own Smacker decode of index 210 is **SSIM 1.0000** too, so the helper's index equals the file's chunk index | retail's half of a *moving* frame (its capture clock does not name a film index), or a second film's palette record |
| 9 campaign flow | **T2 (partial)** | both engine-written autosaves parse to EOF, and the level-2 carry is retail's autosave **bin for bin except the route-dependent bin 73** | a route where bin 73 matches, and a real retail save round-trip (the GOG install ships none) |
| 10 UI panels | **T1** | `--map-shot`/`--ui-visible`/`--load-bar`/`--force-screen` internal; the screens' geometry is retail's own element tables | **round 309**: the retail **main** menu and the port's own render of it, graded by `tools/parity/menu-gate.ps1` at **SSIM 0.9521** (floor 0.90) from matched 640x480 captures - menus draw the shipped art, so the pair measures the renderer, not the content |

Phase 1's own rows sit on top of this: the level-1 **spawn pose** is T2 and the **spawn frame** T3
(`route_cantina`/`route_lower`/`route_street` pairs at 0.72–0.74 against floors of 0.65), while step 4
is the remaining in-repo T2/T4 work and step 5's posed door-health endpoint is now T2 (round 409).
The **deviation ledger** above is the per-deviation view of the same state; this is the per-stage view.

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
| `walk_player` resting height / attachment | **derived from the retail binary and confirmed on the GOG build**: `sithPhysics_FindFloor` (`0x4f5550`) reaches `moveSize + 0.005` for an unattached body and `GetThingHeight * 1.1` for an attached one (shipped `0x523184` = −0.005, `0x52318c` = 1.1); `sithThing_AttachThingToSurface` records the surface **without moving the body**, and `sithPhysics_UpdateAttachedThingPhysics` applies no gravity. **No engine path attaches the player**: the only two callers of `AttachThingToSurface` are `FindFloor` (creation/load, and for a body that is *already* attached — `sithThing_UpdateMove`) and the cog verb, and `sithPlayer_NewPlayer` (`0x4c8ec0`) never calls it. The retail player therefore falls from the level's authored height to the swept sphere contact and is held there by collision. The port had invented an idle attach (a floor probe each frame that zeroed gravity); it is removed — gravity runs every frame and `walker_step` lands at `floor + radius` | retail `coords` at level 1's own spawn: (−10.44,−5.46,−1.13) sector 339 at load, (−10.25,−5.07,**−1.18**) sector 349 after 1 s idle (floor −1.25 + 0.065); the port held −1.129886. Unit check `a_body_placed_above_its_floor_falls_to_the_sphere_contact` | closed for the resting rule; what remains is thing-vs-body collision and the attached-body offset for *script*-attached things (`AttachThingToSurface` cog verb) |
| floor search scope (`floor_under`) | **Round 289 closes the level-1 hull leak.** The scoped search now expands only into rooms whose solid/portal polygons contain the probe (`room_contains`), with a global scan still reserved for a disconnected seed. This removes the tabletop/cantina-wall route; Round 290 then resolves level 5's sector oscillation by removing the two per-frame `sector_for_body` assignments in `walk_player`; only an initially unresolved spawn uses that resolver, while valid movement keeps its sector. Earlier diagnosis: **the search was global, not sector-scoped.** `sithPhysics_FindFloor` (`decomp/004f5550`) searches the body's sector and the sectors its adjoins lead to; this port's data does not tile (a walkway slab can sit in a sector nothing below it links to), so `floor_under` scans every face in the world within reach. That finds floors the body's room does not reach: level 5's body lands on a **downward-facing slope of the sector above** (surface 342, owner sector 33, normal (0.44,0.44,−0.78)) below its own sector 35 floor (0.1001), and walks a plateau at z 0.1703 while retail descends to the room across sectors 33/34/24. The engine's own rule is front-facing only (`sithCollision_SearchForSurfaceCollisions`: `dot(search_dir, normal) < 0`) | `BK_FLOOR_TRACE` names every landing below the body's own sector floor (walk5: surface 342/owner 33 every frame; walk6: surface 2837/owner 316 once). Four candidate fixes were measured and reverted — excluding downward normals (`walk5` 3.70→3.39, `walk6` 1.06→0.85 **broken**, 2 tests failing), the same exclusion at −0.5 (identical), orienting faces from the sector's hull plane (`room_normals`: identical to the exclusion), rejecting a landing below the own sector floor (`walk5` 3.70→3.39, walk6 kept), and preferring the engine's one-hop sector scope with the global scan as fallback (`walk5` 3.43, walk6 kept, all other legs unchanged). Round 290 measured `walk5_spawn` and `walk6_spawn` passing together; round 291 records the separate `walk17_spawn` mismatch | **retired when the non-tiling fallback can be removed**: either the level data's sector tiling is verified for the affected levels (so the search can be sector-scoped outright), or the landing probe distinguishes a *reachable* floor from a neighbouring room's slope by polygon/sector reach rather than by normal — the four measurements above are the acceptance table, and `walk5_spawn` must pass with `walk6_spawn` still passing  **Round 282 makes it a table**: `--start <level> --floor-at "x,y,z"` prints every candidate face a downward sweep lands on (surface, owner sector, floor, authored normal, up/down, room normal) from the same `Solids::floor_candidates` the walker uses. Level 5's walk5 route stands on faces whose stored normals point **out of the room** (`up false`, floor 0.1001 at both the spawn and the wedge), which is the `.jkl` convention `floor_below` already relies on; the port's flip toward the body is the into-room normalisation the engine's `dot(search_dir, normal) < 0` test expects, so the **facing rule is already honoured** and this row's remaining debt is the *scope* (the engine searches the body's sector and its adjoins; this scans every face) and the swept surface test. Measured: the wedge point has exactly one candidate (surface 405, sector 35), and the spawn's floor reproduces retail's own eye height 0.22 there. Measured (round 283, `--faces-at` at retail's own path sample (9.79,14.22,-0.13)): the port's sector assignment agrees with retail (33), the sector plane is -0.389 against retail's body ~-0.13, and the swept `floor_under` finds **no polygon** at any walker height while `--collide` reports no dropped surface — so this leg's residue is a coverage gap in the level data as read here, not a rule. |
| `spawn_point` | **Level 1 closed (round 289); level 17's empty startup recovered (round 291).** Level 17 recorded a shaft in sector 505 with no local standing point; the no-clear-spawn path now searches upward for a different containing room and selects 485 at z −8.135006, whose rendered frame shows the room, floor, and crate. The separate `walk17_spawn` sector trace still drops into 502 and remains a deviation. Level 1: sector 339's floor probe was entering neighbouring sector 340 through a solid wall because it accepted 340's infinite hull. `Solids::room_contains` now requires the point to lie over an actual solid or portal polygon, and both spawn passes use it. The recorded pose no longer nudges 0.286 units to (−10.73,−5.17); live forward travel is 0.21 units and Space lifts 0.11. Other levels' body-width/thing-collision nudges remain governed by this row | `217` binary tests; gates `0 of 66`; level-1 live-play pass; `walk all` no void falls | remaining maps: the movement port models the engine's body width and its thing-vs-body collision |
| `compare.ps1 -Gate` | the port's HUD is inside its frame while the retail world crop has none | round 103: the world-band metric measures the HUD term at 0.07–0.09 of a level-1 pair's score (spawn 0.6246 → 0.7128, route_cantina 0.7279 → 0.7877); the full-frame floors stay as they were and all six pass | **retired round 103**: the HUD-masked metric exists (Phase 0's open bullet) and `thresholds.tsv` accepts a world-band floor as its optional fourth column, so the asymmetry is measured and separated instead of capping the gate |
| `shots.tsv` / `--shot-at` | retail's rendered camera pitch is ~0 while its `coords` reports PYR pitch 1.04, and `warp` cannot set pitch, so a pair cannot reproduce a pitched retail view; rows request pitch 0 | port pitch 0.25° already drops the spawn pair 0.63 → 0.59; `coords` PYR is in every sidecar | a retail-side way to set or read the rendered view pitch (none known); the port side already takes pitch |
| camera pinned vs retail camera on the body | a port capture puts the **player** at the recorded pose (`--warp`) but keeps the camera pinned to the row's eye (`--shot-at`), while retail's camera rides the body; where the body is moved by a script or the floor the two cameras diverge | same framing at the warp instant (sector and yaw match retail: 8/8 rows); no row shows a moved body yet | a capture that lets the port's camera ride the warped body once bodies stay put (attachment state and damage tint below) |
| damage tint | the engine tints the palette red as the player is damaged: `sithActor_DamageActor` adds `damage * 0.04` to the local player's red tint, `sithPlayer_Update` decays 0.4/s, `stdPalEffects_UpdatePalette` applies `byte += round(byte * tint * 0.5)`. Both halves are ported: the state (adds with the engine's clamps, decay, damage flash) and the draw (`src/palette.rs`, a `FullscreenMaterial` pass on the world camera that scales the finished picture's bytes by `1 + tint/2` — measured ×1.52 for a 1.0 red tint, identity byte-identical; the gated cantina pair rose 0.7315 → 0.7497 with it) | `route_catwalk`/`route_door` still score 0.45/0.42 against 0.72–0.74 for the untinted rows, because retail's own series turns red at ~5–6 s and the reference at ~10 s is saturated (r/g 2.0) while the port's 10 s capture lands **0 hits** at the harness invocation (health 100; the 15-hit figure this row used to cite is not reproducible — see the `sithAIAwareness` row) so the frame stays un-tinted where retail's saturation is what the pair sees, and the tint **state** is now proven exact against the EXE's own constants (`0x522e48` = 0 floor, `0x522e4c` = 1.0 red/green ceiling, `0x522e58` = 0.5 blue ceiling, `0x5230b4` = 0.04 per damage point, decay 0.4/s) - so the two rows' remaining difference is **content**, not tint | the AI's engagement timing (Phase 1 step 5); no port-side renderer or tint gap remains. The scripted `NewColorEffect` channels (filter/tint/add) reach the picture through the port's wash quad rather than the engine's palette bytes, so their colour is an approximation of the same rule |
| `sithAIAwareness` propagation | **ported (round 110)**: firing raises an event at the shooter (`CreateTransmittingEvent`, `0x4f2a90`, strength 4.0), and `ProcessEvent` (`0x4f2c30`) records it per **sector and side**, recursing through the adjoin graph with `next = strength − link->attn` and stopping under the floor `0x52315c`, which a direct `.rdata` read shows is **0.0** (the port carried an invented `0.00277` until round 346), so only a positive remainder crosses. `Runtime::sector_adjoins` pairs each opening through its adjoin `mirror` rows and takes the mirror row's `dist` as the attenuation; `transmit_awareness` walks it, `awareness_at` is the reader, and an actor whose room and side hold a fresh record walks to the shooter — reinforcements converge on gunfire, as the engine's do | `--adjoins` checks level 1's own graph: sector 0 → 13 costs **1.13**, a shot's 4.0 crosses it, a below-floor event crosses nothing and a side's record stays its own. In the door capture 16.5k actor-reads happen, but awareness only *walks* an actor toward the fight; firing needs sight | — (implemented; the remaining acquisition gap moved to the `LookForTarget` row) |
| AI turning | a body turns at `maxrotthrust / surfdrag` — 23 deg/s for a Ree-Yees gunner, 60 for a human — rather than the template value read directly (a rate) or a snap. The engine's `ApplyDrag` decays angular velocity the way it decays linear velocity, which is why the division is the reading; the angular integration itself is not recovered. **Round 111 adds the acquisition half**: an idle actor with nothing in sight runs its `LookForTarget` instinct (interval = the shipped first argument, 500 ms) and takes the player as last-seen when it is inside its own `sightdist` with a clear line, so the body turns at the same clamp instead of snapping | door capture at the deterministic 10 s clock: **46 shots, 15 hits with the scan; 31 shots, 2 hits with `BK_NO_SCAN=1`** — the room's engagement is reproduced, and the control shows what a facing-blind idle actor does. The five gated draw pairs improved when the capture clock was fixed (spawn 0.6246 → 0.6602, route_cantina 0.7279 → 0.7381 …) | `sithAICmd_LookForTarget` recovered (round 210: handler 0x50aa80, the acquisition chain is `sithAI_FirstThingInView`'s horizontal+vertical FOV cone → `sithAI_CheckSightThing`'s range−radius/facing/collision test, with an awareness event of strength 3.0 on acquisition), and the port's scan now uses the class's own view cone (round 211: `Ai::sees` in the stand-in instead of `range <= sight && clear`; the new headless test fails on the old condition and passes now). Still approximated: one `fov` number instead of the engine's separate horizontal/vertical limits, the thing origin instead of the model-insert eye point, no target-radius term, and no `sithAI_CanDetectSightThing` re-detect for a lost target; the class layout is recovered (round 214: defaults `maxstep 0.5`, `sightdist 20`, `heardist 10`, `accuracy 0.5`, `fov` stored as `0.5·fov`'s trig, and `wakeupdist` is a key this port keeps but never reads) — and `sithThing_UpdateMove`'s angular branch for the turn itself |
| non-player thing gravity | **ported for every body the engine gives it (round 266)**: a thing starts with its template's `physflags=` and `mass=` (the engine copies both; `spawn_at`/`fire_with_spread`/`explode` set the flags `0` before), and the mover applies the level's gravity when the mass (`+0x204`) is non-zero **and** `physflags & 0x1` is set — with no type test, because the engine has none. That covers the `_debris` family (`0x201`; `0x40000` halves it) and the **actors** (`_actor` ships `physflags=0x4a4f, mass=150`). An actor within `GetThingHeight * 1.1` of the floor it stands over holds its authored height (the attach `sithThing_Initialize` makes with `FindFloor(thing, 1)`), and one that walks off a ledge falls to the sphere contact. The fall stops on the highest floor plane **or solid-thing top** under the body across `sectors_around`, accepting only surfaces at or under where the step started so a lip it fell past is not a floor | `--things` drops a `CreateThing`'d `shrapnel_1` chip over a real sector centre: it falls **0.10** to a resting plane with velocity 0 and a sector still around it; with the gravity arm disabled the chip reports `falls 0.00` and the check fails. Unit check `an_actor_off_its_floor_falls_to_the_contact_and_stops` (falls from 0.5 to the contact, a massless body floats, a body inside the creation reach stands). `--soak-all` 35/35 clean before this round | the stop is still a plane query rather than the engine's swept `SearchForSurfaceCollisions`, `_debris` still rests with its origin on the plane (`shrp_1.3do` insert `[0,0,0]`, so the two agree), and a script-`AttachThingToSurface` body still does not follow its surface |
| player walking | **Resolved by issue #10:** the base is the engine's own arithmetic — `sithControl_PlayerMovement`'s `maxthrust * 0.7` (0x522f24) settled by `surfdrag + 0.2` (0x5231a4), i.e. **0.4375 u/s walk / 0.875 run** on level 1. The "measured 1.2" below was that *run* speed read as the walk, which `Rules::speed` then doubled to 2.4; the per-sample route trace exposed it (a whole sample early at every leg's wall, crossing retail's sector samples). Historical: the base speed was **measured from retail**, not derived: the engine's `sithControl_PlayerMovement` thrust carries a `thing + 0x144` term no recovered template parser produces, so `maxthrust / surfdrag` = 0.667 was 40% under. `Rules::walk` is 1.2 u/s (the clean `walk_street90` leg measures 1.07-1.23), `Rules::speed` keeps the engine ratios (run 2.0, crouch 0.5, back 0.5, strafe 0.7, water 0.5) and plain forward runs by default (shift walks), as retail does | **`walk_street45` now matches retail**: 2.21 units over 2.4 s of held forward against retail's 2.20 (0.92 vs 0.92 u/s), 0 of 5 sector mismatches, 0.60-unit max delta (sample timing); `walk_street90`/`walk_street` stop at 3.265 = retail's 3.27, sector 155 | the `thing + 0x144` term is recovered and the derivation replaces the measured constant **Round 272 reads the formula:** `sithControl_PlayerMovement` (`0x4d8a00`) sets the acceleration to `(thing+0x144 + thing+0x148) × input axis` and then scales it by run 2.0 / crouch 0.5 / water 0.5 / strafe 0.7; both terms are thing floats (0x144 is serialized in the player's save tail beside health), but no recovered parser names either, so the port's `Rules::walk` stays the measured value. **Blocker found and narrowed (round 286)**: the value is not in a save at all. A retail record's entry is `u16 type` + ASCII template name + position + angles + **one** float (100.0 = the player's health), and nothing else — measured on the install's own `saves/autosave.jks` (`docs/formats.md`, "a GOG-shipped save's 0x1001 record is a BKSV list"). So recovering `thing + 0x144` needs the *live* thing, not a saved one: either a retail runtime read of the player thing, or the template/thrust derivation from `sithControl_PlayerMovement`'s own inputs. |
| first-person weapon sway | `jkPlayer_DrawPov`'s sway is drawn with the engine's own composition: `jkSetWaggle`'s three amplitudes (degrees) and its phase rate, the phase advanced by the distance the body moved (`thing + 0x264`, capped at 0.02 a frame), the angles scaled by speed over `maxvel` (`thing + 0x21c`), and the three built into **one** `rdMatrix_BuildRotate34` frame — column 1 is the yaw basis tilted by pitch, and `runtime::rotate_by_angles` already has those elements, so the quaternion comes from that matrix instead of a Bevy Euler order (round 267). What still differs: the `thing + 0x168` eye offset under the rotation is still zero (nothing in the dump writes it) | the formula is pinned by unit checks on the bryar's own firing waggle (`10.0 7.0 0.0`, rate 350): 10°/7° at full speed, phase +7° a frame at the 0.02 cap, and a full waggle moves an authored corner at most **0.011** level units. `the_sway_rotation_is_the_engines_build_rotate_matrix` pins all three columns of `rdMatrix_BuildRotate34`'s elements. The bryar's clip and model are measured by `--weapons` (`bryvpst1.key`, 13 frames; `bryv.3do`, 15 materials) | a QA-window frame pairs a swayed weapon at a known phase |
| AI chatter (`talk`, 198 classes) | the handler's timing, chance and mood table are retail's (round 235), but the mood sound goes out through the port's **one-shot 2D request pipeline** with the actor's position for distance/facing attenuation — not the engine's mixer channels and 3D buffers, the same ceiling every script sound already has — and the handler's upstream one-shot bit is not modelled | `--sound` proves the data (5 of 5 mood events across 102 `.snd` classes; 72 of 73 `soundclass=` classes on level 1 archived, the 73rd a retail gap it prints); unit checks pin the interval/chance and all seven mood branches, and the tick check asserts the queued wav, its position and the interval | the mixer grows channel/3D-buffer management and the AI one-shot state is read from `sithAI.c`'s handler preconditions |
| `dodge` | both mechanical paths are ported (rounds 239–240) through the engine's own event dispatcher (round 241): the **damage** path and the **shot-passing** path (the player's `typeflags & 0x200000` projectile pre-sweep, the actor's own sight test of the shot, the random 45–90° sideways step). Its **event-2 branch is dead content** (round 357): event 2 is produced by the awareness sweep, but the arm at `0x50b5c1` wants the source to be a weapon thing (`[source+0xc] == 3`, `[source+0x10] == 1`), and every shipped record names the thing that made the noise — an actor or the player — never the bolt. The port walks both steps at the actor's own walk speed instead of `SetMoveThing(..., 2.5)`, and the five-unit pre-sweep uses the actor's `movesize` against the shot's line rather than the engine's full collision search | damage path: gate (walking bit `0x1`), step (`arg0`), direction and 1000 ms cooldown unit-checked; shot path: the shot's thingflags, the actor's sight/`sightdist`/AI-bit gates, the class step and the 90–135° sideways direction unit-checked; either call removed -> its checks fail | `SetMoveThing`'s speed parameter is carried by the port's AI walk, and the sweep uses the collision search instead of a line test |
| `sensedanger` | both paths are ported: the **instinct-update path** (round 236: on sight, awake -> alert, `fear`, the 3.0 awareness burst, the player target) and the **damage path** (round 241, through `sithAI_EmitEvent`: `surprise` `0x57`, the attacker's parent as the target, `flags & !0x4 \| 0x800`, no awareness event). Its event-4 branch comes from the actor/player contact test, event 2 from the awareness sweep (round 356), and `0x400` is not in its `0x7` mask. The **event-2 arm is ported (round 357)** — hurt-but-unalerted, the sector's kind-1 source inside `heardist` through a clear line, the instinct's weight above the record's strength, then target + alert word + an 8.0 re-broadcast — with two named substitutions (the arm's sound mode `0x4f` has no name in the port's table, and the cloak rules of `0x4eb090` are not modelled). The one approximation left is the target refresh — `sithAI_sub_4EAD60`'s `CheckSightThing` state is the port's `sighted` test plus the last-seen slot | the awake-bit/alert-bit transition, the `fear` row, the player target and the **3.0** awareness burst are unit-checked; the damage path's `surprise` row, flag swap and attacker-as-target are unit-checked; gates removed -> the checks fail | `CheckSightThing`'s own state is modelled where it differs from the port's sight test |
| AI detection (`CanDetectSightThing`) | the engine's detection **chance** is ported exactly from the `.rdata` constants (round 242: range falloff, awake bit, still/crouched physics targets, hidden flags), but the port rolls it only when an actor **first registers** the player and then keeps the deterministic line test, so an engaged target is never lost to a bad roll; the engine re-rolls on every target refresh. **The darkness term landed (round 314)**: the target's sector light (`AMBIENT LIGHT`) under 0.5 scales the chance by `light + 0.2`, unless the watcher's thingflags carry `0x2000000` — 67 % of the install's sectors bake below 0.5, so it is live across most rooms. **Round 317 closes its second gate too**: `thing + 0x2a8` bit 1 is the player's saber-on bit (`jkSaber_Draw`, `jkPlayer_DrawPov`), and a lit blade — like a target whose `thingflags` carry `0x4` — skips the falloff, darkness and pose terms, so a body holding a lit saber is noticed at any range | the whole chance curve is unit-checked against the retail constants, and acquisition is checked at a 0.2 roll over 200 ticks; gate removed -> the acquisition check fails | sector light is parsed and the roll moves to the refresh path, or a QA-window pair shows the port's acquisition timing matches retail at the recorded vantage |
| sector thrust arm | the engine gates the acceleration on the body's `mass != 0` as well as the `0x400000` physflag (round 237); the port's global thing loop now honours the flag, and `Runtime::step_player` applies the arm to the player's body (x/y through the walk direction, z through the vertical velocity) without the flag gate — the shipped player templates do not carry `0x400000`, so that half shows only if a script sets it | m5 soak `outside` 4 -> 0 with the flag gate; `a_thrust_sector_lifts_bodies_except_the_ones_flagged_against_it` | the physflags/mass pair is read off the thing (`+0x1d0`/`+0x204`) for every body, which is the same prerequisite as the gravity arm |
| particle clouds (`particle=` things) | the `PAR 1.0` clouds are loaded and **drawn** (round 244: one camera-facing `Sprite3d` quad per element at `element + INSERT OFFSET`, yaw-rotated, `SIZE` across, billboarded; `--par` probes all three shipped files). Closed: the shipped **`type=particle` templates** are generated and drawn (round 253), and both update arms are in (rounds 259–260). **Round 315 turns each element's offset by the thing's whole orientation** (`rdParticle_Draw` builds in the thing's frame; the old code used the yaw with the opposite sign), so pitch and roll now reach the drawn cloud. **Round 315 also ports the spin that turns it**: a template's `angvel=(pitch/yaw/roll)` now seeds `rot_vel` (twelve level-1 templates carry one, `fan1` among them), `ThingState::step` integrates it, and the renderer applies a spinning thing's whole pose each frame. Still open: `sithParticle_New`'s spawn path and the material's own effect state — `00teleport.mat` is **texture-less** (5 cels, 0 bitmaps), so the port draws the quads untextured while retail draws the material's effect | `--par` asserts each shipped cloud against `rdParticle_LoadEntry` (completeness, every element cel inside the material's cel count; `test.par`'s missing `test.mat` reported as a retail gap), `--audit` reports `par 3 3`, and the element-offset/yaw check is unit-tested | a QA-window frame shows the Force shield's cloud at a known pose, and the generated (`type=particle`) systems draw their elements |
| non-level-1 draw pairs | level 5's pair scores 0.2982 full-frame / 0.3648 world-band (round 103: the HUD band is *not* its problem) with a **verified pose**: the reference is a player-camera frame and, at the retail view size, the port's sweeps peak at the row's own pitch 1.12 (0.3054 vs ≤0.28 either side) and yaw 134.99 (0.3057 vs 0.23–0.26 for twelve other yaws), so the camera is not the gap. What is: a sharp 24-bit image against retail's blocky 16-bit one, animated water over half the frame, and renderer colour — layout-normalised SSIM (HUD strip cropped, blur, 234×112) is 0.5925 against 0.8344 for the known-good level-1 spawn pair. Level 2 was **mis-diagnosed** in round 101: its reference was not a cutscene but a player-camera frame the harness had cropped to a 1094×355 strip (the world-box detector's "lit" test could not see a dark plaza), and re-captured it reads 0.3359 / 0.3489 with the masked score above the full-frame one, as a coherent pair should | both pairs stay **ungraded**; level-1 thresholds hold 6/6 and the gated pairs pass the world-band check (0.69–0.80); `retail-capture.ps1 -ProbeBox` re-derives any retained raw frame's box | level 2 and level 5 alike: a metric that survives content/renderer differences on dark and sky-and-water vistas (edge or landmark based) |
| interaction triggers (retail side) | a door/switch fires only on the engine's own **use action**: `00_doorswitch.cog` opens on the `activated` message from **sender id 2** and `00_door.cog` on `activate`, while the `blocked` message is only ever sent by a *moving* thing whose animation is obstructed - so walking into a closed door can never trigger it. The profile binds ACTIVATE to DirectInput **0x102 (middle mouse)** plus VK 57 **Round 302 measured both ends**: the port now uses the engine's `moveSize + 0.1` reach (**0.16**). At `switch_cantina` (-4.75) and `switch_cantina2` (-4.29,-2.95) the port's use reaches nothing, which is retail's recorded `doorstatus 0` at both poses; at the new `switch_use` row (-4.29,-3.76, 0.12 from the plate) it sends `activated` to surface 2409 and cog 11 reads `doorstatus 2`. The retail *positive* is still unmeasured: the harness cannot drive the middle-click (see the progress log, round 302) | no retail trigger captured yet — **and the engine's own sweep explains why (round 271)**: `sithPlayerActions_Activate` (`0x4fc0a0`) sweeps from the eye and passes `moveSize - _DAT_00523200` as the radius, and `0x523200` is **-0.1**, i.e. **0.16 units** of reach for the player (round 302 corrected the earlier 0.265), so the recorded switch poses (~0.8–1.0 away) are out of range and `doorstatus 0` is correct behaviour. The port now uses that reach instead of its old 3.0. The exact switch poses are known (`--links` surface centroids: 2409/2403 at -4.29,-3.88/-3.62,-1.15; things 56/57 at -4.75/-4.45,-3.75), the middle-click and `9` key are injected with the cursor inside the game window, and cog 11 stays `doorstatus 0` (`cogstatus11-middledclick-01narshadda.txt`); the port side is verified instead by the **`--door` gate**, which runs the whole chain on the level's own scripts (`Runtime::use_thing` -> `used` message -> `MoveToFrame`/`SetSectorAdjoins` -> `collide::Gates`): cog 0's piece reaches frame 1 after 3 s and **4 doorways the solver refused become passable** | synthetic mouse input reaches the switch (works with a retail-side trigger) or a keyboard-only trigger is found; retire the deviation then The 250 ms throttle (`DAT_008554bc + 0xfa <= clock`, stamped before the sweep even on a miss) is in the live path (round 273). Residual: the port's scan measures to a linked surface's **centre** where the engine's sweep touches the face anywhere, and the AI's `opendoors` keeps a 1.5-unit stand-in reach. | **Round 304 scoped the sweep to the body's sector**, which the engine does: surface 2409 belongs to another room's surface slice (the port's own table, printed by `--use-live`: sector 249 against the player's 260), so retail's `doorstatus 0` at the plate is faithful and the port now reports `use: nothing linked within reach` at all three measured poses. The key->sweep->message->door chain is proved on the port side by the rewritten `--use-live`: standing 0.06 from the plate in its own room, E gives `activated surface 2409` and the cog reads `doorstatus 2 of numdoors 2`. 
| thing collision (solid models) | `sithCollision` treats a collidable thing's model as solid surfaces: a moving body is **blocked** by it and slides. The shape follows the engine's own split (round 410): a `collide=3` thing is its **model's oriented bounds box** (`sithIntersect_CheckSphereThingIntersection` uses the mesh only for that value), and every other collidable thing is a cylinder sized from the model's plan bounds. `move_body` sweeps the body against the box or circle in `sweep_things`; push-out and sight/shot segments use the same shape. Round 124's blocking with the *template* radius was measured and reverted (it sealed gaps retail walks through: `walk_cantina` 102 % → 12 %, `walk4_spawn` 100 % → 50 %); with the model shape those legs hold and the block works | trace gate **10 legs, 0 failures**: `walk_cantina` 102 %, `walk4_spawn` 100 %, `walk5_spawn` 82 %, `walk17_spawn` 100 % (0.70 vs 0.70, xy 0.005); `walk_switch` stops at **-3.6725** in sector 59 against retail's -3.67. Still outside: `walk3_spawn` 266 % and `walk6_spawn` 190 %, because retail's stops there are *not* props (level 3's is scripted; level 6's is stair-climb speed) | the model's real surfaces (not its bounds box) are swept, and the stair/step path is read against retail's climb rate **Correction (round 286)**: the canopy lead is retired twice over. (a) `BK_THINGS_TRACE` lists the same **225** solid things and the same **7** `canopy` entries with and without an engine-rule switch (`has a model` + `flags & 0x80002 == 0`, `decomp/004e8430`), and the legs measure byte-identical (walk3 213 %, walk5 105 %, walk6 95 %) — the port already collides these canopies. (b) The lateral geometry rules them out as the contact anyway: retail's stop line is y -3.60 at x ~ -0.7, and the nearest canopy centre `(-0.377, -3.613)` is 0.32 away against a 0.265 contact radius (canopy 0.2 + body 0.065), so no canopy touches a body there. **So the walk3 stop is still unidentified**, and the two things it is not are now measured: not a missing solid thing, not a canopy. **Ruled out (round 286)**: a swept step along retail's own line — `--move-at "-0.70,-3.00,-0.31,0.0,-0.0217"` — walks y -3.00 to **-4.30** with no distance lost and no sector change, so this port's collision set puts **nothing** in a body's path where retail stops. The contact is therefore either a thing the walker's list does not model (actor bodies are deliberately excluded) or a surface outside that sweep's span; that is the next probe, not another ladder of guesses.  **Round 292:** the cylinder list is now the **moving body's own sector's** list — `SearchForThingCollisions` walks the sector thing list, so a prop in another room cannot touch the body — and it is rebuilt from the live runtime positions every frame, because a door or lift follows its frame path (`18Ascend`'s spawn stands over elevator leaves in sector 504 and the frozen closed pair pinned it). Both were measured on `walk17_spawn`, which closed at 117 % with 0 sector mismatches; `walk_cantina`, `walk4_spawn` and `walk6_spawn` still pass.|
| slanted-floor sliding | the engine's slope slide is `sithPhysics_UpdateThingPhysics` subtracting the **world's** gravity from the thing's **z velocity** (`world->gravity * dt`, `DAT_008339c8 + 0xd4` = the `.jkl`'s `world gravity`, 4.0 in the shipped levels; halved by thing flag `0x40000`) and the floor collision sliding that downward motion along the surface. This port's walker keeps a body *on* its floor (`pos[2] = floor + rest`), so it has no z velocity to slide, and on level 3's slanted ground retail drifts downhill (−x 0.38, first window 1.35 units at 2.25 u/s) while the port drifts +x 1.2 and crosses a different set of ground triangles (3 of 6 sector samples). A **planar** version of the force was built and reverted: it double-counted a body already moved by input — level 5's passing leg regressed (sectors 2 → 3) on its 45° ramp. **Round 413** ports the idle half: `floor_contact` supplies the landing normal and `slope_step` adds the down-step's tangential horizontal part. Level 3's authored body now slides from `(-0.68,-5.23,0.27)` to `(-0.747,-4.947,-0.171)` in sector 370 (retail `(-0.76,-4.75,-0.20)`), no fall damage; the slide is idle-only because the input walk and the gravity addend double-counted `walk9_spawn` (4.38/1.68). Remaining 0.20 units need the engine's combined 3D move | `walk3_spawn` still 125 % (the route uses input); level 9 is 2.00/1.68 with the idle restriction; trace gate 10/10 | port the combined `MoveThing` vector so input and slope are one move, then remove the idle shortcut |
| cutscenes (camera + player lock) | **The premise is contradicted by measurement (round 173): the engine's control lock is the *video-cutscene* camera, not a `SetCurrentCamera` focus.** `sithControl_Update` (`decomp/004d6c70`) skips `stdControl_ReadControls` only while `DAT_0055d5d4` is set, which is the camera object `jkCutscene` installs; a `SetCameraFocus(slot, thing)` + `SetCurrentCamera(slot)` focus (level 14's shape) does **not** lock the controls. Measured: level **20**'s opening holds a script camera on thing 44 and retail's body walks **4.20 units**; level **21**'s holds one on thing 23 and retail walks **0.33**. The port suppresses input whenever a script camera is current, which is right for 14/7 by accident: their bodies are motionless for other reasons (14's Kyle is **prone** after the knockdown cutscene, 7's pin is its own mechanism, 10's has no camera verb at all). Removing the suppression was measured and reverted: `walk21` **passes** (0.34 vs retail 0.33, 0 sectors), `walk20` improves to 2.85/4.20 = **68 %** (3 sector mismatches), but `walk14` 5.21 vs 0.00 and `walk7` 2.54 vs 0.11 break their legs. Level 8 walks 0.96 and stops | the four measurements above; `walk14`/`walk7` are gate legs today (forced-pass by suppression) | **retired when the player-immobility state is ported** — the prone/knockdown body (level 14) and level 7's own pin — so the walker can drop the blanket suppression and levels 20/21 join the gate; the ledger row then becomes "player animation/knockdown state", not "script camera" |
| solid things are floors | a solid template's model **axis-aligned bounding box** (top + footprint, `set_template_floors`) stands in for the engine's drawn collision surfaces, so a thing rotated off the axes is covered a little generously and an L-shaped model offers its whole box as floor | the catwalk pose holds at retail's z; `BK_WARP_TRACE` prints every solid thing's model top around a pose; a unit test pins the probe | thing-model collision surfaces (`sithCollision` against `_walkstruct` models) are ported |
| rank and force-point crediting | **retired (round 108)**: `jkMain_EndLevel` grants nothing. The port used to raise the rank (bin 20) by one and credit three stars (bin 16) per level, but retail's own saves prove the engine does not: level 1's load-time autosave and level 2's save (written after level 1 was finished and its tally shown) carry **no** `16`/`20` at all, and the decomp reaches `jkPlayer_SetRank`/`jkPlayer_FixStars` only from multiplayer's new-character screen — nothing recovered writes the point bins | the port's level-2 carry now matches retail's `jkauto-02narshadda.jks` bin for bin except the route-dependent population bin (`73`): `1, 2, 11, 13, 42, 60, 63, 71=8, 75=20, 99=2000, 100=1, 102=1` | — (`finish_level` still lands the end-of-level state and the tally still reads the rank and bin `0x10`; single-player force powers come from the episode table, not from rank) |
| thing activation | **retired (round 107)**: a thing's class cog is sent `created` when its sector is first drawn, which is what retail's two autosaves prove — the level-1 load-time save carries no population bin (73 = 0), the level-2 save carries 10.5 after the level was played. `Runtime::awaken_sector` tracks the woken set (idempotent) and `cull_sectors` calls it for each visible sector; the `--activation` gate holds the load state to 0 and requires the first wake to credit the bin and the second to do nothing | the port's load state now matches retail's load-time fixture; the after-play number is route-dependent on both sides | — |
| AI engagement at a data-derived vantage | **closed (round 284): the damage divergence no longer reproduces.** At the row's own pose (`--start 03katarn --warp --yaw-z -53`, the vantage it recorded) the port fires **two** `+ebow` bolts and lands **zero** hits — `BK_FIRE_TRACE` lists both (`owner 237` yaw -91.4 pitch 12.3, `owner 245` yaw -0.4 pitch 28.3) and `BK_HIT_TRACE` prints no `hit: the player` line in 300 frames or in 1800 (8 s, 30 s). Both bolts run into the spawn fence geometry, which is the same reason retail's HUD reads 100/50: at this vantage neither engine damages the player. The round-209 reading (shields 100 -> 97 within 2.5 s) required a different code state and is retired | the damage comparison this row existed for, both sides measured at one pose | retail exposes no shot counter (`aistatus` gives instinct masks/modes only), so **shot counts** stay uncomparable — only damage is, and that now agrees |

| `forcepowers` on non-player actors | **ported (round 252)**: the class line's three `(range, chance, cooldown)` triples select ability 1–3 from one `rand/32768` draw, and the actor's own scripts get `user0` with `(victim id, ability)` through the same thing-message path `sithCog_ThingSendMessageEx` uses. What still differs: the message is queued on every instance bound to the thing (the engine's two slots plus mask subscribers), and whether a power actually happens depends on the shipped `darkjedi_*.cog`'s own `okay`/health gates — `yun`'s `user0` returns until `07_Rahnlines.cog` sends `user1` | `forcepowers_queues_user0_on_the_actors_script` and the class-line rule test; a live level-7 run still needs the level's own script hand-off | a level-7 capture at the arena where Yun's `user0` visibly fires, or the AI-side gate when `--level` can be driven through a probe |
| `saberfighting` on non-player actors | **ported (round 252)**: the 500 ms clock, `fltArg[0]` chance, the four `(range, damage, cooldown)` entries walked in order, AI flag `0x400`, puppet mode and `DAMAGE_SABER` damage. Unrecovered: `jkSaber_Enable`'s argument order (the port does not call it), the `PlayModeFirst` sound id (`entry + 0x47`) and track fade, and the target-typeflag `0x202` saber-lock branch; the port also skips the puppet mode's visual clip | `saberfighting_hurts_the_target_and_honours_the_cooldown`; the handler's disassembly pins the ranges/damage/cooldowns against `yun.ai`/`sariss.ai` | the saber engine (`jkSaber.c` swing arcs) lands, or a retail frame at a dark Jedi duel pins the animation/sound |
| head/aim joints | **ported (round 263)**: `sithActor_SetHeadPYR` (`0x4ed280`) stores the head pitch and applies it to `k_neck`/`k_head`; `sithAI_SetLookFrame` clamps by the template's `minheadpitch`/`maxheadpitch`. The port stores `ThingState::head_pitch` (the AI elevation, clamped), and `model_mesh_posed_aim` post-multiplies the node's local pitch at 0.5× (neck) and 0.3× (head), while the body mesh uses yaw only. | `a_head_pitch_turns_the_node_about_its_local_x` and the AI elevation test path | a frame at a target above/below the body, or the `--ai` run with a raised target |
| AI events 2/4/0x200 | **event 2 and event 4 are produced now; `0x200` is not**: round 356 read the trigger for event 2 out of the disassembly around `0x4f2c05` — the per-frame function at `0x4f2b60` bumps its tick counter (`DAT_008553b4`), every `ProcessEvent` call stamps its sector slot with it, and then the level's thing table is walked: a thing whose sector's slot carries the current stamp gets `sithAI_EmitEvent(thing, 2, 0)`. `transmit_awareness` now raises that event (once per thing per tick, which is what clears the `0x1000` one-shot latch); round 264 raises event 4 from the actor/player contact test, which unblocks `specialattack`. `0x200` (`0x4ed203`) has **no reachable producer**: round 356 traced it into a helper installed at `0x848600` (`0x4e6f05`, next to the per-thing-type callbacks at `0x847f38`), and no code reads `0x848600` by address, so the call is dead in this build — which is why `jump`'s `0x200` arm stays dead content | the sweep is disassembly-evidenced (`0x4f2b60`..`0x4f2c19`, `test dword ptr [eax], 0x202`, `mov eax, [eax+0x5c]`, the `cmp dword ptr [edx + 4*ecx], eax` stamp compare, then `push 0; push 2; push esi; call 0x4e9f80`); the unit check `the_awareness_sweep_re_arms_the_one_shot_dodge_latch` asserts the latch gate, the clear and the once-per-tick guard |ness | the actor-collision sweep emits event 4 (and awareness emits 2) at the engine's sites |
| generated `type=particle` clouds | **ported (round 253)**: `sithParticle_Initalize` (`0x4f1da0`) generates `count` elements from two uniform Euler angles (720° default spans) and a radius in `[minsize, range]`, with a random material cel when `typeflags & 4`; the port feeds them through the same `template_particle` draw path a `.par` cloud uses. **Both update arms are ported** (grow round 259, cel cycling round 260: `scale += maxthrust*dt`, per-element cel probability). What differs: the per-element material effect state beyond a keyed bitmap is not modelled, the cloud's `angvel` spin is the thing's angular velocity, which round 315 now seeds from the template and integrates, and the element RNG is a local LCG rather than the engine's global stream | `par::tests::a_generated_cloud_fills_its_authored_radius`; level 1's loader now reports **24** generated clouds, and `--audit`/`--soak-all` stay clean | the update arms land (`sithParticle_Update`) and a frame at a known explosion age pins the drawn cels/scale |
| first-person viewmodel framing | **re-fitted round 279 on a rendered frame, HUD off**: `jkPlayer_DrawPov` (`0x4051e0`) is `view * pretranslate(-eye) * sway` then `rdThing_Draw`, and `rdThing_SetModel3` zeroes the per-joint offsets, so the engine applies no hidden model offset or scale. What the port got wrong was placing the authored box at the eye: `bryv.3do`'s own geometry spans z -0.062..**+0.010**, so the camera sat *inside* the arm and gun and the near plane sliced a slab out of the model — the weapon covered 11.9 % of the lower view band against retail's 4.0 % (`BK_NO_HUD=1`, level 1 `spawn_pose`, both sides in the 1170x730 box retail's fullscreen crop shows, weapon isolated by armed-vs-`__none__`). `VIEW_BASE` now carries z -0.03, which clears the eye plane (nearest vertex -0.092) and lands the mask at 2.5 %, bottom-anchored. `BK_VIEW`/`BK_VIEW_SCALE` remain for measurement; the mount clip (`BK_POV_CLIP=1`) matches retail's arm silhouette but still throws the gun out of frame, so the rest pose is what ships | the mount clip's rotations are reconciled with the joint-matrix convention so the drawn pose is the animated one, and a T3 pair pins it |
| AI walkability (`sithAI_CanWalk`) | **ported (round 276)**: before a step, the engine sweeps a sphere down from the step's **destination** for `GetThingHeight + <class field at +0x10>`; **any** surface hit is walkable (a floor, or a wall the physics step-up handles), no hit is a ledge, and the caller (`sithAI_idk_msgarrived_target`) zeroes the AI's velocity and emits `0x400` — only for a body **attached** to a surface; an airborne body keeps its horizontal motion. The port gates both walk paths (flee and chase/route) with the same sphere sweep — and `jump_from_obstacle` tests its **leap point** with the same helper (the engine's `FUN_004eb4b0(ai, point, 0)`), where it used to ask only whether the line was clear — and counts a **solid thing's top** as ground because its floor search knows the level's faces but not catwalks or lifts. Inferred/not modelled: `+0x10` is taken to be the class's `maxstep` (0.5 default) and the sweep radius is the body's `movesize` (Ghidra's prototype omits the radius); the engine's walkable categories (its surface-flag bits pick 1 vs 2 vs 0) are not used for steering; "attached" is approximated by "ground under where the body stands now", because the port has no floor-attach state | `an_actor_stops_at_a_ledge_instead_of_walking_off_it`: an actor halts before the floor's edge and emits `0x400` and the no-collider control walks on; `the_ledge_reach_is_the_body_height_plus_maxstep` walks a 0.4-unit drop (inside the reach) and stops at a 1.0-unit one; `a_blocked_actor_does_not_leap_onto_a_void` refuses a leap with no ground under the point and takes it with ground | the class-field offsets are pinned from `sithAI_ParseArg`, the surface-flag categories steer, and a QA-window `--ai`/soak shows no actor frozen at a wall, doorway or lift |
| thing-vs-thing collision | the engine collides a moving body against a thing's **model surfaces** (`sithCollision_SearchForThingCollisions`), from either side. **Round 307 ports the body half**: `ai_can_walk` refuses a step into another live body's cylinder (player/actor types, each template's radius, the actor itself skipped), so an actor can no longer stand inside the player — the authored 0.27-unit overlap at level 1's spawn that filled the view with the intro Ry. The model-surface half (a swept body-vs-model test, model bounds rather than the template cylinder) is still open. **The `Gates::passable` shut-sector rule is checked now (round 314)**: `--adjoins` repeats the same sweep with the far room's openings hidden (`SetSectorAdjoins(across, 0)`) and requires the body to stop at the opening's plane and stay in the near room, then reopens and requires it through — the ledger's own fixture, with a body rather than a predicate | `--adjoins`: the shut sector stops the sweep at the plane, `passable` reads false, and the reopen crosses | a swept body-vs-model test — the same prerequisite as the solid-collider row |
| AI projectile spawn (no `fireoffset`) | the engine spawns a projectile at the **weapon thing's** position (`sithWeapon_FireProjectile`, `0x4d5f20`: `param_5`, the offset, is compared against a sentinel — unset means "add nothing", else it is rotated by the weapon thing's orientation and added to the thing's own position), and a shipped weapon thing sits in the actor's hand. This build has no separate weapon thing, so the port stands in with `body + forward * 1.0, z + 0.1`. On level 3's spawn the body stands on a stair, so that point is inside the next riser: `BK_AI_TRACE` prints `ai: shot 345 pushed out at [-2.186,-5.214,-0.551] -> [-2.230,-5.175,-0.551] (sector 402)` and the bolt stops with `vel [0,0,0]` on its first frame. Bolt 351 then dies to `segment_blocked` at `[-0.980,-5.029,0.055]`, i.e. the thing step's position/segment tests against a **stale sector** (the `collide.rs` `step` simplification). Nine shots at 1.2-3.8 units over ten seconds produce **no player damage**, so the HUD reads full health exactly like retail's series — by accident, not by the engine's rule | `--hit` calibrates the current stand-in (`the bolt landed ... for 20`) and `--gates` is 0 of 66 with the chain green; the port's tuskens do fire (`ai: 237 fires PrimaryFire ... damage 20`) but nothing lands. Spawning at the shooter's own position (the engine's sentinel branch) was tried and **broke `--hit`**, because a 0.15-radius bolt born at a body's centre overlaps the floor | a separate weapon thing exists (so the sentinel branch has a real position to spawn at), or the thing step puts projectiles on the swept sector path |
| lateral step allowance | the engine has **no lateral step**: `sithThing_UpdateMove` normalises the move and calls `sithCollision_MoveThing` with no raised retry, so a riser is climbed only by the sphere sliding up its edge. The port carries `STEP_UP = moveSize + 0.01` and a four-way raised test, which climbs risers the engine would reject and rejects ones it accepts (the touch-at-distance-0 case) | `a_low_ledge_is_stepped_onto_and_a_tall_wall_is_not` pins the port's allowance; no retail leg isolates a single riser yet | the four-way test is replaced by the rise-against-top-edge rule the marker names, and a retail leg crosses a stair |
| force point spending | the order a force-point spend applies its rules in (pool → rank ladder → side lockout) is inferred; the amounts, the ladder and the lockout are from the executable | `force.rs` unit checks on the shipped ladder and the lockout | the `jkGui` force screen's click handler is read, or a retail purchase pins the order |
| lighting | the port lights a world surface with `light_scale(baked vertex) * (1.0 + surface extra + scripted sector light + thing light)` — **fixed**: the factor's base is now the sector's own `AMBIENT LIGHT + EXTRA LIGHT` (`Runtime::sector_light_baked`, the value `sithRender_RenderSectors` passes to `rdCamera_SetAmbientLight`), still shaded through the colormap ramp. Level 2's 641 sectors average **0.523** ambient; with 1.0 as the base the spawn pair measured **33.0 vs 16.6 (1.99x)**, with the ambient base it measures **17.0 vs 16.6 (1.02x)**. Level 1 moved only 42.0 -> 40.5 (retail 48.9, ratio 0.83), so the remaining difference is not a constant factor now. The engine also adds per-surface light and dynamic thing light with a distance falloff (the port sums thing light per sector) and the sector ambient on the viewmodel camera; the engine adds per-surface light, dynamic thing light with a distance falloff (the port sums thing light per sector) and the sector ambient on the viewmodel camera | **First lit pair (round 297):** `level2_spawn` with a verified retail pose (booted through the `[Debug]` chooser) grades at **SSIM 0.3705, mean abs diff 23.8/255** — the geometry, tower lights and floor line match, retail's frame is much darker, and the port's HUD is in frame while retail's is not, so the number is an upper bound on the lighting error. `level_mat.rs` and `runtime::set_thing_light` name the inputs | a rendered pair at a lit surface (T3) with the shader taking sector/thing light | **Level 1 is not the same factor, and not a global one:** its 462 sectors average 0.543 ambient too, yet the spawn pair moved only 42.0 → 40.5 (retail 48.9, 0.83), so that view sits in a bright sector and the residual is a different term (sky, or the ambient's form in a lit room). The next step is a per-surface predicted-vs-rendered comparison on that view, not another global constant. 
| force-screen row gating | `hoverId` **is** the element `id` at `+0x04` and the executable screens are read; what no screen carries is the force screen's rows — `jkGuiForce_pElements` is a heap array the engine fills in `jkGuiForce_Startup`, so the port's "hide a power until it is found" rule lives in a unit test instead of the screen data | `menus::tests::force_rows_hide_until_they_are_found`; `--ui-visible` prints 0 shipped screens gated (the reason is on its output) | `jkGuiForce_Startup`'s array build is read and the rows gate from data |
| puppet clip timing | a scripted clip's start frame has no own epoch, the locomotion clip's units-to-frames constant is not recovered (the port normalises at the 0.5 u/s walk/run threshold and clamps ×0.5..×2), a thing track is found by a mesh-name test rather than the engine's root-track rule, and `RotatePivot` turns the body but not the arm (every shipped pivot is yaw) | the actor clip/state checks and `animate_actors` | `sithPuppet_UpdatePuppet`'s advance constant and `sithPuppet_UpdateThingMove`'s root-track rule are read, and a retail stride is measured and the stride state is picked from `walk_vel`'s averaged displacement rather than a thing velocity (`ai_tick` writes positions, so a teleport reads as one huge step) **Round 272 reads the advance:** `sithPuppet_UpdatePuppet` (`0x4e3de0`) advances the locomotion track by the dominant velocity's magnitude `× dt × 280` — `0x522fbc` is 280.0 and the sign test at `0x522fb8` is 0.0, so it is the absolute value — and `sithPuppet_UpdateThingMove` (`0x4e4380`) returns that dominant component and picks the mode (walk under 0.5 u/s, run at or over it; 1.0 for a player thing). The remaining unknown is the key struct's `+0x34` unit, which the advance accumulator wraps on: without it 280 cannot be applied to the port's 15 fps frame law. |
| surface culling (world and models) | level world surfaces are drawn **double-sided** (`StandardMaterial { cull_mode: None }`) because the data's winding is not guaranteed consistent — `--winding` shows 91233 surface normals point into their room and 966 of 87377 wind against their own normal, so honouring winding now would hide walls on the 1.1%; model interiors likewise render when the camera clips in (gaps §4) | `--winding` over all 35 levels (the counts above); no retail pair isolates a hidden back face. **Round 279 measured the engine's own rule** (`sithRender_RenderSectors` `0x4c6c40`: a surface draws only when the eye is on its plane's positive side): implemented as a fragment test on the interpolated world normal it is provably live (inverted, it changes 66.5 % of level 2's spawn frame and 76.1 % of level 5's) but inert in the correct direction (0.07 % of level 5's pixels; A/B on the seven verified poses moves SSIM by at most 0.0011). It was reverted rather than kept, because the mesh ships the **Newell normal from vertex winding** while the engine tests the sector plane's stored normal — precisely the 966 surfaces that wind against their own normal — so the wrong normal would hide walls the engine draws | the plane normal is carried into the mesh and a T3 pair at a model interior holds |
| scrolling surfaces | **The stale bound is one frame now (round 341).** The scroll has its own epoch (`Runtime::scroll_epoch`), bumped every frame the level has a `SlideWall` rate or a sky rate, and the renderer's scroll-only path re-lays **only the batches that contain a moving face** (`build_world`'s filtered build, gated on the key's full surface list so a mixed batch is never half-patched). The offset was up to 66 ms stale at 15 Hz; static batches are no longer touched at all. `--scroll` asserts the scroll epoch moved and `surface_epoch` did not | `--scroll` (the epoch split), plus a spawn capture after the change (`--shot`); the engine's per-vertex shader channel would only save the rebuild itself, not the correctness | a per-vertex scroll channel exists in the port's shader |
| thing attachment (record + following) | `update_attachments` is one level deep and resolves in index order, and the record the engine keeps (`attach_flags`, `attachedSurface`, the surface-info snapshot, scroll-velocity transfer, `entered`/`exited` on a COG_LINKED surface) is missing; nothing follows a surface's motion | no check; no shipped script attaches a thing to a thing that is itself attached | the record is ported and `sithPhysics_UpdateAttachedThingPhysics` moves a script-attached body with its surface |
| weapon auto-select | the bin→template mapping is the engine's own rule first (a type-3 template that names the bin in its `inv=`, which every shipped weapon carries); a name-hint list is the fallback for a level whose templates omit it. The **auto-switch** ranks by `damage=` because `sithWeapon_AutoSelect`'s `AUTOSELECT` query cannot be made from the runtime: the cog VMs live in the message-delivery systems, not on `Runtime` (the item cogs themselves are instantiated now, `load_item_cogs`) | the `inv=` mapping is tried first and `--weapons` starts a weapon's script; `auto_select_weapon` is unit-checked on the damage ranking and tie rule | the VMs are reachable from the runtime (or the auto-select moves into the delivery path), or `weaponData` is recovered |
| cog link filtering | the port carries the link **id** (what `GetSenderId` returns) but not the engine's message *mask*, so every link to a target hears the message. **Sector links now carry the engine's own gate (round 296):** `sithThing_EnterSector` (`0x4cd2c0`) messages only a `SITH_SECTOR_COGLINKED` sector (`flags & 0x04`) and only a thing whose flags carry none of `0x80100`, and `Runtime::sector_messages` applies both — without it, `06_endpart1.cog`'s goal sequence ran from a walk spawn that merely stood in the room and ended level `06abarons` mid-capture. The per-message mask is still unported | `wire_links`' counts (37 thing / 44 surface / 10 sector on level 1) and `runtime::tests::sector_messages_only_reach_a_cog_linked_sector_and_a_watched_body`, which pins the sector gate and the thing-flag exclusion | the message mask is applied per link id and the level's triggers still fire | |
| cog nested `sleep` | `sleep`/`waitformessage` **do** suspend and resume a label across frames (`cog::Vm::call_verb` handles them before the runtime, `Vm::resume`/`resume_sleeping_labels`, gated by `--sleep`), and `setpulse`/`settimer` are per-cog clocks. **Closed (round 295):** `Vm::resume` now carries the *statement path* (`Vm::path`: statement index plus `if`/`else` arm per level) and `Vm::run_path` re-enters at the exact statement, so a `sleep` inside an `if` continues with the statement after the wait instead of re-running the branch prefix. A `sleep` inside a loop still resumes at the loop statement — deliberately: the loop's condition and step belong to the statement, and a mid-body re-entry would have to replay them; `run_path` truncates the descent there. | `--sleep` pins top-level suspension and resumption (and fails when the suspension is removed); `cog::tests::a_sleep_inside_a_branch_resumes_after_the_wait` pins the nested case — it fails under the old top-level-index record ("the branch prefix ran twice") and passes with the path | the VM keeps a statement stack per suspended label and a script with a `sleep` inside an `if` resumes at the statement after the wait |
| map panel camera | **shipped (round 334)** through the engine's own camera: `jkGuiMap_Show`'s **90-degree horizontal fov / near 0.2 / far 10**, the eye 2 units behind and 0.3 above the body (the inverse of the shot's `translate(0,-2,0.3)`), the view axes **(right, forward, up)** fixed by `rdClip_Line3Project`'s division on the point's second float, per-edge near-plane clipping and a rect clip; `sithMap_Draw`'s facing cull and `sithMap_IsSurfaceDrawable`'s seam rule are unchanged | `--map-shot`: **11 765** in-panel ink pixels, 489 edges drawn, 2 109 surfaces culled facing away, 2 447 seams hidden, the marker at the projected body (`300, 239`), and a body moved 3 units draws 18 549 pixels instead | the arrows' 5-degree orbit (pitch clamped to +/-90) and the 0.1 D-pad pan, which need the screen's click routing The arrow **element ids are recovered too**: the two callbacks switch on `id - 0x9a` (4 cases: yaw +5 / -5, pitch -5 at the -90 clamp, pitch +5 at the +90 clamp) and `id - 0x66` (4 cases: `PostTranslate` `(+0.1,0,0)`, `(-0.1,0,0)`, `(0,0,-0.1)`, `(0,0,+0.1)`), so wiring is a table. `--map-orbit <pitch>,<yaw>` drives the same camera headlessly today (level 1 at `-40,30`: 16 968 ink px, 1 776 edges, 2 104 facing-culled, 999 seams; the PNG shows the room from above-behind). The map screen's controls are **half-wired, and the mismatch is now stated rather than implied**: `--ui-tables` gives its six hidden `type 1` elements the ids **100..105** (`{click 0x415b60}`, a handler absent from the 2,400-function set), while `MapCamera::arrow` carries `0x9a..0x9d` for the euler steps and `0x66..0x69` for the 0.1 pans — the recovered pan table (`rdMatrix_PostTranslate34` keyed `id - 0x66`) lands on ids 102..105, so those four are live and 100/101 have no assigned action. `Overlay.map` holds the camera, `activate` routes the ids, and `jkGuiMap_Show`'s reset on open is mirrored; `MapCamera::arrow` is unit-tested (5-degree steps, pitch held inside +/-90, 0.1 pans, ids outside the table ignored). **Retires when `0x415b60` is decompiled** (it is not in the recovered set) or a retail click read pins which two ids rotate. |
| frame-time clamp | `sithTime_Advance` (`0x4dd640`) clamps each frame's delta to **[10, 500] ms** (then scales it by `_DAT_00522f94` when the `0x400` slowmo bit is set) and accumulates game ms at `0x00836c14`. **Round 312:** the round-296 note below was wrong - `sim_dt` still capped at 0.1 s with no floor - and the clamp is now `engine_frame_dt` (`clamp(10, 500) ms`, unit-tested), so above 100 fps the port's clock runs faster than wall time exactly as retail's does; captures and gates stay pinned to 1/60 through `CaptureDt`/`BK_FIXED_DT`, so the change only reaches live play. The slowmo scaling (`0x400` bit) is still not applied, and it is unreachable from content: the bit's only writers are the `setdebugmodeflags` cog verbs, which no shipped script calls (the `--cog-check` sweep names every verb content uses). `_DAT_00522f94` is **0.2** (read from the image), recorded for whenever a debug path needs it | `--gates` 0 of 66, `--walk all` 0 in the void and the soak pass unchanged after the clamp; the clamp is inert under a capture (the step is pinned), so no parity number moves | the `0x400` slowmo scaling is applied on top of the clamp, or its absence is documented as a kept quirk | |

## `ponytail` sweep (Phase 0)

The 41 `ponytail:` markers the sweep started from, each with where its deviation lives (38 remain in
`src/` after the three stale notes were corrected). A marker is **covered** by a
ledger row above, **new** (a row this sweep added), **not a deviation** — a cost, a guard, or the
one-level-at-a-time architecture, none of which retail could observe — or **stale**, where the code
has since outgrown the note. Line numbers drift; the identity is the symbol and the sentence.

| marker | disposition |
|---|---|
| `collide.rs` solid thing = cylinder (template radius, eight radii tall) | covered — *thing collision (solid models)* |
| `collide.rs` `Gates::passable`: no thing-on-thing collision | **new** — *thing-vs-thing collision* |
| `collide.rs` `polygon_normal`: O(n²) per polygon at load | not a deviation — load-time cost |
| `collide.rs` adjoin sweep depth limit | not a deviation — sweep guard |
| `collide.rs` `move_body` four-way step test | **new** — *lateral step allowance* |
| `collide.rs` `move_body` rise-against-top-edge upgrade | **new** — *lateral step allowance* |
| `collide.rs` wall scan O(walls) per moving body per frame | not a deviation — cost |
| `force.rs` force-point spend order inferred | **new** — *force point spending* |
| `level_mat.rs` baked vertex-colour light level | **new** — *lighting* |
| `main.rs` `hoverId` force rows not in a screen | **new** — *force-screen row gating* |
| `main.rs` soak `outside` asserted at `<= 1`, `<= 2` on 19a | not a deviation — test scope; `--soak-all` now also fails a child that panicked (round 313) |
| `main.rs` `build_world` whole world re-batched per change | not a deviation — cost |
| `main.rs` `actors_follow_runtime` rebuilds the mesh asset | not a deviation — cost |
| `main.rs` `animate_actors` scripted clip has no own epoch | **new** — *puppet clip timing* |
| `main.rs` `animate_actors` units-to-frames constant | **new** — *puppet clip timing* |
| `main.rs` `locomotion_state` picks the stride from an averaged displacement | covered — *puppet clip timing* |
| `main.rs` `drop` sweeps `Sim::entities` every frame | not a deviation — cost |
| `main.rs` `draw_viewmodel` sector ambient on the viewmodel camera | **new** — *lighting* |
| `main.rs` `billboard_sprites` picks the first `Camera3d` | not a deviation — plumbing |
| `main.rs` world material is double-sided (no culling) | **new** — *surface culling (world and models)* |
| `palette.rs` tint pass, scripted channels through the wash quad | covered — *damage tint* |
| `runtime.rs` process-global insert table | not a deviation — one level at a time |
| `runtime.rs` `advance_surface_cels` re-batches every surface | not a deviation — cost |
| `runtime.rs` `bump_scroll_epoch` 15 Hz scroll offset | **new** — *scrolling surfaces* |
| `runtime.rs` `set_thing_light` per-surface distance falloff | **new** — *lighting* |
| `runtime.rs` `step` thing track by mesh-name test | **new** — *puppet clip timing* |
| `runtime.rs` `step` vertical stop is a plane query | covered — *non-player thing gravity* |
| `runtime.rs` `step` position/segment tests, not the swept search | covered — *thing collision (solid models)* |
| `runtime.rs` `step` `RotatePivot` yaw only | **new** — *puppet clip timing* |
| `runtime.rs` `update_attachments` one level deep | **new** — *thing attachment (record + following)* |
| `runtime.rs` `transmit_awareness` breadth guard | covered — *`sithAIAwareness` propagation* |
| `runtime.rs` `weapon_for_bin` name-hint fallback | **new** — *weapon auto-select* |
| `runtime.rs` weapon auto-switch by damage ranking | **new** — *weapon auto-select* |
| `runtime.rs` solid-thing floor rule | covered — *thing collision (solid models)*: the collider's `ThingSolid` list feeds `collide::resting_floor`, the one query the player, AI settle, actor grounding and gravity landing share |
| `runtime.rs` `wire_links` id/mask not filtered | **new** — *cog link filtering* |
| `runtime.rs` `sector_route` greedy note | **stale** — the tick uses `Collide::route` (breadth-first); the note described the one-hop `best_portal` helper |
| `runtime.rs` `use_bin` "nothing calls this from a key" | **stale** — the inventory strip calls it; note corrected in round 268 |
| `runtime.rs` `sleep`/`settimer` arm with a "no timer queue" note | **stale** — the cog VM handles all four names and suspends/resumes; the note is corrected in round 269 |
| `tex.rs` 64-cel / 64-bitmap cap | not a deviation — guard past the shipped maximum (13) |
| `tex.rs` one process-global light ramp | not a deviation — one level at a time |
| `ui.rs` map zoom/orbit instead of `jkGuiMap_viewMat` | **new** — *map panel camera* |

## Phases

### Phase 0 — Reset the definition (do first, small)

- **Done:** `tools/parity/` is committed, the staged retail build takes `warp` (the dev-command patch
  is applied to the scratch copy), and `pwsh -File tools/parity/compare.ps1 -Gate` scores every pair
  against `tools/parity/thresholds.tsv` and exits non-zero. The live `spawn` pair is **not** a single-frame
  threshold any more: a room of NPCs makes one frame a phase measurement, so it is gated by
  `tools/parity/series-gate.ps1` on retail's own 7.4 s series span, sampled at retail's **recorded**
  1204 ms settle (round 308; the port's animation is frozen, so the comparison is the world: port
  1.0000 vs retail's world-region 1.0000, and both animated curves print 0.9258 for context). The draw
  gate is 5 thresholds, 0 failures, the series gate 1 pair, 0 failures. The gate is verified in both directions:
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
- **Done (round 269):** the ten stages are tiered in *Where the ten stages stand* (below the tier
  definitions): 8 are T1, stage 2 and stage 9 reach **T2 (partial)** on retail-derived evidence. The
  overlapping summaries in `roadmap.md`, `gaps.md` §1–2 and `engineering.md` stay as the appendices that
  carry the evidence behind each cell, and each now points at the table instead of restating status.
- **Done (rounds 268–274):** the **deviation ledger** sweep. The 41 `ponytail:` markers the sweep
  started from are all assigned in the `ponytail sweep` section after the ledger — **38 remain** in
  `src/` after three stale notes were corrected. Of the 41: **7** are covered by an existing row,
  **19** became new rows (13 rows: thing-vs-thing collision, the lateral step allowance, force-point
  spending, lighting, force-screen row gating, puppet clip timing, surface culling, scrolling
  surfaces, thing attachment, weapon auto-select, cog link filtering, the map panel camera, and cog
  nested `sleep`), **12** are not deviations (load-time/per-frame cost, guards, or the
  one-level-at-a-time architecture), and **3 were stale** — `use_bin` (the inventory strip calls it),
  the `sleep`/`settimer` arm (the cog VM suspends and resumes), and `sector_route`'s greedy note (the
  tick routes breadth-first). Each stale note is corrected.
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
   solid template's model top and footprint from the model it draws, and a falling body lands on it at
   `top + radius` — level 1's catwalk no longer drops the body out of the level;
   `BK_WARP_TRACE`/`BK_WALK_TRACE` are the probes, and the box approximation is on the ledger).
   `sithPhysics_GetThingHeight` is a
   floor-search **reach** (`max(insert, moveSize + 0.005)`, then `× 1.1` for an attached body), not a
   resting height: the level's `walkplayer` placements sit 0.120114 above their floor, the player is
   created without a floor search (`sithPlayer_NewPlayer`, `0x4c8ec0`), and the body falls to the
   sphere contact `floor + radius` — retail's own `coords` at level 1's spawn goes −1.13 → −1.18 over
   the first second. The port applies that rule and has no idle attach at all (see the deviation
   ledger); **non-player thing physics now has gravity** (round 246): a created thing starts with its template's
   `physflags=` (`spawn_at`/`fire_with_spread`/`explode` all set `0` before), the mover applies the
   level's gravity to the `_debris` family's `0x201` bodies (`0x40000` halves it) and stops the fall on
   the highest floor plane under the body, which is the round-120 note's missing ground test — the
   `--things` gate drops a created chip and measures the fall, and `--soak-all` stays clean. **Actor
   gravity is in (round 266)**: the gravity arm is the engine's — mass (`+0x204`) plus
   `physflags & 0x1`, with no type test — and `_actor` ships `physflags=0x4a4f, mass=150`; an actor
   within `GetThingHeight * 1.1` of the floor it stands over is the attach `sithThing_Initialize`
   makes (`FindFloor(thing, 1)`) and holds its authored height, and one that walks off a ledge falls
   to the sphere contact. The AI now probes the step's **destination** (`sithAI_CanWalk`, round 276) and
   stops at a ledge instead of walking off one. Still owed: the engine's exact
   `SearchForSurfaceCollisions` sweep; **actor-vs-world collision** (the engine moves an actor's body
   through the same `MoveThing` the player uses, while the port's AI writes positions — routing a step
   through `move_body` risks freezing an actor the port's approximate thing cylinders consider
   overlapped, so it needs the QA window to measure); the attached-body offset
   (`sithPhysics_UpdateAttachedThingPhysics`) for script-attached bodies; the *walking* settle over a
   solid thing (`top + radius`); and the recorded trace itself.
4. **Interaction.** Doors, lifts, switches, pickups through cogs; thing parenting; timing (`Sleep`,
   pulses) at engine granularity. Exit: cog variable/state trace matches at each trigger. *Port side
   measured (round 257):* the cantina switch row at yaw -153.8, `BK_USE=1 --cogstatus-after 11` prints
   `use: activated surface 2403` and `doorstatus 2` — but that was the port's 3.0-unit reach. The engine's
   own activate sweep reaches `moveSize + 0.1` (**0.16** units; `0x4fc0a0`, constant `0x523200` = -0.1, round 302), so at
   that pose retail's `doorstatus 0` is **correct**, not a harness failure, and the port now uses the same
   reach — corrected to `moveSize + 0.1` = **0.16** in round 302 (`0x523200` is -0.1; the earlier reading
   added a second 0.1 sphere the call does not contain). Measured: retail `doorstatus 0` at `switch_cantina`
   (-4.75) and `switch_cantina2` (-4.29,-2.95), the port likewise at both; at the new `switch_use` row
   (-4.29,-3.76, 0.12 from the plate) the port activates surface 2409 and cog 11 reads **`doorstatus 2`**.
   The retail positive needs a middle-click the harness cannot deliver yet: `-Keys` now restores foreground
   and holds the button, and two clicks at 0.12 still left the door closed, so retail's use input is still
   open (a keyboard-rebound profile is the next route).
   **A movement trigger closes the step's exit instead (round 308).** `00_SecretAreas.cog` (placed cog 28)
   fires on `entered`, not on the use action, so no input has to be synthesised: `--sector-locate 28`
   gives its six sectors, and the harness row `tools/parity/interaction-shots.tsv`'s `secret_enter`
   warps retail into sector 243's centre (-6.40,-4.22,-0.72). Retail's console `cogstatus 28` then reads
   **`maxsecrets 6`, `flags 1`** - the link id `sithThing_EnterSector` passes as the sender id. The
   port's live run (`--warp` into that sector, `--cogstatus-after 28`) reads the same **6 and 1**, and
   the new `--secret` gate asserts both (`--- secret: maxsecrets 6, flags 1 ---`). The use action stays
   open; the step's "cog variable/state trace matches at each trigger" criterion is met for this trigger.
5. **Combat.** Weapon fire, hit detection, damage, AI sight and routing, puppet states. Exit: the same
   encounter plays out with matching health and kill order from the same script. *Partly done:* shots
   leave the shooter's own `fireoffset=` in level units (the port scaled it by ten, which put a
   Ree-Yees' muzzle 0.8 units ahead of a 0.07-radius body) and the `.ai` aim error jitters the aim
   vector the way `sithAI_RandomFireVector` does (the port used a yaw cone ±`error` right angles wide),
   both pinned by the `--muzzle` check and a unit test; projectiles are no longer collectable as
   pickups. Measured at level 1's door with the shot's owner traced: 6 hits on the body (72 damage in
   8 s, ~9 damage/s against retail's ~22) — all from one actor, while the rest of the room's fire lands
   on a Greedo standing in the lines. **Round 308** added the engine's projectile sweep
   (`shot_hits_swept`: the frame's own segment, unit-tested) and re-measured the same door capture:
   **19 shots, 1 hit**. The trace now names the blocker (`segment_blocked_at`), and ten of those shots
   die 0.07 units from the muzzle on `surfflags 0x4` walls. Reading `sithWeapon_WeaponFireProjectile`
   (`0x4d45a0`) settled it: the engine creates the projectile at the shooter's **body** and sweeps it
   to the muzzle, and the port now does that too - the guard fired zero times in the re-measure, so
   those walls sit just *beyond* the muzzle and the engine would stop the bolt on the same frame. The
   remaining gap is the AI's aim and positioning, measured, not the hit test or the spawn. `BK_AI_TRACE` shows only a handful of actors ever see the body
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
   **Door endpoint done (round 409).** The earlier retail HUD readings of 56 and then 44 were wrong: the
   settled `route_door-raw.png` reads **81 health / 0 shields**, not 44 health after 100 shields. Two
   uninstrumented 600-frame port captures both read **83 / 0** (10 hits, 119 raw damage before the
   engine's shield/health rounding). That closes this step's health endpoint for the posed, stationary
   door encounter. Per-hit timing and a route with player kills remain unmeasured; they are not claimed
   by this result.
*Progress (rounds 209–239).* Combat's AI side is now measured off the shipped `.ai` files rather than guessed. The
instinct registry is complete (25 registrations across `sithAICmd_Startup` and the JK-side startup; the four unnamed
entries resolved from the EXE's string data), and `--audit` tallies which names shipped content actually uses: **24**.
Of those, **22 of the 22 instincts shipped content can actually load** are implemented — `follow`, `lookfortarget`,
`primaryfire`, `opendoors`, `blindfire`, `returnhome`, `retreat`, `withdraw`, `circlestrafe`, `hitandrun`,
`randomturn`, `roam`, `turretfire`, `flee`, `listen`, `talk`, `sensedanger`, `dodge`, `lobfire`, `jump`, `forcepowers`
and `saberfighting` (22 handlers, but `randomturn`'s nine class files are named by no shipped content — round 243) —
with `sabermove` unused by any class and `crouch` likewise dead content. The last name is `specialattack` (24 classes),
whose damage branch is event-`4`-driven: event 4 **is** emitted by the engine's thing-collision callback
(`0x4ed24d`), but this build's actor walk emits no thing-vs-thing event, so the branch is filed with the
actor-collision work rather than as a missing handler. `forcepowers` and `saberfighting` closed in round 252 (the
force-cast is the engine's `user0` dispatch; the shipped `darkjedi_*.cog` owns the power itself). Each instinct landed
with a headless test and a measured before/after (suite 149 → **193**). Reading the handlers also corrected behaviour underneath
them: the view cone of `LookForTarget`, the engine's class defaults for omitted `.ai` keys, the script-set AI bits from
`AISetMode` (`0x2000` switches an actor off and `0x4` is the awake bit `listen`/`sensedanger` gate on), turrets firing
from `TurretFire` (a turret class has no `PrimaryFire`), `listen`'s `heardist` gate, its 3-slot sound ring, its
line-of-sight/1.0-unit/60° hearing gates and its per-sound reaction chance, `talk`'s 95-name sound-mode table with its
flag/health mood selection, `sensedanger`'s alert bit `0x800` (which `talk` reads for `fear`/`gloat`) and its
strength-3.0 awareness burst, `dodge`'s damage event and 1000 ms cooldown plus its player-side shot pre-sweep (which also
put the `typeflags=` word on the runtime and showed it is not the `thingflags=` word the sweep tests), the engine's **AI
event dispatcher** with its emit-site inventory (round 241; round 252 corrected it — `2`, `4` and `0x200` are emitted
by the executable, but this build produces none of them), the
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
**6 thresholds / 0 failures**. `walk-preflight.ps1` (round 251) then walked every leg through the port's own collision:
**0 of 30 legs fail** — 22 walk and 8 are script holds (retail's own trace travels under 0.5 units:
`walk_door0`, `walk7_spawn`, `walk10_spawn`, `walk14_spawn`, `walk19_spawn`, `walk21_spawn`,
`walk26_spawn`, `walk28_spawn`). `walk15_spawn`/`walk22_spawn` walk once the ramp slide keeps its vertical
component (2.68 units against retail's 1.56 from the corrected trace-start pose — not a speed difference: 16a's
template walk is 1.20 u/s and retail's own trace slows mid-path and takes a different sector route; the
row-pose check now stops a leg whose pose is not the retail trace start); the
dead `walk_street` duplicate and the yaw-0 probe row were pruned, and `walk16_spawn`'s placeholder pose
now uses 17asarris's own start — their captures were noise, and they are now excluded before a capture runs. Everything still
unmeasurable is listed in "The QA window" below — it is the single list
of what a real-game session would settle, in the order a release candidate needs it.

*Progress (round 308, Phase 2's first new leg).* `walk2_spawn` was captured - the first leg for a
level outside 1/4/5/6/9/12/13/17. Two facts came out of it, both measured:
1. **The level's spawn yaw faces a drop.** Walking forward at retail's own -99.98 walks off the
   platform: the port falls to z -8.66, and retail falls, dies and **restarts**, which is why the first
   trace's game clock ran backwards (`-19328 ms`). The leg now turns a quarter to -189.98, the longest
   of four probed headings that keeps z steady (port 1.8 units, retail 2.9 then a 1.2-unit drop).
2. **The port blocks at the sector 22/23 portal where retail walks through.** `BK_WALK_TRACE` shows
   `move_body ... moved 0.000, blocked true` at (10.447,-13.727,3.065); `BK_STEP=0.2` leaves the
   endpoint identical, so the block is a **face**, not the step allowance. Retail keeps walking to
   (10.24,-14.69) and steps down into sector 39 (floor 1.8). A free-look portrait of the spot:
   sector 22 clearance +0.06, sector 23 clearance -0.06 at the same point. This is the collision
   layer's, not movement's; `walk2_spawn` is therefore **not** in the trace gate's default set yet, the
   same treatment `walk3_spawn` gets, and the leg stays runnable with `-Only walk2_spawn`.
   The **adjoin sets** (printed by `--faces-at`) show the second half: neither 22 nor 23 adjoins 39,
   so the 1.8 floor is two hops down (22/23 -> 33 -> 39); the port's scoped `floor_under` stops at
   the first hop inside the 0.024-deep sliver and the body falls to -8.135. The next fix is that
   downward chain, not the step predicate. **Round 309** tested the other half (a sliver's hull plane supporting the body) by requiring the plane's own surface to cover the point: with the step forced through, the path was byte-identical, so a real platform **face** extends past retail's edge. The seam is a level-data/polygon question; that change was reverted. **Round 311** found the plane that held the body, one layer below the face question: `walker_step`'s fallback to the sector's own plane fires when the swept faces find nothing, and at level 2's drop that plane is 3.0 under a 3.065 body (`--floor-at`: `floor_under -> None`, `standable -> None  (0 candidate face(s))`, plane 3.0) - so the body was held on the plane while the floor polygons had ended, and the wall beyond then stopped the walk. The fallback now requires `room_contains` (the body is over a solid or portal polygon of the room), which is true for 18Ascend's shaft opening and false at level 2's edge. The headless walker now covers 2.75 of retail's 2.82 units on `walk2_spawn`, and `walk-preflight` (22 walkers), `trace-gate` (10 legs, 0 failures), `--walk all` and `--move` all pass. The **live** replay still ends at (10.447,-13.727) in sector 23 against surface 214: the body's sector has already become 23 there (`BK_CONTAIN_TRACE`: `sector 23 rejected origin ...; resolver Some(23)`), and that sector's copy of the 0.2-high riser blocks the move. The next transcription there is `MoveThing`'s containment rule itself - restore the saved position and sector - instead of re-resolving to the sector that was just rejected.

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
gateable**: round 308 showed the earlier reading ("the opening scripts move the body") was the body
**walking off the ledge and dying**, which resets the level and runs its clock backwards; the leg is
now turned along the ledge, and what it measures is the 22/23/39 collision seam above.

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
control lands). The preflight separates `HOLD` (both sides stand still) from `DIVERGE` (retail's capture measured no
walk but the port walks: `07yun` 2.53 vs 0.11 and `15maw` 3.67 vs 0.00). The trace alone cannot say
whether retail was script-held or blocked, and neither level's scripts call a control/camera verb (25 and
29 scripts, checked with `--cogs`), so the retail side needs a probe before either is called a missing
hold. Three buckets were fixed on the way and are on the ledger: the **thing-collision push**
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

**Retail runs are user-invoked only.** The retail harness is the most invasive part (software renderer,
a few frames per second, combat can kill the player while it waits). Do **not** run
`retail-capture.ps1` for basic analysis or as a routine gate; use it only when a claim cannot be
checked from the shipped files, decomp and headless probes, and the user has asked for that live run.
The walk-trace and parity-draw checks are still windowed and should also be run deliberately.

Run, in order, from the repo root with `pwsh`:

| check | command | last measured | expected |
| --- | --- | --- | --- |
| port gates | `cargo build --release` then `target/release/bevi-knight.exe --gates` | **0 of 66 failed, 0 skipped** (round 291) | all pass |
| walk trace | `pwsh -NoProfile -File tools/parity/walk-preflight.ps1` first, then `pwsh -NoProfile -File tools/parity/trace-gate.ps1` | **10 legs, 0 failures** (round 293) — `walk17_spawn` 0.82 / retail 0.70 (117 %, sectors off 0, z 0.055); `walk5_spawn` 3.09 / retail 3.73 (83 %, sectors off 2, z 0.422); `walk6_spawn` 1.01 / 1.08 (94 %, sectors off 2); preflight **0 of 29 failing** (21 walk, 8 script holds; `walk16_spawn` dropped — retail itself is stuck there) | the gate records port traces through the live game; `--gates` is **0 of 66** |
| parity draw | `tools/parity/retail-capture.ps1`, `port-capture.ps1`, then `compare.ps1 -Gate` | **6 thresholds, 0 failures** in round 252; the `spawn` reference had been re-captured after a `-StartLevel 3` run left level 3's pose in the row (score 0.18 → 0.66) | 0 failures |

What the session settles, all of it already on the deviation ledger with its retire condition:

- **`walk17_spawn`** — **closed in round 292** (0.82 / retail 0.70, sectors off 0). Four deviations, all
  against decompiled constants: props were collided globally and at their level-file pose rather than in the
  moving body's own sector at their current position (`SearchForThingCollisions` walks the sector list); the
  first-frame seat was `floor + radius + 0.01` instead of the engine's `floor + moveSize`; the floor sweep had
  no sector-plane fallback where the face list does not tile over the shaft opening; and `IsSphereInSector`
  snaps plane distances within `1e-5` of zero, which the port did not. **Round 293** then narrowed the floor
  sweep's plane fallback to the data-gap case (no surface under the body within `height * 1.1`) and the whole
  gate went to **10 legs, 0 failures**: `walk5_spawn` descends again at 83 % / 2 sector samples.
  **Round 251 correction: three of the gate's default legs (`walk_door0`, `walk19_spawn`, `walk26_spawn`) do not
  move in the port at all — their poses walk into geometry — so their captures were never measuring a walk. The
  gate now preflights every leg with `walk-preflight.ps1` (headless `--walk --warp`) and excludes the blocked ones;
  a stuck leg aborts with a clear error instead of being held against the wall.**
- **`opendoors`** — **the reach is the engine's now (round 339)**: `Activate` passes
  `*(float *)(thing + 0x2c) - _DAT_00523200` (`movesize + 0.1`), and the handler ran the player's action on the
  actor's body, so an actor reaches exactly as far as the player's use key does — the invented 1.5-unit reach is
  gone, and the unit test now proves a door at the cantina's measured 0.12 pose is opened while one at 0.93 is
  refused. What remains is the **cone**: the engine sphere-casts from the eye (`sithIntersect_CheckSphereIntersection`
  with the thing's radius expanded by `0.4`), and the port's two scans stand in for that sweep. Walk an actor at a
  closed door and compare which door it opens — that is the window half.
- **`blindfire`** — **the roll is the handler's now (round 339)**: `FUN_00509b30` rolls the chance once per
  call and, on a **miss, sets its next call to `now + 1000`** (a hit schedules its own 750 ms, `class+0x4c`).
  The port re-rolled at the shot cadence and skipped the roll entirely when the chance was 0; both are the
  engine's now, and the unit test proves a zero-chance class never fires and leaves a 1 s wait behind.
- **`returnhome`** — **closed (round 339)**: `FUN_0050bd70` gates on the AI's **awake** bit (`ai flags & 4`).
  `LookForTarget` runs the two halves of that flag: branch A (the scan) runs only while `0x4` is set, and on a
  find writes `(flags & 0xffff_fffb) | 0x232` — it **clears** the bit and marks the actor engaged; branch B, once
  the class interval passes on an engaged actor, writes `(flags & 0xffff_f9dd) | 4` and the scan runs again. The
  port walked home off the instinct alone (and had read the find's write backwards). Both branches are modelled
  now, and the unit test proves a wake bit that was never set stands still while `AISetMode(actor, 0x4)` walks.
- **`retreat`/`withdraw`** — **decoupled (round 343)**: `retreat` (`0x50bc60`) only writes the mode and the
  target into `ai[0x70]`; the **`withdraw` handler** (`0x50b150`) is what walks the body, and it gates on
  `flags & 0x800` alone. The port demanded both instincts for either, so the **six classes that ship `Withdraw`
  without `Retreat`** never backed off, even though `sensedanger` (`0x50b9b0`) writes the player into `ai[0x70]`
  on a surprise. `SenseDanger` now stores that target, `Retreat` needs only itself, and the flee walk still
  requires the `Withdraw` instinct (a `Retreat`-only class arms a mode nobody acts on, as in the engine). Unit
  tests cover both: the surprise-only path (flags `0x800`, target stored, the body backs off) and the hurt-actor
  `Retreat` path.
- **`retreat`/`withdraw`** — **the lifetime is the engine's now (round 339)**: `FUN_0050b150` clears the mode on
  `ai[0x70] == 0` (the remembered target gone), while the instinct's 5000 ms is only how often the handler runs.
  The port stopped withdrawing when the interval expired; the mode now lasts until the target is destroyed or
  removed, and a retreat with no live target no longer arms at all. The unit test holds the mode through a run
  of frames past the interval and clears it when the target is destroyed. Still open: the **lateral jitter** — the
  constants are known and the call is resolved: `FUN_0044a09e` is `rdVector_Rotate3Acc(vec, angle)` — a Z
  rotation of the (negated) direction — and its argument is the **second** roll, `fStack_18 =
  (rand − 0.5) × 90` (±45°), taken **only** when the target's `thing + 0x1d0 & 0x2000` (`jkFlags`) is set;
  otherwise the angle is 0 and the flee is straight away. The `±180 × 0.5` roll (`fStack_14`) is a dead store
  in this branch. What stays open is that jkFlag's identity (`0x2000`), which no port table names.
- **`circlestrafe`** — **replans on the instinct's own clock now (round 339)**: the handler sets its next
  call to `class+0x4c + now`, the same slot `talk` reads its interval from, and the shipped first number is
  **5000 ms**. The port recomputed the rotation every frame (a smoother orbit than retail's); the ring point
  is now chosen once and held for the interval, and the unit test proves it is untouched after a second and
  replaced after five.
- **`hitandrun`** — **closed (round 339)**: `sithAICmd_HitAndRun` (`0x50bb90`) does `state[8] -= _DAT_005232f4`
  only on the ticks where the just-fired bit (`0x400`) is set, and that constant is **-1000.0** (`.rdata`), so each
  firing tick adds a second of *firing* time; a tick without the bit resets the accumulator, and the break-off
  happens when it passes the instinct's first number. The port ran a wall clock from the first shot, which armed
  the run-away after a single shot. The test proves both halves: continuous fire arms it, one shot does not.
- **`turretfire`** — **closed (round 340)**: the ±160° sweep belongs to the **damaged** arm of `FUN_0050a0f0`
  (below `arg5` = `0.20` of full health). That arm re-rolls a yaw offset inside ±`arg2` and a pitch offset
  inside ±`arg3` on every handler call, fires on its own `arg6` (500 ms) cadence instead of the healthy
  `arg0` (750 ms), and arms `now + arg7` (10 s), after which the handler calls
  `sithActor_KillActor(thing, thing, 2)` on its own body. The port modelled a healthy turret only, so a
  damaged one stayed accurate and never broke down; the sweep is applied to the shot's direction (there is no
  articulated head), and the unit test covers the damaged cadence, the survival before the deadline, the
  self-destruct and a healthy turret that never arms it.
- **`listen`** — **the awareness model was rebuilt on the engine's own shape (round 339)**: the third argument of
  `sithAIAwareness_CreateTransmittingEvent` is an event **kind** (0..2), not an alignment, and `ProcessEvent`
  (`0x4f2c30`) keeps one `(strength, position, source thing)` record per sector and kind, replacing it only when
  the new event is stronger. The port had keyed the record by the shooter's **alignment**, which meant a shot could
  never reach an actor of the other side — and, because it also demanded a `.ai` class on the shooter, the
  **player's gunfire raised no event at all**, so enemies never heard it. The records are kind-keyed now, the
  listener scans kind 2 then 1 then 0 (`heard_event`), a failed roll turns at the event's **source thing** when it
  still exists, and the engine's own sources are wired with their measured strengths: a shot (`WeaponFire`,
  `FireProjectile`) kind 1 at **2.0**, an explosion (`MakeBlast`) kind 1 at **3.0**, a death (`KillActor`) kind 0 at
  **5.0**, and a sound played on a thing (`PlaySoundThing`) kind 0 at **volume × 0.6** (`_DAT_00522f80`, read out
  of the EXE's `.rdata`). The row is closed: nothing on the awareness path is unmodelled.
- **`AISetMode`** — `sithCogFunctionAI_AISetMode`/`AIClearMode` OR and AND-NOT the whole argument into the AI's flag
  word and emit event `0x100` on a change; the port's two verbs already do the OR/AND-NOT and keep the `0x2000`
  switch-off, and the awake bit `0x4` now drives `returnhome`. The event `0x100` is not emitted (no port handler
  reads it).
- **instinct scheduler** — the engine's loop is now read line by line (`sithAI_InstinctUpdate`, `0x4e9e90`, 55
  lines): it walks the class's instinct slots in file order, runs one only when its registered **mask/mode** pair
  accepts the AI flag word (`mask & flags != 0 && mode & flags == 0`) and its own 1 s slot clock is due, then arms
  that slot to `now + 1000`; a handler that returns non-zero **and changed the flags** makes it emit event `0x100`
  with the old word and restart the scan; finally it stores `ai+400 = min(slot.next)`. The port keeps the 1 s arms
  per instinct (`ai_detect_at`, `ai_scan`, `ai_talk`, …) but does **not** gate on the mask/mode words, and its
  `mask 0x100`-style values exist only in the `aistatus` dump. Porting the gate needs the engine's own registration
  table (`sithAICmd_Startup`) and a decision on where the port writes the bits it tests (e.g. `PrimaryFire mask
  0x100`), which the retail-sourced AI timing has no fixture to grade yet — so the scheduler stays the last
  unmodelled piece, with its rules recorded here instead of approximated again.
- **acquisition broadcast** — `FUN_0050aa80`'s find also calls `CreateTransmittingEvent(player->sector, actor->pos,
  0, 3.0, player)`: a spotter **calls its neighbours in** through the awareness graph. The port acquired silently, so a
  room never converged on one pair of eyes. The acquisition path now writes the engine's engaged word once per
  acquisition (the `0x200` bit keeps it to once, not per frame) and raises the kind-0, strength-3.0 event from the
  actor's position in the player's sector; the unit test checks the flags, the strength, the position, the source and
  that a listener in that sector reads the record.
- **AI engagement timing** — the level-3 same-pose pair: a port run and a retail capture at the recorded vantage, to
  confirm the view-cone fix moved acquisition to where retail acquires. The detection **chance** underneath it is
  retail's now (round 242, unit-checked against the `.rdata` constants) and acquisition is a roll; what a window can
  add is the *timing* of acquisition against retail's at the same pose.
- **viewmodel materials** — the data side is measured headless (`--model bryv.3do`: 14 materials over 76 faces, all
  present). Round 255 isolated retail's weapon with a fist-vs-bryar capture pair and fitted the port's rest-pose model
  to it (offset `(-0.026, 0.009, 0)`; retail block x 480–660/y 337–730, port x 473–714/y 336–730 at 1170x731). The
  POV key clips are skeleton tracks the `.3do` parser cannot map, so the rest pose is drawn; a per-material near-plane
  cull for the wrist (`HND-WRT`) is the remaining polish.
- **POV sway** — the amplitudes, phase rate and speed scaling are retail's now (round 234, unit-checked), but
  `rdMatrix_BuildRotate34`'s composition order and sign are not recovered; a frame at a known phase would settle it.

Safety and recovery: the retail harness never touches the real install — it copies it to `target/parity/retailcopy/`
and records the desktop display modes to `target/parity/display-pre.json` before it starts. Keep hands off the mouse and
keyboard while a capture runs; both harnesses drive synthetic input. The port side opens a window per shot and closes it
itself. **Recovery rules (round 251):** a walk leg that stops moving aborts the harness after two segments with under
0.02 units of travel (so it can never sit against a wall), the walk legs are preflighted headlessly before any capture,
and `retail-capture.ps1` now closes a leftover scratch-copy `JK.EXE` from an interrupted run automatically at startup
instead of refusing to continue; `pwsh -NoProfile -File tools/parity/retail-cleanup.ps1` does the same by hand and
leaves a real, foreground game alone. If the desktop is left in 640×480, `tools/parity/restore-display.ps1` restores
the saved baseline.

### Phase 3 — Breadth, by shipped-content weight

Only now take the remaining units from `gaps.md`, ordered by how many shipped levels and scripts need
them (its §5 already ranks them): particles and effects, saber and force-power fidelity, aim joints,
track things, scope, ending screens. Each follows the porting loop above and enters at a tier.

*Progress (rounds 252–265).* **Saber and force-power fidelity on non-player actors is in**:
`forcepowers`/`saberfighting` (45 loadable classes each) ported from `0x40ff40`/`0x40fa40`; **generated
`type=particle` clouds** (level 1 loads 24, was one `.par`) with **both** `sithParticle_Update` arms
(grow, cel cycling); **all 22 loadable instincts**, including `specialattack`'s event-4 contact damage
(round 264); **aim joints** — body yaw on the drawn actor and head pitch on `k_neck`/`k_head` (rounds
262–263). The remaining breadth list is now: the latent free-path `track` branch (no shipped level starts
one; round 265), the scope overlay, the ending screens, and the event-2/`0x200` producers. None is an exit
criterion for Phase 1/2.

### Phase 4 — Ship

Cut a release when: the exit criteria of every phase so far hold, the release note lists the tier per
level, and a clean-machine install plays level 1 to completion. Multiplayer stays out of scope, as now.

**State at round 356 (re-measured each round; the round-265 block below is history, not the current numbers).**

| check | command | measured round 340 |
| --- | --- | --- |
| unit + integration suite | `cargo test --bins --release` | **262 passed**, 0 failed (round 357; the new ones are the awareness sweep's latch check and `sensedanger`'s event-2 arm) |
| every level's runtime | `BK_SKIP_WINDOW=1 BK_MUTE=1 BK_SOAK_SECONDS=5 bevi-knight --soak-all` | **35 of 35 levels**, 0 non-finite positions, **0** bodies outside every sector (round 331's soak-count fix) |
| every cog verb | `bevi-knight --cog-check` | **34 levels, 0 stubbed verb calls, 0 errors** |
| drop-in asset coverage | `bevi-knight --audit` | **35/35 levels clean**, 0 non-drawable models, 10 unresolved asset references, 54 undecoded files |
| campaign graph | `bevi-knight --tour` | **28 rows on both arms**, 25 distinct files, 23 films, 2 terminals, 0 script errors; the `DECIDE` fork's dark arm (`38a.smk` + `54a.smk` -> `16bEscapeShip.jkl`) asserted since round 332 |
| port gates | `bevi-knight --gates` | **67 rows, 0 failed, 2 skipped (round 356)**; the two skipped rows are the windowed `--live-check` (163 pulse/timer messages for 14 armed cogs, 10 sector and 286 thing links) and `--live-play --start 01narshadda`, both **run windowed and passing at round 356** (the map panel joined in round 338; `--tour-live` left the sweep in round 339 - it pays every level's shader compile, ~2 min a level here, so a 21-level tour cannot fit a sweep budget; `--tour` covers the spine headlessly, and the live tour runs by hand)**; 0 failed, 0 skipped on the round-340 sweep |
| every level's walk | `bevi-knight --walk all` | **26/34 contained**, 24 hull-agreeing, 52 crossings, **no body in the void**, the worst *contained* walk 3.26 (`06abarons`); only `09fuelstation` still ends outside its sector (its recorded 21.13-unit walk-off), and the five other former walk-offs (`02narshadda` 11.22 contained, `07yun`, `11gorc`, `17asarris` and `m_boss17` 2.90) stay in the world; the sweep drives `Runtime::step_player` (round 428), so it runs the player's own step, step-up and gravity |
| walk trace | `pwsh -File tools/parity/trace-gate.ps1` | **10 legs, 0 failures (round 428 re-run on the runtime path)** — `walk17_spawn` 100 % / sectors off 0, `walk5_spawn` 82 % / sectors off 2, `walk6_spawn` 94 % / sectors off 2, `walk12_spawn` 127 % / sectors off 0; the preflight and the port replay both drive `Runtime::step_player` now, so the gate's leg set and its traces come from one motion path; `walk3_spawn` stays out of the set at 209 % (the floor slide) and `walk2_spawn` (retail 22 -> 39 over 2.82 units, port 1.78 with the block at the 0.2-high lip) at 63 % / sectors off 6 — the round-308/311 sliver seam |
| parity series | `pwsh -File tools/parity/series-gate.ps1` | **1 graded, 0 failures (round 356)** - `spawn` **world** port 1.0000 / retail 1.0000, |diff| 0.0000, over the sidecar's measured 7.4 s from the same 1204 ms settle. Three fixes made it a real measurement: the harness now records `series_settle_ms` (wall time from "level loaded" to the series' first frame), the gate samples the port at that same game time instead of a fixed 60 frames, and the gate freezes the port's animation (`--no-anim`) and grades it against retail's `series_world_region_ssim`, because retail's own note says its changing pixels are animated characters, not camera motion. Both sides' animated full-series curves still print (port 0.9258 / retail 0.9258) but no longer decide it: level 1's intro Ry stands 0.27 units from the eye, so its clip fills the band and its loop returns the frame bit-identical (1.0000 at 4.2 s) - a pose limitation, not world drift |
| parity menu | `pwsh -File tools/parity/menu-gate.ps1` | **SSIM 0.9661, floor 0.90 (round 356)** - `retail-capture.ps1 -MainMenuShot` writes `target/parity/retail/main-menu.png` at 640x480 and the port renders the same screen with `BK_WINDOW=640x480 --menu singleplayer --shot`; both sides are captured on demand, so the gate runs anywhere the game and the port exist. Round 310's UI tonemapping removal moved it up from 0.9521 |
| parity film | `pwsh -File tools/parity/film-gate.ps1` | **SSIM 1.0000, floor 0.99 (round 356)** - the retail opening-film capture against the port's `BK_FILM_HOLD=147` frame; the crawl holds for film frames 91..178, so no wall-clock alignment is needed |
| parity draw | `pwsh -File tools/parity/compare.ps1 -Gate` | **5 thresholds, 0 failures (round 356)**: `spawn_pose` 0.6393, `spawn_yaw30` 0.5914, `route_cantina` 0.7051, `route_lower` 0.6846, `route_street` 0.7143; 12 pairs compared, 1 not captured, 7 shots carry no threshold row yet. The floors were re-derived this round: the merged rounds' ambient-base lighting rule (`rdCamera_SetAmbientLight`'s `ambient + extraLight`) darkened level 1 by ~5 luma, which moved the three route pairs' MAD past the old floors. A/B against `866793d` and against the `1.0` base is recorded in `thresholds.tsv`; level 1's residual is the *lighting* ledger row |
| retail saves | `bevi-knight --save <file>.jks` | retail lists decode: **357 entries** (quicksave) / **73** (autosave) with the player at that level's own spawn and health 100 (round 340, not re-run since) |
| silent install | `bevi-knight-0.1.1-setup.exe /S /GAMEPATH=… /DIR=…` | **exit 0** (round 359, from a copy outside the repo): installs `bevi-knight.exe`, the uninstaller, `smkdec.exe`, `game-path.txt`, `docs/`, `LICENSE`, `NOTICE`, `README.md`, and the installed exe finds the game through `game-path.txt` and loads `01narshadda.jkl`. The same binary launched from `dist\` (or a fresh untracked directory) reports `Access denied` for every write in this agent shell — a csc write probe reproduces that with no setup code in it, so the denial follows the launching directory, not the installer |

**Round 359-360 moved three rows of the table.** The walk-trace row now reads **10 legs, 0 failures** by default
with `walk2_spawn` tracked as a measured failure (63 % travel, sectors off 6/6 — the sector 22/23 sliver's step
containment, which a `--walk all` experiment showed cannot be relaxed without giving up containment on other
levels); a full sweep of **all 20 walkable legs** is 10 pass / 10 fail, and three of those (`walk11`, `walk20`,
`walk23`) are newly explained: the port zeroed the walk inputs under a script camera, while the engine reads
them in the **view's** frame. That is fixed (`Runtime::script_camera_yaw`), so those legs now move in retail's
direction but over-travel until the script's input-lock timing is modelled. The silent-install row is new
(exit 0 from a copy outside the repo; the earlier "Access denied" was the launch directory).

**Round 340 closed four more AI ledger rows** (evidence in `docs/progress-log.md`): a find now **broadcasts**
(`CreateTransmittingEvent(player->sector, actor->pos, 0, 3.0, player)`) and writes the engine's engaged word, with the
scan's awake bit re-set by the handler's own interval arm; a firing weapon raises awareness **kind 2** beside the shot
(the kind `listen` scans first); `turretfire`'s **damaged** arm sweeps its aim inside ±arg2/±arg3 at its own 500 ms
cadence and destroys itself after `arg7` (10 s); and `withdraw`'s jitter call is resolved (`rdVector_Rotate3Acc` with
the ±45° roll, taken only when the target's `thing + 0x1d0 & 0x2000` jkFlag is set).

**Round 339 added six technical closures** (evidence in `docs/progress-log.md`): the inventory verbs clamp to
`items.dat`'s `min`/`max` like `sithInventory_ChangeInventory`/`SetInventory`; `--force-live` witnesses the lift by the
velocity the script sets (2.85 u/s measured) instead of a razor-thin ceiling contact; `opendoors` reaches with the
engine's `movesize + 0.1` rather than an invented 1.5 units; `circlestrafe` replans on its own 5 s clock;
`withdraw` lasts until its target is gone (the interval is only its call rate); `blindfire` rolls per handler call with
the engine's 1 s retry on a miss; and a failed `listen` roll turns the actor toward the sound it ignores.

Closed since round 265, with the evidence in `docs/progress-log.md`: the AI-engagement row (the port lands no hits at the
row's own pose), the cutscene row (retail cannot be booted into a non-first level — three routes proved closed), the
weapon-capture claim (`BK_ARM=11 BK_POV=bryv.3do` draws it: 8 843 changed pixels, bottom-centre box), and in rounds
325–327 the sky planes' header starting offsets (25 levels checked; `03katarn`/`10cargo` start the horizon at `y = -16`
texels), the saber's `drawsaber` row (`weap_saber.cog` is the only script that selects armed mode 2; Kyle's puppet
answers with `kydrawsb.key`), and the `.3do` face flag word's two-sided bit (3,508 faces across 121 models now build
their reversed winding). Corrected:
the floor rule's normal convention, the canopy/thing-solidity lead, and row 141's premise that `thing + 0x144` is in a
save (it is not — a save entry carries type, name, position, angles and one float).

**State at round 265 (measured, so the next session starts from evidence, not narration).** Every check that can be
taken without driving the game is green, the three QA-window checks were run, and the tree is committed and clean:

| check | command | last measured |
| --- | --- | --- |
| unit + integration suite | `cargo test --release` | **202 passed**, 0 failed |
| port gates (headless sweep) | `BK_SKIP_WINDOW=1 BK_MUTE=1 bevi-knight --gates` | **0 of 65 failed, 1 skipped** (`--live-check`); the **windowed** sweep was **65 of 65, 0 skipped** this round |
| every level's runtime | `BK_SKIP_WINDOW=1 BK_MUTE=1 BK_SOAK_SECONDS=5 bevi-knight --soak-all` | **35 of 35 levels**, 0 non-finite positions, 0 bodies outside every sector |
| drop-in asset coverage | `bevi-knight --audit` | **35/35 levels clean**, 0 non-drawable models, 10 unresolved asset references (retail data gaps), 54 undecoded files (cutscenes and sound; intentionally out of scope) |
| every cog verb | `bevi-knight --cog-check` | **34 levels, 0 stubbed verb calls** |
| data probes | `--things`, `--ai`, `--hit`, `--weapons`, `--sound`, `--par`, `--model`, `--asset` | all pass |
| walk trace (window) | `pwsh -File tools/parity/trace-gate.ps1` | **12 legs, 1 tracked failure** (`walk5_spawn`); three default legs are preflight-blocked and excluded |
| parity draw (window) | `retail-capture.ps1`, `port-capture.ps1`, `compare.ps1 -Gate` | **6 thresholds, 0 failures** (spawn 0.66 after the stale level-3 fixture was replaced) |
| docs | `pwsh scripts/check-docs-links.ps1` | 132/132 links resolve |

What the windowless method has closed: Phase 0 and **0.5 (trace v0, done)**, Phase 1's **step 1** (body, attachments,
camera) and **step 6** (level start, tally, transition carry, retail's autosaves as the fixture), the **whole AI event
dispatcher and all 22 loadable instinct names** (`specialattack` closed in round 264), the gravity family for shipped physics goods, the `PAR 1.0` clouds' data **and** draw path and the generated `type=particle` clouds, both
`--soak`/`--soak-all` and `--gates` as headless nets, and the retail-derived constants for detection, `dodge`, `talk`,
`jump`, `lobfire` and the movement code.

What still blocks Phase 4, in the roadmap's own order:

1. **The QA window's remaining measurements** — the port-gate sweep (`65/65, 0 skipped`) and the draw gate (`6/0`) now
   pass, and the walk trace is **12 legs, 1 tracked failure**. That leaves `walk5_spawn` (the floor-search-scope
   deviation) and the live combat/trigger captures as the only unclosed measurements for Phase 1 steps 2, 3 and 5 and
   Phase 2's per-level trace generalisation.
2. **Phase 1 step 4's exit** (a cog state trace at each trigger) — same window: it needs a retail capture to compare
   against; the port-side trigger paths are all headless-tested, but "matches retail" is a retail measurement.
3. **Phase 3 breadth remainder** (shipped-content order, all ledgered): the latent free-path `track` branch (no
   shipped level starts one), the scope overlay (`jkHudScope.c` is not decompiled and no shipped archive names a scope
   bitmap — the only hits are `TTRNSCOP.MAT`, a transition material), and the ending screens/choice menu (round 357 checked the shipped
   tables: `--ui-tables` lists the `light_or_dark` screen at `0x535c28` with three text items whose keys —
   `GUI_LIGHT_OR_DARK`, `GUI_DECIDE_A`, `GUI_DECIDE_B` — exist **nowhere** in the 313-key `GUI_*` string table in
   `Res1hi.gob`, so by the engine's own rule (`jkGui_InitMenu` only copies a string it finds) retail draws no text there
   either; the credits side does resolve — `GUI_CREDITS` reads "Credits" and the `CREDITS_*` table in `Res1hi.gob` holds
   the whole crawl. The missing piece is therefore the engine code that would turn the screen's clicks into bin 18, not
   its text; `jkPlayer_SetChoice`'s writer is not in `decomp/`).
   **The AI event family is closed as far as this build can reach**: event 2 is produced by the awareness sweep and
   `sensedanger`'s event-2 arm is ported (round 357), `dodge`'s arm is dead content (it demands a weapon-thing source),
   and `0x200` has no reachable producer (`0x4ed1d0` is installed at `0x848600` and never read). The particle arms,
   head/aim joints and `specialattack` are in. None of these is an exit criterion for the phases before it.
4. **`walk5_spawn`** — **passes as of round 293** (83 % travel, 2 sector samples) after the floor sweep's plane
   fallback was narrowed to the data-gap case; its remaining deviation is its slower travel over the same 3 s (83 %), so
   the port stops short of retail's sector-24 end point and its `z 0.422` is that end-position delta, not a standing height
   (round 356 corrected the earlier floor-search-scope reading: at retail's own point the port's floor is 0.07 below).

So: **no release candidate is warranted yet.** Phase 1 steps 2–5 and Phase 2's trace comparison need one deliberate QA
window (the session this document already describes); Phase 3's breadth list is the post-window work order. Everything
achievable from the shipped artifacts, the decomp and the file probes has been measured and committed.

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
retail's reference there is red from the engine's **damage tint**: the port draws that tint and takes
damage now. At the door its settled 10 s state is 83 health / 0 shields against retail's 81 / 0
(round 409); the remaining triage gap is the picture and per-hit timing, not an untouched body.

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
  scripts and AI react to a body as they do in retail's reference. One engine gap still differs after
  the warp: the port re-probes the floor each idle frame instead of keeping the attachment state
  (retail's catwalk body stays 0.145 above the catwalk model; the port's falls). At the door the
  settled damage state now agrees within 2 health (retail 81 / 0 shields, port 83 / 0 at 10 s); the
  tint itself is ported on both halves. Per-hit timing remains open.
- **Input replay determinism.** Retail frame timing is not ours; T4 needs fixed-step or a tolerance, and
  the harness has no replay of movement yet.
- **Better than SSIM.** Confirmed the PCX is palette-indexed on the software path (above); what remains
  is the port emitting indices. The gate's SSIM floors are the triage version.
- **Renderer tolerance.** Retail's renderer and Bevy will never match pixel for pixel. Per-shot floors
  now live in `tools/parity/thresholds.tsv` (0.55 for spawn against 0.62–0.65, 0.65 for route against
  0.72–0.74); tighten them as renderer parity improves, never below the verified-pair noise.
- **Fixture hygiene.** Keep traces and numbers in the repo; keep retail screenshots out of any
  GPL-licensed distribution unless the existing NOTICE terms cover them. **Round 279:** the per-level
  spawn fixture set is not usable as it stands — `retail-level{4,7,10,15,18}-spawn.png` are all-black
  1440x900 frames (mean luma 0.0) and the other fifteen carry no pose sidecar, so their SSIMs measure
  framing, not fidelity. `target/parity/survey-shots.tsv` (generated from `spawn-levels.tsv`, one row
  per level at its own spawn) plus `port-capture.ps1 -Shots` / `compare.ps1 -Shots` is the harness for
  regenerating them in one windowed session.

### Outside review (2026-11-01) — what the rounds missed

A cold read of the roadmap against the code and the retail harness. `cargo test --release` is green on the
uncommitted tree (218/218) and `--feel` passes; none of the items below is caught by a gate. Scope and
evidence: items 1–3 come from reading the decompile (`sithTime_Advance`, `0x4dd640`), the harness
script and the committed fixtures, **not** from a new retail capture; `gaps.md` and `formats.md` were
searched, not read end to end, so this is a targeted review rather than a line-by-line audit.

1. **Retail walk traces record injected key time, not game time (high — it undercuts every retail speed
   number).** `Invoke-JkWalkTrace` writes `held_ms = i * HoldMs` and its header says "real (== game) time".
   That equality does not hold here: `sithTime_Advance` (`decomp/004dd640`) advances the game by the
   *frame* delta, clamped to [10, 500] ms, and DirectInput is sampled once per frame — while the
   harness's own notes say retail renders **a few frames a second** under dgVoodoo. A 600 ms hold then
   spans 2–4 frames and is quantised by ±1 frame (±25–40 %). The committed fixture shows it:
   `walk_street90` moves 1.28, 0.64, 0.29 units over three "600 ms" segments. **Fix:** read the engine's
   game-ms counter (`DAT_00836c14`, the same field the `.jks` writer serialises — `formats.md` §save
   header; `JK.EXE` is not relocated) with `ReadProcessMemory` before and after each segment and write
   that delta as `held_ms`. Re-capture the walk legs before trusting any speed comparison.
   **Harness side landed (same day):** `Read-JkGameMs` (`retail-lib.ps1` `JkRetail.ReadU32`) writes a
   7th `game_ms` column (engine clock since the start pose; the capture throws if it cannot be read),
   and `trace-compare.ps1` used to switch its time axis to it when every row has one.
   **Re-captured and reverted (round 294): the engine clock is not the movement axis.** Ten legs were
   captured with it: a 3 000 ms hold produced **17 019 ms** of engine clock in 19 s of wall time while the
   body moved its usual 3.7 units, because the clock keeps accumulating through the console dialog and the
   foreground waits. With that axis the port was sampled 5x too late and three legs failed; `held_ms` —
   the span the key is down — is within a frame of the simulated time (the 1.2 u/s rule predicts the
   measured 0.5-0.9 units per 600 ms segment). The axis is back on `held_ms`, the comparator prints the
   engine clock as a drift figure, and the capture keeps writing the column. The committed fixtures remain
   the reference; a fresh capture also showed run-to-run variance on NPC-populated levels (cantina 1.86 vs
   the fixture's 2.59), so re-capturing is not a repeatable substitute.
2. **The hold loop contradicts its own comment** (comment corrected; behaviour unchanged). The comment above `Send-JkKeyDown` says the key is held
   across the whole trace and re-asserted after each poll; the loop actually sends `KeyUp` after every
   segment, so each segment restarts from a coast/rest (plus the ≥120 ms foreground wait). Pick one and
   make the comment and the port replay (`BK_WALK_FORWARD`, held continuously) agree — today they don't.
3. **The walk-speed "match" is a wall match.** `Rules::walk = 1.2` was set from `walk_street90`'s
   1.07–1.23 u/s, an average that includes the decel into retail's stop at x 3.27; `walk_street45`'s
   2.21 vs 2.20 is both bodies stopping at that same point (retail's y never changes on the "45°" leg,
   so it is a wall slide from the first segment, not a straight run). Neither measures speed. Meanwhile
   the port's default forward is **run = 2.40 u/s** (`--feel`). Measure on an unobstructed leg with ≥3
   steady segments *after* item 1 lands; until then the ledger's *player walking* row should read
   "unmeasured", and the `thing + 0x144` hunt may be chasing a measurement artefact.
4. **Frame-time clamp differs** (new ledger row *frame-time clamp*): the port caps `dt` at 0.1 s with no
   10 ms floor; the engine clamps to [0.01, 0.5].
5. **Stale comments in code that describe a different engine than the code ports** (cheap, but they
   mislead exactly the reader principle 2 is for): `thing_transform`'s doc said "Y-up world", "only yaw
   is applied" and "yaw faces (cos a, 0, sin a)", while `thing_quat` builds the full `rdMatrix_Build34`
   PYR basis in JK's Z-up frame with forward (−sin a, cos a, 0); the `Rules` radius note said
   "0.065 × 10 = 0.65" with `MODEL_SCALE = 1.0` and `PLAYER_RADIUS = 0.065`. Both corrected in this
   review. Expect more: grep for "MODEL_SCALE", "ten times" and "Y-up" before trusting a doc comment.
6. **Repo hygiene.** ~4.7k lines across `src/`, `tools/parity/` and `docs/` sit uncommitted, spanning
   several rounds — one bad edit now loses all of them; commit per round as the rules ask. An empty
   `package.json`/`package-lock.json` pair was left at the root of this Rust repo by a stray `npm`
   run; delete it. `main.rs` (26.5k lines) is where principle 5 says fixes should *not* land, and it
   keeps growing — move the next walker/physics change into `collide.rs`/`runtime.rs` rather than
   adding to it. `tools/parity/*.jks` are engine-written retail saves committed to a GPL repo; confirm
   they carry no retail level data beyond state before the next release.

### Where the next session starts

- **The outside review's `game_ms` re-capture is done and its premise was wrong (round 294).** The
  engine clock tracks wall time (17 019 ms for a 3 000 ms hold), so `held_ms` stays the movement axis and
  the comparator reports the engine clock as drift. No walk-speed work should be retuned against an
  engine-clock axis.

- **`walk5_spawn` passes again (round 293), with its height deviation still tracked.** Retail's fixture is
  `target/parity/trace/walk5_spawn-walk.tsv`: it descends 0.22 -> ~0 and crosses sectors 35 -> 33 -> 34 -> 33 -> 24.
  The port now walks 3.09 of retail's 3.73 units (83 %, two sector samples off, the gate's own tolerance) and
  descends onto the lower room's slab. The fix was narrowing the floor sweep's plane fallback: it now fires only
  when **no surface exists** under the body within `height * 1.1` (18Ascend's data gap), not when a real floor merely
  lies below this frame's drop — the wider rule held the body on sector 35's infinite plane and the leg never moved
  a room. Remaining deviation, **re-attributed at round 356**: the leg's `z 0.422` is the *end-position* delta, and it is
  travel length, not standing height. Both sides walk the same direction (about 225 degrees; retail's `yaw` column is the
  stale 135.01, the motion is not) and the port is simply slower over the same 3 s (3.09 vs 3.73 units, 1.03 vs 1.24
  u/s, the gate's own acceleration ramp). At retail's own end point (7.19, 13.36, sector 24) `--floor-at` gives the port
  `floor_under -> -0.0724` against retail's -0.00, i.e. **0.07**, not 0.42; the port never reaches sector 24 because it
  covers less ground. So the floor-surface choice is not the row's cause and this leg's remaining deviation is the
  walker's speed on loose floor, not the floor's scope.
- **Spawn combat: mechanism recovered and ported (round 298).** The engine's sight is **probabilistic**, not a
  cone test: `sithAI_CanDetectSightThing` (`0x4ec140`) builds a chance from the gap (`base 0.5` unless the AI word
  carries `0x200`; a `(distance - 2) * 0.1` falloff capped at `0.6`; `x 0.75` / `x 0.5` for a player's crouch and a
  still stance; `x 0.05` when the target's `thingflags` carry `0x80` and the actor lacks `0x8000`, and `x 0.05`
  when the actor carries `0x800`; floor `0.05`) and rolls it against `rand/32768`. The constants were read out of
  the shipped image (`0x00523060..0x005230a0`: `2.0, 0.1, 0.6, 0.5, 0.75, 0.05, 1/32768`). The roll runs **once per
  instinct pass** — `sithAI_InstinctUpdate` (`0x4e9e90`) arms each instinct at `now + 1000` before calling it — so
  acquisition takes seconds at range. `AISetMode` (`0x500f50`) ORs its argument into the AI word with no
  translation, and `sithAI_Process` skips an actor whose word carries `0x3000`; the shipped scripts set `0x2000`
  (24 calls) and never `0x1000`, so the port's `0x2000` skip was already right.
  The port had the formula (`detect_chance`) but rolled it **every frame** and bypassed it entirely on the
  `LookForTarget` path, which seeded `ai_last_seen` from the cone alone — that memory is what `BlindFire` shot at
  0.02 s into level 3. Both are fixed: the roll is armed once a second per actor (`ai_detect_at`), the scan path
  rolls `can_detect` and latches detection before it remembers anything, and every memory write goes through
  `Runtime::remember(actor, at, why)` so `BK_AI_TRACE` names the handler (`sight`, `lookfortarget`, `sensedanger`,
  `surprise`). Measured on level 3's spawn with `BK_AI_TRACE=2`: the actors that still fire at `0.00 s` are the
  ones whose first roll passed (`ai: 237 fires PrimaryFire ... (time 0.00 s)`), which is the engine's own first
  pass; retail's "no damage over the series" is a draw outcome, not a missing rule. Tests:
  `sight_detection_rolls_once_a_second_not_once_a_frame`, `look_for_target_scans_the_class_view_cone` (now rolls
  with a plainly-visible target), and the turret test pre-latches detection because it is about `TurretFire`.
- **Weapon framing.** `view.base` is `VIEW_BASE` and a test keeps the drawn weapon below the centre line. The claim
  that `--shot` cannot see the viewmodel is **stale and now corrected (round 284)**: it held only because the capture
  lacked the documented flag. `BK_ARM=11 BK_POV=bryv.3do` draws it — the trace prints `viewmodel: drew bryv.3do
  (13 material part(s), sector light 0.88)` and the PNG differs from the same capture without `BK_POV` in **8 843
  pixels (1.12 %)**, bounding box **x 475..565 of 1024 (0.464..0.552), y 540..767 of 768 (0.703..0.999)** — a 91 x 228
  box hugging the bottom centre. What remains is the retail side of the fit: retail's own first-person frame at a
  matching pose, which needs a retail run.
- **Retail *can* boot any level — through the `[Debug]` chooser, not a savegame (round 297).** The
  Single Player screen's `[Debug]` toggle opens a numbered "Choose Level" list in campaign order, and
  `retail-capture.ps1 -StartLevel <N>` already drives it (click `[Debug]`, click the list's first row,
  press `DOWN` `N-1` times — this scrolls past the visible nine — then Ok). It has been used for levels 4,
  6, 9, 12, 13 and 17, and round 297 booted **level 2** with it: `chose level 2 through the [Debug] level
  select`, `spawn| Pos: (10.80, -11.98, 3.02) PYR: (0.66, -99.98) Sector: 22` — the row's requested pose
  exactly. The earlier "blocked on a savegame" conclusion is superseded. The savegame route stays closed
  and is now explained: `jkGuiSaveLoad_PopulateList` (`0x41ddd0`) lists `player\<profile>\*.jks` and
  **skips names starting `_JKAUTO`**, while the profile is bound only when a player is chosen, so a fresh
  main-menu Load screen is empty by design. `retail-capture.ps1` now seeds `player\Lee\save0000.jks`
  from the freshest autosave before the cleanup deletes them, for the cases that route still needs.
- **Level 2's spawn pair is graded and its deficit is lighting (round 297).** `level2_spawn` (retail pose
  verified) compares at **SSIM 0.3705 (2.01 dB)**, mean abs diff 23.8/255 — the geometry, the tower lights
  and the floor line all agree; retail's frame is far darker and has no HUD in that moment. That is the
  *lighting* ledger row, not a framing or pose error; the number is now the row's baseline.
- **Round-284 note, kept for the record: retail cannot start a level from the command line.** `-JkArgs
  '-level,15maw'` exits JK.EXE with code 3 before its window appears, the same result `docs/gaps.md` records for
  `02narshadda`; and a row without it silently photographs **level 1** (the harness's `level` field is the row's name,
  not the booted level). The trap is closed: a pose miss above 5 units now throws instead of writing a fixture, the
  sidecar records `pose_verified`, and `series-gate.ps1` SKIPs an unverified pose. The unblock is a **savegame recorded
  in the target level**, which `src/jks.rs` already writes, loaded through retail's own load screen (`-Keys` exists).
  The savegame route is itself blocked one step further on: the patched scratch save is valid (the port's reader shows
  `level '15maw.jkl'`, 452 KB, 7 704 records) and the harness reports `patched the savegame to boot 15maw.jkl (level
  number 15)`, but retail's **Load Game menu lists no savegames at all** — with the patch and without it — while
  `saves/` holds `autosave.jks`/`quicksave.jks` and `player/Lee/save0000.jks` exists. Next step: find what retail reads
  to build that slot list (profile `Lee.plr`? a slot index?) or reach the level by the campaign transition. Until that
  lands, the level-2 spawn refit, the per-level spawn survey and the level-14 cutscene grade stay open.
- **Cutscene comparison — closed as not automatable (round 284).** Level 14's intro camera is exact against
  `macamera.key`, but retail cannot be brought to that level by any of the three routes below — **except through the
  `[Debug]` level list, which round 297 proved open** and which has since booted levels 2, 4, 6, 9, 12, 13 and 17 (see
  the bullet above): this row's "not automatable" verdict predates that and applies only to level 14, which has not been
  driven through it. The other two are proved closed: the command line (`-JkArgs '-level,15maw'` exits JK.EXE with
  code 3 before its window appears; `docs/gaps.md` rounds 60-61) and the **Load Game menu** (empty save list with and
  without a patched `player/Lee/save0000.jks`, while `saves/` holds `autosave.jks`/`quicksave.jks`). Retail capture
  stays user-invoked, so the level-14 leg remains a session task, not an automated one. The timestamps that made this
  row gradable stay in the tooling (`series_times_ms`, `series_span_ms`, and the gate's measured-span capture, proved on
  a 7 200 ms sidecar), so any leg that *does* have a retail series grades on its own interval. The port side of this row
  rests on `macamera.key` rather than on a retail frame — stated as the tradeoff, not as a pass.
- **The film's frame and colour are settled, and the pair is gated (round 310).** The still-open item
  from round 309 (`--film-dump` / index-to-content / colour) needed no new instrument: the frame index
  was already right, the palette already matched ffmpeg's Smacker decode of the same frame exactly, and
  the colour difference was Bevy's default `Tonemapping::TonyMcMapface` applied to the UI camera. With
  `Tonemapping::None` on that camera, the port's `BK_FILM_HOLD=147` crawl frame is byte-identical to
  retail's captured frame (0/307,200 pixels, bbox 44,160..595,244), and `tools/parity/film-gate.ps1`
  gates it (SSIM 1.0000, floor 0.99). The menu pair moved up with it (0.9521 → 0.9661). Two film-path
  bugs came out of making the pair deterministic: the 32-bit helper was called with the frame count in
  its flags slot (and `SmackGoto` cannot seek — it repeats one buffer), and every film shared one
  decoder prefix, so a held frame could be read from an earlier run.
- **The draw gate's three route failures were the ambient-base lighting rule, and are now attributed.**
  The pre-merge commit reproduces the recorded numbers exactly; the merged rounds' engine-derived
  ambient base darkens level 1 by ~5 luma and moved the route MAD past the old floors. The rule stays
  (level 2 went 1.99x → 1.02x of retail's mean under it), the floors were re-derived from the A/B in
  `thresholds.tsv`, and level 1's residual is the *lighting* ledger row's open term — the next rung
  there is a per-surface predicted-vs-rendered comparison on that view, not a global constant.
