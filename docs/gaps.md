# What is left to port

A map of the engine's own source units (`sith*`, `jk*`) against this port, so the remaining work is a list
rather than a feeling. Every row names its evidence: the gate that runs it, or the file that holds it.

The port's checks at the time of writing: **30 gates, 76 tests, 0 failures.** Stubbed script surface:
**80 verbs, 769 call sites across the 34 shipped levels.**

Two rules for reading this file:

* **Absent** means a grep for the engine's identifiers in `src/` finds nothing. It is a statement about this
  port's source, not a claim about the shipped product.
* **Data-driven** means the engine implements the feature in a `.cog` that ships with the game, so the port's
  job is the *verbs*, not the feature. Force powers are the clearest case: `force_*.cog` exists in the
  install, and what the port owes them is `setinvactivated`, `setthingvel`, `setactorflags`.

Regenerate the numbers:

```
bevi-knight --run all                 # the 80 stubbed verbs, verbatim, aggregated over the install
bevi-knight --cog-check               # the same count with an assertion
bevi-knight --collide --level X.jkl   # collision and spawn facts
bevi-knight --walk all                # hull agreement across 34 levels
bevi-knight --rules / --feel / --fall # the player physics
```

---

## 1. Absent subsystems

Nothing in `src/` answers these. Ordered by how much shipped content they gate.

| subsystem | engine units | what is missing | evidence |
| --- | --- | --- | --- |
| Networking and multiplayer | `sithDSS.c`, `sithDSSThing.c`, `sithDSSCog.c`, `sithMulti.c`, `sithComm.c`, `stdComm.c`, `jkGUIMultiplayer.c`, `jkGUINetHost.c`, `jkGUIMultiTally.c` | the whole thing: message streaming, thing replication, cog RPCs, host/join, tally screens. Zero socket code exists. | no `socket`/`UdpSocket`/`TcpStream` in `src/`; `ismulti`, `isserver`, `getnumplayersinteam`, `setplayerscore`, `setplayerteam`, `sendmessageex`, `sendtrigger`, `*multimodeflags` are all stubs |
| Console and cvars | `sithConsole.c`, `sithCvar.c`, `sithCommand.c`, `jkQuakeConsole.c` | the `~` console, the cvar store, and the command dispatcher (`bind`, `set`, `map`, `quit`) | no matches for `cvar`, `sithCommand`, `sithConsole` |
| Particles and effects | `sithParticle.c`, `rdParticle.c`, `sithExplosion.c` (visual half) | effect and particle spawning, and explosion *visuals*. `rt.explode` does create the blast thing and apply its damage (`--hit`), but an explosion template is a `.spr` and sprite drawing is absent, so the blast is invisible. | no particle type; `explosion` matches are class numbers and the damage path (`main.rs:9253`) |
| In-engine cutscene cameras | `jkCutscene.c`, `sithCamera.c` focus modes | `SetCameraFocus`, `SetCurrentCamera`, `SetPovShake`, scripted camera paths. Cutscenes play as `.smk` film over the world; an in-engine camera cutscene would not cut. | no matches for `SetCameraFocus`/`camerafocus`/`shake`; all three verbs are stubs |
| Saber | `jkSaber.c` | blade geometry, swing arcs, saber lock, trace damage. `jksetsaberinfo` is a stub. | `saber` matches are template-name and palette text only |
| Arch lighting | `sithArchLighting.c` | vertex-baked arch light. Level data carries an `archlightIdx` per thing; nothing reads it. | no matches for `archlight` |
| Track things | `sithTrackThing.c` | things that ride a `.key` track (free paths, camera dollies). The port interpolates `frame=` lists, which covers doors and lifts but not a path with its own timing. `pathmoveresume` is a stub. | no matches for `TrackThing`/`MT_PATH` |
| Event queue | `sithEvent.c` | the engine's timed event list. Pulses and timers live in `cog.rs` and cover the scripts; a non-cog timed event (thing expiry, AI tick) has nowhere to live. | no matches for `SithEvent` |
| Overlay map and 3D map screen | `sithOverlayMap.c`, `sithMap.c`, `jkGUIMap.c`, `ui.rs` | the automap overlay and the 3D map view. The screen is a static backdrop. | `ui.rs:365` `ponytail:` — no zoom/orbit transforms |
| Scope | `jkHudScope.c` | the sniper/binos overlay | 2 incidental HUD-flag matches, no rendering |
| Thing parenting | `sithThing.c` (`pParent`, `attachedThings`) | a child thing following its parent's transform: a gun in a hand, a passenger on a mover | `attachthingtothing`, `setthingattachflags`, `getthingparent` are all stubs |

Multiplayer is a project of its own and is deliberately out of scope: the UI holes the docs used to list are
both multiplayer-only screens.

---

## 2. Partial subsystems

| subsystem | what works | what is missing |
| --- | --- | --- |
| Thing attachment | the impact half of `sithThing_AttachThingToSurface`: `(v-2.5)^2*45` above a 2.5 threshold, applied to the runtime's player (`--fall`) | the *record*: `attach_flags`, `attachedSurface`, the surface-info snapshot, scroll-velocity transfer on a scrolling surface, and the `entered`/`exited` cog messages on a `COG_LINKED` surface |
| Physics | velocity integration with `ApplyDrag`, gravity, jump, fall, step, spawn clearance — for the player | `physicsRolloverFrames`/`OLDSTEP_DELTA_50FPS` accumulation (the port uses the frame's `dt`), angular velocity, `SITH_PF_USEANGULARTHRUST`, rotation of things (`sithCollision_RotateThing`), scripted movement of things, and every non-player thing: placed props are static |
| Puppets and animation | `stand` and `death`, `.key` interpolation with parent chains and pivots, `frame=` paths for doors and lifts | the rest of the state machine — `walk`, `run`, `walkback`, `strafe`, `crouch`, `jump`, `attack`, `holster`, `draw`, `victim` — and `sithAnimClass` layering, so an actor never changes pose in reaction to anything. `playkey`/`stopanim`/`jumptoframe` are stubs |
| AI | the `.ai` parser (instincts, flags, moves), actor spawning, and `sightdist`/`fov` deciding whether an armed actor fires its own `weapon=` — `--ai` fires a level-1 actor at 3 units, not at 8, not from behind; 17 actor templates in level 1 name an instinct class | the runtime: instinct **scheduler** (weights, priority, interruption), **awareness** (no hearing, no alert propagation), **pathfinding** (steering is greedy — it picks the doorway that looks closest on both legs), `accuracy=` spread, aim-joint tracking, and the 11 `aiset*` verbs (43 calls each) |
| Actors | placement, health, shields, death, `--ai` | aim joints (`sithActor_UpdateAimJoints`), head pitch as a separate channel, force-power state, mount waits |
| Items and inventory | bins, maxima, activated/available flags, pickups through each item's own script, respawn | item *use*, respawn masks (`setrespawnmask`), pickup sounds, `setmountwait` |
| Weapons | a fired bolt flies with the template's `vel=`, stops at the sector graph, applies `damage=` with `SITH_WF_DAMAGE_DECAY`, a kill plays `death`; ammo bins; `--hit` two-way damage | alt fire (`weapon2`), fire offsets, muzzle flashes, `autoselectweapon`, `setfirewait`, per-weapon animation, and ammo as `weaponData` describes it (the bin→projectile mapping is a name heuristic, `runtime.rs:1294`) |
| Damage | health, shields, two-way, death and respawn, `DAMAGE_DECAY` | damage **types**: the engine carries a `SITH_DAMAGE_*` bitmask (impact/energy/fire/force) that decides shield versus health and force absorption. The port passes a plain float (`types_enums.h:1328`) |
| Force powers | the bins and the verbs the shipped `force_*.cog` scripts call; jump, speed, healing, seeing, pull and grip run | no structural gap, but the *effects* are unverified beyond jump and speed: nothing drives `force_pull.cog`'s grip in a test, and `setpovshake`/`setthingrotvel` are stubs those scripts would want. `force jump`'s velocity once reached `SetThingVel` as zero — fixed, but the path has no assertion of its own |
| Sound | 2D playback of `.wav`/`.snd`, one music track, `--audio-state`, per-sink mute, `create`-class sounds | 3D positional and velocity mixing (`sithSoundMixer.c`), sound classes with mode ranges, `changesoundpitch`/`changesoundvol`/`sectorsound`, ambient-loop bookkeeping beyond a count |
| Surfaces and sectors | the flags, `SURFFLAG_SKY`/`SURFFLAG_CEILING_SKY`, adjoin state, `sithPhysics_FindFloor`, thrust values | surface light animation (`surfacelightanim`, 18 calls), material animation, scroll UVs, `setsectorflags`/`setadjoinflags`/`clearadjoinflags` (17 calls), `setsectorthrust`, runtime sector light changes |
| Sky | unlit sky faces plus a dome, and the camera's clear colour | sky scrolling (`slidehorizonsky`, `slideceilingsky`), the sky's own parallax and rotation (`sithRenderSky.c`) |
| Models and materials | `.3do` parse, geosets, materials, `RADIUS`, `insertOffset`, mipmaps, the light ramp, `--winding` | `setthingmodel` (swap a model at runtime), per-surface material animation, the geometric-mode verbs (`*geomode`), and no back-face culling for models (`main.rs:9822` `ponytail:` — model interiors render when the camera clips in) |
| Rendering | opaque pass, portal-aware sector culling, the light ramp, HUD | no transparency sorting, sprites counted but never drawn, and a changed animation frame rebuilds the whole mesh asset instead of writing into it (`main.rs:8356`) |
| Colormap and lighting | the palette, and each `.cmp` ramp sampled at **32 steps** (`LIGHT_STEPS`) to build the surface light ramp; a single far colour for distance shading | the engine's per-distance table **selection** — a surface picks the 256-entry table matching its distance from the light — plus sector light animation. 256 tables are located per ramp (`(len - CMP_HEADER - CMP_PALETTE) / 256`, `tex.rs:124`) and the ramp uses 32 of them |
| Savegames | the container and thing records: version 6, the 1580-byte header, the `0x28` record header, template/type/position/angles round-trip | the rest of `sithGamesave.c`: cog state, AI state, item respawn masks, the event queue, multiplayer sections, and the header internals |
| Cog | the VM, 34 levels of scripts, pulses and timers, every verb the scripts call except the 80 below | the 80 stubbed verbs in section 3 |
| HUD and UI | the HUD, most screens, menus built from `jkGUIRend` tables | the screens still placeholders (`--ui-tables`), the scope, the map screen, and the tally screens (blocked on multiplayer) |
| Cutscenes | `.smk` decoding of all 25 shipped films (483 MB, 29,493 frames) with their own palette records, seeded from the level | in-engine camera cutscenes (section 1) |

---

## 3. The 80 stubbed cog verbs

The concrete remaining port surface: the shipped scripts call these and get nothing. Aggregated by
`--run all` over the 34 levels.

**AI (11)** — the largest group: `aiclearmode aiflee aisetclass aisetfiretarget aisetlookframe aisetlookpos
aisetmode aisetmoveframe aisetmovepos aisetmovespeed aisetmovething`

**Thing properties and flags (17)**: `clearthingflags jkclearflags getthingmass setthingmass setthinguserdata
setthingtimer isthingvisible createthingatpos teleportthing syncthingpos syncsurface setmountwait setthinglight
thinglightanim setthingrotvel returnex newcoloreffect`

**Attachments and parents (3)**: `attachthingtothing getthingparent setthingattachflags`

**Sectors, adjoins, sky (6)**: `setsectorflags setadjoinflags clearadjoinflags setsectorthrust
slidehorizonsky slideceilingsky`

**Animation and models (6)**: `playkey stopanim jumptoframe materialanim surfacelightanim setthingmodel`

**Weapons and projectiles (4)**: `fireprojectile autoselectweapon setfirewait jksetsaberinfo`

**Camera (3)**: `setcamerafocus setcurrentcamera setpovshake`

**Cog introspection and goal state (8)**: `getmastercog setmastercog getselfcog getgoalframe setgoalflags
gethealth getleveltime loadtemplate`

**Geometric modes (4)**: `getfacegeomode setfacegeomode getthinggeomode setthingcurgeomode`

**Sound (3)**: `changesoundpitch changesoundvol sectorsound`

**Misc (15)**: `clearmultimodeflags getmultimodeflags setmultimodeflags getnumplayersinteam setplayerscore
setplayerteam ismulti isserver sendmessageex sendtrigger getthinguserdata pathmoveresume rotatepivot
setrespawnmask setpovshake`

The groups are not disjoint (`setpovshake`, `getthinguserdata` appear twice); the union is the 80.

---

## 4. Debt already marked in the code

15 `ponytail:` comments name a deliberate ceiling and its upgrade path. The ones that change behaviour:

| where | the shortcut | the ceiling |
| --- | --- | --- |
| `main.rs:9822` | no back-face culling for model triangles | model interiors render when the camera clips into them |
| `main.rs:8356` | a changed animation frame rebuilds the mesh asset | a 60 fps puppet rebuilds its mesh sixty times a second |
| `collide.rs:698` | the four-way clearance test rejects a body touching the riser it was blocked by | a step only succeeds if the lift clears the riser's polygon top; `STEP_UP = 0.66` compensates |
| `collide.rs:905` | the floor probe scans every wall face | O(walls) per moving body per frame |
| `collide.rs` lateral sweep | the sweep is sector-scoped | a body whose recorded sector is a hull-less sliver needs the fallback that swaps sectors |
| `runtime.rs:1294` | the bin → projectile mapping is a name heuristic | not the engine's `weaponData` |
| `runtime.rs:205` | insert offsets live in one process-global table | two worlds loaded at once (level plus cutscene) share it |
| `runtime.rs:1456` | steering picks the doorway that looks closest on both legs | no path search, so an actor can pick a longer route or lose the chase |
| `level_mat.rs:10` | the shader adds no light level | the mesh carries baked vertex colour only |

---

## 5. Where to start

1. **AI verbs, then awareness.** 11 verbs, 43 calls each. Everything the levels want from their enemies goes
   through them, and the `.ai` data is already parsed.
2. **Thing properties** (`setthingmass`, `setthingtimer`, `setthinglight`, `createthingatpos`). Cheap, and
   several ship in scripts that also drive level logic, so their absence silently changes behaviour.
3. **Puppet locomotion states.** Visible, and the state data is already loaded — the pose code is real.
4. **Attachments and parenting.** Needed by carried weapons, passengers, and any script that moves a child.
5. **Damage types.** Small, and it changes combat feel: shields currently absorb everything.
6. **Camera focus and shake.** It is what makes in-engine cutscenes and several force powers read as intended.

---

## 6. Recently closed, so it is not mistaken for missing

* Thing orientation: the row's `pitch yaw roll` is applied through a verbatim `rdMatrix_Build34`
  (`thing_quat`). 138 of 352 things in level 1 are tilted, so 39% of props used to be drawn flat.
* `sithThing_CreateThing`: marker-relative positioning via the insert-offset difference, with the marker's
  orientation, plus `rotate_by_angles`. Test: `create_thing_offsets_by_the_insert_difference_in_the_marker_frame`.
* `sithThing_AttachThingToSurface`'s impact update: fall damage, gated by `--fall`. It caught a real bug —
  `fall_damage` squared `(v-2.5)` without the engine's `v > 2.5` test, so a 1.2 u/s landing was charged 76.
* Player physics derived rather than calibrated: `physics_scale = 1`, walk = `maxthrust/surfdrag` = 0.667,
  radius = `size x MODEL_SCALE` = 0.65, eye = `eyeoffset.z x MODEL_SCALE` = 0.37. `--rules` asserts the
  relationships and fails with a scale in the chain (§23 of `formats.md`).
* `SITH_SURFACE_CEILINGSKY` (0x400) joins the horizon bit in the sky test.
* The huge orange polygons are the level's freighter (`01frt.3do`, 110 units long), not broken geometry.
* The camera used to spawn inside an NPC: the clearance rule wanted `radius * 2` = 0.7, exactly an NPC's
  width. It wants 2.0 units now.