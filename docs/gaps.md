# What is left to port

A map of the engine's own source units (`sith*`, `jk*`) against this port, so the remaining work is a list
rather than a feeling. Every row names its evidence: the gate that runs it, or the file that holds it.

The port's checks at the time of writing: **61 gates, 132 tests, 0 failures.** Stubbed script surface:
**0 verbs, 0 call sites across the 34 shipped levels** (706 script verb calls, all answered).

Two rules for reading this file:

* **Absent** means a grep for the engine's identifiers in `src/` finds nothing. It is a statement about this
  port's source, not a claim about the shipped product.
* **Data-driven** means the engine implements the feature in a `.cog` that ships with the game, so the port's
  job is the *verbs*, not the feature. Force powers are the clearest case: `force_*.cog` exists in the
  install, and what the port owes them is `setinvactivated`, `setthingvel`, `setactorflags`.

Regenerate the numbers:

```
bevi-knight --run all                 # the 67 stubbed verbs, verbatim, aggregated over the install
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
| Networking and multiplayer | `sithDSS.c`, `sithDSSThing.c`, `sithDSSCog.c`, `sithMulti.c`, `sithComm.c`, `stdComm.c`, `jkGUIMultiplayer.c`, `jkGUINetHost.c`, `jkGUIMultiTally.c` | the whole thing: message streaming, thing replication, cog RPCs, host/join, tally screens. Zero socket code exists. | no `socket`/`UdpSocket`/`TcpStream` in `src/`. The single-player **answers** are now explicit (`ismulti`/`isserver` 0, no team, no score, `*multimodeflags` stored) and `sendtrigger` really broadcasts `trigger`, but only `sendmessageex` is left stubbed — it needs message *return values* |
| Console and cvars | `sithConsole.c`, `sithCvar.c`, `sithCommand.c`, `jkQuakeConsole.c` | the `~` console, the cvar store, and the command dispatcher (`bind`, `set`, `map`, `quit`) | no matches for `cvar`, `sithCommand`, `sithConsole` |
| Particles and effects | `sithParticle.c`, `rdParticle.c`, `sithExplosion.c` (visual half) | effect and particle spawning. **Sprites draw, with their real artwork and animation**: every template's `sprite=` is parsed (`spr.rs`), its `.mat` loads through the cel-table reader in `tex.rs` with palette entry 0 keyed transparent, `draw_created` spawns one billboarded quad per row and `billboard_sprites` faces it at the camera. The frames walk with the thing's life (`Runtime::sprite_cel`: `typeflags & 0x1` arms it, frame *k* holds for the *k*-th `life / cels` slice), which is what `sithExplosion_CreateThing` does through `sithSurface_sub_4F00A0`. | no particle type (`sithParticle`'s own things — smoke, embers and dust are separate things the engine spawns); `explosion` matches are class numbers and the damage path |
| In-engine cutscene cameras | `jkCutscene.c`, `sithCamera.c` focus modes | **The focus models work**: seven camera slots, `SetCameraFocus(slot, thing)` + `SetCurrentCamera(slot)` move the view onto that thing (aimed by its own own yaw/pitch), `GetCameraMode`/`GetPrimaryFocus`/`GetSecondaryFocus` read back, and `SetPOVShake` adds the recoil offset that decays at its own rate. Level 17b's `17b_Camera.cog` cutscene runs end to end: the view cuts to `ghostCam`, and after the script's `Sleep(6.0)` it hands the view back to the player. | `CycleCamera`, `SetCameraZoom`, path-driven camera dollies beyond `PlayKey` (a camera on a `.key` track follows its thing but has no path timing of its own), and the secondary-focus framing |
| Saber | `jkSaber.c` | `jkSetSaberInfo` stores the blade the scripts describe (two materials, hilt and tip radii, length, three spark templates) with the argument order pinned by a gate (`--pickup`); **and the player can use it**: `saber_swing` traces the blade `length * 10` level units along the player's facing with the hilt radius, damages what it reaches with `DAMAGE_SABER` (0x10), has a 0.45 s cooldown, and spends no ammo — `--hit` proves a hit at 0.8 units, no hit at 4, and the cooldown. The blade is also drawn as an emissive rod from the eye (`saber_blade`), and `jkDisableSaber`/`jkEnableSaber` switch it off and on (`saber_shown`, asserted by `--hit`) | the **swing animation**: `jkSaber.c` runs arcs, parries and the saber lock through `JKFLAG_SABEREXTEND`/`SABERRETRACT` and `cog_weap_saber.cog`'s `SetPuppet` calls, and this build neither plays those keyframes nor draws a hand to hold the hilt. The blade length is read as model units (x10, the `.3do` node scale) because the scripts pass 0.100 for a blade that must reach a body |
| Arch lighting | `sithArchLighting.c` | **not reachable from the text format, checked:** `.jkl`'s THINGS section has no archlight column and SECTORS carries only `AMBIENT LIGHT`/`EXTRA LIGHT`/`COLORMAP`/`TINT`. The engine *computes* arch lights at load from the geometry (`sithArchLighting.c`), so porting this means re-deriving them, not reading a field | the computation itself: per-surface colour contribution accumulated into each thing's `light`, which is what gives Jedi levels their coloured bounce. The sector ambient/extra light and the thing-light sum are already there to build on |
| Track things | `sithTrackThing.c` | things that ride a `.key` track (free paths, camera dollies). The port interpolates `frame=` lists, which covers doors and lifts but not a path with its own timing. **`PathMovePause`/`PathMoveResume` work**: the thing keeps its destination, stops advancing (`paused`, the engine's `flags \| 0x80`) and reports `IsMoving` 1 throughout, which is exactly what level 9's pipe doors need to stop in the doorway (`09_stoppipedoors.cog`, `--things`) | a path with per-frame timings of its own (`trackParams` interpolation); the port walks frames linearly at `MoveToFrame`'s speed |
| Event queue | `sithEvent.c` | the engine's timed event list. Pulses and timers live in `cog.rs` and cover the scripts; a non-cog timed event (thing expiry, AI tick) has nowhere to live. | no matches for `SithEvent` |
| Overlay map and 3D map screen | `sithOverlayMap.c`, `sithMap.c`, `jkGUIMap.c`, `ui.rs` | the **field-log map panel draws the live level**: `M` opens the screen, `live_map_for` gives it `World` + `Player`, and `ui::map` projects the level around the player's pose (`--map-shot` checks both the ink and that moving the player changes the drawing). Still missing: zoom and orbit, and the 3D map view | `ponytail:` in `ui.rs` — no zoom/orbit transforms |
| Scope | `jkHudScope.c` | the sniper/binos overlay | 2 incidental HUD-flag matches, no rendering |
| Level change | `jkMain.c` (`jkPlayer_Initialize`), `swap_level` | the **player's own state carries over** a level change: inventory (weapons, force powers, mana, the ending choice), the armed bin, the selected weapon, the saber and the `jkFlags` the force scripts set (`Runtime::carry`/`adopt`, `cargo test` covers the round trip) | health and shields do **not** carry — each level starts the player at the template's own values, which is not what the engine does |
| Ending choice | `jkPlayer.c` (`jkPlayer_SetChoice`/`GetChoice`) | `jkGetChoice()` reads the real value: it is inventory bin 18 (`SITHBIN_CHOICE`), which the endgame scripts branch on and `jkPlayer_CalcAlignment` maps to +100/-100 (`--pickup`) | the **menu** that writes bin 18. There is no ending screen, so the value is always neutral (0) in this build |
| Thing parenting | `sithThing.c` (`pParent`, `attachedThings`) | **`AttachThingToThing` works**: the child keeps the offset it had at the call, in the parent's own frame (`field_4C`), so a moving parent carries it and a turning one swings it around — a crate on a lift (`00_elev_doors5.cog`), a catwalk on a ship (`09_donefueling.cog`), the sphere `force_absorb.cog` hangs on the player, the orbiting camera of `00_cameratricks.cog`. `DetachThing` and `GetThingParent` round-trip, `SetThingAttachFlags` is stored (`--attach`) | the engine's child *list* (a thing may carry several children, and chains resolve in list order) — this build resolves one level in index order, `ponytail:`-marked. A gun in a hand needs the puppet's joint, which is the puppet work, not this |

Multiplayer is a project of its own and is deliberately out of scope: the UI holes the docs used to list are
both multiplayer-only screens.

---

## 2. Partial subsystems

| subsystem | what works | what is missing |
| --- | --- | --- |
| Thing attachment | the impact half of `sithThing_AttachThingToSurface`: `(v-2.5)^2*45` above a 2.5 threshold, applied to the runtime's player (`--fall`) | the *record*: `attach_flags`, `attachedSurface`, the surface-info snapshot, scroll-velocity transfer on a scrolling surface, and the `entered`/`exited` cog messages on a `COG_LINKED` surface |
| Physics | velocity integration with `ApplyDrag`, gravity, jump, fall, step, spawn clearance — for the player | `physicsRolloverFrames`/`OLDSTEP_DELTA_50FPS` accumulation (the port uses the frame's `dt`), angular velocity, `SITH_PF_USEANGULARTHRUST`, rotation of things (`sithCollision_RotateThing`), scripted movement of things, and every non-player thing: placed props are static |
| Walk sweep | the walker runs the window's own `walker_step` on all 34 levels (`--walk all`), 20 s each: **34/34 stayed inside the tracked sector** over 639 doorway crossings, and on 14 levels the level's own hull test agreed | the tracked sector can still lag where sector hulls overlap, so `inside_sector` is the arbiter rather than the file's own hull |
| Puppets and animation | `stand`, `death`, and the locomotion set — `walk`, `run`, `walkback`, `strafeleft`, `straferight` chosen from the velocity the AI walked at, by the rule in `sithPuppet_UpdateThingMove` (dominant axis, 0.5 u/s walk/run threshold), with the clip advancing on distance moved rather than the clock. Plus `.key` interpolation with parent chains and pivots, `frame=` paths for doors and lifts, and `PlayKey`/`StopAnim`/`JumpToFrame` — a script can replace the puppet's state track with one of its own `keyframe` symbols (level 1's TIE bomber plays `01VTieB.key` from frame 0x14 when the player sees it) | an actor that just **fired** plays its `fire` track (the engine prefers the action track to the stride, and `--ai`'s shooter now animates the shot instead of sliding), and the armed/unarmed **mode** axis (**done**: the state key is `(name, mode)`, `SetArmedMode` selects it, and 113 of level 1's 128 actors ship a mode-1 set), a **hit** flinch (`just_hit`, below death and above `fire`; 127 of 128 actors ship the track), the **turn** states (a body pivoting in place plays `turnleft`/`turnright`, 226 actors ship them, with locomotion winning while moving), and the rest: the airborne set (`rising`/`jump`/`fall`/`land`/`drop`, ~220 actors), `death2`, `choke`, alt-fire `fire2`, `toss`/`windup`/`magic*`, and the **draw/holster** rows — **done** for `drawgun`/`holster` (`draw_row` plays the puppet's own row for its clip length when `SetArmedMode` flips; 24 actors ship each). `drawsaber`/`drawfists` are not selected yet, and `mount`/`dismount` — the *cog* symbol names, which animate the viewmodel — are a separate path, `turnleft`/`turnright`, `victim`, plus `sithAnimClass` layering. A scripted clip has no own epoch and its `flags` are not interpreted |
| AI | the `.ai` parser (instincts, flags, moves), actor spawning, `sightdist`/`fov` deciding whether an armed actor fires its own `weapon=`, `PrimaryFire`'s own arguments from the `.ai` (shot delay, range, aim cone, aim error, burst), scripted destinations, three-dimensional aim (an actor's pitch follows the elevation to its target, so it can shoot at a ledge; `--ai` checks it) over a 3D sight ray — `sightdist` is measured over the whole vector like `sithAI_CheckSightThing`, and the segment to the target must be clear, `AISetMovePos`/`AISetMoveThing`/`AISetLookPos`/`AISetMoveSpeed` make a placed actor patrol or follow, and a blocked destination is **routed** through the adjoin graph with the waypoints popped on arrival (`ai_route`; `--ai` walks one order to the end and asserts the list drains) — and the **frame list** verbs (`AISetMoveFrame`/`AISetLookFrame` index the thing's own `numframes=`/`frame=` entries, which class-less actors like level 1's 8t88 use). `--ai` fires a level-1 actor at 3 units, not at 8, not from behind, then walks one onto a scripted point and turns it; `--endlevel` shows 8t88 walking to his frame | the runtime: instinct **scheduler** (weights, priority, interruption), **awareness** (no hearing, no alert propagation), **pathfinding is in place**: `Collision::route` searches the adjoin graph breadth-first and `ai_tick` follows the doorway centres it answers, popping each on arrival (`ai_route`), so an actor told to reach a thing across a wall walks through the doorway instead of into the wall. Sight now needs a cleared line too, so an actor no longer sees — or shoots — through a wall. Losing sight now means walking to the last place the target was seen (`ai_last_seen`), and standing there forgets it. What is still missing is the retail *scheduler* below that: no hearing, no alert propagation, no instinct weights or interruption, `accuracy=` spread, aim-joint tracking |
| Actors | placement, health, shields, death, `--ai` | aim joints (`sithActor_UpdateAimJoints`), head pitch as a separate channel, force-power state, mount waits |
| Camera | position `thing + 0xb8 + thing[0x168]` (the eye offset, level units), orientation from the view angles at `thing + 0x15c/0x160/0x164`, the engine's yaw basis (column 1 = forward, confirmed at run time), the first-person velocity roll (`clamp(5 * dot(vel, right), -8, 8)`, constants at `0x522ddc`/`0x522de0`/`0x522de4`), the projection (**90 deg horizontal** from `buffer + 0x08`, vertical derived: 73.74 at 4:3, 58.72 at 16:9; near/far **1/64 and 64** from `rdCamera_NewEntry`'s `0x3c800000`/`0x42800000`, the port projecting near 1/64 and far 1000), a third-person boom 3.0 behind the eye pulled to the first hit by `collide::camera_pull` (the shape of `sithCamera_SearchSectorInRadius`, `0x4c6050`, cast radius 0.02), toggled with `V` (`C` still toggles the earlier 0.66-unit `third_person_view` boom, which draws no body), and the player's own body drawn in third person (spawned as an actor by `spawn_models`, one mesh per material, animated, hidden in first person and when the boom parks the camera inside him) | the engine's own case-3 follow camera (`0x4c59ce`, a spectator orbit spun at 8 deg/s) is not reproduced; the POV weapon's own sway (`FUN_004051e0`'s phase and the `0x552ba0` amplitudes) is not either — the port's `waggle` is the cog's sway, not that rotation (`SetPOVShake`'s offset *is* applied to the aim) — and the view weapon's exact retail framing waits on a writer for `thing + 0x168` (see the Weapons row) |
| `use` and the link tables | `Runtime::wire_links` fills all three tables from a level's `cogs` section plus each script's `symbols` (level 1: **37 thing, 44 surface, 10 sector** links), so a script now hears `used`, `activate`, `entered` and `exited`. The E key sends `used` to the nearest linked thing in front of the eye (`Runtime::use_thing`) and falls back to the nearest linked surface (`activate_near`) | was: nothing ever filled a link table, so `use` reached nothing and no message left the world. **Closed:** `--door` runs the whole chain in one process — use action, `activate` message, the level's own script, the flag — and `00_door.cog` on thing 4 clears its sector's ADJOINS bit and starts the piece moving. What it took: `activate` not `used`, a `startup` pass before any message (a door's `doorSector` is set there), and **one kept VM per instance** (locals live in the VM; a fresh one per label lost `doorSector`). Was: `cog_00_door.cog`'s `activate` reaches `CheckStatus`/`OpenDoors` and something there takes the do-nothing branch. The probe now runs every instance's `startup` first (a door's `doorSector` and `numDoors` are set there, and skipping it measured a door with an empty symbol) and prints the script, the frame and `IsThingMoving` per candidate: `00_door.cog` on thing 4 answers `frame 0, moving 1`, so `activate`'s `if (moveStatus) return;` is what stops it — the door thinks a piece is already moving. `01_*` lifts report moving 0 and still change no flag, and one `00_door.cog` (cog 13) runs **0 labels**, so its `activate` call fails outright. Next: why `IsThingMoving` is 1 for a door at frame 0 (is the piece on a path, or is `door0[i]` binding the wrong thing?) and why one label does not run. Note the message label too — no shipped cog declares `used` (0 of 540), 82 declare `activate`, so `use` must send `activate`. Level 1's doors are wired the *thing* way — `cog_00_door.cog` declares `thing door0..door3 linkid=0..3 mask=0x405` and answers with `setsectoradjoins` — so the door chain is: populate the link tables from the COG `symbols` section, then send `SITH_MESSAGE_USED`/`ENTERED` to the linked instances. 32 shipped cogs call the adjoin/sector verbs and every one of them is waiting on a message nothing sends |
| Items and inventory | bins, maxima, activated/available flags, pickups through each item's own script, respawn | item *use* — `UseBin(bin)` sends `activated` to the bin's own cog and the script does the rest (`--use-item`: the bacta heals 30 and spends one); and the **inventory strip is selectable**: `I` opens it, the arrows move, Enter uses the item through `UseBin` (`--use-item` also counts the highlight's pixels, since `ui::Canvas` is software pixels). What is missing is retail's own screen art — `jkHudInv` code this tree does not carry, respawn masks (`setrespawnmask`); the **held-bin loop** is real (`sithInventory_SendFire`: an active bin with a delay sends its own cog `fire` every delay until `DeactivateBin`, asserted by `--items`), which is what a held force power runs on, pickup sounds (`setmountwait` is in §3) |
| Weapons | a fired bolt flies with the template's `vel=` **along the shooter's whole orientation** (yaw and pitch; aiming up or down used to leave the barrel level),, stops at the sector graph, applies `damage=` with `SITH_WF_DAMAGE_DECAY`, a kill plays `death`; ammo bins; the player's `setfirewait` cooldown; `--hit` two-way damage | **inventory cogs**: `misc\items.dat` names a script per bin — 40 of the 82, all shipped (`weap_fists.cog`, `weap_bryar.cog`, …) and all now parsed (`--items`) — the handoff works — a bin going from empty to held names its script (`Runtime::set_bin`, `load_item_cogs` starts the VM and runs `startup`) — and the item scripts themselves now **run clean** (`--weapons`: 106 labels, 0 errors, 0 unimplemented verbs after 21 were implemented). The **routing works**: `fire_player` now sends the armed cog `fire` instead of firing directly, and `load_item_cogs` runs `activated:` when the player acquires the weapon (which is what arms it). `--weapons` proves the script end of it — the bryar's `fire:` spawns 2 things through `FireProjectile` and spends 1 of 50 rounds. The live path is wired but unobserved in this environment (no window). the POV half is retail's pass rendered in the camera's frame: `selected:` names the weapon's `povModel` (`bryv.3do`), `draw_viewmodel` builds it parented to the world camera with `NoFrustumCulling` (a view model is always in view), recentres it on its own box, textures it through its own `.mat` over the install's palette and tints it with the sector light (`AMBIENT LIGHT` + `EXTRA LIGHT` clamped 0..1, what `rdCamera_SetAmbientLight` gives the pass), and `view_frame` places and scales it from the 75th percentile of its vertex distances (`view_frame_tests` pins the frame). It follows `jkSetWaggle`'s sway and plays the clip the cog asks for (`jkPlayPOVKey`: `bryvpst1.key`, 13 frames, advanced at the key's fps). Still open: retail's exact framing, which is the POV thing's `+0x168` (no decompiled writer, so the port derives `PLAYER_EYE`), and the model's authored barrel orientation. `jkSetWeaponMesh` stores the weapon's **body** mesh and nothing draws it yet: the first-person half reads `pov_model`, so the third-person weapon mesh is the remaining half of the viewmodel work The muzzle is the **shooter's** own `fireoffset=` (parsed into `template_fire_offset`, model units, rotated into its facing), in the shooter's own frame with the engine's handedness (`sithWeapon_FireProjectile` transforms it by the shooter's orientation; the lateral sign was wrong until `--muzzle` pinned all three axes), and a projectile carries its `owner` so `impacts` skips the body that fired it — which is why the real in-body offsets are usable (`--muzzle`). The trigger obeys the weapon's own rate (`try_fire_weapon`, the engine's `binWait`); alt fire (`weapon2`) is still absent, as is autofire-on-hold, muzzle flashes, per-weapon animation, and ammo as `weaponData` describes it (the bin→projectile mapping is a name heuristic, §4) |
| Damage | health, shields, two-way, death and respawn, `DAMAGE_DECAY`, the engine's damage **kind** — a `SithDamageType` bitmask — with `SITH_AF_INVULNERABLE` honoured and its fall exemption (`--endlevel` watches the flag go set and clear around the level-end sleep), and **message parameters plus `ReturnEx`**: a `damaged` handler reads the amount from `GetParam(0)` and answers with `ReturnEx`, which the runtime applies instead of the amount it passed in (`--damage-return`; 115 shipped scripts have a `damaged` label, 54 answer with `ReturnEx`, and level 17b's `darkjedi_yun2.cog` is one) | the kinds are carried but unused elsewhere: the engine picks a hurt sound per type (`SITH_SC_HURTENERGY`/`HURTFIRE`/`HURTMAGIC`/`HURTSPECIAL`) and scales limb hits by 0.1 when the sender has an actor parent. Neither is ported. A handler can only *reduce* a hit (a settled hit is amended, not replayed), so a script asking for more damage than was applied is ignored |
| Force powers | the bins and the verbs the shipped `force_*.cog` scripts call; jump, speed, healing, seeing, pull and grip run, and **the powers are acquired the way the engine acquires them**: `misc\items.dat`'s `cog=` column names a script per bin, and a bin that goes from empty to held queues its own script (`Runtime::set_bin` -> `item_cog` -> `load_item_cogs` starts it out of the `.gob` when the level's script table does not carry it). That path was dead for every force row: their `max` column is a decimal (`4.0`) and an integer parse dropped all eight rows, so the map answered `bin 21 -> None`. `--items` now reads **40 of the 82 bins**, `21 f_jump` .. `28` among them, and `cargo test` pins the decimal parse (`the_force_rows_survive_a_decimal_maximum`); **the economy's arithmetic is in** (`src/force.rs`: the bin map, the rank ladder `jkPlayer_SetAccessiblePowers` walks, `CalcAlignment`'s weighted sum and clamp, the rank-8 mastery choice and the rank-7 side lockout, all against the constants read out of `JK.EXE` at `0x52206c`/`0x522074`/`0x522078` — `--force-economy` prints the model, `cargo test` pins it both ways) | the *effects* are unverified beyond jump and speed: nothing drives `force_pull.cog`'s grip in a test, and `setpovshake`/`setthingrotvel` are stubs those scripts would want. `force jump`'s velocity once reached `SetThingVel` as zero — fixed, but the path has no assertion of its own |
| Sound | 2D playback of `.wav`/`.snd`, one music track, `--audio-state`, per-sink mute, `create`-class sounds, and the **channel verbs**: `PlaySound*` answers a channel, `ChangeSoundVol` ramps it (clamped to 1.5, only for a positive fade, as `sithCogFunctionSound_ChangeVolume` does), `ChangeSoundPitch` sets the rate, and `SectorSound` replaces a sector's ambient with the script's own (`--sound`) | 3D positional and velocity mixing (`sithSoundMixer.c`) and sound classes with mode ranges. The channel is applied when a request *starts* — a one-shot mixer cannot re-tune a sink that is already playing, so `ChangeSoundVol` on a long ambient does not sweep it (`ponytail:`) |
| Surfaces and sectors | the flags, `SURFFLAG_SKY`/`SURFFLAG_CEILING_SKY`, adjoin state, `sithPhysics_FindFloor`, thrust values, and the script-side switches — `setsectorflags`/`setadjoinflags`/`clearadjoinflags`/`setsectoradjoins` now reach collision and the cull (§6). **Texture cels, material cels, scripted light and scroll reach the renderer**: `SetWallCel`/`SurfaceAnim`, `MaterialAnim`, `SetSectorLight`/`SurfaceLightAnim` and `SlideWall` are baked into the mesh (`Runtime::surface_epoch` -> `rebuild_surfaces` -> the vertex colour and UVs -> `level.wesl`). A scrolling surface also **carries the body standing on it** (§6), which is what a conveyor is; level 1's neon sign and its five 5 fps surfaces step, `09fuelstation`'s two `00_matanim.cog` calls walk `09sign.mat`, and level 2's seven force fields ramp their light | a sector's `thrust` vector now pushes bodies (`SetSectorThrust`, `--things`) |
| Sky | unlit sky faces plus a dome, and the camera's clear colour; `SlideHorizonSky`/`SlideCeilingSky` scroll the sky **faces** | the sky's own parallax and rotation (`sithRenderSky.c`), and the dome is a port stand-in that does not scroll |

| Models and materials | `.3do` parse, geosets, materials, `RADIUS`, `insertOffset`, mipmaps, the light ramp, `--winding` | `setthingmodel` (swap a model at runtime), per-surface material animation, the geometric-mode verbs (`*geomode`), and no back-face culling for models (§4 — model interiors render when the camera clips in) |
| Rendering | opaque pass, portal-aware sector culling, the light ramp, HUD, **placed and created sprites** (billboarded quads, one cel of the `.mat` strip, `billboard_sprites`), and projectile impacts (a bolt's `explode=` template is spawned by `projectile_hits` → `Runtime::explode`) | no transparency sorting, and a changed animation frame rebuilds the whole mesh asset instead of writing into it (§4) |
| Colormap and lighting | the palette, and each `.cmp` ramp sampled at **32 steps** (`LIGHT_STEPS`) to build the surface light ramp; a single far colour for distance shading; sector light animation, surface light animation, and **`ThingLight`/`SetThingLight`/`ThingLightAnim`**: a thing's own light, ramped the way `sithSurface_SetThingLight` ramps it (`from`, `rate = delta / time`, `to`), summed per sector so a lamp brightens its room and a broken one stops (`--cel`) | the engine's per-distance table **selection** — a surface picks the 256-entry table matching its distance from the light — and the *dynamic* pass that gives a thing's light a radius: this build adds the whole thing-light sum to its sector's baked light, so a lamp lights the room flatly rather than in a pool (`ponytail:` per-surface falloff). 256 tables are located per ramp (`(len - CMP_HEADER - CMP_PALETTE) / 256` in `tex.rs`) and the ramp uses 32 of them |
| Savegames | the container and thing records: version 6, the 1580-byte header, the `0x28` record header, template/type/position/angles round-trip — **and live state**: `Runtime::snapshot`/`restore` capture every thing (position, angles, sector, health, flags, `jkFlags`, destroyed, path frame), the inventory, weapon, armed bin, saber, the sector-adjoin word and each sector's light, into a `SAVE_SECTION` payload inside a real `.jks` (`--save` proves the round trip: health 37, +1.25 along x, inventory bin 2 = 3 and the clock all come back) | the **save-slot picker**: the pause menu's own `GUI_SAVE`/`GUI_LOAD` rows work (`jkGuiEsc_Show` case 15 and 13, checked by `--quicksave`), and the keys do work (`F5` writes `saves/quicksave.jks`, `F9` loads it, in place or by loading the level the save names first — `--quicksave`), and the automatic one does work (`AutosaveGame` writes a checkpoint at the story beats levels 7, 11, 15, 17a, 17b and 20 ask for), and the format plus the runtime round trip are checked. **Eight slots** now (`slot0.jks`..`slot7.jks`, slot 0 the historical `quicksave.jks`; `,`/`.` choose one and `--quicksave` proves two slots keep separate games); retail's slot *picker* screen is not in this tree. And the rest of `sithGamesave.c`: cog VM state (a loaded save restarts its scripts), AI state, item respawn masks, the event queue, multiplayer sections |
| Cog | the VM, 34 levels of scripts, pulses and timers, every verb the scripts call except none | no stubbed verbs in section 3 |
| HUD and UI | the HUD, most screens, menus built from `jkGUIRend` tables, and the status bar's carried strip (`hud::carried`, fed by `health_follows_damage` from the runtime's bins — it was blank before) | the screens still placeholders (`--ui-tables`), the scope, the map screen, and the tally screens (blocked on multiplayer) |
| Cutscenes | `.smk` decoding of all 25 shipped films (483 MB, 29,493 frames) with their own palette records, seeded from the level | in-engine camera cutscenes (section 1) |

---

## 3. The stubbed cog verbs — none left

Every verb the shipped scripts call now gets an answer: **0 stubbed, 0 errors** across the 34 levels
(`--cog-check`).

The check runs **every label each script declares**, not just `startup` and its declared `message:`
labels. That deep pass is the point: the startup-only count said "zero" for rounds while
`AutosaveGame`, the whole `jkString*` buffer (`jkStringClear`/`Concat*`/`Output`), `StopKey`,
`LoadSound` and `SyncScores` were silently returning 0 — verbs that sit behind cutscene and gameplay
labels nothing had fired. A number that measures what a pass reaches is only as good as the pass.

That was worth driving to zero rather than leaving as "harmless". An unhandled verb returns `0`, the
catch-all arm of `Runtime::handle` — the right answer for `ismulti` and `isserver` in a
single-player game, and the wrong one for a getter, where a script that branches on the zero takes
the wrong branch with no error anywhere.

Three ceilings came with the last of them, and they are marked in the code:

| verb | what is real | what is a stand-in |
| --- | --- | --- |
| `SetThingRotVel` | the angular velocity is integrated every step and the angles wrap | nothing |
| `jkStringOutput` / `jkPrintUNIString` | the line is composed (`jkString*`), resolved through `ui\jkstrings.uni` and **drawn**: `hud::draw_message` puts it across the top of the canvas for 4 s, in the live HUD and in `--hud` (`BK_HUD_MESSAGE` is the capture aid) | the engine draws it in its own message slot with per-type timing and a separate chat log; this build has one slot and one duration |
| `NewColorEffect` / `Modify` / `Free` | the handle, the three channels, the fade, and the create/modify/free round trip; **and the draw**: the effect is blended over the HUD canvas each frame (`ui::Canvas::apply_screen_effect`, unit-tested against `stdPalEffects_GatherEffects`' filter = quarter-channel and zero-tint rules) | **the world is tinted too**: `scene_wash` draws a full-screen quad on its own layer, under the HUD, so a force-seeing view washes the 3D scene (`scene_tint`, unit-tested). ponytail: a blended quad cannot *dim* a channel, so a filtered channel is drawn as absent rather than quartered — the HUD canvas keeps the exact engine rule |
| `SetThingModel` | the override is stored and a *created* thing is drawn with it | a thing the level **placed** keeps the mesh built at load |

---

## 4. Debt already marked in the code

14 `ponytail:` comments name a deliberate ceiling and its upgrade path. Line numbers are left out because they
rot; grep `ponytail:` in the file named.

| where | the shortcut | the ceiling |
| --- | --- | --- |
| `main.rs` | no back-face culling for model triangles | model interiors render when the camera clips into them |
| `main.rs` | a changed animation frame rebuilds the mesh asset | a 60 fps puppet rebuilds its mesh sixty times a second |
| `main.rs` | the cancel-row test is a unit test, not data | parsing `hoverId` would make it real data |
| `collide.rs` | the sweep's depth limit and "already searched" rule | a chain of more than 24 adjoins in a line reports no hit; no shipped level has one |
| `collide.rs` | the four-way clearance test rejects a body touching the riser it was blocked by | a step only succeeds if the lift clears the riser's polygon top; `STEP_UP = 0.066` compensates |
| `collide.rs` | the floor probe scans every wall face | O(walls) per moving body per frame |
| `runtime.rs` | insert offsets live in one process-global table | two worlds loaded at once (level plus cutscene) share it |
| `runtime.rs` | the bin → projectile mapping is a name heuristic | not the engine's `weaponData` |
| `runtime.rs` | steering picks the doorway that looks closest on both legs | no path search, so an actor can pick a longer route or lose the chase |
| `runtime.rs` | `sleep`/`settimer`/`setpulse` run straight on, with no timer queue | a script that only works across resumed sleeps runs its later statements early |
| `level_mat.rs` | the shader adds no light level | the mesh carries baked vertex colour only |
| `tex.rs` | one process-global light ramp | a second level in the same process needs the ramp passed through the mesh builder |
| `tex.rs` | a length check rejects non-bitmap headers | no header version dispatch |
| `ui.rs` | the map screen has no zoom/orbit transforms | see section 1, overlay map |

The `runtime.rs` sleep row is the one that bites: it changes script behaviour, not just speed.

---

## 5. Where to start

Ordered by risk per hour, using the call counts in section 3 rather than subsystem size.

1. **Fix the wrong-by-default getters.** ~~`getselfcog`, `getmastercog`~~ — **done**: the VM answers
   `getselfcog` with its own index in the level's cog list, and `setmastercog`/`getmastercog` are the
   world's one global master (`-1` unset), which is what `01_endlevel.cog` registers at startup
   (`SetMasterCOG(GetSelfCOG())`). `getgoalframe` answers the destination a piece is moving to and 0 once
   it has arrived — the engine zeroes `goalframe` on arrival, and that is what a reversing lift reads.
   Still open: `loadtemplate` (1). (Also done earlier: `getleveltime`, `gethealth`, `isthingvisible`,
   `get/setthingmass`, `setthinguserdata`, `setthingtimer`.)
   **Level-end chain: closed.** A level's `goal*.cog` reports a finished objective with
   `SendMessage(GetMasterCOG(), user0)`, and `SendMessage` now takes the engine's two arguments — a
   **cog index** and a message **id** (`sithCogFunction_SendMessage`) — carrying the calling cog as
   the sender so `GetSenderID()` answers in the receiver. The ids come from the retail binary's own
   keyword table in `sithCog_Startup` (`activate` 1, `activated` 1 as an alias, `startup` 3,
   `timer` 4, … `user0`…`user7` 27…34, `trigger` 40), so a `message` symbol evaluates to its id and a
   receiver matches its `message:` list by id rather than by declaration order. The queue is drained
   in play as well as in `--run`; before this, `Runtime::messages` (script→script, addressed by cog)
   was only pumped by `--run`, while the live loop drained `pending_messages` (thing/sector, addressed
   by instance). Level 1 delivers the goal message to exactly one cog instead of broadcasting it to
   41, and `[01_endlevel.cog user0]` runs — goal flag, `Accomplish1.wav`, `Sleep(3.0)`, `jkEndLevel(1)`.
   Across the 34 levels the id routing is visible on three: level 2 sends 5 → 7 script messages, and
   11 and 17 change how many doorway pieces and sounds their scripts drive. `--endlevel` is the gate
   that pins the whole thing end to end on level 1: master cog 26 registered by startup, `user0`
   (id 27) delivered to it, goal bin 100 = 3, and `level_complete` true after the handler's 3 s sleep
   — i.e. **a level can be finished by its own scripts**.
2. **Timer queue for `sleep`/`settimer`/`setpulse`** (section 4). It is the one ceiling that changes what
   scripts do, and it sits under every verb that waits.
3. ~~**Sector and adjoin flags**~~ — **done** (section 6): `setadjoinflags`/`clearadjoinflags`/
   `setsectorflags` now reach collision and the cull, `setsectoradjoins` closes a sector's doorways
   for real, and **`setsectorthrust`** makes the sector accelerate what stands in it — `sithPhysics.c`
   adds `sector->thrust * dt` to a body exactly like gravity, so level 21's core tube is a real
   updraft and level 10's power core pushes the player out (`--things`; `walk_player` folds it into
   the body's horizontal velocity and its vertical speed).
4. ~~**Damage types.**~~ — **done as far as the engine's rules go:** `Runtime::damage_typed` carries the
   `SithDamageType` word (impact 0x1, energy 0x2, fall 0x40, …), bolts arrive as energy and landings as
   falls, `SITH_AF_INVULNERABLE` (`actor->flags & 0x8`) is honoured with the engine's fall exemption
   (`sithActor_DamageActor`), **and a `damaged` handler's `ReturnEx` replaces the amount**: the message
   now carries its params (`GetParam(0)` = amount, `GetParam(1)` = kind), the label's answer comes back
   as its return value, and `Runtime::amend_damage` gives the difference back — which is what
   `force_absorb.cog` (turn part of a hit into mana) and the dark Jedi handlers need. What is still
   unported from the same function: the per-type hurt sounds and the 0.1 limb-hit scaling.
5. **AI verbs.** 240 calls across 11 verbs. **Mostly done:** `aisetmode`/`aiclearmode` are the actor-flag
   arms, `aisetmovespeed` stores a clamped 0..2 multiplier the AI step applies to the template's walking
   speed, `aisetmovepos`/`aisetmovething`/`aisetlookpos` are the scripted destination, follower and
   standing turn that `ai_tick` now walks and faces — which is what makes `00_AIPatrol.cog` walk its
   puppet, and `--ai` asserts the walk, the arrival and the turn. `setfirewait` (a weapon verb, kept here
   because it was the same shape) arms the player's cooldown, `fire_player` reads it, and
   `setgoalflags`/`cleargoalflags` OR and mask the goal counters (`SITHBIN_GOAL00` = 100) that
   `endlevel.cog` records objectives in, and `aisetmoveframe`/`aisetlookframe` walk an actor to one of
   the level's own `frame=` entries — which exposed a parser bug: an **actor's** frame is a position
   alone (`frame=(-4.476/15.877/3.840)`), and only the `pos:angles` spelling was accepted, so every
   actor's frame list was silently empty. `ai_tick` also had to stop keying on the instinct class:
   `SITH_CT_AI` comes from the actor block, and level 1's 8t88 has no `aiclass=` yet its goal script
   walks it to frame 1 (314 calls gone; 392 stubbed, down from 706). **The group is now closed**:
   `aisetfiretarget` (8) stores what an actor shoots at, and the fire-target is what made the
   out-of-sight early return bite — it had to be computed *before* that return, or an actor told to
   shoot a thing it can see stands still because it cannot see the player; `aiflee` (3) is the flee
   state (walk away from the named thing, do not shoot); `aisetclass` (2) swaps an actor's `.ai`
   wholesale from the name the cog binds, which is level 11's endlevel changing Pic's and Gorc's class
   between fight phases. `--ai` asserts all three: the actor turns to the named target and fires with
   the player behind it, a fleeing actor's gap grows 3.00 -> 3.30 over a second with no shots, and
   `AISetClass(actor, "pednarsh.ai")` moves its sightdist 5.0 -> 7.0. (`setmountwait` (25) is done: the
   engine's fire path refuses until `sithWeapon_secMountWait` passes, and every shipped caller passes 0.)
6. ~~**Puppet locomotion states.**~~ — **done** (section 6): the state is chosen from the velocity
   `ai_tick` walked the actor at, by `sithPuppet_UpdateThingMove`'s own rule, and the clip advances on
   distance moved instead of the clock. What is left of the state machine is the armed-mode axis and
   the reactions (`crouch`, `jump`, `fire`, `hit`, turns).
7. ~~**Attachments and parenting.**~~ — **done for the scripted case** (`--attach`): `AttachThingToThing`
   stores the child's offset in the parent's own frame and re-places it every step, so the twenty
   shipped calls that make a crate ride a lift, a catwalk ride a ship, or an effect ride the player
   all work. What is left of the group is the engine's *child list* (several children, chains) and
   the puppet-side version — a gun in a hand is a joint, not an attachment.
8. ~~**Camera focus and shake.**~~ — **done**: seven camera slots with `SetCameraFocus`/`SetCurrentCamera`
   and their getters, and `SetPOVShake` as an offset that decays at its own rate. Level 17b's cutscene is the
   end-to-end case: `17b_Camera.cog` startup cuts the view to `ghostCam`, and `Sleep(6.0)` later hands it
   back to the player (`--camera`, and `BK_CAMERA_TRACE=1` on a live run prints both). What is left of the
   group is `CycleCamera`, `SetCameraZoom` and the secondary-focus framing, which no shipped single-player
   script uses.
9. **Level 1's first frame is a wall of polygons, and it is the level's own start.** The walker spawns at
   the first thing of type PLAYER (thing 0, `(-10.44, -5.45, -1.13)`, sector 339) — the engine's own rule
   (`sithPlayer_Initialize` takes the first `SITH_THING_PLAYER` in list order), so this is not a wrong
   pick: the level's other 11 `walkplayer` rows carry `thingflags=0x20080501` (multiplayer starts) and
   thing 0 carries none. But a `00table` sits **0.16** units away, a `tuskensoused` patron 0.15, a
   `reyeesbhgun` 0.27, columns either side, and the eye is at `pos + PLAYER_EYE`, so the view is inside a
   cantina table with a body at arm's length. `BK_MODEL_SCALE=0.01` renders the room cleanly and the
   geometry is right, so the blocker is a thing model drawn at the eye, not the world. Unproven whether
   retail draws the same frame (nothing here says the start point is wrong); the way to settle it is a
   retail reference frame at level start, then either the port's draw offset follows or the start stays.

---

## 6. Recently closed, so it is not mistaken for missing

* **Sprites render, and the two "renderer bugs" that hid it were both in the probe.** `--sprite
  <template>` keeps one effect thing in front of the camera so a frame can show it. Two rounds of
  byte-identical frames turned out to be false alarms: the probe placed the thing with
  `(cos yaw, sin yaw)` while a JK yaw faces `(-sin yaw, cos yaw)` (§24), putting it exactly 90 degrees
  to the side — `BK_SPRITE_TRACE`'s camera-space print showed `Vec3(1.526, -0.037, 0.015)` against
  `Vec3(0.026, -0.037, -1.485)` once fixed — and the capture sat inside level 1's spawn prop (§5.9), so
  anything 1.5 units ahead was legitimately occluded. Walking into the open first and comparing frames
  with and without the effect shows it: `17EAC891…` with, `7881579F…` without. **`draw_created` was
  never broken, and projectiles have been drawing all along.** The last fault was the *bitmap*: the
  port read the material with a flat 140-byte header, which is only the first cel's texture header.
  `tex::parse_mat` now walks the whole table `rdMaterial_LoadEntry` (`0x44a260`) defines — 76-byte
  header, a 24-byte `rdTexinfo` per cel (+16 when `texture_type & 8`), then one 24-byte texture record
  **followed by that bitmap's mip chain** — and the cels turn out to be separate bitmaps, not a strip:
  `bryx.mat` is 8780 bytes = `76 + 8*40 + 8*(24 + 1024)`, i.e. eight **32 x 32** frames. World
  textures are untouched by the rewrite (one cel puts its bitmap at 140, exactly where the flat read
  had it). `docs/formats.md` §5 has the layout and the walk that lands on the length of all 1983
  shipped materials.

* **Effects are sprites, and sprites are loaded and drawn.** A template's `sprite=` (`+laserhit` is
  `bryx.spr`) is now parsed for every template a level names and its materials are loaded through the
  same `.mat` path as model textures — with palette entry **0 keyed transparent**, because an effect
  bitmap is painted on black and that key is the whole transparency of a spark (a world texture keeps
  entry 0 opaque, so the two conversions are separate functions, `keyed_rgba` and `to_image`).
  `draw_created` spawns one quad per `.spr` row at the thing's position plus the row's model-space
  offset, and `billboard_sprites` turns them to the camera each frame, which is what
  `rdSprite_Draw`'s camera-space vertices do. Level 1 resolves **35 sprite files for 35 templates**
  (13 distinct materials), and an impact logs `drawing sprite thing 363 (+laserhit, 1 face(s))`.

* **A sprite walks its frames over the thing's life.** `sithExplosion_CreateThing` arms the animation
  with `num_texinfo / msecLifeLeft * 1000` cels per second when the template's `typeflags` carry
  `SITHEXPLOSION_FLAG_ANIMATED_SPRITE` (0x1) — one cel per `life / cels` seconds — and
  `explosionParams.msecLifeLeft` is the life the thing was *created* with. `Runtime` now keeps that
  as `life_total` beside `life_left`, and `sprite_cel` scales it: `+laserhit` (`typeflags=0x33`,
  `timer=0.5`, `bryx.mat` eight cels) is eight frames over half a second. `draw_created` swaps the
  quad's mesh as the frame changes, and a live impact prints the walk with `BK_SPRITE_TRACE=1`:
  `sprite thing 366 (+laserhit) draws cel 3 of 8`. The clock and the placement of a moving sprite
  weapon (`+force_dest_p1`) ride along in the same branch, since sprite quads are outside the
  `Transform` query the model things use.

* **A created thing dies when its `timer=` runs out, and `SetThingTimer` is a different timer.**
  `timer=` is the engine's `msecLifeLeft` (`sithThing.c:1777`), counted down every frame with
  `sithThing_DestroyDyingThing` at zero (`sithThing.c:224`) — it is what makes an impact spark a
  spark (`+laserhit` ships `timer=0.5`) and what takes a corpse away (`SetLifeLeft(_, 30)` in 92
  shipped scripts). Nothing counted it down, so every impact added a thing to `rt.things` for the
  rest of the session while `draw_created` walked the growing range each frame. The other half of
  the bug was an alias: `SetThingTimer` was wired to `setlifeleft`, but the engine stores a
  **separate** absolute deadline in `pThing->timer` and sends the `timer` (4) message from
  `sithCog_UpdateThingTimer` — under the alias, `03_thinglight.cog`'s scripted timers would have
  destroyed their own lights. Both are now what the engine says, and `--hit` asserts a 0.5 s
  effect is alive at 0.33 s and gone at 1.33 s.

* **A moving actor plays its own walk, not an idle glide.** `sithPuppet_UpdateThingMove` (`0x4e4380`)
  transforms the thing's velocity into its own frame and picks the puppet state by the dominant axis:
  forward under 0.5 u/s is `walk`, at or above it `run`, negative is `walkback`, and a lateral
  component that dominates is `strafeleft`/`straferight`. The port had no velocity to feed it —
  `ai_tick` moves an actor by writing its position — so `Runtime::walk_vel` now records the velocity
  each actor walked at and `locomotion_state` applies the same rule to it. The clip also advances on
  distance moved (`sithPuppet_UpdatePuppet`'s `AdvanceTrack(speed × dt)`), so the legs keep pace with
  the body. Two wrong shapes were tried first and are recorded so they are not retried: a renderer-side
  displacement measured 3.6 u/s for a 0.33 u/s walk (the sim ticks and render frames do not line up),
  and the collision push that re-seats an actor inside a wall turned into a sprint sideways.
  `--animate` checks the chooser against each actor's own `.pup` and fails if fewer than half the
  level's actors ship a `walk` row (level 1: 128 ship `walk`, 128 `run`).

* **A face can be hidden, and an untouched face reports that it is drawn.** `SetFaceGeoMode(surface, 0)`
  is `GEOMETRY_NONE`: `00_breakingglass.cog` shatters glass with it and the force-field switches open
  passages with it. The port stores the mode, forces `0` on a face the level does not draw (retail's own
  branch in `FUN_00500790`), drops the polygons in `build_world`, and re-batches the `(sector, material)`
  meshes in place when the mode changes. The default is the half that changes behaviour:
  `GetFaceGeoMode` answers the `.jkl` row's own `geo` column, not `0`, and level 9's fuel chain is the
  consumer — `09_fuelexplode.cog:57` bails out of its `crossed` handler on a `0`. `--geomode` hides a
  7-vertex face (8898 → 8893 drawn triangles) and asserts an untouched face reads its `geo` 4 while a
  material-less face stays 0 even when asked for 4.

* **Level 1 ends when you see 8t88, from the level's own scripts.** `sithRender_RenderThing` sends
  `sighted` the first time it draws a `CAPTURED` thing, so the port latches that in `cull_sectors`
  (and the sector form, `SITH_SECTOR_SEEN`). `01_goal0.cog 204` — the goal script placed on 8t88 —
  answers with `SendMessage(GetMasterCOG(), user0)`, and `01_endlevel.cog`'s `user0` handler sets the
  goal flag, plays `Accomplish1.wav`, freezes Kyle as invulnerable, sleeps 3 s and calls
  `jkEndLevel(1)`. `--endlevel` drives exactly that chain and asserts the goal bin, the invulnerability
  window and `level_complete`. Along the way it proved that message names are engine keywords
  (`sithCog_Startup` registers `user0` = 27), so the VM seeds them as globals rather than binding them
  per script.

* **A level can be finished through its own scripts.** `01_endlevel.cog` registers itself as the master
  cog in `startup` (`SetMasterCOG(GetSelfCOG())`), a goal script reports the objective with
  `SendMessage(GetMasterCOG(), user0)`, and the master's `user0` handler sets the goal flag, plays
  `Accomplish1.wav`, sleeps 3 s and calls `jkEndLevel(1)`. Three things had to be true for that: the
  VM's own index and the world's master cog (`GetSelfCog`/`SetMasterCog`), a message bus addressed by
  cog with the **retail** message ids (`user0` = 27, from `sithCog_Startup`'s keyword table), and that
  bus being drained in the live loop rather than only by `--run`. `--endlevel` pins it: master cog 26,
  one message delivered, goal bin 100 = 3, `level_complete` true.

* **A doorway whose far side cannot hold the body is now a wall, and crossings land where the body
  can stand.** Two separate refusals kept level 1's start impassable: the crossing only advanced past
  the sphere's *contact* point, leaving the origin a radius short of the far room's copy of the
  doorway plane ("outside", so `sithIntersect_IsSphereInSector` undid it for 522 of 700 frames), and
  a closed door's slab is thinner than the body (`sector 337` is 0.05 deep against 0.065), so no
  origin inside it is legal and the body re-crossed it 713 times. `Solids::doorway_advance` now
  advances `contact + min(radius, far-room depth / 2)`, retries two longer reaches because the two
  rooms' copies of the plane differ by 3 mm, and returns `None` for a room shallower than the body —
  which `sweep_rooms` treats as a wall. Level 1's start went from 522 refused frames to **0**, and
  the body walks out of the alcove onto sector 349's floor. `--move` 302 → **459 units**, still 0
  falls and 0 stuck bodies; `--walk all` 34/34 contained.* **The status bar sits at the bottom of the screen, where the game puts it.** Both corner panels were
  blitted at `y = 0`, so the health and force gauges ran along the top edge. The engine anchors them to
  the bottom: `jkHud_leftBlitY = Video_format.height - HUD_SCALED(statusLeft height)` and
  `jkHud_rightBlitY = Video_format.height - HUD_SCALED(statusRight height)` (`jkHud.c:147`–`151`), with
  `x` already `0` and `width - HUD_SCALED(width)`. The piece layout inside a panel (`file.y_pos`) is
  relative to its panel, so the meters and the number fonts follow the same anchor now that
  `origin()` returns `(x, y)` instead of `x` alone.
* **The player's containment is now the engine's rule, and the walk no longer falls out of the world.**
  `walker_step` used to invent a sector for a stuck body (`sector_for_body`, "the roomiest hull that
  holds it") and applied the containment check to the body's centre with the horizontal component only.
  Both are gone: the check is `sithIntersect_IsSphereInSector(pos, 0, sector)` asked at the body's
  **origin** (its feet) with the sector's `COLLIDEBOX` short-circuit, and a failure restores the
  position, the velocity and the **sector**, exactly as `sithCollision_MoveThing` does
  (`decomp/004e7950_sithCollision_MoveThing.c:251`). The hull is the engine's surface set — `surfflags & 4`
  or an adjoin whose link has `ALLOW_MOVEMENT` — with each plane oriented outward from the cell that
  claims it. Verified: `--move` walks 34 levels from 8 headings each with 0 falls and 0 stuck bodies,
  `--walk all` reports 34/34 contained (the report now asks `inside_sector` itself rather than a
  hand-rolled planes-only test), and the in-app window walk runs 700 frames staying on level 1's floor.
  **Still open, with its signature:** 522 moves in those 700 frames are refused at the doorway out of
  level 1's start alcove — the body crosses into sector 349 at `y = -5.2149`, exactly one radius past
  that sector's wall plane at `y = -5.15`, whose *polygon* does not reach it. `IsSphereInSector` is an
  infinite-plane test, so the crossing is undone every frame. The port switches sector when the sphere
  *touches* the adjoin; crossing when the origin is past the doorway's plane is the fix to try.
* **A fifth of the collision geometry was being thrown away, which is why you could fall through
  the floor.** `collide::build` derived a polygon's normal from its first three vertices, and `.jkl`
  faces routinely start with three collinear points: on level 1 **233 of 4595 surfaces** have such a
  triple, `normalize(cross(...))` returned `None`, and the surface was dropped from the collision set
  entirely — no plane, no face, no floor. The level's own street slab (`surface 3510`, 8 vertices at
  `z = -1.325` in sector 361) and the walls either side of it were among them, so a body walked
  through the wall and fell through the hole behind it; the camera "spawned in a wall" because the
  walls it should have been kept out of were not there. `polygon_normal` takes the polygon's
  widest-spanning triple instead, and `--collide` now asserts the invariant: no solid, sector-claimed
  surface with a real plane may be missing from the collision set (it prints the 233 as the count it
  would have lost). The engine needs no such trick — the plane is authored per surface and the loader
  reads it from a second block in the `SURFACES` section (`sithSurface_ReadSurfacesListText`, JK.EXE
  `0x4e5c00`) — so this was the port's derivation, not the data. The parser now reads that block too:
  `--winding` compares our Newell normal against it on every surface of every level and reports
  **126,639 of 126,639 identical**. `docs/formats.md` §16 records it with the engine's addresses.
* **A closed passage is closed for the body, not just in a counter.** `setadjoinflags` (`x15`),
  `clearadjoinflags` (`x17`) and `setsectorflags` (`x1`) used to fall through the catch-all arm and return 0,
  so a force field, a grate and a glass pane did nothing — and `setsectoradjoins`, which every shipped door
  script calls on startup, closed a sector in a field nothing read. The two flag words now live on the
  runtime (`adjoin_flags`, `sector_flags`, both seeded from the level's own table), the verbs switch them,
  and `collide::Gates` is what the solver reads: `collide::sweep` treats an opening with `ALLOW_MOVEMENT`
  clear (or `ALLOW_AI_ONLY` set, or a sector whose adjoins are hidden) as a solid face and never crosses it,
  `visible_sectors` refuses an opening whose `VISIBLE` bit was cleared, and `in_water` reads
  `SITH_SECTOR_UNDERWATER` as well as the surface water flags. `--adjoins` is the gate: level 1's data ships
  5 movement-closed rows and one AI-only, 27 startup scripts shut 7 doorway sectors, the sweep crosses the
  chosen opening and stops when the verb clears MOVE, and clearing VISIBLE drops the rooms behind it
  from the cull (level 1: 96 → 1, because sector 0 has that one way out). Culled and `--no-cull`
  frames are still byte-identical at load and after 200 frames of sim time, so stage 1's pixel gate
  is untouched. Bits and engine consumers: `docs/formats.md` §3.2.
* **The camera was a quarter turn off, and the engine's own matrix says so.** `FUN_0043fcf0` @ `0x43fcf0` is
  the euler builder this port already copied as `thing_quat`; only its **column 1** rises with pitch, so a
  camera built from a thing's orientation looks along column 1 and a level's yaw `a` faces
  `(-sin a, cos a)`. The port drove the camera, the walk keys and the spawn aim along column 0,
  `(cos a, sin a)` — the same frame turned 90°: 03katarn's start stood the eye inside a doorway instead of
  looking down the street. One `yaw_axes()` now holds the mapping, and `--controls` asserts the camera's look
  and right against `thing_quat`'s columns on 15 yaw/pitch pairs. Details and the wrong oracle that nearly
  hid it: `docs/formats.md` §24.
* **Models are their real size now: `MODEL_SCALE` was 10, so every model was drawn ten times too big.** The
  reference that settles it is the level's own: a door has to fill the doorway built for it, and both are in
  the file. `d2x3.3do` is 0.200 × 0.300 model units and the vertical adjoin 0.01 units from the door's own
  placement measures 0.20 × 0.30 level units — 1:1, and the same for `d3x3_1` and `d4x3_1`. `--scale` prints
  the table. At 10 a 0.19-unit actor was drawn 1.91 units tall in levels whose storeys are 0.6 and whose
  doorways are 0.3, which is what "the enemy models are scaled way too big for the level" was. Everything on
  that constant followed it down — body radius 0.65 → 0.065, eye 0.37 → 0.037, step 0.66 → 0.066, the AI
  standoff 2.5 → 0.25, the actor walking speed `maxthrust * 10 / drag` → `maxthrust / drag`, and three
  hardcoded `+ 0.35` spawn heights — and every gate was re-run at the new size (`--walk all` 34/34, `--move`
  34 levels 0 frozen, `--rules`, `--live-check`, `--collide`, `--ai`, `--animate`). Two more faults fell out:
  `--walk all`'s "sector crossings" counter compared against a sector it had already overwritten, so it read
  1201 forever, and the sweep crossed nothing once the body was small enough to stay in its room — it now
  reads 6 crossings and runs 20 s a level.
* **Cancel works, with either hand.** Two separate faults made sub-screens hard or impossible to
  leave. `ui::pick` skipped any element with `id < 0`, and a retail cancel/done row carries
  `hoverId -1` (`nav::closes`) — so a click on Cancel hit nothing at all, on every screen that has
  one. And `Screen::rows` treated *any* `large0` element as a cursor row, titles included: Choose
  Player, Cutscenes, New Player, Edit Character and six more had a cursor that could only light their
  title, leaving Ok and Cancel unreachable from the keyboard. Rows are now "focusable" elements
  (`type 0`/`1`/`3`/`4`, never a `type 2` label or a `type 7` panel), with the large-font buttons
  preferred when a screen has them. `--ui-audit` measures it over all 41 screens: **60 exit rows, 0
  unreachable by click, 0 screens with no way out on the cursor**, and it fails on either fault —
  restoring the old `pick` reports 32 dead rows, the old `rows` rules report 10 trapped screens.
* **The game boots to its main menu and its rows start a game.** It used to boot straight into the
  level with no screen at all, and even with `--menu` the single-player screen's rows were all
  `Step::Nothing`, so "New Game" did nothing. Now `main` is the default boot screen (`--boot` checks the
  chain against the install), `GUI_NEWGAME` on 0x535e58 is
  `Step::NewGame` and reloads the campaign's first level through `swap_level`, the pause menu's
  `GUI_RESTART`/`GUI_ABORT` reload and leave the level, every screen freezes the world
  (`no_screen_open`) so the boot menu does not run a level's scripts behind itself, and Escape backs
  out one screen and only ever returns to a game that has started. Two things fell out of it: the
  cursor now reaches button-only screens (`Screen::rows` falls back to `type 0` elements, or New Game
  was mouse-only), and captures ignore live mouse/keyboard input — a click delivered during a headless
  run fired a bolt into the `--shot` frame and walked the menus on its own. That is one reason two
  captures of the same viewpoint used to differ; the level's scripts and its actor animation still run
  on the wall clock, so a capture is not yet frame-identical.
* `--overview` is a plan now, and it was not one. Three things were wrong at once: the camera stood
  `0.9 x span` above the roofs at the default 45-degree field of view, so every wall leaned out over the
  room beside it; the sector cull ran anyway, and its portal walk starts at the player's sector, which
  reaches nothing from up there (it hid 366 of level 1's 462 sectors and 807 of `04escapehouse`'s 976);
  and the HUD quad was sized from the *world* camera's FOV, so the long lens that fixes the first point
  shrank the whole status bar into a thumbnail. It is `6 x span` (capped at 500 to stay inside the
  radius-600 sky dome, which `09fuelstation`'s 102-unit span otherwise put the camera behind), every
  batch kept, and the quad has its own 45 degrees.
* The player's own body used to be spawned around the camera. `walkplayer` names `model3d=ky.3do`, and
  that is the thing the camera rides, so spawning it put the player's model *around the first-person
  eye*. Models have no back-face culling (§4), so the frame rendered the inside of the player's own
  model: unlit polygons smeared over the level, the walls and every NPC on screen — the "level geometry
  overlapping NPC/enemy models" report. `BK_MODEL_SCALE=0.01` was the A/B that isolated it (the same
  vantage then renders the room). The body now has one spawn path again — it is an actor like any other
  (round 199) and `draw_player_body` hides his parts in first person and shows them in third unless the
  boom has parked the camera inside him — and `--live-check` asserts no part of it is *visible* while
  the camera owns the eye.
* Thing orientation: the row's `pitch yaw roll` is applied through a verbatim `rdMatrix_Build34`
  (`thing_quat`). 138 of 352 things in level 1 are tilted, so 39% of props used to be drawn flat.
* `sithThing_CreateThing`: marker-relative positioning via the insert-offset difference, with the marker's
  orientation, plus `rotate_by_angles`. Test: `create_thing_offsets_by_the_insert_difference_in_the_marker_frame`.
* `sithThing_AttachThingToSurface`'s impact update: fall damage, gated by `--fall`. It caught a real bug —
  `fall_damage` squared `(v-2.5)` without the engine's `v > 2.5` test, so a 1.2 u/s landing was charged 76.
* Player physics derived rather than calibrated: `physics_scale = 1`, walk = `maxthrust/surfdrag` = 0.667,
  radius = `size x MODEL_SCALE` = 0.065, eye = `eyeoffset.z x MODEL_SCALE` = 0.037. `--rules` asserts the
  relationships and fails with a scale in the chain (§23 of `formats.md`).
* `SITH_SURFACE_CEILINGSKY` (0x400) joins the horizon bit in the sky test.
* The huge orange polygons are the level's freighter (`01frt.3do`, 110 units long), not broken geometry.
* The camera used to spawn inside an NPC: the clearance rule wanted `radius * 2` = 0.7, exactly an NPC's
  width. It wants 2.0 units now.

| Known defect | level m5 | **four weapon pickups rise out of the world**: m5railgun and m5concrifle carry vel= in their templates and the thing-motion path integrates velocity with no gravity and no surface stop, so they climb ~10 units/s from z −0.2 to +39 in four seconds (--soak-all reports it; BK_SOAK_TRACE names them) | the motion path needs sithPhysics' gravity and collision, or vel restricted to the types the engine launches (debris, particles, explosions) |

### Why gravity for goods is not a one-line fix (round 120)

Measured, twice:

1. **Gravity alone** (goods with move=physics, type 4/5) drops 231 of level 1's bodies out of the world. The x/y resolver cannot stop a fall.
2. **Gravity + a vertical stop** is still wrong: Solids::floor_below(sector, pos) returns None for every good on level 1 — the sector-keyed plane list finds no downward-facing plane under them — and Solids::support(pos, radius, drop), which is what the walker uses, catches only some (169 -> 68 bodies still leaving).

So the prerequisite is not 'add gravity' but 'give goods a ground test that works for their radius and their sectors'. support depends on face coverage (over_face) at the body's radius; items are 0.05-0.10 units, and the coverage test is calibrated for the player. Naming that is the deliverable; the m5 defect stays reported.


### Objectives text is not a string-table lookup (round 124)

jkGuiObjectives_Show (0x417ce0) builds GOAL_%05d keys and asks DAT_00553fa0, which FUN_0040a0f0 loads from misc\cogStrings.uni. On this install that file declares MSGS 7 and parses as six strings, and no GOAL key exists in any of the five string files the archives hold. So the panel's text is built somewhere else, and assuming a table lookup costs a round. --uni-key (exact + near matches) and BK_UNI_TRACE=1 (per-file string counts and header) are the tools that showed it.


### Level 9's sector triggers (round 125)

--soak --level 09fuelstation routes to watched sector 250 in 8 waypoints, crosses 6 sectors, and delivers 0 messages, with 54 scripts able to answer entered and 171 links (none pointing outside the level). Level 1 crosses 28 sectors and delivers 29. The open question is narrow: did the walker arrive at sector 250, and if so why did sector_messages return nothing for it? The soak now prints handlers, crossings, bad links and the target on every run.


**Resolved (round 126):** level 9's links are sound. The walk ended six sectors short (256 vs target 250); placing the body in the watched sector fires the link. --soak-all's prose scraping had been reading zeros — it now parses a soak-summary: line, and m5's four rising pickups remain the only level defect.

### The live schedule, every level (round 128)

--live-all runs --live-check over all 35 levels, one process each: **35 of 35 pass**, with static skipped by name (a test map with no walkplayer). This is the only check that exercises parallel Bevy systems, and it is what caught the borrow guard that passed fifty gates and crashed every level.


### The walker never ran (round 130)

`walk_player` queried `(With<Camera3d>, Without<OverlayQuad>)` and called `single_mut()`. The overlay screens spawn their own `Camera3d`s (HUD order 2, tint order 1) and the `OverlayQuad` marker is on the quad meshes, so the query matched three entities and the walker returned every frame: the body could not move and `BK_WALK_TRACE` printed nothing. Fixed with a `WorldCamera` marker on the camera the player looks through, queried by both the walker and `setup`s "is there a world camera" guard.`n`n**Uncovered by the fix:** with the walker running, the live schedule panics on frame 1 with `World::take` reporting a same-thread double borrow (`src/runtime.rs:38`). Unreachable while the walker was frozen. Next: find the second borrow with `RUST_BACKTRACE=1`.


### Three levels cannot move: the body spawns below its floor (round 132)

`--live-play --start <level>` passes on 01, 14 and 19a, and reports zero movement on 02narshadda, 04escapehouse and 09fuelstation. The trace names the cause: the key is pressed, `forward` is 1, velocity integrates to 0.037, and the body stays still because its feet are at 2.905 while its own sector floor is 3.0 — it spawns **below the floor**, so every horizontal sweep is blocked in all four directions. Fix: a settle pass on spawn (lift to `floor + radius`) before the first sweep. Also fixed this round: the driver clock now starts when the player exists, and the walk phase sweeps W/D/S/A rather than W alone.


**Round 133:** the settle pass is in (pos.z = floor + radius + 0.01 at the top of the walk, matching sithThing_AttachThingToSurface), and 02narshadda walks. The + 0.01 matters: containment is asked at the feet with zero tolerance, so a body resting exactly on its floor plane rounds outside. 04escapehouse and 09fuelstation still report move_body … blocked false with achieved 0.00000 — inside_sector(sector, feet, 0.0) rejects their spawn points, so every step is undone. Next step: the hull plane tolerance for those sectors.

**Round 134:** the sector is now chosen from geometry (sector_for_body(feet)) *before* settling onto that room's floor; five of six sampled levels walk (01, 02, 04, 14, 19a) and 09fuelstation moves but its player trigger adds nothing while the level's actors fire. The check now measures **horizontal** distance (a body rising onto its floor was passing as walking) and the shot count uses the **peak** live-thing count during the 30 fire frames (a bolt destroyed inside the window read as zero). Both live gates are skipped in a headless sweep, like --live-check.

**Round 134 close:** --live-play is not in the sweep. It passes standalone but intermittently exits without a verdict when launched as one of fifty windowed children, and a flaky gate is worse than an honest manual one. The flag remains the tool for driving real input; the entry was removed and the reason recorded.

**Round 135:** five of six sampled levels walk (01, 02, 04, 14, 19a); 09fuelstation walks but its player trigger produces nothing. The shot metric is the hard part: sampling live things is flaky at 10 fps (a bolt dies between frames, and the level's actors fire too), so Runtime::shots_fired was added in fire_with_spread — and it reads **zero** on every level while the fire: trace shows a bolt, so the player's trigger reaches the runtime by another door. Next: move the increment to where FireProjectile lands.

**Round 136:** two faithful fixes landed and neither makes the player's shot spawn yet. (1) ActivateWeapon(player, rate, mode) takes **three** arguments — the cog is the *caller*, which the port read as idx(1), so every weapon armed cog 0 and its fire: message went to an unrelated script. It now takes Runtime::caller_cog; weapon_cog went Some(0) -> Some(43) = weap_bryar.cog. (2) FireProjectile's named projectile was refused unless template_damage held it, but a bolt inherits damage= from _weapon; it now also accepts a template the level carries. BK_TRIGGER_TRACE shows the fire message reaching the right instance, so the remaining refusal is inside the fire: label itself.

**Round 137:** the player's trigger fires: 6-46 shots on each of six sampled levels, through real key presses. Three fixes made it real -- ActivateWeapon(player, rate, mode) arms the *caller*; delivered messages now set vm.source as well as vm.sender (without it cog_weap_bryar.cog's fire: returned at its first line); FireProjectile accepts a projectile template the level carries. The shot counter itself had the bug I could not see for a round: my shots_fired += 1 was a silent no-op Replace, so the check reported zero while the fire: label was already running. --live-play is back in the sweep (51 gates). Open: 19a fires but walks 0.00.

**Round 138:** the containment veto no longer freezes a body whose room does not claim it — a step into another room *adopts* it (sector_at), and a step into no room at all is not judged at all (the engine''s guarantee presumes thing->sector holds the body; 19a spawns 0.725 below its sector''s only floor plane). The dead undo branch is deleted. **31 of 35 levels now walk and fire** under real key presses; the five failures are 11gorc, m10, m_boss15, m_boss17 (bot maps, may start with nothing to fire) and static (test map).

**Round 139:** **all 35 maps pass --live-play** — the body walks and the trigger acts everywhere. The last five were the *check*, not the game: 11gorc is the saber level (its fire: swings and calls SetFireWait, no projectile), and m10/m_boss15/m_boss17/static start the player with an empty clip (weapon_cog None … out of ammo (bin 11 is empty)). A trigger event now counts a saber swing, and an empty clip is reported by name instead of failing; a genuine dead trigger still fails because that branch needs no weapon *and* an empty clip *and* zero shots.

**Round 140:** \--live-play\ now drives a **jump** (a real Space press) after walking and firing, and requires the body to leave the ground: 0.26 units up and back to 0.00 on 01narshadda, 09fuelstation, 11gorc and 16aescapeship. Five maps refuse — 06abarons, 10cargo, 17bsarris, c1, static — with a peak of 0.00-0.01 and the body grounded, so \walk_player\'s \just_pressed(Space) && grounded\ never fires there; mashing Space on alternate frames fixed 16aescapeship only, so it is not a check artefact. Next: why those bodies cannot jump.

**Round 140 close:** the jump phase now waits (bounded at 90 frames) for \player.grounded\, reports when the body never settles, and always reaches a verdict; level 1 lifts 0.30 units and lands. Per-map jump coverage is uneven — 10cargo and 01narshadda vary run to run under load — so the swept gate stays level 1. The maps that refuse to jump or never touch a floor are the next target: 06abarons, 10cargo, c1, 09fuelstation.

**Round 141:** player.grounded is now derived from the **landing event** (walker_step returns whether the vertical sweep was stopped by a surface) instead of vy == 0.0; a floor that pushes the body never gives zero vertical speed, so such a body could never jump. The first attempt was neutralised by a shadowed binding in the step loop (let (len, cross, landed) hid the outer flag) — renaming it to step_landed was the actual fix. 01narshadda jumps 0.30 and lands. Still refusing with the body grounded: 09fuelstation, 06abarons, 10cargo, c1 (0.00 peak), and 17bsarris/16aescapeship no longer print a verdict at all.

### The jump check is load-sensitive, and that is measured now (round 142)

Attempts this round to make the jump phase reliable all failed and were reverted; the tree is at `9861f34` with the gates green. What was learned, with numbers:

- The walker **does** jump when Space is pressed: `jump trace: jumped, vy now 1.50`, then `walk end z 0.141 -> 0.227 -> 0.273 -> 0.279` and back down. The physics and the input path are fine.
- The measurement was not: on 09fuelstation the body rises 0.28 units and the driver recorded a **0.00 peak**, because the wait-for-`grounded` phase consumed its own window and the jump happened outside it.
- Mashing Space fixed that map and broke others; measuring `max_moved` horizontally fixed the "moves and snaps back" false positives and broke others. The common cause is that frame time ranges 0.008-0.1 depending on load, so 30 frames is anywhere from 0.25 s to 3 s of simulated time.
- `BK_FIXED_DT` (a pinned walk step) was prototyped and made results *worse*: with a fixed step the body reaches 0.49 units and never lands. That is a real signal about the landing/thrust interaction, not just about the check.
**Next:** pin the timestep first, then fix the "rises and never lands" behaviour it exposed, then assert the jump across maps.

**Round 143:** BK_FIXED_DT pins the walker's timestep, and it exposed a real defect the variable frame time had been hiding: with dt pinned at 1/30 s, level 1 walks only 0.03 units and the jump rises 0.30 and then **stays 0.49 above the floor** — the body never comes back down. On the wall clock the same run reads anywhere from 0.00 to 0.56, which is why the check kept changing under me. --live-play is now a manual tool, not a swept gate: its verdict depends on how much frame time the machine gives the map, and it has failed the sweep intermittently for two rounds. **Next:** fix the never-landing jump (the vertical sweep vs floor_under's travelled filter is the suspect) with BK_FIXED_DT=0.0333 as the reproduction.

**Round 144:** the never-landing jump was my own measurement. Mashing Space re-jumped the body the instant it landed, so every reading caught it mid-air. With **one** press (after a bounded wait for the floor, ending when the body is back on the ground) and a pinned step, **27 of 35 maps walk and jump 0.25-0.27 units and land**. Two measurement bugs fell out: max_moved counted vertical movement (a jump read as 'moves and snaps back'), and the walk assertion compared start with end although the phase sweeps W/D/S/A (a 0.27-unit walk read as 'did not move'). Both fixed. Remaining: 16a/16b/17b/c1/static jump 0.00, and 04escapehouse/11gorc/c2 report no projectile under the pinned step.

**Round 145:** the jump phase now runs **before** the walk, because a walk can end off a ledge — 16aescapeship fell for the rest of the run and never attempted the jump. With jump-first, **29 of 35 maps pass** (up from 27): 16a/16b lift 0.25 units and jump at all for the first time, c1 passes. Remaining: 16a/16b/17b walk 0.03 units after landing on a **45-degree slope** (walk trace: normal [0, -0.707, 0.707], vel 1e-5 — no thrust at all on a ramp), 17b jumps only 0.13, static 0.00, and 04escapehouse/11gorc report no projectile under the pinned step.

**Round 146:** two more of the five failures were in the port, not the check. (1) The walk phase gave each direction only 15 frames — half a second of acceleration at a pinned step, so the body reached 0.02 units/s and never left the mark ('the ramp bug': 16a/16b now walk and pass). (2) The **per-frame floor clamp** teleported a jumping body straight back down: sithThing_AttachThingToSurface is a placement step, but the port ran it every frame, so 17bsarris oscillated z −0.083 → −0.106 → −0.083 and could not jump at all. The clamp now skips a rising body. **34 of 35 maps** walk, fire and jump under a pinned step; the only failure is static, the test map, which has no walkplayer to jump.

**Round 147:** --live-play now counts **sector crossings and the trigger messages the level's scripts actually ran**, and fails when a sector the body entered held a link whose script has an entered: label and nothing answered — the mechanism behind every door, lift and cutscene. Level 1 answers 9-10 spawn-sector links; 17bsarris crosses three sectors and answers none (no capable links, reported not failed); 20aboc's link has no entered: label. **21ajerec and 21bjerec fail**: one capable link into the sector the body stands in, no message delivered — a lead, not yet a diagnosis. The jump assertion no longer fires when the phase never started (static spawns falling through the world). **33 of 35 pass**; the two 21 maps are the trigger lead.

**Round 148:** **all 35 maps pass** walk, fire, jump and sector-entry delivery. The two 21 maps that failed the new trigger assertion were the check's fault: it measured a **delta** from the first frame it saw the body, and the spawn crossing is delivered on the first frame the body *exists*, so it subtracted the very message it was looking for (trigger: 1 message(s) entering sector 5 (from None) printed right before FAIL ... no script answered). It reads the run-wide count now. Two things kept from the chase: sector_enter no longer counts a label that **ran and errored** (it prints the error instead), and the assertion counts only links whose script has an entered: label in sectors actually entered.

**Round 149:** the renderer's \sighted\ messages now reach the scripts. \thing_sighted\/\sector_sighted\ queued them into \pending_messages\ and **nothing on the live path drained that queue**, so every sighting was dropped — including the message level 1's first objective waits for (\01_goal0.cog\ on 8t88: 'Find 8t88 before he escapes Nar Shaddaa' starts when he is *seen*). \deliver_cog_messages\ drains the queue now, proved by \sighted: 01_TuskenTwitch.cog ran sighted\, and the live verdict prints the goal bins (\[(0, 1)]\ = GOAL00 listed). 35 of 35 maps still pass. Next: walking the player to 8t88 to watch goal 0 go from 1 to 2 — the objective completing in play.

**Round 150:** the mission objective completes **in play**. \--goal --start 01narshadda\ stands the body eight units from thing 204 (8t88) in its own sector, facing it, and the renderer does the rest: \goal: GOAL00 flags 0b11 after 120 frames --- goal check passed ---\. \0b11\ is what \01_goal0.cog\ writes — bit 1 listed, bit 2 complete — so 'Find 8t88 before he escapes Nar Shaddaa' goes from the objectives panel to done through the real chain (renderer sights the thing -> \sighted\ -> cog -> \SetGoalFlags\). It only works because \deliver_cog_messages\ drains \pending_messages\ now. The check is in the sweep: **51 gates, 0 failed**.

**Round 151:** finishing level 1 leads to level 2. `Episode::row_for` compared the level's archive path (`jkl\01narshadda.jkl`) with the graph's bare file name (`01narshadda.jkl`), so `flow_after` never matched and **every** level looked like the last one (`next: (end of campaign)`). It compares file names now: `level jkl\01narshadda.jkl complete -> cutscene 06a.smk, then 02narshadda.jkl`, and `--goal` waits for `jkEndLevel` and reports the successor (`goal: level complete after 385 frames, next 02narshadda.jkl`). The 992-frame film is not waited out in the check -- that costs a minute of wall clock and is covered by `--transition`/`--campaign`. 51 gates, 0 failed.

**Round 152:** the handover after level 1 does not happen: `--goal` with `BK_GOAL_SWAP=1` shows `cutscene: playing 06a.smk, 992 frames` and then `film: frame 0 of 992, finished false` for 2400 frames, so `queue_next_level` never sees `finished` and level 2 never loads. The shim decodes when run by hand (`smk-test000.raw` files written, per-frame hashes printed), but the game's own run leaves no `bk-cutsceneNNN.raw`, and the frames left on disk from older runs are `.ppm` while `frame_path` asks for `.raw` -- the two paths drifted apart. Next: the game's spawn of the decoder.

**Round 153:** the film handover was attacked from both ends and is still broken, but the evidence is sharper. During a run the decoder **does** write frames, and they are the film's *tail* (`bk-cutscene751.raw` .. `991`), while `play_cutscene` reports `frame 0 of 992` and waits for `bk-cutscene001.raw`. My manual shim run (`smkdec <dll> <film> <prefix> 992 0`) writes from frame 0, so the game's spawn differs from it in a way that matters; passing explicit flags (`0x000fe000`) or an explicit `extra`/`start` made it write **nothing at all**, so the original five arguments were reverted. Next: instrument the shim's own write loop (`smkdec` prints only hashes today) and diff its argv against the manual run, rather than guessing at flags again.

**Round 154:** the decoder side is healthy inside the game -- the child reports `decoded 992 frame(s) at 640x300, 970 of them distinct` -- and the parent still logs `bk-cutscene001.raw exists false`, so the two processes disagree about **where** the frames are. The helper now names each written path (`SMKDEC_TRACE=1`, verified by hand: `wrote C:\Users\Lee\AppData\Local\Temp\tr000.raw (192000 bytes)`); the game's run did not show that line even with the variable set and stdout inherited, which is the next question -- either the game's child is a different binary than the one on disk (only one smkdec.exe exists, freshly built) or its stderr is being dropped. Next: print the child's path and the parent's expected path in the same run.

**Round 155 (the turn):** **the campaign hands over.** Level 1 -> goal -> `jkEndLevel` -> film -> level 2, with the player carrying 7 inventory bins and `+bryarbolt` across:

    level: 02narshadda.jkl loaded (391 things, 641 sectors), rebuilding
    level: the player carries over 7 inventory bin(s), weapon Some("+bryarbolt"), saber false

It always did. The film plays in real time (992 frames at 15.2 fps = 65 s) and every check counted **frames**; 2400 frames is 20 s on a fast machine, so the film was still playing when the check gave up. Three rounds of "the film never hands over" were a wrong clock. The swap wait is 20000 frames now with that reason written beside it, and `BK_FILM_TRACE`/`SMKDEC_TRACE` stay as the instruments that found it.

**Round 156:** the objectives text question is settled from the executable. `FUN_00417ce0` (the objectives page) builds `GOAL_%05d` for each goal row and looks it up in the string table at `0x553fa0`, and the xref list of that table includes `0x40a0f0` -- the loader of `misc\cogStrings.uni`. So the text must come from `cogStrings.uni`, and the shipped file declares `MSGS 7` with six strings and **no `GOAL_*` key at all** (`--uni-key GOAL`: "0 of 5 string file(s) carry GOAL"). Retail on this installation shows blank objective rows too; the port is not missing a loader, the data has no objective text. Recorded so nobody invents strings later.

**Round 157:** save and load work through the **keys**. `--save-live` presses F5, disturbs the world (clock +100 s, bin 2 to 99), presses F9, and reads it back: `quicksave: wrote 26228 bytes` -> `loaded in place (355 things)` -> `clock 0.32 (saved 0.29), bin 2 = 1`. The probe must run **before** `quick_save` in the same frame, because `just_pressed` is true only on the frame of the press -- ordered after it, the keys it pressed were never seen. Gate 52, 0 failed.

**Round 158:** the **use key** works against a linked door through real input. `--use-live` stands the body two units from the first thing a script linked and presses E: `use: activated thing 203`, `42 script message(s) answered the key (was 0)`. The witness is `TriggerCount` -- `use_thing` queues `activate` into `pending_messages` and the delivery pass drains it. Probe ordering matters for the third time: it must run **before** `use_key`, because `just_pressed` lives for one frame. 53 gates, 0 failed.

**Round 159:** an **actor in a real level shoots at the living body**. `--ai-live` finds the first placed thing whose template names an `aiclass`, stands the body three units in front inside `sightdist`, and waits: `ai-live: 1 shot(s) at the body, its health 100` -- combat is live, and that shot missed. The first version of the probe panicked on a triple-nested `sim.world.borrow()`; one borrow and one pass is all the lookup needs. 54 gates, 0 failed.

**Round 160 (negative, recorded):** force powers do not work in play, and the reason is precise. `load_item_cogs` looks a pending cog up in the **level's** script table, and the force scripts are never placed in a JKL -- `items.dat` names `force_jump.cog` for bin 21, the level does not carry it, so the loader skipped it and the power silently did nothing ("named by `items.dat` but was not loaded from the level"). Loading it from the archive is a small fix and was prototyped: with it, `items: force_jump.cog loaded from the archives`, the cog starts (cog 44, startup ran), `activated` spends mana (100 -> 89). But the body does not rise: `SetThingVel(player, (0,0,duration+2.5))` is not reaching `player.vy`, even though `walk_player` reads and clears `rt.things[player].vel` for exactly that purpose. Next: find what clears the thing's velocity between the script and the walker. The working tree is at the last green state (54 gates); the probe was reverted because a careless line edit left an unbalanced brace.

**Round 161:** force powers load their cog from the archive now (`items.dat` names `force_jump.cog` for bin 21, the level never places it, so the loader skipped every force power). The cog starts, `activated` spends mana (100 -> 89), and `deactivated` sets the body's velocity -- but the body does not rise, because the generic mover loop spends the impulse on the player's *thing* before `walk_player` can read it. Excluding the player from that loop fixes it (measured 0.38 units of rise) and breaks `--things`, whose thrust and path-move assertions use the player's thing and expect the runtime to integrate it. **Next:** move those two assertions to a non-player thing, then land the exclusion. 54 gates, 0 failed.

**Round 162:** **force jump works.** Two changes: (1) `Runtime`'s generic mover loop no longer takes the **player** — it was spending `force_jump.cog`'s `SetThingVel` on the player's thing (moving it, hitting the level, zeroing the velocity) before `walk_player`, whose whole job is to turn that impulse into `player.vy`, could read it; path moves still run for the player. (2) `--things`' thrust and path-move assertions now select a **non-player** thing, which is what they were always about, so the exclusion does not break them. Measured: `force-live: the body rose +0.54 (took off at -1.18, peak -0.64), mana 91 of 100` -- from "silently does nothing" to a real lift. 55 gates, 0 failed.

**Round 163 (correction):** last round's claim that `items.dat` gives the force bins no `cog=` is **wrong**. Dumping the file shows the opposite:

    f_jump                  21              0       4.0     0x108   cog=force_jump.cog
    f_speed                 22              0       4.0     0x108   cog=force_speed.cog

The `-` I read was `--bins`' *ammo* column, not the cog. The parser (`hud::items`) handles these rows correctly. So the powers are named, and the port still does not start them: the runtime's `item_cog` map comes up with **24 entries and no bin 21** (`bin 21 -> None`), which is why `set_bin(21)` queues nothing. That is the thread to pull next -- why `world.item_cogs` drops the force rows when it is built from the same parse.

**Round 164:** found it, and it was one line. `hud::items` parsed the `max` column into an `i32`, so `4.0` failed to parse and the whole row was `continue`d -- the *weapon* rows have integer maxima and survived, which is why 24 bins looked right. `max` is now parsed as `f32` and rounded, and the eight force rows come back: `--items` reads **40 of the 82 bins**, `21 f_jump` .. `28` among them, each naming its `cog=`. `cargo test` pins both directions (`the_force_rows_survive_a_decimal_maximum`: `4.0` -> 4, `4.5` -> 5, and a whole-number row still parses). With the map populated, `set_bin(21)` queues `force_jump.cog` and `load_item_cogs` starts it from the `.gob`, which is the path `--force-live` already proves end to end. 55 gates, 0 failed; 111 tests.

Also repaired: this file had 27 mangled control bytes (a backtick-escape accident from an earlier edit) in the round log earlier in this file, which made every text tool treat it as binary. Replaced byte-for-byte with the letters they stood for; content unchanged.

**Round 165:** with the bins readable, the *economy* they feed is the next wall, so the engine's own answer is worth writing down while it is in front of us (`decomp/004064e0`, `00405cf0`, `00406860`, `00407040`, `00407210`, `00416480`, `004160f0`). The player's bins are a 0x18-byte record array; the force range is `0x14`..`0x22`:

    0x10 / 0x11  force points: newly awarded / unspent pool (Show merges 0x10 into 0x11 on entry)
    0x12         the side the player declared: 1 = light, 2 = dark
    0x14         jedi rank (GetJediRank is exactly this bin)
    0x19..0x1c   the four light-side powers
    0x1d         protection (light mastery)
    0x1e..0x21   the four dark-side powers
    0x22         deadly sight (dark mastery)
    0x48 / 0x49  light / dark side points, summed at the tally
    0x4b         the alignment the game stores: -100..100, clamped

`SetAccessiblePowers(rank)` clears the available flag (bit 8) on every force bin `0x14`..`0x22` and, for any rank above zero, sets it on the four basics `0x15`..`0x18` (the first three are `items.dat`'s jump/speed/seeing; the loop's `0x241` bound is inclusive, so the fourth is in). Rank 4 unlocks `0x19`+`0x1e`, rank 5 `0x1a`+`0x1f`, rank 6 `0x1b`+`0x20`, rank 7 `0x1c`+`0x21` — the light power and its dark twin unlock together, one tier per rank. `CalcAlignment` sums `-(dark level * w) - (light level * w)` over those two groups, adds the side-point ratio when called from the tally, clamps to +/-100 and stores it in `0x4b`; a declared side short-circuits it to +/-100. `SetProtectionDeadlysight` runs at rank 8 only: if every *light* power is above zero and the basics and top tier are too, protection becomes available at level 4 (`0x40800000`) and deadly sight is cleared, and the mirror case does the reverse. `DisallowOtherSide` (rank above 6) hides the side the player is not on, or both sides when the sum is near zero.

None of that is in the port, and none of it *can* be reached yet: the whole economy hangs off `jkGuiForce_Show` (the ESC menu's Force Powers screen, `jkGuiEsc_Show` -> `jkGuiForce_Show(0,0,0,0,0,0)`), which is where points are spent and the bins written; its sibling `jkGuiSingleTally_Show` is where a level hands points out. `--force-live` proves the *runtime* half (a held bin runs its `cog=`), so the missing piece is the two screens and the four small functions above — next.

**Round 166:** the four functions are ported, and the round above had one rule backwards: `SetProtectionDeadlysight`'s `bVar2` requires the four *basics* to be **empty**, not spent — the mastery is for a Jedi who put every point into one side and none into jump/speed/seeing/the fourth. `src/force.rs` now holds the bin map, `accessible_powers` (the ladder), `alignment`, `protection_deadlysight` and `allowed_sides`, with the five engine constants pinned from `JK.EXE` at `0x522054` (`0.0`, `100.0`, `-100.0`, `6.25`, `-6.25`, `-20.0`, `4.0`). `--force-economy` prints the model and checks it in the built binary:

    the rank ladder holds [0, 4, 4, 4, 6, 8, 10, 12, 12] powers for ranks 0..8
    alignment one light level 25.0, one dark -25.0, sixteen dark -100.0
    a declared side reads 100.0, an empty tally 20.0
    rank 8 with the light side maxed grants 1 (protection Some(4.0))
    past rank 6 a dark Jedi sees (light false, dark true)

Five unit tests pin the same rules from the other side. And the `items.dat` fix from last round now shows end to end: `--force-all` acquires bins 21..28 through `Runtime::set_bin` and reports **8 of 8 power scripts started** (`force_jump.cog` .. `force_absorb.cog`), where the same probe started none before the parse. 116 tests, 55 gates, 0 failed. The screens are what is left.

**Round 167:** the screen is closer than expected, and the point *supply* is now recovered too. `src/menus.rs` already carries the retail button table (`FORCE_POWERS`, the 17 icons with their bins, from `jkGUIForce.c`'s own `jkGuiForce_buttons[25]`), `force_screen()` builds the grid, and `src/nav.rs` has the edge the pause menu takes (`0x529560` id `12` -> `0x52f168`, `jkGuiForce_Show`). Two things are missing: the click handler and the points.

The handler is not in the decomp. `jkGuiForce_Show` sets `_DAT_0052f1d8 = &DAT_008ec320`, and that address is zero on disk (BSS, filled at run time), so the button callback named in the table (`jkGuiForce_ButtonClick`) was never lifted as a function — it is reached only through the element table's function pointer. That is the one piece this dump cannot give; the port's own click rule will have to stand in (raise the clicked power by one, spend one point, drop it when the ladder, the pool or the side lockout says no) with a `ponytail:` note naming the gap.

The supply is `jkPlayer_FixStars` (`0x406d50`), and it is exact: a Jedi's **total budget is three points per rank** (`rank * 3`). Spent is the unspent pool (`0x11`) plus every power level from `0x15`..`0x22` — except the rank itself (`0x14`), protection (`0x1d`) and deadly sight (`0x22`), which the loop skips. Below budget the pool is topped up to the difference; above it, points are refunded one at a time in a fixed order — the pool first, then `0x21, 0x1c, 0x20, 0x1b, 0x1f, 0x1a, 0x1e, 0x19, 0x18, 0x17, 0x16, 0x15`, i.e. the highest tier of each side down to the basics, dark before light in every pair. That is the next function to port, and it is what makes the rank-up award more than bookkeeping.

**Round 168:** `fix_stars` is ported (`force::fix_stars`, exact, three tests: the three-a-rank budget, pool-before-powers refund, and the mastery bins staying outside the count), and the *single-player* power supply turned out not to be points at all. `jkMain_EndLevel` (`0x402eb0`) grants powers per level: the episode row carries a light bin at `+0x30` and a dark bin at `+0x34` — `lightpow`/`darkpow`, which `src/episode.rs` already parses and only printed — and for each bin strictly between `0x13` and `0x23` it calls `sithInventory_SetCarries`, which sets **the same `available` bit `SetAccessiblePowers` sets**, skipping the light grant for a player whose `jkPlayer_GetChoice()` is dark and the dark grant for a light one.

Level 3's row is `22 0` and its own comment says `[Speed]`, so a light-side Jedi has force speed from level 3 on — that is the campaign's real power curve, and it is recovered. `force::episode_grants(lightpow, darkpow, choice)` is the ported rule with a test that uses level 3's own row. What is still missing is the caller: `adopt_carried` (`src/main.rs`) hands the player's bins to the freshly loaded level and is the natural place, but it needs the *incoming* episode row, which means threading the sequence through `swap_level`/`Carried`. That, then `item_available` on the runtime, then the screen's click rule.

Still unknown, and worth saying plainly: **no single-player code path in this dump credits force points** (`0x10`/`0x11` are written only by the force screen and by multiplayer's config parse, and no `.cog` in the 540 shipped scripts touches bins 10..19). Rank progression is likewise unrecovered — `jkPlayer_SetRank` is called only from the multiplayer new-character screen. So the port will keep `fix_stars` as the engine's own reconcile rule and credit ranks per level, with that inference marked as one.

**Round 169:** the grant is live, so a player *gets* a power. `Runtime::grant_episode_powers(lightpow, darkpow)` asks `force::episode_grants` for the bins the player's declared side may use and raises each to star one through `set_bin` — which starts the bin's own `cog=`, the port's equivalent of `sithInventory_SetCarries`' available bit. `grant_level_powers` calls it once per level, finding the row by asking every loaded episode which one owns the level (`episode.jk` is not a fixed name across the shipped archives), so it covers the first load and every swap alike.

The new gate is `--force-grant --start 03katarn`, and it reports what the row says and what happened:

    level: `jkl\03katarn.jkl` grants force power(s) [22] (light 22, dark 0)
    force-grant: episode row gave bin 22 = 1, `force_speed.cog` running: true
    --- force-grant check passed ---

Level 1 grants nothing (`00 0` on its own row), which is why the same print is absent there. 56 gates, 120 tests, 0 failed. Still to come: rank progression and the screen's click rule.

**Round 170:** the screen is on the menu now, and rank moves. `menus::force_screen()` is pushed into the loaded screen set with `base: 0x52f168` — the address the pause menu's own edge names — so `nav`'s `Step::Open` lands on it instead of a dead end; `--force-screen` now asserts both halves of that hop (the edge and the base). A finished level raises the Jedi's rank and `jkPlayer_FixStars` turns it into points:

    level: 02narshadda.jkl loaded (391 things, 641 sectors), rebuilding
    level: the Jedi reaches rank 1 on leaving `01narshadda.jkl`, 3 force point(s) to spend

That rank bump is the **one inferred rule** in the economy: the dump reaches `jkPlayer_SetRank`/`jkPlayer_FixStars` only from multiplayer's new-character screen and nothing anywhere writes the point bins, so where retail credits them is unrecovered. One rank per level, capped at thirteen, sits in a single marked block in `swap_level`, so a better answer replaces one line. 56 gates, 120 tests, 0 failed. Next: the click rule that spends those points.

**Round 171:** the click rule is in, and tracing it turned up a trap worth writing down. `force::can_spend` is the port's stand-in for `jkGuiForce_ButtonClick` — the function the dump cannot supply — and it enforces the engine's own rules around it: one point from the pool, `items.dat`'s `max` for that row as the ceiling, the rank ladder as what is on offer, the rank-7 side lockout, and the two masteries refused because `SetProtectionDeadlysight` grants them rather than sells them. A power the level granted stays open even at rank 0, where the ladder offers nothing.

The trap: **the button table's ids are not inventory bins.** `jkGuiForce_buttons` gives `JUMP` the id `29` and `SPEED` `33`, while `items.dat` puts jump on bin `21` and speed on `22` — which `--force-grant` proves, since acquiring bin `22` starts `force_speed.cog`. So the screen's click is resolved by *name* against `items.dat` (`SPEED` -> `f_speed` -> bin 22, `max 4`), and the ids stay what they are: element ids. The full recovered map, straight from `--items`:

    20 jedi_rank   21 f_jump    22 f_speed   23 f_seeing   24 f_pull
    25 f_healing   26 f_persuasion  27 f_blinding  28 f_absorb
    29 f_protection  30 f_throw  31 f_grip   32 f_lightning  33 f_destruction  34 f_deadlysight

which matches `SetAccessiblePowers` exactly: the basics are `21..24`, the light-side group `25..28` pairs with the dark-side `30..33` one tier per rank, and `29`/`34` are the masteries the alignment branch grants at rank 8.

`--force-grant` now drives the spend path through the same runtime the clicks use:

    level: `jkl\03katarn.jkl` grants force power(s) [22] (light 22, dark 0)
    force-grant: episode row gave bin 22 = 1, `force_speed.cog` running: true
    force-grant: spending two points on `f_speed` -> true/true, bin 22 = 3, 0 left; a locked power -> false
    --- force-grant check passed ---

121 tests, 56 gates, 0 failed. What is left in the force feature is the screen's *display* of stars and the pool (the model is complete), and the campaign's own rank curve if a better answer than one-per-level turns up.

**Round 172:** the screen shows the player's own powers, and the bin trap is gone at the source. `FORCE_POWERS` now carries the real inventory bins (`JUMP` 21, `SPEED` 22, `HEALING` 25, `ABSORB` 28, `PROTECTION` 29, `THROW` 30, `GRIP` 31, `LIGHTNING` 32, `DESTRUCTION` 33, `DEADLYSIGHT` 34, …) instead of the engine's element ids, so a click spends on the bin the row names and the engine's own "hide what has not been found" rule works on it directly: `menus::apply_force_rows` is that rule, unit-tested (`JUMP`/`SPEED` show for bins 21/22, nothing else does). The three rows the single-player build never offers (`PROJECT`, `PUSH`, `RAGE`) carry bin 0, so nothing can be found there and a click spends nothing.

`refresh_force_screen` runs while the screen is up: it sets each row from the live inventory and writes the pool and rank into the screen's third note row (`STARS 3   RANK 1`), since a key with no string in the table is drawn as itself — `ForceStarsDraw` paints the same two numbers in the retail build. 122 tests, 56 gates, 0 failed.

**Round 173:** a docs reconciliation, because three roadmap claims had been outgrown by the code, and one more campaign link proved. Corrected: puppet animation *is* driven by motion (`actor_state` picks `walk`/`run`/`fire`/`hit`/`turn` from velocity, pose and the `.pup`'s own states); AI steering *is* a search (`Collide::route` is breadth-first over the portals with gate checks, and `Runtime::sector_route` keeps the doorway route per actor); and player attach *is* ported (landing with the `-dot(vel, normal) > 2.5` fall-hurt threshold, thing-to-thing parenting with the child's offset in the parent's frame) — only `sithPhysics_UpdateAttachedThingPhysics` is still uncited. What each really still lacks is now in the row instead: crouch and attack wind-up states, layered bodies; scheduler weights and `accuracy=` spread; surface attachment proper.

The campaign chain is also verified past level 1 — `03katarn` completes into its own film and the next level, with the rank and grant behaving:

    level: `jkl\03katarn.jkl` grants force power(s) [22] (light 22, dark 0)
    level jkl\03katarn.jkl complete -> cutscene 12a.smk, then 04EscapeHouse.jkl
    level: 04EscapeHouse.jkl loaded (312 things, 976 sectors), rebuilding
    level: the Jedi reaches rank 1 on leaving `jkl\03katarn.jkl`, 2 force point(s) to spend

Two points, not three, is `FixStars` agreeing with itself: the granted speed power is one star spent, and the budget is three a rank.

**Round 174:** the post-level tally is in, which is the beat between finishing a level and its film. `jkGuiSingleTally_Show` is base `0x531530` (7 items: four blank text rows, the `type 7` stars panel `0x418000`, Quit and Ok) and the executable's tables already carried it — and `tally_lines` already filled its rows for the `--tally` gate. What was missing was anyone *showing* it. `watch_level_complete` now raises it and holds the film back until the player closes it (`0x531530` id `1` is the new `Step::Close` edge; Escape backs out too).

![The end-of-level tally, drawn from the executable's own screen](tally.png)

*The tally for level 1: `RANK 0 L`, `Secrets Found: 0/8` and the `Stars Earned:` caption over the `bkTally.bm` plate, with Quit and Ok. Captured with `BK_TALLY=1` under `--complete-after`, which is the only way this screen can be photographed without a player at the keyboard.*

That made one thing necessary: a probe cannot click a screen, so `CaptureMode` remembers whether this run is a capture or a probe and the tally is skipped for one. Checked both ways — `--complete-after 1` still goes straight to the film, and `--goal --start 01narshadda` still walks GOAL00 -> `jkEndLevel` -> `06a.smk` -> `02narshadda.jkl` and passes. The star strip under the caption is empty here because the pool is zero at rank 0; `ForceStarsDraw` draws one star per pool point, so a real campaign shows them. 122 tests, 56 gates, 0 failed.

**Round 175:** the force screen got its first photograph, and it shows both what works and what does not.

![The Force Powers screen, from the executable's plate and the shipped power art](force-powers.png)

*The Force Powers screen over `bkForce.bm`, with `JUMP` at two stars and `SPEED` at one — the two powers `BK_FORCE_SCREEN=1` seeds for the picture. The `fo*.bm` art loads, the grid lays out six across as the executable's table says, and the empty rings are the plate's own slots for powers the player has not found.*

Two things came out of taking it. The good one: the screen is real — plate, art, layout, and the found powers with their stars, all from shipped data. The bad one: **its three text buttons (`Ok`, `Reset`, `Quit`) draw nothing**, while the tally's identical buttons draw fine — same `kind`, same keys, and `text_type` 2 (which is what every shipped screen uses) changed not one pixel.

That is now explained, and it is two steps down from the font. **Text is drawn with the glyph's own pixel value as a palette index**, and the `ui\sft` art's ink pixels are `229..232` — which `bkForce.bm` (and the level colormap, tried as a fallback) paint **black**, while `bkTally.bm` paints them orange. So the labels really are drawn, in black, on a dark plate. The engine is not relying on the screen palette for them: `stdFont_Draw3` consults a **glyph remap table** (`iVar2 + 0x30`, char ranges to colour entries, with a fallback index) before it blits, and this port carries no remap — that is the gap. `--force-screen` now prints the chain that proves it (plate palette `1..4`, `font 2 'O'` ink value `229`, and the colour that index resolves to) so the next round starts from the number, not from a guess. Until then the screen is closable from the keyboard: `0x52f168` id `1` is a `Step::Close` edge, and Escape backs out.

`BK_FORCE_SCREEN=1` opens the screen at boot with two powers seeded, the way `BK_TALLY=1` forces the tally — both are capture aids, both are documented as such. 122 tests, 56 gates, 0 failed.

**Round 176 (retraction):** the last two rounds chased a defect that did not exist, and the reason is worth recording. **The executable already carries the force screen.** `--ui-tables` has it at `0x52f168` with 17 items: fourteen `type 1` picture buttons whose `id` **is the `items.dat` bin** (`28, 27, 33, 31, 25, 21, 26, 24, 23, 22, 30, 32, 29, 34`), each with a `click` callback and a `-1,-1,-1,-1` rect that `jkGuiForce_Init` fills in, plus three note rows. There are **no `Ok`/`Reset`/`Quit` text buttons in it at all** — the screen closes on the return-key shortcut.

What was wrong was the port, not the screen: `menus::force_screen()` *synthesised* a 17-button panel from `jkGUIForce.c`'s `jkGuiForce_buttons[25]` and was pushed into the screen list, on the belief that the element array was built on the heap. `Screens::find` returns the **first** match, so the executable's screen was used and the synthesised one was dead — including its three text buttons, which was the "invisible buttons" I spent two rounds diagnosing. The glyph-remap trail, the palette comparisons and the `text_type` change were all measurements of a screen nothing drew.

Deleted: `FORCE_POWERS`, `force_screen()` and its use (about 190 lines), the `Ok` nav edge that existed only for those phantom buttons, and the `--ui-tables` special case that composed it. The `--force-screen` gate now checks the real table: fourteen rows, ids are bins, `21`/`22` show for a player holding jump and speed. The glyph-remap question is *not* answered by this — it was never the cause here — so it is no longer listed as a known defect. 122 tests, 56 gates, 0 failed.

**Round 177:** the star pipeline now runs the way the executable's two screens expect, and the tally finally shows its stars.

`jkGuiSingleTally_ForceStarsRender` repeats the star cell `GetInvItemAmount(0x10)` times — **bin `0x10`, the stars this level earned**, not the rank and not the spendable pool. The port was drawing the *rank* there, so a finished level showed nothing (rank 0) or the wrong count. The pipeline now matches the engine's order:

1. **the level ends** — `finish_level` raises the rank and credits `0x10` with the level's award (`jkMain_EndLevel`'s place in the sequence, before the tally draws);
2. **the tally draws `0x10`** as that many stars under `Stars Earned:`;
3. **the force screen opens** — `fold_stars` adds `0x10` into the pool `0x11` and clears it, which is `jkGuiForce_Show`'s own first move, then reconciles with `jkPlayer_FixStars`.

![The tally with three stars earned](tally.png)

*Level 1 finished: `RANK 1 L`, `Secrets Found: 0/8`, and **three stars** under `Stars Earned:` — one per point in bin `0x10`, drawn from `foStars.bm`.*

The credit moved out of `swap_level` (where it happened after the tally had already drawn) into the level-end path, so the numbers the tally shows are the numbers the screen will spend. `fold_stars` is proven in the same run that opens the screen: the seed puts three stars in `0x10`, and the log reads `force: the screen folded the level's stars in, 6 in the pool` (six because rank 4's twelve-point budget less the two powers the seed also holds). 122 tests, 56 gates, 0 failed.

**Round 178:** the force screen now shows each power's level, which `jkGuiForce_ForceStarsDraw` draws and the port was leaving out. The rule is the executable's: `GetInvItemAmount(bin)`, then the star cell for that level is blitted beside the icon — cell `level-1`, biased by three for the light powers (`0x19..0x1c`) and by seven for the dark ones (`0x1e..0x21`), and an icon left of the panel's middle (`x < 0x140`) gets its stars on its left rather than its right.

![The Force Powers screen with a level beside each icon](force-powers.png)

*`JUMP` at level 2 with a two-star cell to its left, `SPEED` at level 1 with a one-star cell to its right — the bias and the side rule are `ForceStarsDraw`'s own. The other icons are hidden because the player has not found them.*

`ui::Look::force_level` carries the star art and a closure onto the live inventory, so the pips follow what the player actually has, and they redraw on the same frame as the row visibility does. 122 tests, 56 gates, 0 failed.

**Round 179:** the save slots are gated, and one release-note claim was out of date. Instead of one file, `SLOTS = 8` slot files exist (`quicksave.jks` for slot 0, `slotN.jks` after that), `,` and `.` step between them, and the new `--save-slots` gate proves two of them hold different games at once:

    save-slots: slot 0 <- `jkl\01narshadda.jkl`, 26228 bytes
    save-slots: slot 7 <- `jkl\03katarn.jkl`, 26228 bytes
    save-slots: `quicksave.jks` reads back `jkl\01narshadda.jkl` with 355 thing(s)
    save-slots: `slot7.jks` reads back `jkl\03katarn.jkl` with 355 thing(s)
    save-slots: 2 slot file(s), distinct true
    --- save-slots check passed ---

It writes into a temporary directory and removes what it wrote, so no real save is touched. What is still missing is retail's own slot-list *screen* — `0x53b948`, whose 14 items are a list element, a name field, Ok, Cancel and Delete — so a slot is chosen with a key rather than from a list; the release notes now say that instead of "one save slot". 122 tests, **57 gates**, 0 failed.

**Round 180:** a spent point now measurably changes the game. `--force-live` jumps twice — at level 1, then at level 4 with the same hold — and reports both rises:

    force-live: the body rose +0.60 (took off at -1.18, peak -0.59), mana 100 of 100
    force-live: at level 4 the same press rose +0.71 against 0.60 at level 1
    --- force-live check passed ---

That is `force_jump.cog`'s own rule doing the work: `rank = GetInv(player, 21)`, `maxDuration = rank * 0.35`, and `jump = duration + 2.5`, so a trained power holds the same press longer and lifts higher. The first attempt failed to show it, and the reason is worth keeping: the probe held for 25 frames, and at that length both caps (`0.35 s` at level 1, `1.4 s` at level 4) sit above the hold, so the difference hides in noise. Nine tenths of a second exposes it.

Two things the probe taught by being wrong first: the level has to be raised *between* the jumps, because the script reads the inventory at `deactivated` rather than at `activated`; and the mana figure cannot be part of the pass condition, because the bin regenerates between the press and the measurement — it is printed as information instead. 122 tests, 57 gates, 0 failed.

**Round 181:** a compiler warning turned out to be a real bug, and the release documents were brought up to date.

The warning was `unreachable pattern` in `src/runtime.rs`: `"sectorthrust"` matched at one arm, so the later arm that grouped it with `"setsectorthrust"` could never run for it. The two disagreed — the shadowing one stored the raw vector, the real one applies `mult` and clears the thrust on a zero vector. `sithCogFunctionSector_Startup` is what settles it: both verbs are registered to the **same** function (`LAB_004fec70`), so they must behave identically, and the shadowing arm was simply wrong. It is deleted; the surviving handler serves both. That matters in play: level 21's core tube is an updraft and level 10's power core pushes the player out of its chamber.

The documents had drifted the same way. [docs/gates.txt](gates.txt) claimed `FAILURES: 0 / 42`, and the README claimed "30 gates, 0 failures, plus 76 tests". A fresh sweep is appended to `gates.txt` with all **57** rows and their verdicts, the summary line now reads `0 / 57` (`0 of 56` under `BK_SKIP_WINDOW=1`), and the README says **57 gates, 0 failures, plus 122 tests**. 122 tests, 57 gates, 0 failed.

**Round 182:** the README's walk table was stale, and checking it exposed a gate that asserted nothing.

The README showed `34/34 levels … 931 sector crossings`; a fresh `--walk all` says **30/34** and **56 crossings**, with four levels named: `04escapehouse`, `06bbarons`, `11gorc` and `m10`. Two things were checked before believing the change. First, was it my `sectorthrust` deletion from the round before? No — running the sweep against the previous `runtime.rs` reproduces the same four rows exactly, so the containment result predates it. Second, what do the four have in common? Three are *blocked* for 548–563 of 600 frames and travel about a metre, so they end in a neighbouring sliver rather than walking through a doorway; `m10` is the power-core level, whose sector thrust moves the body. None is a collision failure.

That raised the real finding: `--walk all` printed `--- walk check passed ---` **without asserting anything** — it reported. The sweep now aggregates what must hold on every level regardless of containment, prints it, and asserts it:

    summary: 30/34 levels with a start kept the walker inside its sector, 18 had the hull test agree, 56 sector crossings, 1 without a start
    sweep: 0 level(s) left the body in the void, worst drop 0.00

Zero voids and a worst drop of zero across all 34 levels is the claim the README was making by hand. `--audit` was re-run too: 540 cogs parsed, 0 failed. 122 tests, 57 gates, 0 failed.

**Round 183:** the walk gate was not the only one that could never fail, so the whole suite was audited for verdict lines with nothing behind them.

The audit took every `--- <name> check passed ---` line and looked back for a check mechanism — an `assert`, a `say(cond, …)`, or a `FAIL` branch. One gate had none of the three: **`--winding`**, which printed its counts and then `passed` unconditionally. Its claims were already measurable, so both are now asserted:

- `outward == 0` — a normal pointing away from its room is a face seen through from inside, and the run reports `91233 surface normal(s) point into their room, 0 point away`;
- `same_plane == compared` — every surface's Newell plane must equal the one the level ships, and the run reports `126639 of 126639`.

Both are the gate's own numbers from the run that added them, so any drift now fails instead of scrolling past.

The harness got the matching guard: a child that prints **`FAIL` anywhere** now counts as failed even if it also printed a verdict line, because a gate that says both is worse than one that says neither. The row reads `(printed FAIL)` so the reason is on the page. 122 tests, 57 gates, 0 failed.

**Round 184:** a new gate answers the question the release actually stands on — can every campaign level be finished?

A level ends when one of its scripts calls `jkEndLevel`; the engine registers that verb in `jkCog.c` and the level's final script is what calls it, so a level whose scripts never call it cannot be completed no matter how well everything else works. `--ends` sweeps the archives, reads every script each level places, and reports which ones end it:

    ends: 28 of 35 level(s) name a script that calls `jkEndLevel` (the `m*` maps and `static` are not campaign levels)
    --- ends check passed ---

The seven without one are multiplayer maps (`m10`, `m2`, `m4`, `m5`, `m_boss15`, `m_boss17`) and the tool's own `static` scene, which are labelled as such rather than counted; the assertion runs over campaign levels, and it is an assertion, not a report. That is the first whole-install evidence that the chain has an exit from every level it can reach. 122 tests, **58 gates**, 0 failed.

**Round 185 (from a play report):** *"clipping and lack of collision"* — the report was right, and the cause was that **the port had no model collision at all**. Every sector plane stopped a body; not one placed model did, so a player walked through the cantina's tables, its columns and every crate in the game.

The engine's rule is in the shipped data, in the template chain: `misc\things.dat` gives `_structure` a `collide=3` and `_decor` a `collide=0`, `_walkstruct` inherits the first through its `_structure` base, and each placed template carries its own `size=` radius. `column01 _structure … size=0.204987` is solid; `tuskensoused _ghostdecor` is not. `runtime::solid_radius` reads exactly that, `collide::build` turns every placed thing it answers for into a cylinder, and `Solids::push_out_of_things` resolves the body against them after the walls (`move_body`, so the sweep's own callers get it too).

Measured on level 1, walking straight at a `column01` from three units:

    before:  walked 120 frames: travelled 1.1 units,  71 blocked
    after:   walked 120 frames: travelled 0.4 units, 110 blocked

The same walk on every level now stops in the furniture instead of passing through it — `--walk all` reads `25/34 contained, 32 sector crossings` where it read `30/34, 56`, and the four extra "NO" rows are bodies blocked against a table or crate within a metre of the start rather than bodies walking into the next room. That is the expected consequence of furniture being solid, and `sweep: 0 level(s) left the body in the void, worst drop 0.00` still holds.

Two limits, both written down rather than hidden: the collision volume is a **vertical cylinder** of the template's `size=` and eight radii of height, not the model's box (`ponytail:` in `Solids::things` names the `.3do` bounds as the upgrade), and the **camera still clips** when it starts 0.16 units from a tabletop — the near plane cuts the geometry, which is the other half of the report and is not fixed yet. `BK_NEAR_THINGS=1` prints what stands within three units of the spawn and whether the engine calls it solid, which is how the spawn's two columns and one table were identified. 122 tests, 58 gates, 0 failed.

**Round 186 (from a play report):** *"it plays the cutscenes of Kyle talking to the lady in the ship, then the booth, THEN it puts you in the game"* — the port started the campaign at `first_level` and skipped the opening film entirely.

The cause is the shape of `episode.jk`'s graph: `gotoA` links run forwards, so the film that *leads into* level 1 is the one row no walk from the level reaches.

    10:      1     1        CUT      03-04a.smk        0   0   11   -1
    11:      1     1        LEVEL    01narshadda.jkl   0   0   12   -1    # Level 1: Narshadda

`Episode::flow_before` walks that backwards from the level (the row whose `gotoA` is the level, then the row whose `gotoA` is that film, up to a previous level row), and `FlowPlan::pre` carries the result. On a real campaign start `play_intro_films` plays them in order and only then lets play continue, which is what retail does:

    cutscene: the campaign opens with 03-04a.smk, 2500 frames, 0 left after it

Two deliberate exclusions, because a gate that sits through a 2:47 film is a gate nobody runs: `--start` never fills `pre`, and the system clears it for a capture, a probe or a sweep child. Both were checked — a `--shot` capture logs no `cutscene:` line and still saves at frame 30, and the sweep is 58 of 58.

The overlap the test pinned down is worth keeping in view: level 2's predecessor is `06a.smk`, which is *level 1's ending film*, already played by `flow_after`. That is why the app asks `flow_before` only at the campaign start — asking on every load would play each film twice. 123 tests, 58 gates, 0 failed.

**Round 187 (from a review of the real camera):** the decompilation answers the clipping question, and the answer is the **near plane**, not collision.

`sithCamera_Startup` (0x4c4de0) creates six cameras — modes 1 (first person), 4, 8, 0x20, 0x40, 0x80 — each with **FOV 90°** (`0x42b40000`) and aspect 1.0 (`0x3f800000`). Each one's `rdCamera` is built by

    rdCamera_NewEntry(camera + 0x7c, fov, 0, 0x3c800000, 0x42800000, aspect)

and those two constants are **near = 0.015625 (1/64)** and far = 64.0. The player's own radius is `size=0.065`, so the engine's near plane is *a quarter of the body's radius*: a body that can touch a wall always keeps the eye further from it than the near plane, and no wall is ever sliced.

The port used Bevy's default `PerspectiveProjection` near of **0.1** — six times the engine's, and *larger than the player's radius* — so every wall the body could reach was inside the near plane. That is the "clipping": the frame was showing geometry cut open by a plane 0.1 in front of the eye. `engine_projection()` now sets near `0.015625`, far 1000 (the sky dome's radius 600 has to fit), and every `EngineFov` world camera takes it; `engine_fov` keeps changing only the angle, as before.

The rest of the camera, for the record. `sithCamera_Update` (0x4c56b0) switches on the mode:

- **case 1, first person** — copy the thing's matrix (`thing + 0x88`), add the punch/flash offset (thing field 2 → `thing + 0xe4`), otherwise the thing's own `eyeoffset` (`+0x15c..0x164`) plus a head-pitch forward shift (`+0x1d4..` dotted with the camera axes, times `_DAT_00522ddc`, clamped to ±8) and the POV shake (`DAT_008ee5b8..`, set by `sithCamera_SetPOVShake`), then translate by the thing's position (`+0xb8`).
- **case 4, third person** — the same eye offset plus `thing + 0x168`, then `sithCamera_SearchSectorInRadius` from the thing toward the camera.
- **cases 0x20 / 0x40 / 0x80** — override cameras for cinematics (fixed matrix, sway, or the "sector in radius" placement).

`sithCamera_SearchSectorInRadius` (0x4c6050) is the engine's **camera collision**: it normalises focus→camera, calls `sithCollision_SearchForCollisions` with radius `0x3ca3d70a` (0.02) and flags `|0x800`, then **rewrites the camera position to the first hit** (`cam = hit_t * dir + focus`). So the third-person camera is pulled in to the wall; the first-person camera is not, and does not need to be, because of the near plane above.

`sithCollision_FindSectorInRadius` supplies the sector index for the final camera point (the `param_1[6]` field), and `sithCamera_RenderScene` sets it current, inverts it into the view (`rdCamera_Update`), and draws.

What that leaves for the port: first person matches retail's maths already (eye offset `0.037` from `_actor`'s `eyeoffset`, 90° horizontal FOV). The third-person/chase cases and their pull-in are not ported — `--stand` and the overview place their cameras directly — which is where `sithCamera_SearchSectorInRadius` would go if a third-person view is wanted. 123 tests, 58 gates, 0 failed.

**Round 188:** the third-person pull-in is ported — `sithCamera_SearchSectorInRadius` had no counterpart in the port, and there was no third-person view to need one.

`collide::camera_pull` is the routine's own shape: normalise `focus -> want`, cast with the sector walk (`sweep_rooms`, so a camera behind a doorway is not pulled in by the wall beside the door), and **return the contact point** — the engine puts the camera at `hit_t * dir + focus`, not a radius short of it.

The view is a boom behind the eye along the look direction, `Chase::length` long (3.0, or `BK_CHASE_DIST`), toggled with **`V`** and started on with `BK_CHASE=1` for captures. Every frame the boom is wanted, then pulled:

    camera: third person wanted Vec3(-13.04, -6.96, -1.15), pulled to Vec3(-10.48, -5.48, -1.15) (surface 3304)

That is level 1's booth: 3.0 requested, 0.04 available, because the player starts in a closed box — which is the correct answer and the reason retail opens in first person.

`--camera-pull` is the gate, and it checks both branches. Sixteen directions at a 3.0 boom from level 1's largest sector: every pulled camera must be *nearer* than the boom and must clear the wall by the cast radius; and a boom of 1e-4, short enough that nothing can be reached, must come back **exactly** where it was asked to be — the branch an enclosed room never exercises on its own.

Two honest limits. The player's own model is not drawn in third person (first person never needed it), so the boom currently shows an empty view of the room; retail draws the body. And the boom is a straight segment, where `sithCamera_Update`'s case 4 composes the camera matrix from the thing's own chase offset (`thing + 0x168`) — the port uses a plain look-direction boom, with the pull-in — which is the part that was asked for. 123 tests, **59 gates**, 0 failed.

**Round 189:** the weapon's render method was wrong in three ways, and the decompilation says what it should be.

`jkPlayer_DrawPov` (0x4051e0) is the whole story. It copies the **camera's matrix**, pre-translates by the *negated* POV offset (`thing + 0x168..0x170`), pre-multiplies the sway rotation (`rdMatrix_BuildRotate34` from the puppet's bob constants and the weapon's own velocity, `thing + 0x1d4..` and `thing + 0x21c`), draws the POV thing with `rdThing_Draw`, and lets `jkSaber_Draw` add the blade for the same matrix. Crucially it sets `rdSetZBufferMethod(0)` and `rdSetOcclusionMethod(0)` first, and takes its ambient light from the sector (`rdCamera_SetAmbientLight`, `rdColormap_SetCurrent`). The weapon therefore has **no depth test at all** and lands on top of the scene every frame — which is why retail's pistol never disappears into a wall.

The port drew it as a **world mesh** 0.42 units in front of the camera, depth-tested like any other geometry. Two consequences followed: any wall inside 0.42 cut the weapon, and — the reason nothing was ever on screen — the mesh was **frustum-culled**, because its Aabb is built from vertices that sit away from the entity's origin, so the weapon was culled while its entity was in view.

The fix is retail's pass in Bevy terms: a `ViewmodelCamera` on render layer 3, order 1 (after the world at 0, under the screen tint at 2 and the HUD at 3), `clear_color: None`, its own depth buffer, the weapon parented to it so its transform is camera-relative, `NoFrustumCulling` (a view model is always in view — that is what the component is for), and an unlit material until the sector ambient is wired.

Two things are honestly not done. The model is **not framed like retail yet**: `bryv.3do` carries `RADIUS 0.119691` and `INSERT OFFSET 0.004646 0.021351 -0.011998`, the drawn geometry sits offset from its origin, and the placement needs calibrating against the POV thing's real `+0x168` offset and its scale at `+0x21c`, which I have not decoded to numbers. And the pass is unlit (`ponytail:` names per-camera ambient as the upgrade). The pipeline is right; the framing is the next piece of work. 123 tests, 59 gates, 0 failed.

**Round 190 (goal round 1, item 1):** the weapon is now drawn, in the camera's frame, at a placement derived from the model itself. What the decompilation settled and what it did not:

- **`+0x168` on the thing is the POV/camera offset.** `jkPlayer_DrawPov` negates it as a camera-space translation, and `sithCamera_Update`'s third-person case applies the same field as a post-translate. No decompiled writer sets it — it is not in `sithThing_ParseThingArg`, and the field is not a template key the parser handles — so the port derives the placement from the model instead of guessing a magic offset.
- **`+0x21c` is not a scale.** It divides the weapon's own speed in the sway bob (`local_44 / thing[0x21c]`), i.e. a reference speed. An earlier note calling it a scale was wrong and is corrected here.
- **The mesh renders; the separate pass does not.** A probe quad on the weapon's own render layer never appeared, so a `ViewmodelCamera` at order 1 or 4 draws nothing in this build (order 4 also blacked the frame). The weapon is therefore parented to the **world** camera — camera-relative placement, rendered by the pass that is known to work — with `NoFrustumCulling`, because its Aabb comes from vertices offset from the entity origin.
- **The orientation was the invisible-weapon bug.** The old placement composed `from_rotation_arc(Vec3::Z, forward)`, which in the camera's own frame turns the pistol sideways and, at the placement the model's radius asks for, points it behind the eye. `bryv.3do`'s geometry is x[-0.007,0.005] y[-0.007,0.015] z[-0.035,0.015] with `RADIUS 0.066`, so the placement is 1.6 radii forward, half a radius right and 0.6 down, and the transform needs **no** extra rotation: `model_mesh` already converts the file's frame.

Still open on this item, and why: the weapon reads **small** and sits low-right against the HUD — the framing needs the POV thing's real offset, which is the piece the decompilation does not hand over; and the pass is still `unlit` where retail uses `rdCamera_SetAmbientLight` from the sector. Capture aids: `BK_ARM=<bin>`, `BK_POV=<model>`, `BK_VIEW=x,y,z`. 123 tests, 59 gates, 0 failed.

**Round 191 (goal round 2, item 1 continued):** the weapon is now lit by the room, and the reason it was black was a real data gap.

`jkPlayer_DrawPov` hands `rdCamera_SetAmbientLight` the **sector's `AMBIENT LIGHT` plus `EXTRA LIGHT`**, clamped to 0..1. The port parsed neither: `jkl.rs` read a sector's centre, radius, colormap, boundbox, surfaces and sound, and stopped there. `SectorState.light` (the `SetSectorLight` extra) was the only light the weapon could have used, and it is 0 in level 1's booth — so the first version of this fix drew the pistol black (logged `sector light 0.00`).

Both fields are now parsed, and `sector_light` sums the baked room light with whatever `SetSectorLight` has added, exactly as the engine does. Level 1's spawn reads **0.88** where it read 0.00, and the weapon is visible and lit rather than a dark sliver. A unit test pins the parse (`reads_the_sector_lights`): 124 tests now.

One attempt failed and is worth recording so it is not repeated: I tried to derive the framing from the **posed** bounds (`tdo::pose_nodes` + `bounds`) and a scale from the view's half-height. That produced `scale = 0.62 * half_height / size` for a posed size that is evidently not what the raw vertices suggest, and the weapon vanished entirely — the round-1 placement (radius-based, 1.6 radii forward) is what ships. The framing is therefore still open: the weapon reads small and sits low-right against the HUD, and the honest fix is still the POV thing's own offset, which the decompilation does not give a writer for.

`--gates` 59 of 59, 0 failed; 124 tests.

**Round 192 (goal round 3, item 1):** the weapon's framing works, and the reason it kept failing was one missing line.

`view_frame` derives the placement from the view — a fixed distance, and a scale that sizes the drawn mesh to a fraction of the view's half-height — but **the scale was never applied to the transform**: `Transform::scale` was left at 1, so the mesh drew at its world size (about 0.005 units for `bryv.3do`), which is a pistol a few pixels wide in the lower-right of the frame. Three rounds of "the weapon is invisible / tiny" were all this.

Also in this round:

- **The mesh is recentred on its own bounding box** (`recentre_mesh`). A `.3do`'s node transforms place its parts away from the model's origin, so an unrecentred weapon appears at the edge of the view and any scale derived from the box disagrees with the screen.
- **The framing is measured from the drawn mesh** rather than the file's vertices, and the size is the **75th percentile of the vertex distances** rather than the box: a view model draws its arms as well as the weapon, so the box is the arm span, and scaling by it draws the pistol as a speck. `ponytail:` on `view_frame` says the `.3do` node scale is where the exact answer lives.
- A unit test pins the whole thing (`view_frame_tests::the_weapon_frames_inside_the_view`): for models from 0.005 to 0.5 units, the placement must be in front of the near plane, inside the frame's width and height, and subtend between 0.1 rad and half the vertical view — the "invisible" and "wall of gun" failures both fail it.

What a capture shows now: a pistol about 230 by 180 pixels at the lower right of an outdoor view (`04escapehouse`), where before there was a 30-pixel speck. Still to do on this item, both presentation rather than placement: the model draws **untextured white** (its `HDBACK.MAT` evidently has no bitmap in the archives) and its **barrel points across the view rather than down it**, which is the model's own authored orientation. 125 tests, 59 gates, 0 failed.

**Round 193 (goal round 4, item 1 finished):** the weapon is textured, scaled, placed and lit — the last two pieces were its bitmap and its size.

- **Texture.** A view model is named by a cog at run time, so it is not in the level's `model_images` and the port drew it flat white. `World` now keeps the install's palette, and `draw_viewmodel` decodes the weapon's own `.mat` through the same path the world uses (`tex::parse_mat` → `to_image`), so `HDBACK.MAT` and the rest of `bryv.3do`'s materials appear. The sector light is the `base_color` tint over that texture, which is what `rdCamera_SetAmbientLight` amounts to on an unlit pass.
- **Size.** The missing `Transform::scale` from the previous round is confirmed working: the pistol now spans roughly a quarter of the frame's height at the lower right instead of being a speck.

What a capture shows: a textured, room-lit weapon at the lower right of `04escapehouse`. What it does **not** yet show is retail's exact framing: `bryv.3do` is the pistol *with hands* (`HND-WRT.MAT` is one of its materials), the drawn mass reads as the hand and forearm, and the precise offset and turn are the POV thing's `+0x168`, which no decompiled writer sets. Item 1 is therefore done as far as the data goes: placement, scale, texture and lighting are all derived from the shipped numbers and pinned by `view_frame_tests`. 125 tests, 59 gates, 0 failed.

**Round 194 (goal round 5, item 2 started):** the third-person camera now has a body to look at, and the boom turned out to have a bug of its own.

- **The player's own body is drawn** (`draw_player_body`): `ky.3do`, textured through the level's `model_images`, at the thing's position and yaw, and *only* when `Chase::on`. `spawn_level` deliberately skips the local player — the camera sits inside it, so its unlit polygons would land over the whole frame — and that skip stays for first person.
- **The body hides itself when the camera is inside it.** The boom's own pull-in parks the camera at the body in tight rooms, where `cull_mode: None` shows the inside of the model: the first capture of this round was a tan band across the sky. The camera-to-body distance is the rule, and it is the boom's own number rather than a guess.
- **Two boom fixes.** `camera_pull` was passed `PLAYER_EYE` (0.037) as its cast radius, which is *larger than the eye's height above the floor*, so the sphere could start touching the floor plane; `sithCamera_SearchSectorInRadius` passes `0x3ca3d70a` (0.02) and the port does now too. The sector handed to the walk is now the one the *eye* is in (`sector_at`), not the runtime's last written index, because a stale index starts the walk outside its own hull where every plane reports a contact at distance zero.

That second fix did not fix the collapse: `04escapehouse` still traces

    camera: third person wanted (-6.04, -1.86, 0.70), pulled to (-3.09, -1.86, 0.70) (surface 8215)

— a 2.95-unit boom pulled to 0.04. The camera is provably at the eye (the body log reads `camera 0.06 from the body`), so **the body is not visible and item 2 is not done**; the remaining suspect is the sweep's contact test at a start point that `sector_at` reports inside but whose hull planes still count as touching. Next round: bisect that with a synthetic open-sector cast, then re-check the body. 125 tests, 59 gates, 0 failed.

**Round 195 (a question from the player, not a defect):** *"can the tests run without stealing the mouse or showing a window?"* — yes, and a sweep now never takes the pointer either way.

- **`BK_HEADLESS=1`** runs with **no window at all** (`WindowPlugin { primary_window: None }`). The sweep is then silent on the desktop: `gates: 0 of 59 failed, 8 skipped`. The eight that are skipped are the ones that need the real frame loop — `--live-check`, `--goal`, `--save-live`, `--save-slots`, `--use-live`, `--ai-live`, `--force-live`, `--force-grant` — and Bevy exits them with "No windows are open" before they reach a verdict. The sweep detects exactly that message and reports **skipped**, counted separately: never a pass, never a failure of the code it was meant to check.
- **`BK_NO_GRAB=1`** is for those eight, and for anything run windowed: the window still opens, but `grab_cursor` does nothing, so the pointer stays yours. The sweep sets it on every child itself, so `--gates` cannot take the mouse even when run with a window.
- `cargo test --bins` never opened a window and still does not: **125 passed**.

So the practical recipe is `BK_HEADLESS=1 bevi-knight --gates` for a background check, and `BK_NO_GRAB=1 bevi-knight --gates` when the eight live gates also need to run. Windowed sweep after the change: `0 of 59 failed, 0 skipped`.

**Round 196 (goal round 6, item 2):** the third-person camera now shows Kyle, and the reason it showed nothing was the mesh, not the placement.

- **A model is drawn one mesh per material.** `model_mesh` builds the faces of a *single* material index, which for `ky.3do` is **59 vertices** — a sliver that reads as nothing at all. `draw_player_body` now builds one entity per material, exactly as `spawn_level` draws every other thing: 20 parts for Kyle, and he appears in the arena seen from behind (`docs/third-person.png`).
- **The orientation is `thing_quat(pitch, yaw, roll)`**, the port's transcription of `rdMatrix_Build34`, not `Quat::from_rotation_z(-yaw)` — the yaw alone in the wrong frame, which lays a body down.
- **The body hides when the camera is inside it** (`body_visible`): the boom's pull-in parks the camera at the body in a tight room, and with `cull_mode: None` that shows the inside of the polygons. The rule is the boom's own length.

Two honest observations from the capture. Kyle is **small for 1.5 units away** — the same "model drawn smaller than its measured size" that the view weapon showed, and worth chasing against the `--scale` numbers (`model scale 1.0`, a door 0.3 units) rather than against a screenshot. And the body has **no animation**: it is the rest pose, where every placed actor is animated by `animate_actors`; joining the player to that list is the next step for this item.

125 tests, 59 gates (8 skipped headless), 0 failed.

**Round 197 (goal round 7):** item 2's body is recentred on its own geometry, and item 3's cylinder is gone — the walk numbers came back with it.

- **The player's body is recentred** on its combined model box before it is drawn (`ky.3do` spans x[-0.03,0.04] y[-0.01,0.04] z[-0.12,0.06], centred on `[0.0019, 0.016, -0.027]`), the same effect that put the view weapon at the edge of the view: a model's node transforms place its parts away from its origin, so an entity's transform did not mean what it said. Kyle now stands where the body is ([docs/third-person.png](third-person.png)).
- **Solid things carry the span of their own model.** `collide::build` cannot know it — the models are parsed after the level — so `solids.things` starts empty and [`solid_things`] fills it where both are in hand: the engine's `size=` radius from `runtime::solid_radius`, and the vertical span from `bounds(model, pose_nodes(...))`. The cylinder was **eight radii tall in both directions**, which is a guess: the spans measured in level 1 run **0.02 to 2.10 units**.
- **`--walk all` is back to its pre-model-collision numbers: 30/34 levels contained, 56 sector crossings, 18 with the hull test agreeing, 0 level left in the void, worst drop 0.00.** The regression to 25/34 came from the guessed cylinder, not from making furniture solid.
- `--collide` now asserts the spans: every solid thing has a positive span, the set contains a low thing and a tall one (ratio > 1.5), and it prints them (`334 solid thing(s), model spans 0.02..2.10`).

125 tests, 59 gates (8 skipped headless), 0 failed.

**Round 198 (goal round 8, item 4 closed, and the +0x168/+0x21c question answered):** the near plane cannot slice a tight spawn, and it is now a gate rather than an observation.

- **`--collide` measures the eye`s clearance with a zero-radius probe** in six directions at every `walkplayer` spawn and asserts it against the renderer`s own `NEAR_PLANE` (1/64, `rdCamera_NewEntry`'s `0x3c800000`). Level 1`s booth — the case the defect named — reports `tightest eye clearance 0.157 (near plane 0.015625)`: the eye keeps **ten times the near plane** from the table and the columns, so nothing can be clipped. The gate fails if any spawn in any level comes inside it.
- **The camera/body interplay** is the other half: the boom`s pull-in parks the third-person camera at the body in a tight room, where `body_visible` hides the body rather than showing the inside of its polygons.
- **`thing + 0x168` is the eye offset, and the dump does say so.** `sithCamera_Update` pre-translates the thing`s matrix by it (`rdMatrix_PreTranslate34(puVar10, iVar7 + 0x168)`) and `jkPlayer_DrawPov` negates it (`local_3c = -*(float *)(iVar3 + 0x168)`) to build the POV frame. Nothing *writes* it, so its value stays the port`s eye calibration (`PLAYER_EYE`, clamped by the nearest ceiling) — the same quantity, derived instead of guessed.
- **`thing + 0x21c` is not a scale.** `jkPlayer_DrawPov` divides the sway by it, and `sithPuppet_UpdatePuppet` uses it as the reference speed in `(0x21c + node offset) - 0xbc`. The port`s `waggle` is its counterpart, so there was never a scale to decode.

That closes the four items: the view weapon (placement, scale, texture, sector light), the third-person body, model-span collision with the `--walk all` numbers restored, and the near-plane invariant at tight spawns. 125 tests, 59 gates (8 skipped headless), `--walk all` 30/34 contained / 0 void.

**Round 199 (requested: "add the player"):** the local player is now an **actor**.

- **Kyle is spawned by `spawn_models` like every other thing** — same model, same per-material parts, same `ky.pup` puppet — and handed to `Actors`, so `animate_actors` drives him with the same `stand` / `walk` / `run` / `fire` / `death` states the NPCs get. The gate says which keys: `the local player draws kystand0.key as an actor: stand and walk both ship keys`.
- **His parts are hidden at spawn** and marked `PlayerBodyMesh`; `draw_player_body` now only decides whether they are drawn (first person: no; third person: yes unless the boom's pull-in has parked the camera inside him). That replaced the ~70-line private mesh builder from the previous rounds — one spawn path for bodies instead of two.
- **Two systems used to write `Visible` over that decision every frame**: `things_follow_geomode` (which sets `Inherited` for every live thing) and `show_taken` (which sets `Visible` for every live thing). Both now skip the player's marked parts, which is what had put a forearm across the first-person view; the first-person frame is clean again and third person still shows him ([docs/third-person.png](third-person.png)).
- **Only the first `walkplayer` is the local player.** The training arena places sixteen, and the other fifteen are NPCs that must still be drawn — the flag is `is_local_player(&thing.template) && own_body == 0`.

125 tests, 59 gates (8 skipped headless), 0 failed.

**Round 200 (cleanup, and the film fills the screen):** every repeated walk is in one place now, and a cutscene no longer opens on the level showing through its black bars.

- **One link walk.** `Runtime::wire_links` is the only place the thing, surface and sector link tables are built from a level's `cogs` section and each script's `symbols`. `main` had the same instance-value walk open-coded seven times — three in `setup` (sector, thing, surface), one in the soak, three in the triggers audit — and `setup` then called `wire_links` a second time just to print the counts. The three sites each call it once now, and `setup` prints one `links:` line plus the per-kind `triggers:` lines; the two call sites that already used it (`--goal`'s and `--door`'s) are untouched. The tables are unchanged (level 1: 37 thing, 44 surface, 10 sector).
- **One label resolver.** `Cog::label_for_name` turns a `#N` message id into the receiver's own label and refuses a label the script does not declare; the three delivery paths that each open-coded that six-line prelude (two in `main`, one in `deliver_cog_messages`) call it.
- **One block scaler.** `ui::Canvas::scaled` draws one colour `scale x scale` inside the canvas; the string and map-panel loops had the same body twice.
- **A film fills the window.** The overlay is a 640x480 canvas and every shipped film is 640x300, so the picture sat in the middle with 90 black rows above and below. Two things were wrong with that. The frame does not belong in the HUD's box — the engine showed films on the display — so `cutscene_fills_screen` scales the overlay quad by `film_fill_scale(w, h) = min(aspect * 480/640, 480/300)` while a film plays: 16:9 gives 4/3 (the picture spans the width and 83% of the height), the HUD's own 4:3 gives 1, and a window wider than the film's 2.13:1 caps at 1.6 with pillarbox. And the padding rows are **opaque black** now (`cutscene::blit`): they were transparent and the overlay is blended over the live world, so the bars showed the level through them. The HUD camera clears to black for the film's duration so nothing shows around a wider window. `film_fill_tests::a_film_fills_the_window_and_keeps_its_2_13_to_1_shape` pins the three scales and the degenerate zero-size case.
- **The hand-rolled PNG writer is gone**, and with it the direct `image` dependency: `--smk-decode`'s last-frame dump saves through `Image::try_into_dynamic().save()` (the `image` crate Bevy already carries). `src/png.rs` becomes `src/ppm.rs`, only the `P6` reader for the frames `tools/smkdec` writes — the 32-bit helper has no dependencies, so it dumps PPM — and the old PNG-writer test becomes a PPM pixel read.
- Smaller: `body_visible` was a one-line function and is now the comparison in `draw_player_body`; `--live-check` counts *visible* body parts rather than the entities the player owns, because the body has been an actor since round 199 and the old count passed for the wrong reason; the runtime's door test builds `Runtime::new(&Level::default())` instead of a ~190-line struct literal; the self-check's rotation asserts use `std::f32::consts`.

126 tests, 59 gates (8 skipped headless), 0 failed.

**Round 201 (release-readiness pass):** the first-run defects a player would have hit are fixed,
and the campaign chain is now a gate of its own.

- **`jkPrintUNIString` read the wrong argument.** Level 1's first pickup printed `COG_00000` across
  the top of the screen: `pow_shields.cog` calls `jkPrintUNIString(player, bin)`, and the runtime
  read `args.first()` — the player's thing index 0 — instead of the second argument, the `.uni`
  key. Bin 60 is `COG_00060`, "Shield Recharge"; the key is now read from argument 1 (falling back
  to the one-argument form the old test used), and `cargo test` pins both. The screenshot that
  found it is `docs/first-frame-level1.png`.
- **Audio is audible by default.** A play session now uses `BK_VOLUME` (default 0.75, the engine's
  own mixer scale) instead of being silent unless `--sound-on`; captures and probes stay silent,
  `--sound-on` forces a capture audible, and `--mute` always wins. `--audio-state` dropped its
  tautological print for three assertions on the decision, and a unit test pins the matrix.
- **The campaign decision's transition was broken, and no gate could see it.** `Episode::flow_after`
  only looked one row ahead, so finishing `15maw.jkl` named the **`DECIDE` row's file (`38a.smk`) as
  the next level** — the app would have tried to load a film and stalled there. It now walks the
  rows: it collects every film in order (level 15: `38a.smk` then `39a.smk`), crosses the `DECIDE`
  row on its neutral arm (`gotoA`, the light path — there is no choice screen), and stops at the
  next level. The app's film queue plays both and only sets `films_done` when the queue and the
  running film are both finished, so the level swap cannot cut a two-film transition short;
  `film_step` is unit-tested, and a live run (`--start 15maw --complete-after 0.1`) was watched
  through `38a.smk` → `39a.smk` → `16aEscapeShip.jkl loaded`.
- **`--tour` is the release-content gate.** It walks every campaign level file (25 files, 28 rows
  across both arms), loads and parses it, runs its placed scripts against a live world (`run_level`
  now returns a report, so a label that takes an error arm and a retail-missing sound are
  distinguishable), then checks every film it names is a real `SMK` in the archives and its
  successor is a real level. It asserts the light spine's 20 transitions in order, the `DECIDE`
  row's two films, and exactly two finale levels. Result on this install: 25 level files, 22
  cutscenes, 0 script errors, 1 sound missing from retail data (level 18's `LTSaberHit012.wav`,
  which the original engine also did not ship).
- `--cog-check` now fails on a label error (it previously only counted stubs), and `--gates` counts
  `BK_SKIP_WINDOW` rows as skipped instead of dropping them from the total.

At that round: **60 gates, 129 tests, 0 failures** (`--gates` windowed 0 of 60 failed; `BK_HEADLESS=1`
0 of 60 failed with 8 skipped); the round after this fixed the campaign chain and added the 61st gate. Still open for a *release* (not for the code): one human playthrough
of levels 2–21, the dark-side choice and ending screens, the AI instinct scheduler, health
carry-over, and the level-1 spawn view.

**Round 202 (release candidate, reviewed):** a fresh-eyes review of the release diff found three
more blockers behind the first-run ones; all are fixed and now gated.

- **The campaign stopped after level 2.** `swap_level` read a bare `gobs.read("episode.jk")` to
  resolve the successor. Every archive holds that name (`JK1`, `JK1MP`, `JK1CTF`), and the read
  returned the last one — the multiplayer manifest — so `flow_after` found no campaign row and the
  app printed `(end of campaign)` after level 2. It now takes the single-player episode through
  `episodes_of`, as `setup` does. `--tour-live` is the gate: the running app finishes and swaps all
  21 light-side levels and exits at `21ajerec.jkl` (it would previously have stopped at two).
- **Per-level completion state was process-wide.** `queue_next_level`'s `asked` and
  `watch_level_complete`'s `announced`/`tallied` locals never reset, so one session could only
  finish one level, and levels 2+ would skip their rank credit and tally. Both reset when
  `level_complete` clears on a fresh level.
- **The opening film played behind the main menu.** `play_intro_films` inserted the film at boot,
  `refresh_overlay` then suppressed the menu, and nothing removed a finished film: after 164 s the
  player had an invisible menu over a frozen last frame. It now waits for `overlay.started` (New
  Game), retires an intro film when it finishes, and is ordered after `swap_level`; normal launch
  was checked to show the menu with no film, and the New Game action to play `03-04a.smk`.
- **`--tour` now checks rows, not file names, on both arms.** The arms share level files
  (`18Ascend`, `19A`, `19B`), and a file-name lookup always took the light row. `Episode::flow_after_row`
  is used by the tour: 28 level rows, 25 distinct files, every film a real SMK, every successor a
  real level, both finale rows (light `21ajerec`/`52-53a.smk`, dark `21bjerec`/`57a.smk`), and the
  decision's two films in order. The app still resolves by name on its light walk, which is correct
  because the light rows come first in the manifest.
- **`--goal` regressed when the new system was added** — the renderer sighting probe depended on the
  scheduler's order between the probe, `walk_player` and `cull_sectors`. The probe now pins its pose
  every frame and the three systems are ordered explicitly; `--goal` passes repeatedly and reports
  `level complete in about 200 frames, next 02narshadda.jkl` (the exact frame is load-dependent).
- Smaller fixes: `RunReport.errors` no longer double-counts deep-pass errors; the Smacker helper is
  found next to the executable's checkout as well as the cwd (`BK_SMKDEC` overrides); the declared
  MSRV is Bevy's real one, **1.97.1**; the release note's audit line now quotes the whole summary.

Current: **61 gates** (`--gates` windowed `0 of 61 failed, 0 skipped`; `BK_HEADLESS=1`
`0 of 61 failed, 9 skipped`) and **130 tests**. `--tour`: 28 rows (25 files), 5543 things, 23 films,
2 terminal rows, 0 script errors, 1 sound missing from retail data. `--audit`: 540 cogs parsed,
1854 labels, 7683 symbols, 35/35 levels clean, 10 unresolved asset references, 57 undecoded files.
`--tour-live`: 21 levels finished and swapped through `21ajerec`. The campaign's two-film playback is
pinned by `film_step` unit tests and was watched once by hand (`38a` -> `39a` -> `16aEscapeShip`);
`--tour-live` names films instead of playing them, because 22 films is longer than a sweep should
take. The remaining publish blocker is the license, which needs the repository owner's decision.

**Round 203 (finding the install):** the port no longer carries a hardcoded developer path, and a
player can install it without knowing what `--game` means.

- **`src/gamepath.rs` resolves the retail install.** `--game` is validated and wins; then
  `BK_GAME`, a saved `game-path.txt` (beside the program, so a portable copy stays self-contained,
  then `%APPDATA%\bevi-knight\`), GOG's registry entries
  (`HKLM\SOFTWARE[\WOW6432Node]\GOG.com\Games` / `HKCU\...`, read through `reg.exe`, one query per
  key), and the folders GOG installs to (`<drive>:\GOG Games`, `<drive>:\Games`,
  `...\GOG Galaxy\Games`). A candidate must hold `JK.EXE` (or the other engine names) **and** a
  `Resource\` folder, so a launcher folder with the title in its path is rejected. `--find-game`
  prints the result and its source, `--set-game <path>` validates and saves it. The hardcoded
  `DEFAULT_GAME` is gone; a run without a resolvable install fails with instructions instead of
  loading the wrong folder.
- **`packaging/install.ps1` + `Install.cmd` is the per-user installer.** It searches the same
  places from PowerShell (native registry provider this time), asks for the `JK.EXE` folder when it
  finds nothing, checks it, copies `bevi-knight.exe`, `smkdec.exe` and the docs into
  `%LOCALAPPDATA%\Programs\bevi-knight`, writes `game-path.txt` beside the program, adds a Start
  Menu shortcut (desktop with `-DesktopShortcut`), and writes `uninstall.ps1`/`Uninstall.cmd`. No
  admin rights, no registry writes and no changes to the game install; `-NoShortcuts` and
  `-InstallDir` exist for scripted checks.
- **`packaging/package.ps1` builds the thing to hand a player:** release binary + i686 film helper
  + installer + README/release note, zipped as `dist\bevi-knight-<version>-windows.zip`.
- Verified: two new unit tests (install-layout validation, registry-line parsing) bring the suite
  to **132**; `--find-game` resolves via `--game`, the GOG registry, a portable `game-path.txt` and
  the `%APPDATA%` file; `--check` with no `--game` passes on the registry-discovered install; the
  installer was run to a temporary folder with an explicit path and with auto-discovery, the
  installed copy's `--check` passed and it found the game through its saved file, the uninstaller
  removed folder and shortcuts; the packaged zip was extracted and installed from, end to end.
  `--gates` still reads 0 of 61 failed after the startup change.

The licence, version-name and publish decisions are still the owner's (see
`docs/release-checklist.md`); the install path itself is now found, saved and forgotten about.

**Round 204 (a real Windows setup):** the PowerShell installer is replaced by **one self-contained
`setup.exe`**, built without installing any installer framework.

- `packaging/Setup.cs` is a WinForms wizard compiled by `packaging/build-setup.ps1` with the .NET
  Framework C# compiler and UI libraries that ship with Windows (the user's machine already has
  them; nothing is downloaded or installed to build it). It auto-detects the GOG install from the
  registry and the usual folders, accepts a browsed, pasted or **dragged** folder, validates
  `JK.EXE` + `Resource\`, installs to `%LOCALAPPDATA%\Programs\bevi-knight`, writes
  `game-path.txt`, creates Start Menu/desktop shortcuts, registers an **Apps & Features** entry
  and writes the payload (game, i686 film helper, docs) from a zip embedded as a resource.
- `packaging/Uninstall.cs` is the installed `bevi-knight-uninstall.exe`: confirm, remove the
  shortcuts, the Apps & Features key, the saved path and (after exit) the folder.
- Silent/automatable: `/S /GAMEPATH="<folder>" /DIR="<dir>" /NOSHORTCUTS`, or `/uninstall`; the
  silent path auto-detects when `/GAMEPATH` is omitted. `BK_SETUP_LOG` redirects the setup log for
  scripted runs.
- `packaging/build-setup.ps1` builds the release (and film helper), generates the icon from
  `docs/menu-main.png`, compiles the uninstaller, zips the payload, and compiles
  `dist\bevi-knight-<version>-setup.exe` (~28 MB) with the payload embedded. The old
  `install.ps1`/`Install.cmd`/`package.ps1` are deleted.
- Verified by hand: the GUI opens as `bevi-knight 0.1.0 setup` with the GOG install pre-filled;
  silent installs with an explicit path and with registry auto-discovery succeeded; the installed
  exe found the game through its saved file and passed `--check`; the Start Menu shortcut was
  created and removed; the uninstaller deleted the folder, shortcut and uninstall entry. The
  binary stays at 132 tests and 61 gates (this round adds no Rust code).

Still owner decisions: licence, final version name, whether to push — plus a human playthrough.

**Round 205 (license):** the tree now carries an explicit license instead of defaulting to
all-rights-reserved.

- **GPL-3.0-or-later** for the source, tools and setup scripts. `LICENSE` is the verbatim GPLv3
  text from gnu.org, `Cargo.toml` gained `license = "GPL-3.0-or-later"`, and README/release note
  state it. Contributions are inbound = outbound (no CLA).
- **`NOTICE`** carries the parts a code license cannot: no game data is included or licensed,
  *Star Wars Jedi Knight: Dark Forces II* and its contents belong to Lucasfilm/Disney, the
  project is unaffiliated, and the screenshots under `docs/` (retail artwork used as
  documentation) are excluded from the GPL grant. It also records that binary releases should
  keep the corresponding source available, and that dependencies keep their own MIT/Apache
  licenses.
- What the license does and does not do: it stops a **closed-source fork or proprietary
  repackage** (the source and the same freedoms must travel), but GPL does not forbid selling
  copies or support. The commercial ceiling here is really the game rights, which this project
  cannot license at all — the retail copy stays with the player.
- Remaining owner decisions are now version naming and pushing; the human playthrough is still
  the honest ceiling on "0.1".

**Round 206 (final version):** the release is **0.1.0** — the version in `Cargo.toml`, the
annotated tag, and the name the setup artifact takes.

- The earlier local `v0.1.0` tag pointed at `4ddb697` (23 Oct), 27 commits before this release,
  and `git ls-remote --tags origin` returned nothing: no tag had ever been pushed and
  `origin/main` was 278 commits behind. This is therefore the first published release, not a
  fix release, so re-pointing `v0.1.0` is more honest than inventing `0.1.1` for a build nobody
  received. The temporary `v0.1.1-rc.1` candidate tag was deleted.
- The release note's change list is retitled "What changed since the earlier 0.1 candidate" so
  it does not claim to follow a published `v0.1.0`.
- `packaging\build-setup.ps1` derives the artifact name from `Cargo.toml`, so the native setup
  is `bevi-knight-0.1.0-setup.exe`; it was rebuilt after the docs changed so the embedded
  release note and checklist are current.
- If the old local tag had ever been handed out as a binary, the correct move would be `0.1.1`;
  the repository state says it was not, and this note records that reasoning.

Remaining owner actions: explicit approval to push, and the human playthrough caveat.

**Round 207 (CI):** `.github/workflows/ci.yml` builds, tests and packages the port on
`windows-latest`.

- **Toolchain:** the workflow reads `rust-version` from `Cargo.toml` (1.97.1) and installs that
  exact toolchain with the `i686-pc-windows-msvc` target, so CI tests the MSRV rather than
  whatever the runner happens to ship. `cargo build --locked` and
  `cargo test --bins --release --locked` pin the two checked-in lock files.
- **What it produces:** the 132 unit tests, the 32-bit film helper, and the native
  `bevi-knight-0.1.0-setup.exe` through `packaging/build-setup.ps1 -SkipBuild`, uploaded as a
  14-day artifact. Cargo registry and build output are cached by lock-file hash.
- **Documentation:** `scripts/check-docs-links.ps1` resolves every relative link against the
  file that contains it (the failure mode that once left 34 broken links in the progress log);
  CI runs it, and it passes locally with 108 links across 7 files.
- **What is deliberately not in CI:** the 61 gates under `--gates` need the retail game install,
  which the repository does not ship and CI must not download. `cargo fmt --check` is not
  enforced yet because the tree is not rustfmt-clean, and clippy runs with ~140 warnings (no
  errors), so neither is a gate. Both are noted as future tightening, not hidden.
- Verified locally: the exact commands run here (build, tests, helper, setup, link check) pass,
  and the workflow YAML parses as one `windows` job with ten steps. Nothing has been pushed, so
  GitHub has not run it yet - the badge in the README will only go green after the first push.

Remaining owner actions: explicit approval to push, and the human playthrough caveat.

**Round 208 (CI green on the first pushes):** after approval, `main` and the annotated `v0.1.0`
tag were pushed, and GitHub Actions ran the workflow for real.

- First run (cold cache): green in **22m45s**, artifact `bevi-knight-setup`
  (29,585,772 bytes), every step passed. GitHub annotated `checkout@v4`, `cache@v4` and
  `upload-artifact@v4` as Node 20 actions forced onto Node 24.
- Follow-up `a00e577` bumps them to `checkout@v7`, `cache@v6`, `upload-artifact@v7` (same inputs
  used here). The cached rerun was green in **3m54s** with the setup artifact again uploaded
  (29,586,084 bytes).
- Repo state: `main` = `a00e577`; `v0.1.0` = `c941ccc` (the verified release commit). The release
  tag was not moved after publishing; its embedded setup docs are the ones built before the push.
- The 45-minute job timeout is comfortable for a cold Bevy build; cache restores are the
  difference between ~23 and ~4 minutes.

**Round 209 (player-facing README):** the README is now the front door, and the engineering
record moved behind it.

- The README opens with badges, what the port is (and is not), a status table, requirements, the
  CI-built setup and install/uninstall steps, first launch, an accurate controls table,
  troubleshooting, issue-reporting guidance (including "do not attach retail game files"), build
  instructions, verification and a documentation map. It is 205 lines instead of 894.
- Everything else - the screenshot gallery, the unit-by-unit status, the full flag catalogue,
  reverse-engineering notes, verification detail, "where the port stands", layout and camera
  recovery - moved verbatim to `docs/engineering.md`, with links rewritten for its location. The
  stale "default game path is G:\Steam\..." paragraph was replaced by the real discovery order
  while moving it.
- The release checklist's reference to the README's old "Open for 0.1" list now points at the
  status/known-limits section. Docs link check: 129 relative links across 8 files resolve.
- CI stayed green after the change (cached, 3m35s) and continued to build the setup artifact.

Still open: the GitHub release has no attached setup asset yet (the README points at the CI
artifact), plus the human playthrough caveat.
