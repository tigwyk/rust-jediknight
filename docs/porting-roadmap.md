# Porting roadmap — the north star

`roadmap.md` lists *subsystems*. `gaps.md` lists *missing units*. Neither says **how to decide that
something is ported**, or **what to work on next when the build is not yet fun to play**. This file does.

## Why this exists

As of 0.1.1 the port reports 10 of 10 roadmap stages "done", 61 gates and 133 tests green, a signed-off
release, an installer, CI and a landing page — and level 1 still has fundamental playability problems.
The first retail reference frame (`tools/parity/`, gaps §5.9) found one at once: the spawn view sits
inside a cantina table that retail does not draw through the eye.

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

Today almost everything is T1. The roadmap's "done" rows should be relabelled with the tier they have
actually reached; most will read T1 and a few T0.

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

## Phases

### Phase 0 — Reset the definition (do first, small)

- Relabel every `roadmap.md` / `gaps.md` row with its tier. Merge the three overlapping lists
  (`roadmap`, `gaps`, `engineering` status) into one status table; keep the long prose as appendix.
- Make `tools/parity/` a first-class gate: commit it, add one command that prints a per-shot score
  against a threshold file, enable retail `warp`, and add pitch to the port's `--shot-at`. (The harness
  is untracked at the time of writing.)
- Build the **deviation ledger**: grep the `ponytail`/`shortcut:` markers and the gaps §2 partials into
  one list with owner, measurement and retire condition.
- Freeze: installer, site, new UI screens, version bumps. Releases pause until Phase 1 exits.

### Phase 1 — Level 1 vertical slice (the real work)

Ordered so each step is observable and unblocks the next. Each item ends with a retail fixture and a
tier, not a checkbox.

1. **First frame.** Spawn pose, eye height, near plane, which things draw at the eye, player-body
   exclusion in first person. Exit: spawn shot pair within threshold. *(in progress elsewhere)*
2. **The room as retail draws it.** Model back-face behaviour, transparency order, sky, light ramp at
   distance, sprites. Exit: a fixed set of poses along the level-1 route (cantina, corridor, first
   door) within threshold.
3. **Movement.** Replace the unscoped sweep with the engine's sector-scoped collision; non-player
   thing physics (props, corpses, dropped items); step, slope and ledge behaviour. Exit: a recorded
   input script yields the same position/sector trace as retail, within a stated epsilon, across the
   level-1 route.
4. **Interaction.** Doors, lifts, switches, pickups through cogs; thing parenting; timing (`Sleep`,
   pulses) at engine granularity. Exit: cog variable/state trace matches at each trigger.
5. **Combat.** Weapon fire, hit detection, damage, AI sight and routing, puppet states. Exit: the same
   encounter plays out with matching health and kill order from the same script.
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

Reviewed `tools/parity/` (retail-capture, retail-lib, port-capture, compare, shots.tsv). It drives the
GOG `JK.EXE -devMode` on a scratch copy, so it is the oracle this roadmap needs; it only has one verified
row so far (`spawn`).

| Question | Answer from the harness |
|---|---|
| Can we read retail state without a debugger? | **Partly.** The dev console's `coords` prints position, PYR and **sector**, and the console list box is scraped for engine output (e.g. level-parse lines). That is enough for T2 on position/sector. Health, velocity, bins and cog variables are not exposed yet. |
| Can we drive retail without a human? | **Yes.** Menus take synthetic mouse clicks at fixed coordinates; in gameplay synthetic keys work (keys held 300 ms or more, or the engine drops them). Walking scripts look feasible but are unproven. |
| Can we put retail at a chosen pose? | **Not yet.** `warp` is "not recognized" on the GOG build. The cause and a one-patch fix are found (see below). Until the harness applies it, only the level spawn is a verified pose, which blocks every Phase 1 step past 1. |
| Is there a retail frame to compare with? | **Yes.** F12 is the engine's own screenshot (world only, no HUD). Frames are cropped to the world box (736x767 today) and scored by SSIM/mean difference, which the README itself calls triage, not equality. |
| Is the harness safe to run routinely? | **With care.** It takes over the display (640x480 fullscreen, mouse captured), needs hands off, and has restore scripts for an interrupted run. Not CI-able: needs a retail install and a desktop session. |

## Getting the rest from Ghidra and REA

The exported decompilation in `decomp/` already settles part of this. Plan for the remainder:

**Found in `decomp/` (read, not yet run):**

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
   offset `0xed099` (jz at `0xed0a4`). The harness already restages the install into
   `target/parity/retailcopy/` on every run, so it can apply the patch after staging and assert the pattern
   is unique first. The real install is never touched.
4. **Screenshots.** `jkGame_Screenshot` (0x4025e0): on the software path it converts the framebuffer to a
   bitmap, swaps in `stdDisplay_GetPalette()` and calls `stdPcx_Write`. So the F12 PCX is **8-bit indexed
   with the display palette**: palette-index comparison is possible, not just SSIM. The 16-bit/3D path
   writes BMP instead; keep 3D acceleration off.

**Not yet done (needs a retail run, which takes over the display, so not run from here):**

- Apply the patch and confirm `warp`, `cogstatus`, `aistatus`, `slowmo` register and print.
- Decompile the `LAB_` handlers for `coords` (0x4ee960), `cogtrace` (0x4ee170), `coglist` (0x4ee2f0) to
  pin their output formats for parsing.
- Frida/`observe_native_calls` hooks on `sithThing_SetPositionAndOrient` and the damage function for a
  per-tick CSV (position, velocity, health). `slowmo` plus a fixed key script may make this unnecessary.
- Palette-index diff: have the port emit an indexed frame, or quantise through the retail palette.

Still open after the review:

- **Per-tick state.** `coords` is a manual poll. The unlocked `cogstatus`/`aistatus` commands may cover
  script and AI state; health, velocity and bins likely still need a hooked trace (see above).
- **Pitch and non-spawn poses.** The port's `--shot-at` ignores pitch; retail needs `warp`. Both are
  Phase 0 prerequisites for the route shots in Phase 1 step 2.
- **Input replay determinism.** Retail frame timing is not ours; T4 needs fixed-step or a tolerance, and
  the harness has no replay of movement yet.
- **Better than SSIM.** Confirmed the PCX is palette-indexed (above); what remains is the port emitting
  indices.
- **Renderer tolerance.** Retail's software renderer and Bevy will never match pixel for pixel; pick
  per-shot thresholds from known-good shots and tighten them over time.
- **Fixture hygiene.** Keep traces and numbers in the repo; keep retail screenshots out of any
  GPL-licensed distribution unless the existing NOTICE terms cover them.
