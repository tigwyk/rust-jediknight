//! The world state COG scripts act on.
//!
//! The interpreter (`cog.rs`) runs every shipped script, but a script only becomes
//! game logic when its verbs reach real state. This module holds that state — thing
//! positions, per-thing flags, sector adjoin flags, inventory — and answers the verbs
//! that read or write it. Everything starts from the level file and is then mutated
//! by scripts, exactly as the engine does.

use std::collections::{BTreeMap, BTreeSet};

use bevy::math::{Quat, Vec3};

use crate::cog::Value;
use crate::collide::{Gates, SECTOR_ADJOINS_OFF};
use crate::jkl::Level;

/// Shared, single-threaded world state. Scripts hold a handle and borrow it briefly
/// per verb call; nothing here is concurrent. It is `Arc<Mutex<…>>` rather than
/// `Rc<RefCell<…>>` only because Bevy resources must be `Send + Sync`; the guard
/// methods keep every call site identical to a refcell.
#[derive(Clone)]
pub struct World(std::sync::Arc<std::sync::Mutex<Runtime>>);

impl World {
    pub fn new(runtime: Runtime) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(runtime)))
    }

    /// Take the runtime, or **fail loudly** if *this thread* already holds it.
    ///
    /// Two different mistakes look the same to `Mutex::lock`: one thread borrowing twice (a wiring
    /// bug, which used to hang a gate with no message) and two systems wanting the world at once
    /// (normal — Bevy runs systems in parallel, and waiting is correct). Only the first should
    /// panic, so the thread-local flag distinguishes them: a same-thread second borrow panics by
    /// name, cross-thread contention waits.
    #[track_caller]
    fn take(&self) -> Borrow<'_> {
        let here = std::panic::Location::caller();
        // Which line took the guard that is still alive. Without this the panic names the *second*
        // borrow (somewhere deep in a system) and leaves the first one to be guessed.
        if let Some(first) = HELD_AT.with(|at| at.replace(Some(here))) {
            panic!(
                "the runtime is already borrowed on this thread:\n  first borrowed at {first}\n  \
                 borrowed again at {here}\ndrop the guard you hold before borrowing again"
            );
        }
        match self.0.lock() {
            Ok(guard) => Borrow { guard, _not_send: std::marker::PhantomData },
            Err(e) => Borrow { guard: e.into_inner(), _not_send: std::marker::PhantomData },
        }
    }

    #[track_caller]
    pub fn borrow(&self) -> Borrow<'_> {
        self.take()
    }

    #[track_caller]
    pub fn borrow_mut(&self) -> Borrow<'_> {
        self.take()
    }
}

/// The player can release the runtime for scripts at a contact. Generic ticks already hold it,
/// so they queue messages for the existing tick dispatch.
enum ContactRuntime<'a> {
    Player { world: &'a World, eye: f32, dispatch: &'a mut dyn FnMut() },
    Tick(&'a mut Runtime),
}

impl ContactRuntime<'_> {
    fn with<T>(&mut self, f: impl FnOnce(&mut Runtime) -> T) -> T {
        match self {
            Self::Player { world, .. } => f(&mut world.borrow_mut()),
            Self::Tick(rt) => f(rt),
        }
    }

    fn dispatch(&mut self, state: &mut crate::collide::MoveState) {
        if let Self::Player { world, eye, dispatch } = self {
            Runtime::player_move_phase(world, state, *eye, dispatch);
        }
    }

    fn position(&self, state: &crate::collide::MoveState) -> [f32; 3] {
        let mut at = state.pos;
        if let Self::Player { eye, .. } = self { at[2] += *eye; }
        at
    }
}

thread_local! {
    /// Where this thread took the guard it still holds. The only writer is `take` (to record) and
    /// `Borrow`'s `Drop` (to clear), both on the same thread.
    static HELD_AT: std::cell::Cell<Option<&'static std::panic::Location<'static>>> =
        const { std::cell::Cell::new(None) };
}

/// A borrow of the runtime that clears this thread's "held" flag when it drops.
pub struct Borrow<'a> {
    guard: std::sync::MutexGuard<'a, Runtime>,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl std::ops::Deref for Borrow<'_> {
    type Target = Runtime;
    fn deref(&self) -> &Runtime {
        self.guard.deref()
    }
}

impl std::ops::DerefMut for Borrow<'_> {
    fn deref_mut(&mut self) -> &mut Runtime {
        self.guard.deref_mut()
    }
}

impl Drop for Borrow<'_> {
    fn drop(&mut self) {
        HELD_AT.with(|at| at.set(None));
    }
}

/// One sound a script asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundRequest {
    pub name: String,
    pub volume: f32,
    /// World position the sound comes from, when the verb named one.
    pub pos: Option<[f32; 3]>,
    /// The channel `PlaySound*` answered with: what `ChangeSoundVol`/`ChangeSoundPitch` name
    /// (`sithSoundMixer_GetChannelHandle`).
    pub channel: usize,
    /// `ChangeSoundPitch`'s multiplier for this channel (`sithSoundMixer_SetPitch`).
    pub pitch: f32,
    /// The engine call that queued it, with its address: the one-shot schedule's `source` column.
    /// It names the function that owns the sound, not the port's plumbing.
    pub source: &'static str,
}

/// A positional/ambient **loop** a script started (`PlaySoundThing`/`PlaySoundPos` with a loop or
/// ambient flag in its final argument, e.g. `00_PlayAmbient.cog`'s `0x045`). Retail's mixer keeps
/// these as loop channels next to the sector's `SOUND` loop; the capture's `ambient` column is the
/// loudest *playing* loop channel, not the sector's declared sound.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundLoop {
    pub name: String,
    pub volume: f32,
    pub pos: Option<[f32; 3]>,
    pub min_radius: f32,
    pub max_radius: f32,
    pub flags: i32,
}

/// `RotatePivot(thing, frame, seconds)`: spin a thing about the pivot its own `frame=` entry names.
///
/// The engine keeps this on the track parameters (`curOrient.scale` is the pivot, `moveFrameDeltaAngles`
/// the rotation, `field_54 = 1 / seconds` the rate), which is what this is. The shipped users are an
/// orbiting camera (a fulcrum the camera is attached to, `00_cameratricks.cog`), level 3's pool
/// paddles and level 6's swinging door.
#[derive(Clone)]
pub struct PivotSpin {
    /// The frame's position: what the thing turns *around*.
    pub centre: [f32; 3],
    /// The frame's angles, in degrees: the rotation it turns *by*.
    pub angles: [f32; 3],
    pub duration: f32,
    pub done: f32,
    /// The pose the spin started from, so the whole turn is a function of progress rather than an
    /// accumulation that drifts.
    pub start_pos: [f32; 3],
    pub start_angles: [f32; 3],
}

/// One observation the `BK_ENCOUNTER_OUT` trace writes: a shot as it leaves a shooter, or damage
/// that landed on the local player. Both carry the runtime's own clock, so the encounter TSV is a
/// game-time timeline rather than a sample of wall time.
#[derive(Clone, Debug)]
pub enum EncounterEvent {
    Shot {
        t: f32,
        owner: usize,
        template: String,
        pos: [f32; 3],
        /// What the shooter was aiming at when it fired: the AI's own target thing, or `None` for a
        /// shot nothing names (the player's weapon, a scripted `FireProjectile`).
        target: Option<usize>,
    },
    /// A fired shot that landed on the player: its launch time and shooter, written as the trace's
    /// `shot` row. Retail's fixture can only see shots through their impact on the player, so this
    /// is the comparable observation; the plain `Shot` event is the spawn record.
    ShotLanded {
        t: f32,
        owner: usize,
        template: String,
        pos: [f32; 3],
        target: Option<usize>,
    },
    /// A fired shot that died without landing on the player — a wall, an actor, or its own `timer=`.
    /// Written as the trace's `miss` row, so the chronology says on every shot hit or miss.
    Miss {
        t: f32,
        owner: usize,
        template: String,
        target: Option<usize>,
    },
    /// The player was hit: the amount the two bars actually lost, the bar values after the hit, and
    /// the per-hit split the bars took — the same numbers the retail capture writes from
    /// `thing + 0x134` and inventory bin 60.
    Hit {
        t: f32,
        amount: f32,
        shields: f32,
        health: f32,
        /// Health and shields this one hit removed (the per-hit split).
        health_lost: f32,
        shields_lost: f32,
    },
}

/// One origin actor's AI phase from the retail engine capture (issue #21): the flag word the
/// engine's AI block held and the instinct deadlines it had already scheduled at the origin clock.
/// The capture is the only source — a run that names none keeps the port's own defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapturedPhase {
    /// `ai_flags`: the engine's per-actor flag word (`ai + 8`).
    pub flags: u32,
    /// `fire_next_ms`: the next legal shot, absolute on the engine clock. `0` means nothing is
    /// scheduled and `-1` that the class carries no fire instinct.
    pub fire_ms: i64,
    /// `acquire_next_ms`: the `LookForTarget` scan's own next run.
    pub acquire_ms: i64,
    /// `next_pass_ms`: the instinct scheduler's next pass over this actor — the earliest deadline of
    /// its instincts, so it is the shot for a gunner already firing and the scan otherwise.
    pub next_pass_ms: i64,
}

/// Whether the door-encounter telemetry is on (`BK_ENCOUNTER_OUT`). A process-wide check: the flag
/// is set once by the capture harness, and queuing an event per shot is only wanted then.
fn encounter_capture() -> bool {
    static ON: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("BK_ENCOUNTER_OUT").is_some());
    *ON
}

#[derive(Clone, Default)]
pub struct ThingState {
    /// `0x4fbe10`'s re-touch clock (`thing+0x148`, engine ms): `touched` is suppressed until it
    /// passes, then set 500 ms ahead. 0 = never touched.
    pub next_touch_ms: u32,
    pub insert: [f32; 3],
    pub pos: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// Visual head pitch in degrees: `sithActor_SetHeadPYR`'s `thing+0x15c`, clamped by the
    /// template's `minheadpitch`/`maxheadpitch`. `pitch` stays the body/aim angle.
    pub head_pitch: f32,
    pub sector: i32,
    pub flags: u32,
    pub health: f32,
    /// Position along the thing's keyframe path, fractional while moving.
    pub frame: f32,
    /// Where `MoveToFrame` is heading, and how fast.
    pub target: Option<(f32, f32)>,
    /// The thing's own keyframe path from the level file.
    pub frames: Vec<[f32; 6]>,
    /// `DestroyThing`: gone from the world, skipped by the sector iteration.
    pub destroyed: bool,
    /// Straight-line velocity, in level units per second: what a fired projectile uses.
    pub vel: [f32; 3],
    /// Where this actor's AI is walking to, if anywhere.
    pub ai_target: Option<[f32; 3]>,
    /// Seconds until this actor's AI may fire again; its instinct class lives in
    /// `Runtime::ai`.
    pub ai_wait: f32,
    /// Shots left in the current burst: `sithAICmd_PrimaryFire`'s `fltArg[8]` counter. While it is
    /// positive the wait is the burst gap (`fltArg[9]`), not the full shot delay.
    pub ai_burst: f32,
    /// Set by `ThingState::step` on the tick a frame path reaches its target frame, so the caller
    /// can send `arrived` (the message a door or lift script waits on).
    pub arrived: bool,
    /// Damage a flying projectile still carries, its floor (`mindamage=`), and how fast it
    /// loses it (`rate=`, when the typeflags carry `SITH_WF_DAMAGE_DECAY` 0x4000).
    pub damage: f32,
    pub min_damage: f32,
    pub decay: f32,
    /// `SetActorExtraSpeed`: an *additive* thrust term the force-speed script sets (and
    /// clears to 0 when the power ends). The engine adds it to `maxThrust`, so it converts
    /// to a settled speed the same way `maxthrust` does: `extra / drag`.
    pub extra_speed: f32,
    /// `PathMovePause`: the thing keeps its destination but stops advancing —
    /// `sithTrackThing_PathMovePause` sets `trackParams.flags |= 0x80` and `...Resume` clears it.
    /// Level 9's pipe doors stop in the doorway this way (`09_stoppipedoors.cog`).
    pub paused: bool,
    /// `SetThingPulse(thing, seconds)`: the thing's own pulse interval and when the next one is due.
    /// Every `seconds` the engine messages the thing's own script slots with `pulse` — the blood
    /// trickle `00_backpack` starts, and the damage cadence of `00_deathfloor`/`00_slideceiling`.
    pub pulse: f32,
    pub pulse_at: f32,
    /// `GetPhysicsFlags`/`SetPhysicsFlags` (`physflags=`), and the `jkSetFlags` bits the
    /// force scripts set (`0x20` while a power is mid-flash).
    pub phys_flags: u32,
    pub jk_flags: u32,
    /// `AttachThingToThing(child, parent)`: the parent this thing rides, and where it sits in that
    /// parent's own frame — `sithThing_AttachThingToThing` stores exactly that offset (in
    /// `field_4C`), so a moving parent carries the child.
    pub attach: Option<usize>,
    /// The floor seat `sithPhysics_FindFloor` (`0x4f5550`) records — the surface or thing face
    /// the body stands on (the engine's attach kind `thing+0x34` plus its surface pointer).
    /// `None` is a body in the air.
    pub floor_attach: Option<crate::collide::Attach>,
    /// The thing that fired this projectile, so `impacts` can skip it: the engine carries the
    /// owner on the shot, and a muzzle inside the shooter's own body radius needs it.
    pub owner: Option<usize>,
    pub attach_offset: [f32; 3],
    /// `SetThingAttachFlags`: stored, and `SITH_ATTACH_NOMOVE` (0x2) is the one read.
    pub attach_flags: u32,
    /// `RotatePivot`'s in-flight spin, if any.
    pub spin: Option<PivotSpin>,
    /// `SetThingRotVel`: `physicsParams.angularVelocity`, in degrees per second about the thing's
    /// own axes. The engine integrates it into the orientation every physics step, which is how a
    /// hurled crate tumbles.
    pub rot_vel: [f32; 3],
    /// `ThingLight`/`SetThingLight`/`ThingLightAnim`: how much light this thing emits. Retail uses it
    /// in the dynamic-light pass, which this build does not have, so it is summed into the light of
    /// the sector the thing stands in — a lamp brightens its room and a broken one stops.
    /// `light_end`/`light_rate` are the ramp (`sithSurface_SetThingLight`'s `field_40`/`field_44`).
    pub light: f32,
    pub light_end: f32,
    pub light_rate: f32,
}

impl ThingState {
    /// One animation step: advance `frame` toward the target and re-derive the pose.
    /// Returns whether the thing moved.
    pub fn step(&mut self, dt: f32) -> bool {
        // `SetThingRotVel`'s angular velocity, integrated the way `sithPhysics` integrates it: the
        // orientation advances by `rotVel * dt` every step, and the angles wrap so a long spin does
        // not drift into a huge number.
        if self.rot_vel != [0.0; 3] {
            self.yaw = (self.yaw + self.rot_vel[1] * dt).rem_euclid(360.0);
            self.pitch = (self.pitch + self.rot_vel[0] * dt).rem_euclid(360.0);
            self.roll = (self.roll + self.rot_vel[2] * dt).rem_euclid(360.0);
        }
        // `PathMovePause` keeps the destination: the thing stands still where the pause caught it
        // and `PathMoveResume` carries on toward the same frame.
        if self.paused {
            return self.rot_vel != [0.0; 3];
        }
        let Some((target, speed)) = self.target else { return false };
        let delta = target - self.frame;
        let step = speed.max(0.001) * dt;
        let mut arrived = false;
        if delta.abs() <= step {
            self.frame = target;
            self.target = None;
            // The frame path's end: `SITH_MESSAGE_ARRIVED` is what a door or lift script waits on
            // after `MoveToFrame`, and level 1 declares it 19 times.
            arrived = true;
        } else {
            self.frame += step * delta.signum();
        }
        if self.frames.is_empty() {
            return true;
        }
        let last = self.frames.len() - 1;
        let f = self.frame.clamp(0.0, last as f32);
        let i = (f.floor() as usize).min(last);
        let j = (i + 1).min(last);
        let t = f - i as f32;
        let (a, b) = (self.frames[i], self.frames[j]);
        for k in 0..3 {
            self.pos[k] = a[k] + (b[k] - a[k]) * t;
        }
        self.pitch = a[3] + (b[3] - a[3]) * t;
        self.yaw = a[4] + (b[4] - a[4]) * t;
        self.roll = a[5] + (b[5] - a[5]) * t;
        self.arrived = arrived;
        true
    }

    pub fn is_moving(&self) -> bool {
        self.target.is_some()
    }
}

/// Force mana per second, and the pool ceiling.
///
/// The ceiling is **data**: level 1 starts the player with 400 and `force_heal.cog` takes it to 200
/// (§16.5), so the pool is 400. The *rate* is calibrated — the engine regenerates in `jkPlayer.c`,
/// which this source tree does not carry — chosen to refill a spent pool in about twenty seconds.
pub const MANA_REGEN: f32 = 20.0;
/// See `MANA_REGEN`.
pub const MANA_MAX: f32 = 400.0;

/// How fast a grabbed thing is dragged in, how far a pull reaches, and where it stops.
///
/// All three are **calibrated**, not recovered: `jkSetTarget`'s drag lives in `jkPlayer.c`, which this
/// source tree does not carry. The reach is about three body lengths and the stop is arm's length, so a
/// pull brings a crate to the player instead of into them.
pub const PULL_SPEED: f32 = 12.0;
/// See [PULL_SPEED].
pub const PULL_RANGE: f32 = 30.0;
/// See [PULL_SPEED].
pub const PULL_REACH: f32 = 2.0;
/// `SITHBIN_GOAL00` (`types_enums.h`): the first of the goal counters `SetGoalFlags` writes.
pub const GOAL_BIN: i32 = 100;
/// The `.jks` section kind this build puts [`Snapshot::to_bytes`]'s payload in. The engine's own
/// records are 0x0C (things) and up; this claims an unused kind so a retail save and a port save can
/// be told apart by their contents rather than by their name.
pub const SAVE_SECTION: u32 = 0x1001;
/// `SithDamageType`: what kind of hurt, as a bitmask. Only the bits this port distinguishes.
pub const DAMAGE_IMPACT: u32 = 0x01;
pub const DAMAGE_ENERGY: u32 = 0x02;
/// Fire (0x04) is the other kind `item_shields.cog` protects, beside energy.
pub const DAMAGE_FIRE: u32 = 0x04;
pub const DAMAGE_SABER: u32 = 0x10;
pub const DAMAGE_FALL: u32 = 0x40;
/// `sithActor_PlayDamageSoundFx` (`0x4ece90`) only cries out for a blow of at least **3** points
/// (`_DAT_005230c4`), at a volume of `damage / health * 1.5` (`_DAT_005230c8`) clamped to 1.0, and
/// stays silent under `0.01` (`_DAT_005230cc`). The health in that ratio is `thing + 0x134`, read
/// *after* the subtraction, so a bigger share of what is left is a louder cry.
pub const DAMAGE_SOUND_MIN: f32 = 3.0;
pub const DAMAGE_SOUND_SCALE: f32 = 1.5;
pub const DAMAGE_SOUND_FLOOR: f32 = 0.01;
/// `SITH_AF_INVULNERABLE`: `actor->flags` bit 3. `sithActor_DamageActor` checks it before anything
/// else, so `SetActorFlags(player, 0x8)` is how a script makes someone untouchable.
pub const AF_INVULNERABLE: u32 = 0x8;
/// The actor flag that exempts a hit from `sithActor_DamageActor`'s actor-to-actor scale: when the
/// attacker is an actor whose `+0x4c` word lacks this bit, damage dealt to another actor is
/// multiplied by **0.1** (`_DAT_005230b0`). No shipped script sets it (`SetActorFlags` values in the
/// corpus are 0x4/0x8/0x200/0x2000/0xA00000), so in practice AI-versus-AI damage is a tenth.
pub const AF_ACTOR_FULL_DAMAGE: u32 = 0x1000000;
/// `SITH_SECTOR_COGLINKED`: a cog is placed on this sector, so it can hear `sighted`/`entered`.
pub const SECTOR_COGLINKED: u32 = 0x4;
/// `SITH_SECTOR_SEEN`: latched the first time the renderer draws the sector.
pub const SECTOR_SEEN: u32 = 0x4000;
/// `SITH_TF_CAPTURED`: the thing has a cog placed on it (`sithRender_RenderThing` is what checks it).
pub const TF_CAPTURED: u32 = 0x400;
/// `SITH_TF_INCAMFOV`: latched the first time the renderer draws the thing.
pub const TF_INCAMFOV: u32 = 0x100000;

/// The `sourceType`/`senderType` words `sithCog_SendMessageEx` (`0x4dedc0`) stores.
///
/// The engine's senders pass 3 for a thing, 5 for a sector and 6 for a surface as the message's
/// source, and either 0 (the world: sender index -1) or 3 (a thing) as the sender. `GetSenderId()`
/// answers the link id, not these; `GetSenderType()`/`GetSenderRef()` answer the sender half.
pub const THING_SOURCE: i32 = 3;
pub const SECTOR_SOURCE: i32 = 5;
pub const SURFACE_SOURCE: i32 = 6;

/// `SITH_SURFACE_COG_LINKED` (2): `sithCog_LinkCog` (`0x4df610:67`) ORs it into a surface's
/// surfflags when a cog links it; `sithThing_AttachThingToSurface` (`0x4cde80:50`) tests it
/// before sending `entered`.
pub const SITH_SURFACE_COG_LINKED: u32 = 2;

/// What the player's use action hit, in the engine's own two shapes (`sithPlayerActions_Activate`,
/// `0x4fc0a0`): a linked thing's collision, or a linked surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseHit {
    Thing(usize),
    Surface(usize),
}

/// The identity `sithCog_SendMessageEx` (`0x4dedc0`) delivers beside a message.
///
/// The engine's argument order is `(cog, msg, sourceType, sourceIndex, senderType, senderIndex,
/// param0, f0..f3)`; it stores `param0` — the **link id** of the link that matched, which is what
/// `GetSenderId()` reads — at `+0x24`, the source index at `+0x28` with its type at `+0x2c`, and the
/// sender's index at `+0x30` with its type at `+0x34`. Each `*SendMessageEx` normalises a null
/// sender to `(-1, 0)` and a thing sender to `(thing index, 3)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceSender {
    pub source_type: i32,
    pub source: i32,
    pub sender_type: i32,
    pub sender: i32,
}

impl SourceSender {
    pub fn new(source_type: i32, source: i32, sender: Option<usize>) -> Self {
        match sender {
            Some(thing) => Self {
                source_type,
                source,
                sender_type: THING_SOURCE,
                sender: thing as i32,
            },
            None => Self { source_type, source, sender_type: 0, sender: -1 },
        }
    }
}

/// The JKL `thingflags` difficulty bits: `0x1000` not in Easy, `0x2000` not in Medium, `0x4000`
/// not in Hard. The loader does not create a placement the active difficulty excludes. The port
/// runs Medium (`GetDifficulty` answers 1), so a row carrying this bit is not spawned.
pub const TF_NOT_MEDIUM: u32 = 0x2000;
/// A template's `type=` word as the engine's `SithThingType` number.
///
/// From `types_enums.h`: camera 1, actor 2, weapon 3, debris 4, item 5, explosion 6,
/// player 10, particle 11, hint 12, sprite 13, polyline 14. `cog` and the other words a
/// level uses for inert placement are type 0 (free), which is what `GetThingType`
/// reports for them.
/// A vector parameter as the level writes it: `vel=(0.000000/4.000000/0.000000)`.
fn vec_param(raw: Option<&str>) -> Option<[f32; 3]> {
    let text = raw?.trim().trim_start_matches('(').trim_end_matches(')');
    let mut parts = text.split('/');
    let x = parts.next()?.trim().parse().ok()?;
    let y = parts.next()?.trim().parse().ok()?;
    let z = parts.next()?.trim().parse().ok()?;
    Some([x, y, z])
}


/// The radius a placed thing's **collision volume** has, from the engine's own template rule.
///
/// `misc\things.dat`'s class rows carry `collide=`: `_structure` is `collide=3` and `_decor` is
/// `collide=0`, and `_walkstruct` inherits the first through its `_structure` base. Every placed
/// template carries its own `size=` (the radius), so a thing is solid exactly when its chain's
/// `collide` is non-zero. That is why `column01` (`_structure`, `size=0.204987`) stops a body in
/// retail while `tuskensoused` (`_ghostdecor`) does not.
///
/// `collide=3` selects the engine's model-mesh collision, which the walker models as the model's
/// bounds box (`SolidShapes`); this function still supplies the `size=` fallback for a template
/// whose model did not load and the movement sphere for the other collidable values.
///
/// Returns `None` for a thing the engine lets a body pass through.
pub fn solid_radius(level: &crate::jkl::Level, template: &str) -> Option<f32> {
    let collide = level.template_param(template, "collide")?.trim().parse::<i32>().ok()?;
    if collide == 0 {
        return None;
    }
    // The radius is the template's own `size=`; `movesize=` is the same for everything placed. The
    // walker uses the model's bounds box instead when `collide=3` (the engine's mesh case); this
    // radius remains the engine's movement sphere for every other collidable thing.
    let size = level.template_param(template, "size")?.trim().parse::<f32>().ok()?;
    (size > 0.0).then_some(size)
}
/// A placed template's thing type (`type=camera`, `actor`, `item`, ...), for probes and for the camera
/// slots: `thing_type` is the file-format word table, this is the level lookup around it.
pub fn thing_type_of(level: &Level, template: &str) -> i32 {
    thing_type(level.template_param(template, "type"))
}

fn thing_type(word: Option<&str>) -> i32 {
    match word.map(str::to_ascii_lowercase).as_deref() {
        Some("camera") => 1,
        Some("actor") => 2,
        Some("weapon") => 3,
        Some("debris") => 4,
        Some("item") => 5,
        Some("explosion") => 6,
        // `sithThing_Startup` (0x4cc130) registers the name table 1..17; `_decor` — the base of
        // every `_structure`/`_walkstruct` furniture chain — is `type=cog`, so placed furniture is
        // type 7 (`0x4e8430`'s `0x2000` gate, the (10,7) pair cell).
        Some("cog") => 7,
        Some("player") => 10,
        Some("particle") => 11,
        Some("hint") => 12,
        Some("sprite") => 13,
        Some("polyline") => 14,
        _ => 0,
    }
}

/// Per-surface state the wall/cel verbs mutate: the texture's current cel, its animation
/// rate, the offset it has been slid by, and its centre for `SurfaceCenter`.
#[derive(Clone)]
pub struct SurfaceState {
    pub center: [f32; 3],
    pub cel: f32,
    pub anim: f32,
    pub slide: [f32; 3],
    /// When the next cel is due, on the runtime's clock — the engine's `rdSurface.field_30`.
    pub next: f32,
    /// `SurfaceLightAnim`'s extra light, added to the level's baked intensity: `start` when the ramp
    /// begins, `end` where it settles, and the rate it walks at.
    pub light: f32,
    pub light_end: f32,
    pub light_rate: f32,
}

/// One thing's saved state: everything about it a save has to put back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThingSave {
    pub template: String,
    pub pos: [f32; 3],
    pub angles: [f32; 3],
    pub sector: i32,
    pub health: f32,
    pub flags: u32,
    pub jk_flags: u32,
    pub destroyed: bool,
    pub frame: f32,
}

/// A whole savegame: the things, what the player carries, and the clock.
///
/// `sithGamesave.c` writes a thing record per thing plus the player block and the per-sector and
/// per-surface state; this is the same list, in this build's own layout (the container is still a
/// `.jks`, so the engine's prefix is written and a retail file still parses). Cog VM state is **not**
/// here: a save that restores the world but restarts every script is the ceiling this marks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub time: f32,
    pub things: Vec<ThingSave>,
    pub inv: BTreeMap<i32, f32>,
    pub weapon: Option<String>,
    pub cur_bin: i32,
    pub saber: Option<SaberInfo>,
    pub adjoin_flags: Vec<u16>,
    pub sector_light: Vec<f32>,
}

/// The snapshot's on-disk bytes. Little-endian, length-prefixed strings, one record per thing:
/// `sithGamesave.c` writes the same information into a `.jks`, and [`crate::jks`] carries that
/// container; this is the payload that goes inside one of its sections.
impl Snapshot {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"BKSV");
        out.extend_from_slice(&self.time.to_le_bytes());
        out.extend_from_slice(&(self.things.len() as u32).to_le_bytes());
        for thing in &self.things {
            let name = thing.template.as_bytes();
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(name);
            for value in thing.pos.iter().chain(thing.angles.iter()).chain([&thing.health]) {
                out.extend_from_slice(&value.to_le_bytes());
            }
            out.extend_from_slice(&thing.sector.to_le_bytes());
            out.extend_from_slice(&thing.flags.to_le_bytes());
            out.extend_from_slice(&thing.jk_flags.to_le_bytes());
            out.push(u8::from(thing.destroyed));
            out.extend_from_slice(&thing.frame.to_le_bytes());
        }
        out.extend_from_slice(&(self.inv.len() as u32).to_le_bytes());
        for (bin, amount) in &self.inv {
            out.extend_from_slice(&bin.to_le_bytes());
            out.extend_from_slice(&amount.to_le_bytes());
        }
        let weapon = self.weapon.as_deref().unwrap_or("");
        out.extend_from_slice(&(weapon.len() as u16).to_le_bytes());
        out.extend_from_slice(weapon.as_bytes());
        out.extend_from_slice(&self.cur_bin.to_le_bytes());
        match &self.saber {
            Some(blade) => {
                out.push(1);
                for text in [&blade.side, &blade.tip, &blade.wall_sparks, &blade.blood_sparks, &blade.saber_sparks] {
                    let bytes = text.as_bytes();
                    out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
                    out.extend_from_slice(bytes);
                }
                for value in [blade.base_rad, blade.tip_rad, blade.length] {
                    out.extend_from_slice(&value.to_le_bytes());
                }
            }
            None => out.push(0),
        }
        out.extend_from_slice(&(self.adjoin_flags.len() as u32).to_le_bytes());
        for flag in &self.adjoin_flags {
            out.extend_from_slice(&flag.to_le_bytes());
        }
        out.extend_from_slice(&(self.sector_light.len() as u32).to_le_bytes());
        for light in &self.sector_light {
            out.extend_from_slice(&light.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(data: &[u8]) -> Option<Snapshot> {
        let mut at = Reader { data, at: 0 };
        if at.take(4)? != b"BKSV" {
            return None;
        }
        let mut save = Snapshot { time: at.f32()?, ..Snapshot::default() };
        for _ in 0..at.u32()? {
            save.things.push(ThingSave {
                template: at.text()?,
                pos: [at.f32()?, at.f32()?, at.f32()?],
                angles: [at.f32()?, at.f32()?, at.f32()?],
                health: at.f32()?,
                sector: at.i32()?,
                flags: at.u32()?,
                jk_flags: at.u32()?,
                destroyed: at.u8()? != 0,
                frame: at.f32()?,
            });
        }
        for _ in 0..at.u32()? {
            save.inv.insert(at.i32()?, at.f32()?);
        }
        let weapon = at.text()?;
        save.weapon = (!weapon.is_empty()).then_some(weapon);
        save.cur_bin = at.i32()?;
        if at.u8()? != 0 {
            save.saber = Some(SaberInfo {
                side: at.text()?,
                tip: at.text()?,
                wall_sparks: at.text()?,
                blood_sparks: at.text()?,
                saber_sparks: at.text()?,
                base_rad: at.f32()?,
                tip_rad: at.f32()?,
                length: at.f32()?,
            });
        }
        for _ in 0..at.u32()? {
            save.adjoin_flags.push(at.u16()?);
        }
        for _ in 0..at.u32()? {
            save.sector_light.push(at.f32()?);
        }
        Some(save)
    }
}

/// A cursor over a snapshot's bytes.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let slice = self.data.get(self.at..self.at + n)?;
        self.at += n;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn text(&mut self) -> Option<String> {
        let len = self.u16()? as usize;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
}

/// `NewColorEffect`'s screen effect: `stdPalEffects`' filter/tint/add channels and a fade.
#[derive(Clone, Debug, Default)]
pub struct ColorEffect {
    /// Per-channel mask (0/1) — the `filter` arguments.
    pub filter: [i32; 3],
    pub tint: [f32; 3],
    /// Additive brightness, in the engine's 0..255 scale.
    pub add: [i32; 3],
    /// Seconds until it expires.
    pub fade: f32,
    /// The level time it was created at, so the fade can be measured.
    pub started: f32,
}

/// `sithPlayer_AddDynamicTint`: how much red a point of damage adds to the player's palette tint.
///
/// `sithActor_DamageActor` (0x4ecc90) calls `sithPlayer_AddDynamicTint(damage * 0.04, 0, 0)` for
/// the local player; the 0.04 is `DAT_005230b4`.
pub const DAMAGE_TINT: f32 = 0.04;
/// `sithPlayer_Update`'s decay on every dynamic-tint channel, per second (`DAT_00522e38`).
pub const TINT_DECAY: f32 = 0.4;
/// `sithPlayer_AddDynamicTint`'s ceilings after adding: red and green stop at 1.0
/// (`DAT_00522e4c`), blue at 0.5 (`DAT_00522e58`).
pub const TINT_CEILING: [f32; 3] = [1.0, 1.0, 0.5];

/// `sithPlayer_AddDynamicTint` (`0x4c8c10`): **add** to the player's palette tint and clamp each
/// channel to its ceiling. The damage flash and the `AddDynamicTint` cog verb share these channels,
/// which is why this replaces nothing.
fn add_dynamic_tint(tint: &mut [f32; 3], add: [f32; 3]) {
    for k in 0..3 {
        tint[k] = (tint[k] + add[k]).clamp(0.0, TINT_CEILING[k]);
    }
}

/// [`Runtime::carry`]: the player's own state, which survives a level change.
#[derive(Clone, Debug, Default)]
pub struct CarryOver {
    pub inv: BTreeMap<i32, f32>,
    pub weapon: Option<String>,
    pub cur_bin: i32,
    pub saber: Option<SaberInfo>,
    pub jk_flags: u32,
    /// Health and the shield bar are `jkPlayer_*` state too, so a level change keeps them.
    pub health: f32,
    pub shields: f32,
}

/// `jkSetSaberInfo`'s blade, as `jkSaber_InitializeSaberInfo` stores it: the two blade materials,
/// the base and tip radii, the length, and the three spark templates a hit spawns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SaberInfo {
    pub side: String,
    pub tip: String,
    pub base_rad: f32,
    pub tip_rad: f32,
    pub length: f32,
    pub wall_sparks: String,
    pub blood_sparks: String,
    pub saber_sparks: String,
}

#[derive(Clone)]
pub struct SectorState {
    /// `SetSectorThrust(sector, vector)`: the force the sector applies.
    pub thrust: [f32; 3],
    /// `SetSectorLight(sector, value, time)`: the **extra** light the sector carries, on top of what
    /// the level baked into its vertices. Zero is the level's own lighting, which is why every sector
    /// starts there and a door that adds `0.5` brightens its room by half.
    pub light: f32,
    /// Where the fade started, where it is going and how fast, for a non-zero `time` argument. A
    /// rate of 0 is a settled value.
    pub light_from: f32,
    pub light_end: f32,
    pub light_rate: f32,
    pub tint: [f32; 3],
    pub colormap: i32,
}

/// Template name to its model INSERT OFFSET, filled once when the level models are parsed.
///
/// ponytail: one process-global table, because `Runtime::handle` only sees the level and the offsets live
/// on the parsed `.3do` files. This build draws one level at a time, the same ceiling the light ramp has;
/// loading two levels at once would need the map passed in instead.
static INSERT_OFFSETS: std::sync::OnceLock<BTreeMap<String, [f32; 3]>> = std::sync::OnceLock::new();

/// Record the insert offsets for a level's templates, once the models are parsed.
pub fn set_template_inserts(map: BTreeMap<String, [f32; 3]>) {
    let _ = INSERT_OFFSETS.set(map);
}

/// A template's model INSERT OFFSET, or zero when it has no model.
///
/// The z component is the engine's `sithPhysics_GetThingHeight` fallback: the height of the thing's
/// model, which is also why a placed thing's `pos` sits that far above its feet (`ky.3do` 0.120114,
/// `c3x10.3do` -0.012306).
pub fn template_insert(template: &str) -> [f32; 3] {
    INSERT_OFFSETS
        .get()
        .and_then(|m| m.get(template))
        .copied()
        .unwrap_or([0.0; 3])
}

/// Rotate a vector by a thing's `pitch yaw roll`, exactly as `rdMatrix_Build34` orients a model.
///
/// `CreateThing` positions a new thing at `marker.position + marker_orient * (insertDiff)`, so this is what
/// puts a spawned prop where the engine puts it. Degrees, same as everywhere else in the level files.
pub fn rotate_by_angles(v: [f32; 3], pitch: f32, yaw: f32, roll: f32) -> [f32; 3] {
    let (sx, cx) = pitch.to_radians().sin_cos();
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sz, cz) = roll.to_radians().sin_cos();
    let r = [-(sz * sy) * sx + (cz * cy), ((sz * cy) * sx) + (cz * sy), -sz * cx];
    let l = [-sy * cx, cy * cx, sx];
    let u = [((cz * sy) * sx) + (sz * cy), -sx * (cy * cz) + (sy * sz), cz * cx];
    [
        r[0] * v[0] + l[0] * v[1] + u[0] * v[2],
        r[1] * v[0] + l[1] * v[1] + u[1] * v[2],
        r[2] * v[0] + l[2] * v[1] + u[2] * v[2],
    ]
}
/// The flat forward vector of a yaw in degrees: the engine's `lvec`, whose z the turn blend keeps
/// flattened (`sithAI_SetLookFrame` zeroes the look vector's z before blending).
fn forward_of_yaw(yaw: f32) -> [f32; 2] {
    let (sin, cos) = yaw.to_radians().sin_cos();
    [-sin, cos]
}

/// The yaw of a flat unit vector: the inverse of `forward_of_yaw`.
fn yaw_of_forward(f: [f32; 2]) -> f32 {
    (-f[0]).atan2(f[1]).to_degrees()
}

/// One tick of the engine's turn blend, `sithAI_sub_4EA630` (`0x4ea630`) on a yaw-only body.
///
/// While the AI flag `0x8` is set (written by `ai_set_look_frame`), the body's forward vector is
/// blended toward the stored look vector by `forward += look * maxrotthrust * _DAT_00523058 * dt`
/// (the constant is **0.1**, read from `JK.EXE`'s `.rdata`) and renormalised — an ease-out vector
/// blend, not a constant angular rate. The blend ends, clearing `0x8`, when the look vector is
/// within `_DAT_00523054` = **0.01** of the body's right axis and not behind it. The body's right
/// vector is rebuilt flat as `(lvec.y, -lvec.x)`, which is what the engine's cross product with the
/// world up produces.
fn turn_blend(forward: [f32; 2], look: [f32; 2], rotthrust: f32, dt: f32) -> ([f32; 2], bool) {
    let rvec = [forward[1], -forward[0]];
    let along = forward[0] * look[0] + forward[1] * look[1];
    let across = (rvec[0] * look[0] + rvec[1] * look[1]).abs();
    if across <= 0.01 && along >= 0.0 {
        return (forward, true);
    }
    let k = rotthrust * 0.1 * dt.max(0.0);
    // Dead astern (`0x4ea630`): the engine steps along the **right vector** there
    // (`fVar6 = *pfVar2 * param_2 + *(float *)(iVar3 + 0x94)`), because `look * k` against an
    // opposite forward renormalises back to the same forward — the blend would never turn a
    // target exactly behind. Every other case steps along the look vector.
    let step = if along < 0.0 && across <= 0.01 { rvec } else { look };
    let blended = [forward[0] + step[0] * k, forward[1] + step[1] * k];
    let len = (blended[0] * blended[0] + blended[1] * blended[1]).sqrt();
    if len < 0.01 {
        // `rdVector_Normalize3` of a collapsed blend copies the look vector.
        return (look, false);
    }
    ([blended[0] / len, blended[1] / len], false)
}

/// `sithAI_Create` (`0x4e9cb0`) seeds every AI's movespeed (`ai + 0x1bc`) with `0x3fc00000` = 1.5.
pub const AI_MOVE_SPEED: f32 = 1.5;

/// `sithAIAwareness_ProcessEvent`'s floor (`0x52315c`): a propagated event stops crossing adjoins
/// once its remaining strength falls below this. The executable spells the constant **0.0** (read
/// from `JK.EXE`'s `.rdata`; the port carried an invented `0.00277` until the read), so the guard is
/// "any positive strength crosses, a spent one does not".
pub const AI_AWARE_FLOOR: f32 = 0.0;
/// The strength a fired shot's event carries: `sithWeapon_WeaponFire` (`0x4d44f0`) creates it with
/// `0x40800000` = 4.0. `sithActor_KillActor` uses 5.0 and `sithWeapon_DestroyWeapon` 2.0; the port
/// raises the firing one, which is what a fight is mostly made of.
/// `sithWeapon_WeaponFire`'s and `sithWeapon_FireProjectile`'s strength (`0x40800000`), kind 1.
pub const AI_AWARE_SHOT: f32 = 2.0;
/// `sithActor_KillActor`'s strength (`0x40a00000`), kind 0.
pub const AI_AWARE_KILL: f32 = 5.0;
/// `sithExplosion_MakeBlast`'s strength (`0x40400000`), kind 1.
pub const AI_AWARE_BLAST: f32 = 3.0;
/// `sithWeapon_Update`'s strength while the weapon keeps firing (`0x40000000` = 2.0), kind 2 — the
/// kind `listen` scans first.
pub const AI_AWARE_WEAPON: f32 = 2.0;
/// `sithSoundMixer_PlaySoundThing`'s scale (`_DAT_00522f80` = 0.6, `.rdata`): a sound's awareness
/// strength is its volume times this.
pub const AI_AWARE_SOUND_SCALE: f32 = 0.6;
/// How long a record stays fresh. The engine's own fade is not recovered; the flat value keeps the
/// old stand-in's window.
pub const AI_AWARE_SECONDS: f32 = 8.0;
/// The strength `sensedanger` gives the awareness event it raises when an awake actor spots the player:
/// the handler (`0x50b9b0`) passes `0x40400000` = 3.0, so the same side converges on the fight.
pub const AI_AWARE_ALERT: f32 = 3.0;
/// `FUN_0050aa80`'s strength when `LookForTarget`'s scan finds the player (`0x40400000`), kind 0:
/// a spotter calls its neighbours in. The same branch clears the actor's awake bit.
pub const AI_AWARE_SIGHT: f32 = 3.0;
/// `sithAI_CanDetectSightThing`'s (`0x4ec140`) range model, read from the retail `JK.EXE`'s `.rdata`:
/// `_DAT_00523078` = **2.0** units (inside it a target is always noticed), `_DAT_00523084` = **0.1**
/// falloff per unit beyond that, capped at `_DAT_00523088` = **0.6**.
pub const DETECT_NEAR: f32 = 2.0;
pub const DETECT_FALLOFF: f32 = 0.1;
pub const DETECT_MAX_FALLOFF: f32 = 0.6;
/// `_DAT_00523070` 0.5: a physics target that is perfectly still is half as noticeable;
/// `_DAT_00523090` 0.75: one that is crouched (`physflags & 0x10000`) three-quarters.
pub const DETECT_STILL: f32 = 0.5;
pub const DETECT_CROUCH: f32 = 0.75;
/// `_DAT_00523094` 0.05 — the multiplier for a hidden target/watcher and the floor of the final clamp
/// (the cap is `_DAT_00523068` 1.0).
pub const DETECT_UNSEEN: f32 = 0.05;

/// A `soundclass=` value reduced to the `.snd` class key the loader uses: the lowercase basename,
/// because templates name `ky.snd` while the archive may carry it under a path.
pub(crate) fn sound_class_key(name: &str) -> String {
    name.rsplit(['\\', '/'])
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_lowercase()
}

/// `sithAICmd_Talk`'s mood table (`0x50be20`): the AI flag word's `0x200` is the gate (`00_aisleep.cog`
/// wakes an enemy with `AISetMode(enemy, 0x200)`; `0x202` is that plus the "enemy" bit scripts also
/// set), then `0x800` and `0x400` pick a mood family and the health fraction picks the word inside it.
/// The two health gates are the handler's own constants, **0.5** and **0.25** (`_DAT_005232f8` read
/// from the retail EXE's `.rdata`). Returns the `.snd` class event name the mode is indexed by:
/// `happy`/`search`, `boast`/`fear`, or `gloat`/`fear`.
fn talk_mode(flags: u32, health_fraction: f32) -> Option<&'static str> {
    if flags & 0x200 == 0 {
        return None;
    }
    if flags & 0x800 != 0 {
        return Some(if health_fraction > 0.5 { "gloat" } else { "fear" });
    }
    if flags & 0x400 != 0 {
        return Some(if health_fraction >= 0.25 { "boast" } else { "fear" });
    }
    Some(if health_fraction >= 0.5 { "happy" } else { "search" })
}

/// `sithAICmd_ForcePowers` (`0x40ff40`): which of the class's three force abilities fires, if any.
///
/// The handler reads the shipped line in pairs — `fltArg[1..2]`, `[3..4]`, `[5..6]` are the three
/// abilities' distance ranges, `fltArg[7..9]` their chances, and `fltArg[10..12]` their cooldowns
/// (milliseconds). One random draw is compared with all three chances (`__ftol` of
/// `rand / 32768`), and the first ability whose chance passes **and** whose range contains the
/// target is the one cast. The caller holds the line's first number as the instinct `weight`, so
/// these are the port's `args[0..]`: `args[0]` is engine `fltArg[1]`.
///
/// Returns `(1..=3, cooldown_ms)`.
fn force_power(args: &[f32], distance: f32, roll: f64) -> Option<(u8, f32)> {
    let arg = |index: usize| args.get(index).copied().unwrap_or(0.0);
    for power in 0..3 {
        let (near, far) = (arg(power * 2), arg(power * 2 + 1));
        let chance = arg(6 + power) as f64;
        if roll < chance && distance >= near && distance <= far {
            return Some(((power + 1) as u8, arg(9 + power)));
        }
    }
    None
}

/// `sithAICmd_SaberFighting` (`0x40fa40`): which saber swing an actor takes, if any.
///
/// The class line is four `(range, damage, cooldown)` triples after the opening chance:
/// `fltArg[1..3]`, `[4..6]`, `[7..9]`, `[10..12]`. The handler rearms every 500 ms, rolls
/// `fltArg[0]` for a swing, then walks the four entries **in order** and stops at the first whose
/// range is below the target distance. The number of entries that survived picks the swing with
/// one uniform draw; the shipped dark Jedi fill the first two, so their line offers 20/30 damage at
/// 1600/925 ms. Returns `(damage, puppet mode, cooldown_ms)`.
fn saber_attack(
    args: &[f32],
    distance: f32,
    chance_roll: f64,
    pick: f64,
) -> Option<(f32, i32, f32)> {
    let arg = |index: usize| args.get(index).copied().unwrap_or(0.0);
    if chance_roll >= arg(0) as f64 {
        return None;
    }
    let mut count = 0usize;
    for entry in 0..4 {
        let range = arg(1 + entry * 3);
        if range > 0.0 && range >= distance {
            count += 1;
        } else {
            break;
        }
    }
    if count == 0 {
        return None;
    }
    let entry = ((pick * count as f64) as usize).min(count - 1);
    let damage = arg(2 + entry * 3);
    if damage <= 0.0 {
        return None;
    }
    // The modes are the handler's own four immediates (`sithPuppet_PlayMode` ids 8, 0x12, 9, 0xa),
    // and the sound it plays is the same index + 0x47.
    let mode = [8, 0x12, 9, 0xa][entry];
    Some((damage, mode, arg(3 + entry * 3)))
}

/// `jkPlayPOVKey`'s flags word, as `sithPuppet_PlayKey` turns it into a track status: `0x4` holds
/// the last frame, `0x20` fades the track out at its end, `0x2` ends it through the callback. A clip
/// that sets none of those loops. The shipped calls are mount `0x14` (hold), fire `0x38` (fade) and
/// dismount `0x18` (neither bit, so the engine loops it until the weapon switch replaces it).
fn pov_key_loops(flags: u32) -> bool {
    flags & 0x26 == 0
}

/// One physics step's pose changes and events.
///
/// `moved` and `turned` are the things whose transform changed. `turned` names a thing whose
/// angular velocity is nonzero this step; it may also be in `moved`, and a thing that only turns
/// is never in `moved`. `blocked` and `arrived` are the messages the step already queued for
/// scripts.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ThingMotion {
    pub moved: Vec<usize>,
    pub turned: Vec<usize>,
    pub blocked: Vec<usize>,
    pub arrived: Vec<usize>,
}

/// One frame of `sithThing_Update`'s loop (`0x4cc1e0`): the per-frame tables its movers read and
/// what they changed.
pub struct ThingPass {
    dt: f32,
    player_at: Option<usize>,
    body_radius: Vec<f32>,
    thrust: Vec<[f32; 3]>,
    /// The runtime's own collider when the caller passed none.
    stored: Option<std::sync::Arc<crate::collide::Solids>>,
    moved: Vec<usize>,
    turned: Vec<usize>,
    blocked: Vec<usize>,
    arrived: Vec<usize>,
}

/// The player's own body: where it is, how it moves, and where it looks. The runtime writes it
/// through `step_player`, `teleport_player` and `respawn_player`; everything else reads it.
#[derive(Debug, Clone)]
pub struct PlayerBody {
    pub pos: Vec3,
    pub vy: f32,
    pub sector: usize,
    pub grounded: bool,
    pub gravity: f32,
    pub water: bool,
    pub vel: [f32; 2],
    pub step_rollover: f32,
    pub last_move: f32,
    pub fall_killed: bool,
    pub fall_fade: f32,
    /// The body's facing in radians.
    pub yaw: f32,
    /// The aim's pitch in radians.
    pub pitch: f32,
    /// How high the eye sits above the body's centre.
    pub eye: f32,
    /// What the body is attached to (`sithPhysics_FindFloor`, `0x4f5550`); `None` while it falls or
    /// swims. The attached update holds the body `GetThingHeight` above it.
    pub attach: Option<crate::collide::Attach>,
    /// `0x4ce050:71-74`: the rider's origin in the attached parent's local frame.
    pub floor_anchor: Option<(usize, [f32; 3])>,
    /// Physflag `0x100000` and `thing+0x48`: how far below a dry sector's adjoin the body's centre is,
    /// while that adjoin is within `sithPhysics_FindFloor`'s 0.05 upward search (`0x4f5550`).
    pub water_surface: Option<f32>,
    /// Last velocity published to the thing, so script impulses differ from ordinary physics feedback.
    pub published_vel: [f32; 3],
    /// Last published eye pose, used to consume script teleports without rounding untouched axes.
    pub published_pose: Option<([f32; 3], usize)>,
}

impl Default for PlayerBody {
    /// A body that has not been warped rests at the movement sphere's contact, so `rest` defaults to
    /// the player's radius rather than zero: zero would seat it on its own floor.
    fn default() -> Self {
        PlayerBody {
            pos: Vec3::ZERO,
            vy: 0.0,
            sector: 0,
            grounded: false,
            gravity: 0.0,
            water: false,
            vel: [0.0; 2],
            step_rollover: 0.0,
            last_move: 0.0,
            fall_killed: false,
            fall_fade: 1.0,
            yaw: 0.0,
            pitch: 0.0,
            eye: crate::PLAYER_EYE,
            attach: None,
            floor_anchor: None,
            water_surface: None,
            published_vel: [0.0; 3],
            published_pose: None,
        }
    }
}

/// One frame of player intent. The app reads the keys and the body; the runtime turns this into
/// a target velocity, integrates it at the engine's fixed rate, and applies a jump.
#[derive(Clone, Copy, Debug)]
pub struct PlayerInput {
    pub forward: f32,
    pub strafe: f32,
    /// The engine ladder's speed multiplier (`0x4d8a00:19-41`): 1.0, 2.0 under always-run or
    /// FAST, halved by SLOW or physflag `0x200000`. Kept as the value, not a bool, so the
    /// halved state survives the trip — a `run: multiplier == 2.0` bool dropped Shift's 0.5
    /// to 1.0 on the live keyboard path.
    pub multiplier: f32,
    pub crouch: bool,
    pub jump: bool,
    /// The body's facing in radians. The walk is built in the body's frame.
    pub yaw: f32,
    /// `SetActorExtraSpeed`'s force-speed term.
    pub extra_speed: f32,
}

impl Default for PlayerInput {
    /// The engine's resting ladder is 1.0 (`0x4d8a00:19`), not the derived 0.0: a defaulted
    /// input must still walk.
    fn default() -> Self {
        Self { forward: 0.0, strafe: 0.0, multiplier: 1.0, crouch: false, jump: false, yaw: 0.0, extra_speed: 0.0 }
    }
}

/// Map input in the body's frame to a world-space horizontal target.
///
/// The engine's basis has `forward = (-sin yaw, cos yaw)` and `right = (cos yaw, sin yaw)`;
/// `crate::yaw_axes` is the single definition of that mapping.
pub(crate) fn walk_target(along: f32, across: f32, yaw: f32) -> [f32; 2] {
    let (forward, right) = crate::yaw_axes(yaw);
    [
        forward[0] * along + right[0] * across,
        forward[1] * along + right[1] * across,
    ]
}

/// One frame of the player's horizontal velocity, integrated as the engine does.
///
/// `sithPhysics_UpdatePlayerPhysics` runs a fixed 50 Hz step (`OLDSTEP_TARGET_FPS`), adding
/// `acceleration * OLDSTEP_DELTA_50FPS` to the velocity and applying `sithPhysics_ApplyDrag`,
/// which multiplies by `1 - drag * dt`. With `acceleration = target * drag` the velocity
/// approaches `target * (1 - drag * dt)` exponentially; the decay runs inside the same sub-step.
///
/// The rollover is the engine's `physicsRolloverFrames`: leftover time carries to the next frame,
/// so 60 fps and 144 fps produce the same motion. Returns the number of 0.02 s sub-steps, which is
/// also how many times the caller must sweep the body.
pub(crate) fn integrate_walk(vel: &mut [f32; 2], target: [f32; 2], drag: f32, dt: f32, rollover: &mut f32) -> usize {
    const STEP: f32 = 1.0 / 50.0;
    let drag = drag.max(0.0);
    *rollover += dt.max(0.0);
    // A long stall (a load, a menu) must not turn into hundreds of steps.
    *rollover = rollover.min(STEP * 8.0);
    let mut steps = 0;
    while *rollover >= STEP {
        *rollover -= STEP;
        for axis in 0..2 {
            vel[axis] += target[axis] * drag * STEP;
            vel[axis] *= (1.0 - drag * STEP).clamp(0.0, 1.0);
        }
        steps += 1;
    }
    steps
}

pub struct Runtime {
    pub things: Vec<ThingState>,
    /// Template name per thing, for `GetThingTemplate`.
    pub templates: Vec<String>,
    /// `GetThingType` per thing, from the template's `type=` (`SITH_THING_*` values).
    pub types: Vec<i32>,
    /// Template name -> thing type, for `CreateThing` to give the new thing its type.
    pub template_types: BTreeMap<String, i32>,
    /// A template's `move=`: `physics` goods fall and slide, `path` things follow keyframes, and a
    /// projectile flies its own `vel=`. Gravity applies only to the first kind.
    pub template_move: BTreeMap<String, String>,
    /// A template's `collide=` mode, read by `MoveThing` when it selects search flags.
    pub template_collide: BTreeMap<String, u32>,
    /// A template's `physflags=`: a created thing starts with it (the engine copies the template word),
    /// and the generic mover reads bit `0x1` (gravity) and its thruster exemption `0x400000`.
    pub template_phys_flags: BTreeMap<String, u32>,
    /// A template's `mass=`. `sithPhysics_UpdateThingPhysics` (`0x4f6270`) gates its gravity arm on the
    /// thing's mass (`+0x204`) being non-zero as well as on `physflags & 0x1`, so a massless body with
    /// the gravity bit still floats. `_actor` ships `mass=150`, `_debris` `mass=0.1`.
    pub template_mass: BTreeMap<String, f32>,
    /// A template's `surfdrag=` and `staticdrag=` (`sithPhysics_ParseArg` `0x4f5ec0` stores them at
    /// `+0x210`/`+0x214`; an unset one stays zero), read by the attached drag arm.
    pub template_drag: BTreeMap<String, (f32, f32)>,
    /// World gravity, from the level's own `World Gravity` line: what pulls a physics thing down.
    pub gravity: f32,
    /// Every shot the runtime spawned, so a check can ask "did the trigger work" without sampling
    /// live things: a bolt can be born and destroyed between two frames at 10 fps. The count is
    /// cheap and always kept; the per-event timeline below is only queued when the harness asks.
    pub shots_fired: usize,
    /// `BK_ENCOUNTER_OUT`: shots and landed player hits in time order, drained by the app into the
    /// door-encounter TSV. Empty unless the capture harness sets the variable.
    pub encounter_events: Vec<EncounterEvent>,
    /// The launch time and target of every fired projectile by its thing index, so a hit or a miss
    /// can report the shot that produced it (retail's fixture only records shots that landed on the
    /// player, so a miss has no counterpart there and the trace writes it as its own row).
    pub encounter_shot_time: BTreeMap<usize, (f32, Option<usize>)>,
    /// Whether new events are queued. The capture harness turns it on by setting `BK_ENCOUNTER_OUT`
    /// at launch; tests flip it directly.
    pub encounter_events_enabled: bool,
    /// A template's `health=`/`maxhealth=`, so a created thing starts with real health.
    pub template_health: BTreeMap<String, f32>,
    /// A template's `vel=` — a projectile's launch speed — and its `damage=`.
    pub template_vel: BTreeMap<String, [f32; 3]>,
    /// A template's `angvel=(pitch/yaw/roll)`: the initial angular velocity, degrees per second
    /// (fans, spinning debris, the force shield's cloud). `ThingState::step` integrates it.
    pub template_angvel: BTreeMap<String, [f32; 3]>,
    pub template_damage: BTreeMap<String, f32>,
    /// A projectile's `explode=` template: the thing the engine spawns where it lands.
    pub template_explode: BTreeMap<String, String>,
    /// A template's `fireoffset=(x/y/z)`: where its shots leave the shooter, in model units
    /// (x lateral, y forward, z up) and the shooter's own frame. The port used a flat unit ahead of
    /// the eye as a stand-in; the shipped templates carry the real numbers.
    pub template_fire_offset: BTreeMap<String, [f32; 3]>,
    /// A projectile's `mindamage=`, its `rate=` (damage lost per second while flying) and
    /// whether its typeflags carry `SITH_WF_DAMAGE_DECAY` (0x4000).
    pub template_min_damage: BTreeMap<String, f32>,
    /// `maxhealth=`, the ceiling `HealThing` clamps to.
    pub template_max_health: BTreeMap<String, f32>,
    /// Templates in the shipped `_debris` family (the template itself or anything deriving from it).
    /// They are `type=weapon` but they are rubble, not shots: the mover must let them land instead of
    /// destroying them on the first surface they brush. The fired-projectile arm keys on this, not on
    /// the gravity bit, because shipped gravity-falling projectiles (`+grenade2`, `08tiebomb`) are
    /// still shots even though a cog creates the tie bomb.
    pub template_debris: BTreeSet<String>,
    /// A template's `thingflags=`, the word `SITH_TF_*` bits live in: a shot starts with it, and the
    /// player-fire pre-sweep tests its bit `0x200000` before raising `dodge`'s shot event.
    pub template_thingflags: BTreeMap<String, u32>,
    /// A template's full `typeflags=` word (the `SITH_WF_*`/`SITHEXPLOSION_FLAG_*` bits). `+bryarbolt`
    /// ships `0x20440d`, whose bit `0x200000` is what `sithWeapon_WeaponFireProjectile`'s player
    /// pre-sweep tests before it raises `dodge`'s shot event.
    pub template_typeflags: BTreeMap<String, u32>,
    /// A template's `damageclass=`, the `SithDamageType` word `sithActor_DamageActor` branches on.
    /// `_weapon` ships energy (`0x2`), `_debris` and `08tiebomb` impact (`0x1`), `_explosion` and
    /// `+grenade2` fire (`0x4`). Read through the template chain, so `+bryarbolt` inherits `_weapon`'s.
    pub template_damageclass: BTreeMap<String, u32>,
    /// A template's `soundclass=`, keyed by template name and reduced to the `.snd` class's lowercase
    /// basename: the class `talk` picks its mood sounds from (`sithSoundClass_PlayModeRandom`).
    pub template_soundclass: BTreeMap<String, String>,
    /// Every shipped `.snd` class's rows, by the class's lowercase basename and then by the engine's
    /// sound-mode **event name** (`create`, `happy`, `fear`, …), each a list of `(wav, maxVolume)`.
    /// The runtime has no file access, so `setup` hands this over after the loader parses them.
    pub sound_classes: BTreeMap<String, BTreeMap<String, Vec<(String, f32)>>>,
    /// The stream the **AI's own shot sound** picks from. Every other sound-mode pick draws on
    /// `cog::pseudo_random`, as retail's `rand()` does, because those picks sit in the same stream
    /// as the AI's fire delays and every script's `Rand()`. A shot names no wav in retail, so this
    /// fallback must not draw there — a runtime-local LCG keeps it random per run and deterministic.
    sound_rng: u64,
    /// The row last played per `(thing, mode)`, so a check can name the sound a hit just queued
    /// without drawing again.
    last_sound: BTreeMap<(usize, String), String>,
    /// `timer=`: how long a created thing lives, in seconds, before `sithThing_Update` destroys it.
    /// An impact effect's whole lifetime is this field.
    pub template_timer: BTreeMap<String, f32>,
    /// `minheadpitch`/`maxheadpitch` in degrees (the shipped `_actor` says -80/80).
    pub template_minheadpitch: BTreeMap<String, f32>,
    pub template_maxheadpitch: BTreeMap<String, f32>,
    /// `SetThingTimer`'s deadline per thing, on the world clock: the engine's `pThing->timer` plus
    /// `SITH_TF_TIMERSET`. When it passes, the thing's own scripts get the `timer` message.
    pub thing_timer: BTreeMap<usize, f32>,
    /// The player's shield bar (`SITHBIN_SHIELDS`, 0..=200) and whether health has reached 0.
    pub player_shields: f32,
    pub player_dead: bool,
    /// `(cog instance, link id, surface)` for every `surface` symbol a level's scripts declare:
    /// `sithCog_LinkCogToSurface` in tabular form. The player's use action messages these.
    pub surface_links: Vec<(usize, i32, usize)>,
    /// `(cog instance, link id, thing)` for every `thing` symbol a level's scripts declare:
    /// `sithCog_LinkCogToThing` in tabular form.
    pub thing_links: Vec<(usize, i32, usize)>,
    /// Messages a script has earned but that have not been delivered yet: the runtime holds no VM,
    /// so it queues `(cog instance, label, params, link id, identity)` and a system drains it. The
    /// params are what `GetParam(0)`/`GetParam(1)` answer inside the handler — `damaged` carries the
    /// amount and the damage type, which is how `force_absorb.cog` decides how much of it to take.
    /// The link id is `GetSenderId()`; the identity is the engine's `SourceSender`, whose `sender`
    /// and `sender_type` are `GetSenderRef()`/`GetSenderType()` (`-1`/0 from the world, the thing's
    /// index and 3 from a body). A push that labels itself with a link id alone leaves those two
    /// fields as they were.
    pub pending_messages: Vec<(usize, &'static str, [f64; 2], Option<i64>, Option<SourceSender>)>,
    /// `DAT_008554bc`: when the use action last ran, on the world clock. One stamp for the whole
    /// world — `sithPlayerActions_Activate` (`0x4fc0a0`) writes it before its sweep, and an actor's
    /// `opendoors` instinct runs the same function, so the two share the 250 ms wait.
    pub last_activate: f32,
    /// Force mana regeneration per second, and the pool's ceiling. The rate is calibrated: the
    /// engine's own tick is in `jkPlayer.c`, absent here. `force_heal.cog` spends `20 * rank` of bin
    /// 14, so a full pool is 200.
    pub mana_regen: f32,
    /// See `mana_regen`.
    pub mana_max: f32,
    /// Per-thing engine state the force scripts read and write, kept beside the core record because
    /// only those scripts use it: `GetActorFlags`/`SetActorFlags` (grip's `0x40000` hold, blinded
    /// `0x800`, excluded `0x100`), `GetItemFlags` (CTF flag `0x8`), `GetLifeLeft`/`SetLifeLeft`, the
    /// `ApplyForce` impulse, and `GetActorWeapon`/`SetActorWeapon`.
    ///
    /// Every one of these is named by a shipped script — `cog\force_pull.cog` and
    /// `cog\force_grip.cog` — which is why they exist at all (§16.5).
    pub actor_flags: std::collections::BTreeMap<usize, u32>,
    /// The AI's **own mode word** (`ai + 8` in `sithAI_Create`, `0x4e9cb0`): the word every instinct
    /// handler tests and `AISetMode`/`AIClearMode`/`AiGetMode` address. It starts at `0x1004` (the
    /// awake bit `0x4` plus the one-shot latch `0x1000` that only the awareness sweep's event 2
    /// clears). Kept apart from `actor_flags` because the engine's AI bit `0x8` ("turning") collides
    /// with `AF_INVULNERABLE` `0x8` on the sithActor word.
    pub ai_flags: std::collections::BTreeMap<usize, u32>,
    pub item_flags: std::collections::BTreeMap<usize, u32>,
    pub life_left: std::collections::BTreeMap<usize, f32>,
    /// The life a thing was created with — `explosionParams.msecLifeLeft` in the engine, captured
    /// so a sprite's frame count can be spread over the thing's whole life rather than a guess.
    pub life_total: std::collections::BTreeMap<usize, f32>,
    /// `Get/SetThingMass` and `Get/SetThingUserData`: plain per-thing values a script stores.
    pub mass: std::collections::BTreeMap<usize, f32>,
    pub user_data: std::collections::BTreeMap<usize, f32>,
    pub force: std::collections::BTreeMap<usize, [f32; 3]>,
    pub actor_weapon: std::collections::BTreeMap<usize, i32>,
    /// `SetFaceGeoMode`: per-surface draw mode (`sithSurface_SetGeoMode`). 0 is
    /// `GEOMETRY_NONE` — the surface is not drawn at all, which is how a script makes broken glass
    /// vanish or a slashable panel open. Anything else draws normally.
    pub face_geomode: std::collections::BTreeMap<usize, i32>,
    /// The engine's seven camera slots (`sithCamera_g_aCameras[7]`). `SetCameraFocus(slot, thing)`
    /// points one at a thing and `SetCurrentCamera(slot)` picks which the view follows; slot 0 is
    /// POV, which is the default, so an untouched world looks out of the player's own eyes.
    pub camera_focus: [Option<usize>; 7],
    pub camera_secondary: [Option<usize>; 7],
    pub current_camera: usize,
    /// `sithCamera_CycleCamera`'s own index (`DAT_008ee5a8`), which walks the engine's two-entry
    /// table `{0, 1}`: the toggle is not "whatever slot is current, plus one", it is this cursor.
    pub camera_cycle: usize,
    /// `SetCurrentCamera`: a **script** camera is current, not the player's own view. A level's opening
    /// uses it to take the body over — level 14 points slot 0 at thing 45 (`MAcamera.key`) and retail's
    /// body then moves 0.00 units with forward held.
    pub script_camera: bool,
    /// `SetPOVShake(posOffset, angleOffset, posDelta, angleDelta)`: the offset the view gets and the
    /// rate it decays at, back to zero. Every weapon script in the game calls it on a shot.
    pub shake_pos: [f32; 3],
    pub shake_angle: [f32; 3],
    pub shake_pos_rate: f32,
    pub shake_angle_rate: f32,
    /// Bumped on every geomode change, so the renderer knows to rebuild the world's batches instead
    /// of scanning the map each frame.
    pub geomode_epoch: u32,
    /// Bumped when a surface's **cel** changes (`SetWallCel`, or an animation stepping), which the
    /// renderer bakes into the mesh the same way. Separate from `geomode_epoch` so a per-frame
    /// animation does not look like a stream of geometry changes.
    pub surface_epoch: u32,
    /// Bumped **every frame** while anything on the level scrolls. The scroll offset is a pure
    /// function of the clock (`slide * time`), and the renderer only has to re-lay the scrolling
    /// batches, so this rides the frame rate instead of sharing `surface_epoch`'s 15 Hz bucket:
    /// the stale offset was up to 66 ms, it is at most one frame now.
    pub scroll_epoch: u32,
    /// Per surface, how many cels its material holds (`num_texinfo`), where an animation wraps. The
    /// runtime cannot read a `.mat`, so the loader hands this over; absent means 1, i.e. nothing to
    /// animate.
    pub surface_cels: Vec<u32>,
    /// The same table by **material**: `MaterialAnim(material, fps, mode)` wraps at the material's own
    /// cel count, and the level's material names are what its argument carries (`09sign.mat`).
    pub material_cels: BTreeMap<i32, u32>,
    pub material_names: BTreeMap<String, i32>,
    /// The material animations a script asked for, as `(material, cel, next due, fps)`, and what each
    /// material currently draws — `rdMaterial.curCelNum`, which a surface with no cel of its own uses.
    pub material_anims: Vec<(i32, f32, f32, f32)>,
    pub material_cel: BTreeMap<i32, f32>,
    /// `SetFaceLightMode`: a face's lighting mode, where 0 is `RD_LIGHTMODE_FULLYLIT` — drawn at full
    /// brightness — and everything else is the bake this build always does.
    pub face_light_mode: BTreeMap<usize, i32>,
    /// `GetThingGeoMode`/`SetThingCurGeoMode`: a thing's draw mode, where 0 is not drawn. The
    /// renderer watches `geomode_epoch` and hides the entities of a thing whose mode is 0.
    pub thing_geomode: BTreeMap<usize, i32>,
    /// `SetRespawnMask`: the player's respawn mask, i.e. which items a dying player keeps.
    pub player_respawn_mask: u32,
    /// Per sound channel, the volume `ChangeSoundVol` sets, and its in-flight fade
    /// `(target, rate)`. The mixer reads the volume; the fade advances in `step`.
    pub sound_volume: Vec<f32>,
    pub sound_pitch: Vec<f32>,
    pub sound_fade: BTreeMap<usize, (f32, f32)>,
    /// `SectorSound(sector, sound, volume)`: a sector's ambient sound as a script overrode it,
    /// which the ambience system prefers over the level's own (`sithSoundMixer_SetSectorAmbientSound`
    /// writes `sector->hAmbientSound`/`ambientSoundVolume`).
    pub sector_ambient: BTreeMap<i32, (String, f32)>,
    /// Per thing, the amount the last `damage_typed` applied, so a `damaged` handler that answers
    /// with a smaller number can be honoured after the fact (`amend_damage`).
    pub damage_applied: BTreeMap<usize, f32>,
    /// The last 15 Hz bucket a scrolling surface asked the renderer to re-lay its texture in.
    /// `SlideHorizonSky(x, y)` / `SlideCeilingSky(x, y)`: the rate the sky backdrop moves at, in
    /// texels a second, for `[horizon, ceiling]`. The engine keeps one world-wide offset per kind
    /// (`sithWorld.horizonSkyOffset`) and moves it by this vector; the offsets are `rate * time`.
    pub sky_scroll: [[f32; 2]; 2],
    /// `sithWeapon_secMountWait`: when the equipped weapon finishes mounting, as an absolute clock time
    /// (the engine stores `wait + gameTime` in a global, not per thing).
    pub mount_wait_until: f32,
    /// `PlayKey`: which of the script's own `keyframe` symbols a thing plays instead of its puppet's
    /// own state track — (keyfile, flags, start frame). Level 1's TIE bomber swaps in `01VTieB.key`
    /// the moment the player sees it.
    pub play_key: std::collections::BTreeMap<usize, (String, i32, i32)>,
    /// The parsed `.key` files the scripts name, so a key that drives a **thing's** pose (a script
    /// camera's dolly) can be sampled. The loader fills this once; `key_lengths` already needed it.
    pub keys: std::collections::BTreeMap<String, crate::key::Key>,
    /// The pose each `PlayKey` started from: a key's node transform is relative to it.
    pub key_base: std::collections::BTreeMap<usize, ([f32; 3], f32, f32, f32)>,
    /// `PlayMode`: which animation a script asked a thing to play.
    pub play_mode: std::collections::BTreeMap<usize, i32>,
    /// `GetThingSignature`: a stable id per thing, so a timer can tell "still the same thing" from
    /// "that index was recycled".
    pub signature: Vec<u32>,
    /// The snapshot `FirstThingInView` walks: candidates in the view cone, then `NextThingInView`
    /// steps through them.
    pub view_list: Vec<usize>,
    pub view_at: usize,
    /// `GetParam(n)`: the payload a message or `SetTimerEx` carried.
    pub last_params: Vec<f64>,
    /// The level's collider, once the caller has one. A moving thing that runs into geometry stops
    /// and sends `blocked` — `SITH_MESSAGE_BLOCKED`, the eighth most common label on level 1 (x8) —
    /// which is what a door script uses to know a body is in the way.
    pub solids: Option<std::sync::Arc<crate::collide::Solids>>,
    /// Whether `sithThing_Initialize`'s creation attach (`FindFloor(thing, 1)`) has run for the
    /// placed physics bodies — once, on the first step that has a collider.
    pub seated_load: bool,
    /// `(cog instance, link id, sector)` for every `sector` symbol a level's scripts declare:
    /// `sithCog_LinkCogToSector` in tabular form, so a sector's `entered`/`exited` can find its cogs.
    pub sector_links: Vec<(usize, i32, usize)>,
    /// The mask of each link whose symbol is not the default `0x401`, keyed by `(source type,
    /// instance, link id, target)`; `link_hears` reads it. Kept beside the link tables rather than in
    /// them so the many readers of those tuples stay unchanged.
    pub link_masks: std::collections::HashMap<(i32, usize, i32, usize), u32>,


    /// Force seeing's infrared mode, and the map-mode flags the same script clears.
    ///
    /// `force_seeing.cog` does the timing and the mana (`ChangeInv(player, 14, -30)`); the engine
    /// only has to remember the mode. `--force seeing` showed the script running and the mode verbs
    /// falling through to the null answer, so nothing changed and nothing could be checked.
    pub ir_mode: bool,
    pub map_mode_flags: u32,
    /// A template's body as `(radius, height)`, from its `size=x/y/z` or `radius=`. Collision
    /// targets are bodies, not points: a bolt fired at eye height flies a metre above a placed
    /// actor's origin, and a point test at `0.5` therefore never landed (§16.5).
    pub template_body: BTreeMap<String, (f32, f32)>,
    /// Per template, the body a **projectile** has to touch to hit it: `(radius, height)` from the
    /// engine's `moveSize` (`movesize=` in the template, the value the `getthingmovesize` verb
    /// returns), with the height the shipped models imply (2.8 x the radius; actor models measure
    /// 2.7-2.8). Separate from `template_body`, which the explosion pass uses.
    pub template_hit: BTreeMap<String, (f32, f32)>,
    template_height: BTreeMap<String, f32>,
    pub template_rate: BTreeMap<String, f32>,
    pub template_decay: std::collections::BTreeSet<String>,
    /// Templates whose `typeflags` carry `SITHEXPLOSION_FLAG_ANIMATED_SPRITE` (`0x1`), the bit
    /// `sithExplosion_CreateThing` tests before it walks a sprite's cels.
    pub template_sprite_anim: std::collections::BTreeSet<String>,

    /// An item template's `inv=` bin and `count=`, and the bin stand-in used when the level
    /// leaves the inventory number to the item's own cog.
    pub template_inv: BTreeMap<String, i32>,
    pub template_count: BTreeMap<String, f32>,
    pub template_bins: BTreeMap<String, i32>,
    /// A template's `respawn=` seconds, for a powerup that comes back after being taken.
    pub template_respawn: BTreeMap<String, f32>,
    /// Text a script printed (`jkPrintUNIString`), in order: the `.uni` key it names.
    pub prints: Vec<String>,
    /// The line `jkStringOutput`/`jkPrintUNIString` last put up, and the level time it stops showing.
    /// The engine's message area holds a line for a few seconds; this build keeps the text and the
    /// deadline, and the HUD draws it while it lasts.
    pub hud_message: Option<(String, f32)>,
    /// (`jkCog_jkstring`, 128 characters) that the concat verbs append to. A script builds a line in
    /// pieces and `jkStringOutput` puts the finished line up.
    pub string_buffer: String,
    /// `LoadSound`'s table: the sound paths scripts have asked for, in order, so the verb can answer a
    /// stable id the way the engine answers `sithSound->id`.
    pub loaded_sounds: Vec<String>,
    /// The weapon cog armed for the player: `ActivateWeapon(thing, cog, fireRate, mode)` stores it and
    /// the engine's weapon controls send that cog `fire`/`autofire` while the trigger is held. The
    /// runtime records it; the app delivers the message, since the VMs live there.
    pub weapon_cog: Option<(i32, f32, i32)>,
    /// `ActivateWeapon`'s fire rate, which `ChangeFireRate` replaces and the trigger reads.
    pub weapon_rate: f32,
    /// The level time the armed weapon may fire again: `ActivateWeapon`'s rate, which
    /// `ChangeFireRate` updates — the engine's `binWait`/`activationDelaySecs` pair.
    pub weapon_ready: f32,
    /// `SetAutoSwitch`/`GetAutoSwitch`: the player's weapon-auto-switch bit field.
    pub auto_switch: u32,
    /// `SetAutoPickup`/`GetAutoPickup`: the same, for items.
    pub auto_pickup: u32,
    /// `jkGetSuperFlags`/`jkSetSuperFlags`/`jkClearSuperFlags`: the player's surge word. The light and
    /// dark powerups set bit `4` (`pow_keyyellow`/`pow_crossbow`) and "power" sets bit `1`
    /// (`pow_power`), and every one of them also reads it back with `& 4`/`& 1` to toggle off.
    pub super_flags: u32,
    /// `SetWeaponMesh`/`jkSetPOVModel`/`jkSetWaggle`/`jkPlayPOVKey`: the first-person weapon's own
    /// data. The amplitudes are **degrees** and the rate is the degrees the sway phase advances per
    /// unit of movement (`jkPlayer_SetWaggle(thing, velocities, rate)`); `jkPlayer_DrawPov` uses both.
    pub weapon_mesh: BTreeMap<usize, String>,
    pub pov_model: BTreeMap<usize, String>,
    pub pov_key: BTreeMap<usize, String>,
    /// Whether the `jkPlayPOVKey` clip loops, from the verb's **fourth** argument (the flags word).
    /// The third argument is the start frame (the mount passes 0, the fire clip 1). The mount flags
    /// `0x14` and the fire flags `0x38` both clear `0x26`, so both play once -- `0x4` is the
    /// hold-last "no loop" status, `0x20` the fade/callback status, `0x2` the end-callback status
    /// (`sithPuppet_PlayKey`/`rdPuppet_AdvanceTrack`). This build used to loop every clip, so the
    /// mount animation replayed from its first frame and the weapon flickered out and back.
    pub pov_key_loop: BTreeMap<usize, bool>,
    pub waggle: BTreeMap<usize, [f32; 3]>,
    pub waggle_rate: BTreeMap<usize, f32>,
    /// `.key` name -> frame count, for `GetKeyLen`. The loader fills it.
    pub key_lengths: BTreeMap<String, f32>,
    /// The level time each actor last fired, so the renderer can play its `fire` track. The engine
    /// picks the action track from `sithPuppet_UpdateThingMove`, which this build has no analogue of.
    pub last_shot: BTreeMap<usize, f32>,
    /// The level time each thing last took damage, for the puppet's `hit` track.
    pub last_hit: BTreeMap<usize, f32>,
    /// `SetArmedMode`/`SetMajorMode`: the puppet's armed state and its major mode.
    pub armed_mode: BTreeMap<usize, u32>,
    pub major_mode: BTreeMap<usize, u32>,
    /// `sithPuppet_SetMoveMode`: land (0) or water (1), combined with the armed row when drawing.
    pub movement_mode: BTreeMap<usize, u32>,
    template_puppet: BTreeSet<String>,
    /// `SetCurWeapon`'s weapon thing, when a script named one.
    pub cur_weapon_thing: Option<usize>,
    /// `GetThingClassCog`: the cog a thing's class names.
    pub thing_class_cog: BTreeMap<usize, i32>,
    /// Spawned items awaiting their template COG instance before movement can touch them.
    pub item_cog_spawns: Vec<usize>,
    /// `CaptureThing`'s cog: the engine stores the calling cog at `thing + 0x2a0`, and the thing's
    /// `damaged`/`killed` messages go there (`FUN_00502e40`). One cog per thing; a later capture
    /// replaces it.
    pub thing_capture_cog: BTreeMap<usize, i32>,
    /// Each class script's `skill:` answers (`bin -> ReturnEx`), keyed by its cog instance.
    /// `SkillTarget` returns the victim's class script's answer, which is where a force power's
    /// damage actually lives (`actor_pm.cog`: 10 for grip, 5 for deadly sight).
    pub class_skill: BTreeMap<i32, Vec<(i32, f32)>>,
    /// `jkEnableSaber`/`jkDisableSaber`, and the saber camera the scripts switch to.
    pub saber_enabled: bool,
    pub saber_cam: i32,
    /// `misc\items.dat`'s `cog=` column: inventory bin -> the script that bin runs. The loader fills
    /// it once, because the runtime has no file access.
    pub item_cog: BTreeMap<i32, String>,
    /// `misc\items.dat`'s `max` column: inventory bin -> the ceiling `GetInvMax` answers. The engine
    /// keeps it in the bin record (`sithInventory_GetInventoryMaximum`); the loader fills it here,
    /// because the runtime has no file access.
    pub item_max: BTreeMap<i32, f32>,
    /// The same table's `min` column. `sithInventory_ChangeInventory` and `SetInventory` clamp a bin to
    /// `[min, max]`, which is why retail's mana never reads above 100 however often a powerup lands.
    pub item_min: BTreeMap<i32, f32>,
    /// Inventory bin -> the `cog_index` of the VM running that bin's script, filled by the loader when
    /// it starts one. `UseBin` needs it: the engine sends the bin's message to *its own cog*, and the
    /// runtime cannot see the app's instance list.
    pub bin_cog_index: BTreeMap<i32, i32>,
    /// Item scripts waiting to be instantiated: a bin went from empty to held, so its cog should
    /// start. The app owns the VMs, so the runtime only names them.
    pub pending_cogs: Vec<String>,
    /// Item scripts already running, so a bin that goes to zero and back does not start a second.
    pub spawned_cogs: std::collections::BTreeSet<String>,
    /// Script-to-script messages queued by `SendMessage`: `(label, sending cog, target cog)`.
    /// The engine's arm is `SendMessage(cogIdx, messageType)` — `sithCogFunction_SendMessage` —
    /// so the target is a **cog index**, not a thing, and the label is either a name or `#N`
    /// (a message number resolved against the receiving script's `message:` list). A target of
    /// `None` is what an unset master cog produces; the port then broadcasts rather than dropping
    /// the message, because that is the only delivery the shipped scripts have today.
    pub messages: Vec<(String, Option<usize>, Option<i32>, [f64; 2])>,
    /// The cog asking the world a question right now: what `SendMessage` records as the sender, and
    /// what `GetSenderID` answers in the receiver (`pCog->idx` in the engine's call). The VM sets it
    /// before each world verb, because only the VM knows which script is running.
    pub caller_cog: i32,
    /// The script that is running a verb right now (`Vm::script_name`): a trace names the script that
    /// set a flag or called an engine verb, which `caller_cog` alone cannot (see `cog_names`).
    pub caller_script: String,
    /// The script name behind each `cog_index`, so a trace can name the script that called a verb (the
    /// level's own cog table only reaches the placed instances, while a started VM's index counts every
    /// VM in start order).
    pub cog_names: std::collections::BTreeMap<i32, String>,
    pub messages_sent: usize,
    pub sectors: Vec<SectorState>,
    /// Each sector's `SITH_SECTOR_*` word, as the level ships it and as `SetSectorFlags` and
    /// `SetSectorAdjoins` change it. `0x2` underwater, `0x80` adjoins hidden — the two the
    /// shipped scripts switch, and the two collision reads (`collide::Gates`).
    pub sector_flags: Vec<u32>,
    /// Per sector, the level's baked `AMBIENT LIGHT + EXTRA LIGHT`, clamped to `0..1` — the value the
    /// engine hands `rdCamera_SetAmbientLight` for that sector (`sithRender_RenderSectors`,
    /// `0x4c6c40`). The world draw used a base of `1.0`, which is why a level with a mean sector
    /// ambient of 0.52 (`02narshadda`: 641 sectors) came out ~2x the retail brightness.
    pub sector_light_baked: Vec<f32>,
    /// Each sector's `AMBIENT LIGHT` — the engine's own sector light (`thing + 0x5c` is the sector
    /// pointer, `+4` its light). `EXTRA LIGHT` is the POV pass's addition, not this field, and
    /// `sector_ambient` is the sector's ambient *sound*, so the name says which one this is.
    pub sector_ambient_light: Vec<f32>,
    /// Each `World adjoins` row's `SITH_SURFACE_ADJOIN_*` word: `SetAdjoinFlags`/
    /// `ClearAdjoinFlags` switch these, which is how a force field opens and closes.
    pub adjoin_flags: Vec<u16>,
    /// Per `World adjoins` row, the two sides' `dist` column summed — the depth
    /// `sithAI_GetThingsInView` (`0x4ebe80`) adds when it recurses through the opening.
    pub adjoin_depth: Vec<f32>,
    /// Surface → the adjoin row it carries (`-1` for a wall), so a script's `surface` symbol
    /// reaches its `pAdjoin` without the runtime holding the whole level.
    pub surface_adjoin: Vec<i32>,
    /// Each face's `geometryMode` as the loader sets it: the `.jkl` `geo` column, forced to
    /// `GEOMETRY_NONE` for a face with no material (`sithSurface.c:225`–`237`). A script's
    /// `SetFaceGeoMode` overrides it in `face_geomode`; `GetFaceGeoMode` reads both.
    pub surface_geomode: Vec<i32>,
    /// Per-surface state, indexed the way `.jkl` surfaces are: cels, animation, slides.
    pub surfaces: Vec<SurfaceState>,
    /// Each surface's `SITH_SURFACE_*` word as the level ships it. The water bits
    /// (`SITH_SURFACE_WATER` 0x20000 / `VERYDEEPWATER` 0x100000) are what halve the player's move;
    /// `SetSurfaceFlags` is not implemented, so the level's own word is the whole answer.
    pub surface_flags: Vec<u32>,
    /// Last `touched` clock per linked surface (`sithSurface+0x58`, engine milliseconds).
    surface_touched_ms: BTreeMap<usize, u32>,
    /// Which sector claims each world surface (`SURFACES <first> <count>` per room), `-1` when no room
    /// does. `sithPlayerActions_Activate` searches the **body's own sector** for a switch, so a surface an
    /// adjoining room owns is not reachable from here: the cantina plate is sector 31's, the player stands
    /// in 260, and retail's `doorstatus` stays 0 at every pose measured (round 302/303).
    pub surface_sector: Vec<i32>,
    /// Sounds a script asked for, in order: the wav, its volume, and where it came from.
    ///
    /// `PlaySoundThing` names a thing and `PlaySoundPos` a position; a request with no place
    /// is `PlaySoundLocal`, which the mixer does not attenuate by distance.
    pub sounds: Vec<SoundRequest>,
    /// Script-started loop channels (`PlaySound*` with the loop/ambient flag), one entry per
    /// name+place: a timer that repeats the same request every 0.5 s refreshes it, it does not stack.
    pub sound_loops: Vec<SoundLoop>,
    /// Pending timers, summed — enough to show scripts driving a clock.
    pub timers: f32,
    pub inv: BTreeMap<i32, f32>,
    /// `SetInvAvailable(bin, 0|1)`: whether the HUD shows that bin.
    pub inv_available: BTreeMap<i32, bool>,
    /// `SetInvActivated`: a force power is switched on, and when it was switched on (the
    /// hold time between `ActivateBin` and `DeactivateBin` decides a force jump's height).
    pub inv_activated: BTreeMap<i32, bool>,
    pub bin_active: BTreeMap<i32, f32>,
    pub bin_wait: BTreeMap<i32, f32>,
    /// Bins waiting to send their own cog a `fire`: `(due, bin, cog, player)`.
    ///
    /// `sithInventory_ActivateBin` keeps the bin's `activationDelaySecs` and `sithInventory_SendFire`
    /// re-sends `fire` to the bin's cog every delay while it stays active — that loop is how a held
    /// force power keeps working (lightning's damage every 0.3 s). The port stored the activation
    /// (for `DeactivateBin`'s hold time) and never sent the message, so an item script's own effect
    /// never ran.
    pub bin_due: Vec<(f32, i32, i32, usize, f32)>,
    /// Seconds this world has been stepped, which the hold timers read.
    pub time: f32,
    /// The millisecond game clock (`0x836c14`) sampled by the item collision handler.
    game_clock: std::time::Duration,
    /// `SetFireWait`: the player's weapon cooldown, and the level time the next shot is allowed at.
    /// `sithWeapon_SetFireWait` arms `nextFire = now + wait`, and a negative wait clears both.
    pub fire_wait: f32,
    pub next_fire: f32,
    /// `SetMasterCog`/`GetMasterCog`: the engine keeps one global master cog index
    /// (`sithCog_g_pMasterCog`), which sidekick and `endlevel` scripts compare with `GetSelfCog()`.
    pub master_cog: Option<i32>,
    /// `sithMulti_multiModeFlags`: what the network verbs read and write. Zero in a single-player game,
    /// and stored rather than dropped so a script that sets and re-reads a flag sees its own value.
    pub multi_mode_flags: u32,
    /// Per-actor instinct classes, keyed by thing index: what the `.ai` said and what this
    /// build does with it (see `ai_tick`).
    pub ai: BTreeMap<usize, crate::ai::Ai>,
    /// `AISetMovePos` / `AISetMoveFrame` / `AISetMoveThing` / `AISetLookPos`: the move position
    /// (`ai + 0x1a0`, written by `sithAI_SetMoveThing`) and the spot a standing actor turns to face.
    /// `sithAI_Tick`'s move arm (`0x4ea890`) pushes the body's velocity toward `ai_move`.
    pub ai_move: BTreeMap<usize, [f32; 3]>,
    pub ai_look: BTreeMap<usize, [f32; 3]>,
    /// Where an actor last had the player in view (`sithAI`'s last known position). Losing sight
    /// means "walk there", and arriving clears it.
    pub ai_last_seen: BTreeMap<usize, [f32; 3]>,
    /// When `remember` last wrote `ai_last_seen` — the engine's `ai+0x81` clock, which
    /// `lookfortarget`'s branch B reads as a memory-decay timeout (`last_seen + fltArg[1] < now`).
    pub ai_last_seen_at: BTreeMap<usize, f32>,
    /// Actors that have **registered** the player: `sithAI_CanDetectSightThing` (`0x4ec140`) is a
    /// probability roll and the engine re-rolls it while refreshing a target; the port rolls it on
    /// acquisition and then keeps the deterministic sight test (ledger).
    pub ai_detected: std::collections::BTreeSet<usize>,
    /// The inventory a created `+backpack` thing carries (`sithInventory_CreateBackpack`,
    /// `0x4d2740`), keyed by that thing.
    pub backpack: BTreeMap<usize, Vec<(i32, f32)>>,
    /// Actors a script switched the AI **off** for: `AIClearMode(actor, 0x606)` at level 1's
    /// intro — the shot below shows `01_initialry.cog` calling it on the first enemy before
    /// `AISetMovePos` walks it into the bar. An off actor still follows its scripted moves; it
    /// does not look, acquire, chase or shoot.
    pub ai_cleared: std::collections::BTreeSet<usize>,
    /// When each actor may next roll its sight detection. The engine rolls once per **instinct pass**
    /// (`sithAI_InstinctUpdate` sets the instinct's own `next` to `now + 1000` before calling it), so a
    /// 50 % chance at 2 units takes seconds to latch there. Rolling every frame latched it in one or
    /// two frames and made every gunner see the player the instant the level loaded.
    pub ai_detect_at: std::collections::BTreeMap<usize, f32>,
    /// The scheduler's per-instinct instance state (`ai + 0x10`, stride `0x18`): each instinct's
    /// own `next` clock, in class-file order. `sithAI_InstinctUpdate` (`0x4e9e90`) arms a due
    /// instinct at `now + 1000 ms` **before** calling its handler, and a handler may re-arm itself
    /// (`lookfortarget` at its class interval, `talk` at its own first number). A clock of 0.0 is
    /// due immediately. This retires the port's per-instinct clock maps (`ai_talk`, `ai_sense`,
    /// `ai_scan`, `ai_opendoors`, the `roam`/`randomturn`/`circlestrafe` due halves).
    pub ai_inst: BTreeMap<usize, Vec<f32>>,
    /// The slot's whole-AI pass deadline (`ai + 0x190`): the earliest armed clock of the instincts
    /// the mode word accepts, at most `now + 5000`. `sithAI_Process` (`0x4e9e20`) skips the actor
    /// entirely while it is in the future.
    pub ai_next_due: BTreeMap<usize, f32>,
    /// `DAT_008ee66c & 1`: the engine's global AI kill-switch, checked by `sithAI_EmitEvent`
    /// (`0x4e9f80`) before any dispatch. Nothing in the executable's reachable paths sets it in
    /// single player, so the port carries the gate and never raises it.
    pub ai_disabled_global: bool,
    // (talk's clock lives in `ai_inst`: the handler re-arms at its own first number.)
    /// When a **damaged** turret's `turretfire` handler kills it. `FUN_0050a0f0` arms this slot to
    /// `now + arg7` (the shipped `10000` = 10 s) the first time it runs below `arg5` of full health,
    /// and once the deadline passes it calls `sithActor_KillActor` on its own body.
    pub ai_turret_down: BTreeMap<usize, f32>,
    // (lobfire's clock lives in `ai_inst`; its armed-mode write runs from the 0x100 event branch.)
    // (opendoors' clock lives in `ai_inst`: the handler returns and the scheduler's now+1000 arm holds.)
    /// When each actor may run its `forcepowers` handler next (`0x40ff40`). The handler starts by
    /// setting its own next run to `now + fltArg[0]` (the class line's first number), so the
    /// interval is per class and not global.
    pub ai_force: BTreeMap<usize, f32>,
    /// Each actor's force-power cooldown (`ai + 0x288` in the handler): the selected ability's own
    /// timer, `fltArg[10..12]`. The handler returns without casting while it is in the future.
    pub ai_force_cool: BTreeMap<usize, f32>,
    /// When each actor may run `saberfighting` again: the handler rearms its scheduler slot at a
    /// fixed 500 ms before it decides anything.
    pub ai_saber: BTreeMap<usize, f32>,
    /// The selected saber attack's own cooldown (`ai + 0x288`), `fltArg[3]/[6]/[9]/[12]`.
    pub ai_saber_cool: BTreeMap<usize, f32>,
    /// When each actor may fire `specialattack`'s contact event again (event 4).
    pub ai_special: BTreeMap<usize, f32>,
    // (sensedanger's clock lives in `ai_inst`: the scheduler arms now+1000 and the alert's 0x800
    // write stops the next pass through the registry's reject word.)
    // (`dodge`'s own cooldown clock is retired: its damage path writes the shared move lockout
    // `ai_move_lock`, the engine's `ai + 0xa3`, at `now + 1000` ms.)
    /// When each actor's `jump` may nudge again: the handler (`0x50abf0`, event `0x400`) sets its own
    /// next check to `now + the class line's first number` (`jump 2000.0, 3.0, 3.0`).
    pub ai_jump: BTreeMap<usize, f32>,
    /// The engine's `ai + 0x6f`: the speed argument the last `sithAI_SetMoveThing` (`0x4eac90`)
    /// carried (dodge passes 2.5, `sithAI_Jump` 2.0), multiplying `maxthrust` in the walk step.
    pub ai_move_speed: BTreeMap<usize, f32>,
    /// The engine's `ai + 0xa3` jump lockout, in seconds: `sithAI_Jump` sets `now + 2000` ms and
    /// `sithAI_SetMoveThing` refuses a new move while the lockout holds and the actor is moving.
    pub ai_move_lock: BTreeMap<usize, f32>,
    /// The engine's `ai + 0x65` look **direction** (`sithAI_SetLookFrame`, `0x4eab80`): the flat
    /// unit vector the body blends its facing toward. Presence tracks the flag `0x8` turn.
    pub ai_look_dir: BTreeMap<usize, [f32; 2]>,
    /// Each actor's home anchor: where it was placed. `returnhome`'s handler (`0x50bd70`) walks the
    /// actor back to `ai + 0x93` (`SetMoveThing`) and then turns it there (`SetLookFrame`), so the
    /// anchor is the thing's spawn position. Set when the instincts are attached to the live runtime.
    pub ai_home: BTreeMap<usize, [f32; 3]>,
    /// The placed look vector `sithAI_Create` copies to `ai + 0x96` (`thing + 0x94`):
    /// `returnhome`'s arrival arm looks along it.
    pub ai_home_look: BTreeMap<usize, [f32; 3]>,
    /// When each scripted `AIFlee` ends. The verb (`0x501420`) only sets the fleeing mode; the class's
    /// `Flee` instinct is what walks, and its third argument is the duration in seconds (1–2 s in the
    /// shipped classes), after which the handler clears the mode. The port records the same deadline.
    pub ai_flee_until: BTreeMap<usize, f32>,
    /// Actors in the `retreat`/`withdraw` mode: the thing they are backing away from, and when the
    /// mode expires. `retreat` (`0x50bc60`) sets it when health falls under `arg0 × max` and its
    /// `arg1` chance succeeds; `withdraw` (`0x50b150`) walks the actor away from that thing. The port
    /// reuses the scripted flee walk and re-arms on the withdraw instinct's own interval (5000 ms in
    /// the shipped classes).
    /// `(sender, deadline)`: `None` for `retreat`'s withdraw, which the engine ends **only** when the
    /// target it remembers is gone (`ai[0x70] == 0` clears the mode); `Some` for `hitandrun`'s break-off,
    /// whose own second argument is a duration.
    pub ai_withdraw: BTreeMap<usize, (usize, Option<f32>)>,
    /// `hitandrun` (`0x50bb90`) accumulator, in **milliseconds of firing time**: the handler runs on
    /// its scheduler pass (every 1000 ms while the mode word has `0xc00`), adds 1000 per firing
    /// pass, resets to zero on a pass that did not fire, and breaks off when the first argument is
    /// smaller than the accumulator.
    pub ai_hitrun: BTreeMap<usize, f32>,
    /// Each idle actor's chosen `randomturn` yaw (`0x50b700`): every class interval (1000–2000 ms
    /// shipped, on the instinct's scheduler slot) the actor picks a random direction. The port turns
    /// to it at the body's own rate.
    pub ai_randomturn: BTreeMap<usize, f32>,
    /// `sithAIAwareness`: a position other actors of the same side were told about, and when. A
    /// shot is the event; the engine keeps per-sector records that propagate through adjoins and
    /// fade. This build has neither the per-adjoin attenuation nor the recovered reader, so an
    /// alert lands on buyers within a radius and expires (see `AWARE_RADIUS`/`AWARE_SECONDS`).
    /// `sithAIAwareness`'s records: **one event per sector and kind**. `CreateTransmittingEvent`'s
    /// third argument is that kind, not an alignment — every call site passes `0` (a death, a thrown
    /// weapon, a sound), `1` (a weapon shot, an explosion) or `2`, and `ProcessEvent` (`0x4f2c30`)
    /// keeps strength, position and the **source thing** in the kind's own slot, replacing it only
    /// when the new event is stronger. The listener (`0x50a6f0`) scans kind 2, then 1, then 0, and
    /// turns at the event's source thing when it has one. Keying by alignment instead meant a shot
    /// could never reach an actor of the other side — and a shot by the player, who has no `.ai`
    /// class, raised no event at all.
    pub sector_awareness: BTreeMap<(i32, u8), (f32, [f32; 3], f32, Option<usize>)>,
    /// When each thing last received AI event 2 (the awareness sweep raises it once per tick).
    pub awareness_event_at: BTreeMap<usize, f32>,
    /// The adjoin graph the record propagates through: `(neighbour sector, cost)` per sector. The
    /// cost is the adjoin's own `dist` float — the only per-portal number a level carries, and the
    /// field `004f2c30_sithAIAwareness_ProcessEvent.c` reads through the adjoin link.
    pub sector_adjoins: Vec<Vec<(i32, f32)>>,
    /// `AISetFireTarget(actor, thing)`: what an actor shoots at, when a script names one instead of
    /// leaving it to its instincts (`00_AITurnCoat.cog` turns an actor on a victim, `06_drama.cog`
    /// stages a shooting). Absent means "the player", which is what `ai_tick` does by default.
    pub ai_fire_target: BTreeMap<usize, usize>,
    /// `AIFlee(actor, from)`: the actor runs *away* from that thing and stops shooting.
    pub ai_flee: BTreeMap<usize, usize>,
    /// The level's `.ai` classes by name (`"picup.ai"` -> its text), so `AISetClass` can swap an
    /// actor's instincts mid-fight: level 11's `endlevel` gives Pic and Gorc a new class when the
    /// fight changes phase. Filled by the loader; the runtime cannot read the archives.
    pub ai_classes: BTreeMap<String, String>,
    /// Per-template `weapon=` — the template an actor fires.
    pub template_weapon: BTreeMap<String, String>,
    /// (`thing + 0x55`; the Ree-Yees punches with `+ry_punch` at `fltArg[7]` chance 1.00).
    pub template_weapon2: BTreeMap<String, String>,
    /// A puppet-flagged actor's deferred shot (`sithAI_FireWeapon` stores `fl|8`, the error and
    /// the mode when the thing carries `0x20000`): fired when the fire track reaches its marker.
    /// The port has no marker clock, so the next fire pass is the stand-in.
    pub ai_deferred_fire: BTreeMap<usize, (u32, f32, u32, usize)>,
    /// The engine's fire lockout `ai + 0xa2`: `LobFire` arms it and `sithAI_FireWeapon` refuses
    /// while it is in the future.
    pub ai_fire_lock: BTreeMap<usize, f32>,

    /// An actor template's walking speed, `maxthrust / surfdrag` — the same conversion `Rules`
    /// uses for the player.
    pub template_speed: BTreeMap<String, f32>,
    /// An actor template's `maxthrust=` (`thing + 0x148`), which the AI move arm multiplies.
    pub template_maxthrust: BTreeMap<String, f32>,
    /// An actor template's turn rate, `maxrotthrust=` in degrees a second (`_actor` 180, a Ree-Yees
    /// gunner 70). The body turns at this rate; it does not snap.
    pub template_rotthrust: BTreeMap<String, f32>,
    pub template_jumpspeed: BTreeMap<String, f32>,
    /// Redbook music: `(start track, end track, loop-back track)` as `PlaySong` set it.
    pub music: Option<(i32, i32, i32)>,
    /// `jkEndLevel(flag)`: the level is finished; the campaign advances when the app notices.
    /// The engine registers the verb as `jkendlevel` (`jkCog.c`), and a level's final script
    /// is what calls it.
    pub level_complete: bool,
    /// `DAT_0082f0a8`, set by `sithMain_SetEndLevel` (`0x4c4bf0`) and never cleared: the main loop's
    /// "leave this level" flag. The app clears [`Self::level_complete`] in a capture run to stay in the
    /// level it booted, so this is the latch a trace reads to show the level's own script finished it.
    pub end_level_called: bool,
    /// `AutosaveGame()`: a script asked for a checkpoint. The app writes it and clears this — the
    /// runtime does no file I/O. The shipped story levels call it at their own beats (7, 11, 15, 17a,
    /// 17b and 20), so the save happens where retail's does.
    pub autosave: bool,
    /// `SetMusicVol(v)`, 0..1 — the scripts fade this from a `pulse`.
    pub music_volume: f32,
    /// The last hold time `DeactivateBin` reported, kept so a caller can inspect it after
    /// the entry has been popped.
    pub last_hold: f32,
    /// `AddDynamicTint`'s player palette tint: the damage flash adds to it and `sithPlayer_Update`
    /// decays it 0.4/s. Scripts share the same channels (`force_seeing`, powerup flashes).
    pub tint: [f32; 3],
    /// `NewColorEffect`'s live screen effects, keyed by the handle the verb answered. Each carries the
    /// engine's three channels — a per-channel `filter`, a `tint` and an additive `add` — plus the fade
    /// in seconds after which it expires (`stdPalEffects_*`).
    pub color_effects: BTreeMap<i32, ColorEffect>,
    /// The handle `NewColorEffect` hands out next; the engine's palette-effect table starts at 1.
    pub next_color_effect: i32,
    /// `SetThingModel`: the `.3do` a thing should be drawn with instead of its template's, when a
    /// script swaps one (`ctf_main`'s team models are the only callers).
    pub thing_model: BTreeMap<usize, String>,
    /// The thing a pull or grip is holding, from `jkSetTarget`. `step` drags it toward the player and
    /// `jkEndTarget` releases it — the engine keeps this in `jkPlayer.c`, which this tree does not
    /// carry, so the drag itself is calibrated (§16.5).
    pub force_target: Option<usize>,
    /// `jkSetTargetColors`' tint, in the order the script passed it. Recorded, not drawn.
    pub force_target_tint: Vec<f32>,
    /// The player's current weapon template, when a pickup has changed it.
    pub weapon: Option<String>,
    /// `jkSetSaberInfo`: the blade this player carries. The engine keeps it in
    /// `playerInfo->saberInfo` and `jkSaber.c` swings it; this build keeps the same numbers and
    /// sweeps the blade itself (`saber_swing`), because the swing animations are the puppet work.
    pub saber: BTreeMap<usize, SaberInfo>,
    /// Seconds until the next swing, so holding the button is not a machine gun of blades.
    pub saber_wait: f32,
    /// The armed inventory bin (`SITHBIN_*`), which is how the engine names a weapon or a
    /// force power.
    pub cur_bin: i32,
    /// The weapon the level starts the player with (its `walkplayer` template's `weapon=`,
    /// else the first projectile template).
    pub default_weapon: String,
    /// Powerups taken and counting down to respawn: `(thing index, seconds left)`.
    pub respawning: Vec<(usize, f32)>,
    /// Powerups the events slot brought back this frame, for the thing loop's `moved`.
    respawned: Vec<usize>,
    /// What the scripts actually changed, for the run report.
    pub adjoins_changes: usize,
    pub things_moved: usize,
}

/// `sithAI_FireWeapon` (`0x4eba10`) outcome. `Deferred` is the engine's own "return 1" for a
/// puppet-flagged actor: the handler schedules its cadence and sets `0x400` while the bolt
/// itself waits for the fire track's marker.
#[derive(Debug)]
enum FireOutcome {
    Refused,
    Deferred,
    Shot(usize),
}

/// The aim `sithAI_FireWeapon` computed from its own arithmetic (the sight direction with the
/// lead, lob and accuracy-jitter rules applied), handed to the shared spawn path so an AI bolt
/// leaves along the engine's direction from the engine's fire point rather than the body's yaw.
#[derive(Clone, Copy)]
struct AiAim {
    dir: [f32; 3],
    /// `WeaponFire`'s speed parameter: the lobbed shot's `len / weaponspeed`.
    speed_scale: f32,
    /// `ai + 0x82`: the oriented `fireoffset=` point the bolt is born at.
    origin: [f32; 3],
}


impl Runtime {
    pub fn new(level: &Level) -> Self {
        Self::new_inner(level, None)
    }

    /// Build with the level's one geometry record: the surface owners and sector adjoin graph are
    /// taken from it instead of being derived a second time.
    pub fn new_with_geometry(level: &Level, geometry: &crate::collide::LevelGeometry) -> Self {
        Self::new_inner(level, Some(geometry))
    }

    fn new_inner(level: &Level, geometry: Option<&crate::collide::LevelGeometry>) -> Self {
        // Template tables first: a placed thing's health and type come from its template,
        // not from a default. A projectile's damage is only meaningful if the actor it
        // hits started with the `health=` its template names.
        let template_health: BTreeMap<String, f32> = level
            .templates
            .iter()
            .map(|t| {
                (
                    t.name.to_ascii_lowercase(),
                    level.template_f32(&t.name, "health").unwrap_or(1.0),
                )
            })
            .collect();
        let template_types: BTreeMap<String, i32> = level
            .templates
            .iter()
            .map(|t| (t.name.to_ascii_lowercase(), thing_type(level.template_param(&t.name, "type"))))
            .collect();
        // `explode=<template>`: what the engine spawns where a projectile lands.
        let template_explode: BTreeMap<String, String> = level
            .templates
            .iter()
            .filter_map(|t| {
                level
                    .template_param(&t.name, "explode")
                    .map(|e| (t.name.to_ascii_lowercase(), e.to_ascii_lowercase()))
            })
            .collect();
        // A surface's sector: the level groups surfaces per room (`SECTOR` rows carry
        // `(first, count)` over the surface list). The app hands the one geometry record over;
        // the fallback keeps a bare `Runtime::new` usable for tests and probes.
        let surface_sector: Vec<i32> = geometry
            .map(|geometry| {
                geometry
                    .surface_sectors
                    .iter()
                    .map(|&sector| {
                        if sector == usize::MAX {
                            -1
                        } else {
                            sector as i32
                        }
                    })
                    .collect()
            })
            .unwrap_or_else(|| {
                let mut table = vec![-1i32; level.surfaces.len()];
                for (sector, room) in level.sectors.iter().enumerate() {
                    let (first, count) = room.surfaces;
                    for index in first..(first + count).min(level.surfaces.len()) {
                        table[index] = sector as i32;
                    }
                }
                table
            });
        // `sithAIAwareness` propagates through the **adjoin graph** (`ProcessEvent` recurses over a
        // sector's adjoin list, subtracting each adjoin's attenuation). An adjoin is shared by the
        // two surfaces of one opening, so pairing those surfaces gives the sector-to-sector links;
        // the cost is the adjoin row's `dist` float, which is the only per-portal number shipped.
        let sector_adjoins: Vec<Vec<(i32, f32)>> = if let Some(geometry) = geometry {
            geometry.sector_adjoins.clone()
        } else {
            let mut links: Vec<Vec<(i32, f32)>> = vec![Vec::new(); level.sectors.len()];
            // Each surface owns one adjoin row, and the row's `mirror` names the row of the surface on
            // the other side of the same opening (`World adjoins 0: 0x7 50 2.00` — flags, mirror,
            // dist). Pairing those rows gives the sector-to-sector links.
            let mut surface_of_adjoin: Vec<Option<usize>> = vec![None; level.adjoins.len()];
            for (index, surface) in level.surfaces.iter().enumerate() {
                if let Some(slot) = usize::try_from(surface.adjoin)
                    .ok()
                    .filter(|slot| *slot < level.adjoins.len())
                    && surface_of_adjoin[slot].is_none()
                {
                    surface_of_adjoin[slot] = Some(index);
                }
            }
            for (adjoin, row) in level.adjoins.iter().enumerate() {
                let Ok(mirror) = usize::try_from(row.mirror) else { continue };
                let (Some(first), Some(second)) = (
                    surface_of_adjoin[adjoin],
                    surface_of_adjoin.get(mirror).copied().flatten(),
                ) else {
                    continue;
                };
                if mirror <= adjoin {
                    continue; // one link per opening
                }
                let (a, b) = (surface_sector[first], surface_sector[second]);
                if a < 0 || b < 0 || a == b {
                    continue;
                }
                // `ProcessEvent` subtracts the attenuation it finds through the adjoin's link; the
                // link is the mirror row, so its `dist` is the cost.
                let cost = level.adjoins.get(mirror).map(|a| a.dist).unwrap_or(0.0).max(0.0);
                links[a as usize].push((b, cost));
                links[b as usize].push((a, cost));
            }
            links
        };
        // `fireoffset=(x/y/z)`: where a shot leaves the shooter, in the template's own frame and
        // level units (x lateral, y forward, z up) — the port used to scale it by ten, which put a
        // Ree-Yees' muzzle 0.8 units ahead of a 0.07-radius body and made retail's point-blank hits
        // impossible. The port spawned every bolt a flat unit ahead of the eye as a stand-in; the
        // shipped templates carry the real numbers.
        let template_fire_offset: BTreeMap<String, [f32; 3]> = level
            .templates
            .iter()
            .filter_map(|t| {
                let raw = level.template_param(&t.name, "fireoffset")?;
                vec_param(Some(raw)).map(|v| (t.name.to_ascii_lowercase(), v))
            })
            .collect();
        // A thing's body: `size=x/y/z` gives the half-extents, so the radius is the larger of x/y
        // and the height is twice z; `radius=` alone gives a cylinder of the default height.
        let template_body: BTreeMap<String, (f32, f32)> = level
            .templates
            .iter()
            .map(|t| {
                let name = t.name.to_ascii_lowercase();
                // `radius=` is a real radius. `size=` is **not**: JK templates write it as a model
                // *scale* (`size=0.02/0.02/0.02` is a normal-sized actor), so reading it as
                // extents gives a two-centimetre body and no bolt ever lands. Until the engine's
                // own thing-radius rule is recovered, the body is the standard JK actor cylinder.
                let body = level
                    .template_f32(&t.name, "radius")
                    .map(|r| (r.max(0.3), 1.8))
                    .unwrap_or((0.6, 1.8));
                (name, body)
            })
            .collect();
        // The engine's collision radius is the template's `movesize=` (every shipped actor carries
        // it, and it is what `getthingmovesize` reads). The port used a 0.6 default, which made a
        // bolt count as a hit on any actor within 0.75 of its path, so a crowd absorbed every shot
        // aimed at the player. The height is the model height the templates imply. No floor: a bolt
        // ships `movesize=0.001` and `sithIntersect_CheckSphereIntersection` combines it with the
        // target's radius, so flooring it at 0.02 would make a bolt reach 0.019 units too far. The
        // projectile **wall** sweep floors it itself (`Runtime::step`), where a zero-radius ray is
        // what would let a bolt through a jamb.
        let template_hit: BTreeMap<String, (f32, f32)> = level
            .templates
            .iter()
            .map(|t| {
                let name = t.name.to_ascii_lowercase();
                let radius = level
                    .template_f32(&t.name, "movesize")
                    .or_else(|| level.template_f32(&t.name, "size"))
                    .unwrap_or(0.065);
                (name, (radius, radius * 2.8))
            })
            .collect();
        let template_phys_flags: BTreeMap<String, u32> = level
            .templates
            .iter()
            .filter_map(|t| {
                level
                    .template_param(&t.name, "physflags")
                    .and_then(|raw| {
                        let raw = raw.trim();
                        u32::from_str_radix(raw.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
                    })
                    .map(|flags| (t.name.to_ascii_lowercase(), flags))
            })
            .collect();
        let template_angvel: BTreeMap<String, [f32; 3]> = level
            .templates
            .iter()
            .filter_map(|t| {
                vec_param(level.template_param(&t.name, "angvel"))
                    .map(|v| (t.name.to_ascii_lowercase(), v))
            })
            .collect();
        // `mass=` is the other half of the gravity gate (`0x4f6270` reads `+0x204`): a thing's mass is
        // copied from the template, and a zero mass means the gravity arm never runs. The shipped
        // `_debris` template (`type=weapon physflags=0x201`) declares **no** `mass=` in any level, yet
        // its shrapnel falls in retail, so the engine's parse default is non-zero (`1.0`); reading
        // "absent" as zero is what left a created chip floating (`--things`).
        let template_mass: BTreeMap<String, f32> = level
            .templates
            .iter()
            .map(|t| {
                let mass = level
                    .template_param(&t.name, "mass")
                    .and_then(|raw| raw.trim().parse::<f32>().ok())
                    .unwrap_or(1.0);
                (t.name.to_ascii_lowercase(), mass)
            })
            .collect();
        // A thing's mass comes from its template (`sithThing_ParseArg`'s `mass=`, `+0x204`), which is
        // also the gravity arm's gate. Built here because the struct literal moves `template_mass`.
        let placed_mass: BTreeMap<usize, f32> = level
            .things
            .iter()
            .enumerate()
            .filter_map(|(index, thing)| {
                template_mass
                    .get(&thing.template.to_ascii_lowercase())
                    .copied()
                    .map(|mass| (index, mass))
            })
            .collect();
        // `thingflags=` (`sithThing_ParseArg`'s `typeflags` is the other word): a shot inherits it, and
        // `sithWeapon_WeaponFireProjectile`'s player pre-sweep tests `thingflags & 0x200000` on the
        // projectile before it emits `dodge`'s event — `+bryarbolt` ships `0x20000001`.
        let template_thingflags: BTreeMap<String, u32> = level
            .templates
            .iter()
            .filter_map(|t| {
                level
                    .template_param(&t.name, "thingflags")
                    .and_then(|raw| {
                        let raw = raw.trim();
                        u32::from_str_radix(raw.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
                    })
                    .map(|flags| (t.name.to_ascii_lowercase(), flags))
            })
            .collect();
        // The same parse for the `typeflags=` word: `sithWeapon_WeaponFireProjectile`'s player pre-sweep
        // tests `typeflags & 0x200000` on the projectile (`+bryarbolt` is `0x20440d`).
        let template_typeflags: BTreeMap<String, u32> = level
            .templates
            .iter()
            .filter_map(|t| {
                level
                    .template_param(&t.name, "typeflags")
                    .and_then(|raw| {
                        let raw = raw.trim();
                        u32::from_str_radix(raw.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
                    })
                    .map(|flags| (t.name.to_ascii_lowercase(), flags))
            })
            .collect();
        // The shipped `_debris` family is `type=weapon physflags=0x201`, yet it is rubble: the mover
        // lets it land. Its name chain is the actual distinction, because the gravity bit alone also
        // covers fired projectiles (`+grenade2 physflags=0x225`, `08tiebomb` the same), and a cog
        // creates the tie bomb with `CreateThingAtPos`, so "has an owner" cannot stand in for it.
        let in_debris_family = |name: &str| {
            let mut current = name;
            for _ in 0..8 {
                if current.eq_ignore_ascii_case("_debris") {
                    return true;
                }
                let Some(template) = level
                    .templates
                    .iter()
                    .find(|t| t.name.eq_ignore_ascii_case(current))
                else {
                    break;
                };
                match template.base.as_deref() {
                    Some(base) => current = base,
                    None => break,
                }
            }
            false
        };
        let template_debris: BTreeSet<String> = level
            .templates
            .iter()
            .filter(|t| in_debris_family(&t.name))
            .map(|t| t.name.to_ascii_lowercase())
            .collect();
        // `damageclass=` selects the hurt kind: `_weapon` ships energy, `_debris`/`08tiebomb`
        // impact, `_explosion`/`+grenade2` fire. The hit path passed `DAMAGE_ENERGY` for every shot,
        // which sent a grenade's fire and a tie bomb's impact down the energy branch.
        let template_damageclass: BTreeMap<String, u32> = level
            .templates
            .iter()
            .filter_map(|t| {
                level
                    .template_param(&t.name, "damageclass")
                    .and_then(|raw| {
                        let raw = raw.trim();
                        u32::from_str_radix(
                            raw.trim_start_matches("0x").trim_start_matches("0X"),
                            16,
                        )
                        .ok()
                    })
                    .map(|kind| (t.name.to_ascii_lowercase(), kind))
            })
            .collect();
        // `timer=` is the thing's life left, in seconds: `THINGPARAM_TIMER` stores
        // `lifeLeftSec * 1000` in `msecLifeLeft` and `sithThing_Update` destroys the thing when it
        // runs out. It is what makes an impact spark a spark — `+laserhit` ships `timer=0.5`.
        let template_timer: BTreeMap<String, f32> = level
            .templates
            .iter()
            .filter_map(|t| {
                level
                    .template_f32(&t.name, "timer")
                    .filter(|secs| *secs >= 0.0)
                    .map(|secs| (t.name.to_ascii_lowercase(), secs))
            })
            .collect();
        let template_minheadpitch: BTreeMap<String, f32> = level
            .templates
            .iter()
            .filter_map(|t| {
                level
                    .template_f32(&t.name, "minheadpitch")
                    .or_else(|| level.template_f32("_actor", "minheadpitch"))
                    .map(|v| (t.name.to_ascii_lowercase(), v))
            })
            .collect();
        let template_maxheadpitch: BTreeMap<String, f32> = level
            .templates
            .iter()
            .filter_map(|t| {
                level
                    .template_f32(&t.name, "maxheadpitch")
                    .or_else(|| level.template_f32("_actor", "maxheadpitch"))
                    .map(|v| (t.name.to_ascii_lowercase(), v))
            })
            .collect();
        // A row the active difficulty excludes is not created: the retail loader leaves its pool
        // slot empty (type 0), which is also why a later `CreateThing` reuses those low slots.
        let excluded: Vec<bool> = level
            .things
            .iter()
            .map(|t| t.flags & TF_NOT_MEDIUM != 0)
            .collect();
        Self {
            things: level
                .things
                .iter()
                .enumerate()
                .map(|(index, t)| ThingState {
                    pos: t.pos,
                    next_touch_ms: 0,
                    insert: template_insert(&t.template.to_ascii_lowercase()),
                    yaw: t.yaw,
                    pitch: t.pitch,
                    roll: t.roll,
                    head_pitch: 0.0,
                    sector: t.sector,
                    // The **effective** word (`sithThing_ParseThingArg`): a placement's
                    // `thingflags=` replaces the template's; a placement without one inherits
                    // it. The live word is what `0x4fbe10`'s `test ah,1` and every other
                    // thingflags read must see (#85).
                    flags: if excluded[index] {
                        0
                    } else if t.flags != 0 {
                        t.flags
                    } else {
                        template_thingflags
                            .get(&t.template.to_ascii_lowercase())
                            .copied()
                            .unwrap_or(0)
                    },
                    health: if excluded[index] {
                        0.0
                    } else {
                        template_health
                            .get(&t.template.to_ascii_lowercase())
                            .copied()
                            .unwrap_or(1.0)
                    },
                    frame: 0.0,
                    target: None,
                    frames: t.frames.clone(),
                    destroyed: excluded[index],
                    vel: t.vel,
                    // Placed things are not projectiles; a `CreateThing` gives them their
                    // damage, and only a fired weapon decays.
                    damage: 0.0,
                    min_damage: 0.0,
                    decay: 0.0,
                    extra_speed: 0.0,
                    ai_target: None,
                    ai_wait: 0.0,
                    ai_burst: 0.0,
                    arrived: false,
                    phys_flags: template_phys_flags
                        .get(&t.template.to_ascii_lowercase())
                        .copied()
                        .unwrap_or(0),
                    jk_flags: 0,
                    attach: None,
                    floor_attach: None,
                    owner: None,
                    attach_offset: [0.0; 3],
                    attach_flags: 0,
                    spin: None,
                    // The template's `angvel=` seeds the angular velocity; `ThingState::step`
                    // integrates it and the angles wrap.
                    rot_vel: template_angvel
                        .get(&t.template.to_ascii_lowercase())
                        .copied()
                        .unwrap_or([0.0; 3]),
                    paused: false,
                    pulse: 0.0,
                    pulse_at: 0.0,
                    light: 0.0,
                    light_end: 0.0,
                    light_rate: 0.0,
                })
                .collect(),
            templates: level.things.iter().map(|t| t.template.clone()).collect(),
            types: level
                .things
                .iter()
                .enumerate()
                .map(|(index, t)| {
                    if excluded[index] {
                        0
                    } else {
                        thing_type(level.template_param(&t.template, "type"))
                    }
                })
                .collect(),
            template_types,
            template_health,
            template_move: level
                .templates
                .iter()
                .filter_map(|t| {
                    level.template_param(&t.name, "move")
                        .map(|m| (t.name.to_ascii_lowercase(), m.to_string()))
                })
                .collect(),
            template_phys_flags,
            template_mass,
            template_drag: level
                .templates
                .iter()
                .map(|t| {
                    let drag = |key| level.template_f32(&t.name, key).filter(|d| *d >= 0.0).unwrap_or(0.0);
                    (t.name.to_ascii_lowercase(), (drag("surfdrag"), drag("staticdrag")))
                })
                .collect(),
            gravity: level.gravity,
            shots_fired: 0,
            encounter_events: Vec::new(),
            encounter_shot_time: BTreeMap::new(),
            encounter_events_enabled: encounter_capture(),
            prints: Vec::new(),
            hud_message: None,
            string_buffer: String::new(),
            loaded_sounds: Vec::new(),
            weapon_cog: None,
            weapon_rate: 0.0,
            weapon_ready: 0.0,
            auto_switch: 1,
            auto_pickup: 1,
            super_flags: 0,
            weapon_mesh: BTreeMap::new(),
            pov_model: BTreeMap::new(),
            pov_key: BTreeMap::new(),
            pov_key_loop: BTreeMap::new(),
            waggle: BTreeMap::new(),
            waggle_rate: BTreeMap::new(),
            key_lengths: BTreeMap::new(),
            last_shot: BTreeMap::new(),
            last_hit: BTreeMap::new(),
            armed_mode: BTreeMap::new(),
            major_mode: BTreeMap::new(),
            movement_mode: BTreeMap::new(),
            template_puppet: level.templates.iter().filter(|t| level.template_param(&t.name, "puppet").is_some())
                .map(|t| t.name.to_ascii_lowercase()).collect(),
            cur_weapon_thing: None,
            thing_class_cog: BTreeMap::new(),
            item_cog_spawns: Vec::new(),
            thing_capture_cog: BTreeMap::new(),
            class_skill: BTreeMap::new(),
            saber_enabled: true,
            saber_cam: 0,
            item_cog: BTreeMap::new(),
            item_max: BTreeMap::new(),
            item_min: BTreeMap::new(),
            bin_cog_index: BTreeMap::new(),
            pending_cogs: Vec::new(),
            spawned_cogs: std::collections::BTreeSet::new(),
            messages: Vec::new(),
            caller_cog: -1,
            caller_script: String::new(),
            cog_names: Default::default(),
            messages_sent: 0,
            template_angvel,
            template_collide: level.templates.iter().map(|t| {
                let mode = level.template_param(&t.name, "collide").and_then(|s| s.parse().ok()).unwrap_or(0);
                (t.name.to_ascii_lowercase(), mode)
            }).collect(),
            template_vel: level
                .templates
                .iter()
                .map(|t| {
                    (
                        t.name.to_ascii_lowercase(),
                        vec_param(level.template_param(&t.name, "vel")).unwrap_or([0.0; 3]),
                    )
                })
                .collect(),
            template_inv: level
                .templates
                .iter()
                .filter_map(|t| {
                    level
                        .template_f32(&t.name, "inv")
                        .map(|bin| (t.name.to_ascii_lowercase(), bin as i32))
                })
                .collect(),
            template_count: level
                .templates
                .iter()
                .filter_map(|t| {
                    level
                        .template_f32(&t.name, "count")
                        .map(|count| (t.name.to_ascii_lowercase(), count))
                })
                .collect(),
            template_bins: BTreeMap::new(),
            template_max_health: level
                .templates
                .iter()
                .filter_map(|t| {
                    level
                        .template_f32(&t.name, "maxhealth")
                        .map(|h| (t.name.to_ascii_lowercase(), h))
                })
                .collect(),
            template_body,
            template_hit,
            template_height: level.templates.iter().filter_map(|t| {
                level.template_f32(&t.name, "height").map(|height| (t.name.to_ascii_lowercase(), height))
            }).collect(),
            template_explode,
            template_fire_offset,
            template_timer,
            template_minheadpitch,
            template_maxheadpitch,
            template_debris,
            template_thingflags,
            template_typeflags,
            template_damageclass,
            template_soundclass: level
                .templates
                .iter()
                .filter_map(|t| {
                    level
                        .template_param(&t.name, "soundclass")
                        .map(|name| (t.name.to_ascii_lowercase(), sound_class_key(name)))
                })
                .collect(),
            sound_classes: BTreeMap::new(),
            sound_rng: 0x9E37_79B9_7F4A_7C15,
            last_sound: BTreeMap::new(),
            thing_timer: BTreeMap::new(),
            player_shields: 100.0,
            player_dead: false,
            ir_mode: false,
            map_mode_flags: 0,
            sector_links: Vec::new(),
            link_masks: Default::default(),
            surface_links: Vec::new(),
            mana_regen: MANA_REGEN,
            mana_max: MANA_MAX,
            actor_flags: Default::default(),
            ai_flags: Default::default(),
            item_flags: Default::default(),
            life_left: Default::default(),
            life_total: Default::default(),
            // A thing's mass comes from its template (`sithThing_ParseArg`'s `mass=`, `+0x204`), which is
            // also the gravity arm's gate. Scripts can still overwrite it with `SetThingMass`.
            mass: placed_mass,
            user_data: Default::default(),
            force: Default::default(),
            actor_weapon: Default::default(),
            play_mode: Default::default(),
            signature: Vec::new(),
            view_list: Vec::new(),
            view_at: 0,
            last_params: Vec::new(),
            thing_links: Vec::new(),
            pending_messages: Vec::new(),
            last_activate: f32::NEG_INFINITY,
            solids: None,
            seated_load: false,
            template_min_damage: level
                .templates
                .iter()
                .filter_map(|t| {
                    level
                        .template_f32(&t.name, "mindamage")
                        .map(|d| (t.name.to_ascii_lowercase(), d))
                })
                .collect(),
            template_rate: level
                .templates
                .iter()
                .filter_map(|t| {
                    level
                        .template_f32(&t.name, "rate")
                        .map(|r| (t.name.to_ascii_lowercase(), r))
                })
                .collect(),
            // `typeflags=` is hexadecimal (`0x20440d`), so it cannot go through the f32
            // parser used for the other params: that returns None and every weapon looked
            // non-decaying.

            template_decay: level
                .templates
                .iter()
                .filter(|t| {
                    level
                        .template_param(&t.name, "typeflags")
                        .and_then(|raw| {
                            let raw = raw.trim();
                            u32::from_str_radix(raw.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
                        })
                        .is_some_and(|flags| flags & 0x4000 != 0)
                })
                .map(|t| t.name.to_ascii_lowercase())
                .collect(),
            // Bit 0 of the same word: `SITHEXPLOSION_FLAG_ANIMATED_SPRITE`, which is what makes
            // an impact spark walk its frames instead of showing the first one for its whole life.
            template_sprite_anim: level
                .templates
                .iter()
                .filter(|t| {
                    level
                        .template_param(&t.name, "typeflags")
                        .and_then(|raw| {
                            let raw = raw.trim();
                            u32::from_str_radix(raw.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
                        })
                        .is_some_and(|flags| flags & 0x1 != 0)
                })
                .map(|t| t.name.to_ascii_lowercase())
                .collect(),
            template_respawn: level
                .templates
                .iter()
                .filter_map(|t| {
                    level
                        .template_f32(&t.name, "respawn")
                        .filter(|s| *s > 0.0)
                        .map(|secs| (t.name.to_ascii_lowercase(), secs))
                })
                .collect(),
            template_damage: level
                .templates
                .iter()
                .map(|t| {
                    (
                        t.name.to_ascii_lowercase(),
                        level.template_f32(&t.name, "damage").unwrap_or(0.0),
                    )
                })
                .collect(),
            sectors: level
                .sectors
                .iter()
                .map(|_| SectorState { thrust: [0.0; 3], light: 0.0, light_from: 0.0, light_end: 0.0, light_rate: 0.0, tint: [0.0; 3], colormap: 0 })
                .collect(),
            sector_flags: level.sectors.iter().map(|s| s.flags).collect(),
            sector_light_baked: level
                .sectors
                .iter()
                .map(|s| (s.ambient_light + s.extra_light).clamp(0.0, 1.0))
                .collect(),
            sector_ambient_light: level.sectors.iter().map(|s| s.ambient_light).collect(),
            adjoin_flags: level.adjoins.iter().map(|a| a.flags).collect(),
            // `sithAI_GetThingsInView` (`0x4ebe80`) recurses with
            // `depth + adjoin.dist + mirror.dist` — the `World adjoins` `dist` column of both
            // sides of the opening, nonzero on every shipped level.
            adjoin_depth: level
                .adjoins
                .iter()
                .map(|a| a.dist + usize::try_from(a.mirror).ok().and_then(|m| level.adjoins.get(m)).map_or(0.0, |m| m.dist))
                .collect(),
            surface_adjoin: level.surfaces.iter().map(|s| s.adjoin).collect(),
            surface_sector,
            surface_geomode: level
                .surfaces
                .iter()
                .map(|s| if s.material >= 0 { s.geo.max(0) } else { 0 })
                .collect(),
            surfaces: level
                .surfaces
                .iter()
                .map(|s| {
                    let mut sum = [0.0f32; 3];
                    let mut n = 0.0f32;
                    for &(v, _) in &s.verts {
                        if let Some(p) = level.vertices.get(v as usize) {
                            for k in 0..3 {
                                sum[k] += p[k];
                            }
                            n += 1.0;
                        }
                    }
                    let center = if n > 0.0 {
                        [sum[0] / n, sum[1] / n, sum[2] / n]
                    } else {
                        [0.0; 3]
                    };
                    SurfaceState { center, cel: 0.0, anim: 0.0, slide: [0.0; 3], next: 0.0, light: 0.0, light_end: 0.0, light_rate: 0.0 }
                })
                .collect(),
            surface_flags: level.surfaces.iter().map(|s| s.surfflags).collect(),
            surface_touched_ms: BTreeMap::new(),
            sounds: Vec::new(),
            sound_loops: Vec::new(),
            timers: 0.0,
            inv: BTreeMap::new(),
            inv_available: BTreeMap::new(),
            inv_activated: BTreeMap::new(),
            bin_active: BTreeMap::new(),
            bin_due: Vec::new(),
            bin_wait: BTreeMap::new(),
            time: 0.0,
            game_clock: std::time::Duration::ZERO,
            fire_wait: 0.0,
            next_fire: 0.0,
            master_cog: None,
            multi_mode_flags: 0,
            play_key: BTreeMap::new(),
            keys: BTreeMap::new(),
            key_base: BTreeMap::new(),
            face_geomode: BTreeMap::new(),
            camera_focus: [None; 7],
            camera_secondary: [None; 7],
            current_camera: 0,
            camera_cycle: 0,
            script_camera: false,
            shake_pos: [0.0; 3],
            shake_angle: [0.0; 3],
            shake_pos_rate: 0.0,
            shake_angle_rate: 0.0,
            geomode_epoch: 0,
            surface_epoch: 0,
            scroll_epoch: 0,
            surface_cels: Vec::new(),
            material_cels: BTreeMap::new(),
            material_names: BTreeMap::new(),
            material_anims: Vec::new(),
            material_cel: BTreeMap::new(),
            face_light_mode: BTreeMap::new(),
            thing_geomode: BTreeMap::new(),
            sound_volume: Vec::new(),
            sound_pitch: Vec::new(),
            sound_fade: BTreeMap::new(),
            sector_ambient: BTreeMap::new(),
            player_respawn_mask: 0,
            damage_applied: BTreeMap::new(),
            sky_scroll: [[0.0; 2]; 2],
            mount_wait_until: 0.0,
            ai: BTreeMap::new(),
            template_weapon: level
                .templates
                .iter()
                .filter_map(|t| {
                    level
                        .template_param(&t.name, "weapon")
                        .map(|w| (t.name.to_ascii_lowercase(), w.to_ascii_lowercase()))
                })
                .collect(),
            template_weapon2: level
                .templates
                .iter()
                .filter_map(|t| {
                    level
                        .template_param(&t.name, "weapon2")
                        .map(|w| (t.name.to_ascii_lowercase(), w.to_ascii_lowercase()))
                })
                .collect(),
            ai_deferred_fire: BTreeMap::new(),
            ai_fire_lock: BTreeMap::new(),
            ai_move: BTreeMap::new(),
            ai_look: BTreeMap::new(),
            ai_last_seen: BTreeMap::new(),
            ai_last_seen_at: BTreeMap::new(),
            ai_detected: std::collections::BTreeSet::new(),
            backpack: BTreeMap::new(),
            ai_cleared: std::collections::BTreeSet::new(),
            ai_detect_at: std::collections::BTreeMap::new(),
            ai_inst: BTreeMap::new(),
            ai_next_due: BTreeMap::new(),
            ai_disabled_global: false,
            ai_turret_down: BTreeMap::new(),
            ai_force: BTreeMap::new(),
            ai_force_cool: BTreeMap::new(),
            ai_saber: BTreeMap::new(),
            ai_saber_cool: BTreeMap::new(),
            ai_special: BTreeMap::new(),
            ai_jump: BTreeMap::new(),
            ai_move_speed: BTreeMap::new(),
            ai_move_lock: BTreeMap::new(),
            ai_look_dir: BTreeMap::new(),
            ai_home: BTreeMap::new(),
            ai_home_look: BTreeMap::new(),
            ai_flee_until: BTreeMap::new(),
            ai_withdraw: BTreeMap::new(),
            ai_hitrun: BTreeMap::new(),
            ai_randomturn: BTreeMap::new(),
            sector_awareness: BTreeMap::new(),
            awareness_event_at: BTreeMap::new(),
            sector_adjoins,
            ai_fire_target: BTreeMap::new(),
            ai_flee: BTreeMap::new(),
            ai_classes: BTreeMap::new(),
            level_complete: false,
            end_level_called: false,
            autosave: false,
            music: None,
            music_volume: 1.0,
            template_maxthrust: level
                .templates
                .iter()
                .map(|t| (t.name.to_ascii_lowercase(), level.template_f32(&t.name, "maxthrust").unwrap_or(0.0)))
                .collect(),
            template_speed: level
                .templates
                .iter()
                .map(|t| {
                    let thrust = level.template_f32(&t.name, "maxthrust").unwrap_or(0.0);
                    let drag = level
                        .template_f32(&t.name, "surfdrag")
                        .filter(|d| *d > 0.0)
                        .unwrap_or(3.0);
                    (t.name.to_ascii_lowercase(), thrust / drag)
                })
                .collect(),
            // The raw `maxrotthrust=` the engine's turn blend reads from the thing (`thing + 0x14c`,
            // the same field `sithControl_PlayerMovement` scales mouse pitch by): the blend in
            // `ai_turn_blend` multiplies it by `_DAT_00523058` = 0.1. The port's old settled
            // `maxrotthrust / surfdrag` rate retired with `turn_toward`.
            template_rotthrust: level
                .templates
                .iter()
                .map(|t| {
                    (
                        t.name.to_ascii_lowercase(),
                        level.template_f32(&t.name, "maxrotthrust").unwrap_or(180.0),
                    )
                })
                .collect(),
            // `jumpspeed=` (`_actor` 1.5): what `sithPlayerActions_JumpWithVel` adds to the velocity
            // when `sithAI_Jump`'s puppet play has no keyframes to drive.
            template_jumpspeed: level
                .templates
                .iter()
                .map(|t| {
                    (
                        t.name.to_ascii_lowercase(),
                        level.template_f32(&t.name, "jumpspeed").unwrap_or(1.5),
                    )
                })
                .collect(),
            last_hold: 0.0,
            tint: [0.0; 3],
            color_effects: BTreeMap::new(),
            next_color_effect: 1,
            thing_model: BTreeMap::new(),
            force_target: None,
            force_target_tint: Vec::new(),
            weapon: None,
            saber: BTreeMap::new(),
            saber_wait: 0.0,
            cur_bin: 0,
            default_weapon: level
                .template_param("walkplayer", "weapon")
                .map(|w| w.to_string())
                .or_else(|| {
                    let mut names: Vec<&str> = level.templates.iter().map(|t| t.name.as_str()).collect();
                    names.sort();
                    names
                        .into_iter()
                        .find(|name| {
                            level.template_param(name, "type").is_some_and(|k| k.eq_ignore_ascii_case("weapon"))
                                && level.template_f32(name, "damage").unwrap_or(0.0) > 0.0
                                && level.template_param(name, "vel").is_some()
                        })
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "+bryarbolt".to_string()),
            respawning: Vec::new(),
            respawned: Vec::new(),
            adjoins_changes: 0,
            things_moved: 0,
        }
    }

    pub fn handle(level: &Level) -> World {
        World::new(Self::new(level))
    }

    pub fn handle_with_geometry(level: &Level, geometry: &crate::collide::LevelGeometry) -> World {
        World::new(Self::new_with_geometry(level, geometry))
    }

    /// `sithSurface_Update`'s cel animation: a surface with a rate walks its material's cels.
    ///
    /// The engine's arm is `if (flags & SITH_SURFACE_200000) && field_30 <= now` then
    /// `wallCel += (now - field_30) / field_34 + 1` with `field_34 = 1000 / fps` and the next due
    /// time snapped to the step grid, wrapping at `material->num_texinfo` (and honouring the
    /// ping-pong/bounce flags, which no shipped surface's `flags` word carries). A rate of 0 is a
    /// still surface.
    ///
    /// The renderer bakes a cel into the mesh, so a step bumps `surface_epoch` — measured at 1.4 ms to
    /// re-batch all of level 1's 4595 surfaces, i.e. under 2% of a frame at 15 fps. `ponytail:` the
    /// whole world is re-batched rather than the sectors holding the animated surface, which is the
    /// same ceiling `SetFaceGeoMode` already lives with.
    fn advance_surface_cels(&mut self) {
        let now = self.time;
        let mut changed = false;
        for (index, surface) in self.surfaces.iter_mut().enumerate() {
            // Nothing to animate when the material has one cel, or when the loader never handed the
            // counts over (the script-only checks run without a `.mat`).
            let cels = self.surface_cels.get(index).copied().unwrap_or(1);
            if cels < 2 || surface.anim <= 0.0 {
                continue;
            }
            let step = 1.0 / surface.anim.max(0.001);
            if surface.next <= 0.0 {
                surface.next = now + step;
                continue;
            }
            if now < surface.next {
                continue;
            }
            let elapsed = now - surface.next;
            surface.cel += (elapsed / step).floor() + 1.0;
            surface.next = step + now - elapsed % step;
            surface.cel %= cels as f32;
            changed = true;
        }
        if changed {
            self.surface_epoch += 1;
        }
    }

    /// `sithSurface_MaterialAnim`: a material's own cel walks at `fps` and every surface of that
    /// material that has no cel of its own draws it (`rdActive`'s `cel == -1` rule). The engine keeps
    /// the record on the same animation list as the surface cels and dispatches by flag
    /// (`SITH_SURFACE_METAL` -> `material->curCelNum`), which is what this walk is.
    fn advance_material_cels(&mut self) {
        let now = self.time;
        let mut changed = false;
        for (material, cel, next, fps) in self.material_anims.iter_mut() {
            let cels = self.material_cels.get(material).copied().unwrap_or(1);
            if cels < 2 {
                continue;
            }
            let step = 1.0 / fps.max(0.001);
            if *next <= 0.0 {
                *next = now + step;
                continue;
            }
            if now < *next {
                continue;
            }
            let elapsed = now - *next;
            *cel += (elapsed / step).floor() + 1.0;
            *next = step + now - elapsed % step;
            *cel %= cels as f32;
            self.material_cel.insert(*material, *cel);
            changed = true;
        }
        if changed {
            self.surface_epoch += 1;
        }
    }

    /// A scrolling surface (`SlideWall`) needs its texture re-laid, and only the scrolling batches
    /// have to be rebuilt for it: the offset is a pure function of the clock (`slide * time`), so
    /// [`Self::scroll_epoch`] moves with the frame and the renderer re-lays that subset.
    fn bump_scroll_epoch(&mut self) {
        if !self.surfaces.iter().any(|s| s.slide != [0.0; 3])
            && self.sky_scroll.iter().all(|rate| *rate == [0.0; 2])
        {
            return;
        }
        self.scroll_epoch += 1;
    }

    /// Create a thing from a template at a pose: the one constructor `CreateThing` (marker-relative)
    /// and `CreateThingAtPos` (absolute) share, so the two cannot drift apart.
    fn spawn_at(&mut self, template: String, pos: [f32; 3], sector: i32, angles: [f32; 3]) -> usize {
        let insert = template_insert(&template);
        let health = self.template_health.get(&template).copied().unwrap_or(1.0);
        let kind = self.template_types.get(&template).copied().unwrap_or(0);
        self.things.push(ThingState {
            pos,
            next_touch_ms: 0,
            insert,
            owner: None,
            pitch: angles[0],
            yaw: angles[1],
            roll: angles[2],
            head_pitch: 0.0,
            sector,
            flags: 0,
            health,
            frame: 0.0,
            target: None,
            frames: Vec::new(),
            destroyed: false,
            vel: self.template_vel.get(&template).copied().unwrap_or([0.0; 3]),
            damage: self.template_damage.get(&template).copied().unwrap_or(0.0),
            // A created thing starts with its template's `physflags=`: the engine copies the word, so a
            // spawned `_debris` chip keeps its gravity bit `0x1` (round 246).
            phys_flags: self.template_phys_flags.get(&template).copied().unwrap_or(0),
            jk_flags: 0,
            attach: None,
            floor_attach: None,
            attach_offset: [0.0; 3],
            attach_flags: 0,
            spin: None,
            rot_vel: self.template_angvel.get(&template).copied().unwrap_or([0.0; 3]),
            paused: false,
            pulse: 0.0,
            pulse_at: 0.0,
            light: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
            extra_speed: 0.0,
            ai_target: None,
            ai_wait: 0.0,
            ai_burst: 0.0,
            arrived: false,
            min_damage: self
                .template_min_damage
                .get(&template)
                .copied()
                .unwrap_or(0.0),
            decay: if self.template_decay.contains(&template) {
                self.template_rate.get(&template).copied().unwrap_or(0.0)
            } else {
                0.0
            },
        });
        self.templates.push(template);
        self.types.push(kind);
        let index = self.things.len() - 1;
        if kind == 5 {
            self.item_cog_spawns.push(index);
        }
        // A created thing inherits its template's mass, exactly like `physflags` (`sithThing_CreateThing`
        // copies both), and the gravity arm is gated on it.
        let mass = self
            .template_mass
            .get(&self.templates[index].to_ascii_lowercase())
            .copied();
        if let Some(mass) = mass {
            self.mass.insert(index, mass);
        }
        self.arm_life(index);
        index
    }

    /// `SetThingLight`/`ThingLight`/`ThingLightAnim`: set or ramp a thing's own light, which
    /// `sithSurface_SetThingLight` does with the same `(from, rate, to)` record as a sector fade.
    fn set_thing_light(&mut self, index: usize, value: f32, time: f32) {
        if let Some(thing) = self.things.get_mut(index) {
            if time > 0.0 {
                thing.light_end = value;
                thing.light_rate = (value - thing.light) / time;
            } else {
                thing.light = value;
                thing.light_end = value;
                thing.light_rate = 0.0;
            }
        }
    }

    /// The light a thing emits, summed per sector: what the renderer adds to a sector's own light.
    /// Retail lights a radius around the thing; this build has one baked sector light, so the sum is
    /// the granularity available (`ponytail:` a per-surface distance falloff is the upgrade).
    pub fn thing_light_by_sector(&self) -> Vec<f32> {
        let mut out = vec![0.0f32; self.sectors.len()];
        for thing in &self.things {
            if thing.light <= 0.0 || thing.destroyed {
                continue;
            }
            if let Some(slot) = usize::try_from(thing.sector).ok().and_then(|i| out.get_mut(i)) {
                *slot += thing.light;
            }
        }
        out
    }

    /// Advance a thing's light ramp: `sithSurface_Update`'s `0x400000` arm, the same one a sector fade
    /// and `SurfaceLightAnim` use.
    fn advance_thing_lights(&mut self, dt: f32) {
        for thing in self.things.iter_mut() {
            if thing.light_rate == 0.0 {
                continue;
            }
            thing.light += thing.light_rate * dt;
            let arrived = if thing.light_rate > 0.0 {
                thing.light >= thing.light_end
            } else {
                thing.light <= thing.light_end
            };
            if arrived {
                thing.light = thing.light_end;
                thing.light_rate = 0.0;
            }
        }
    }

    /// Advance `ChangeSoundVol`'s fades: the mixer reads `sound_volume`, so the ramp lives here.
    fn advance_sound_fades(&mut self, dt: f32) {
        let done: Vec<usize> = self
            .sound_fade
            .iter()
            .filter_map(|(channel, (target, rate))| {
                let volume = self.sound_volume.get_mut(*channel)?;
                *volume += rate * dt;
                let arrived = if *rate > 0.0 { *volume >= *target } else { *volume <= *target };
                if arrived {
                    *volume = *target;
                }
                arrived.then_some(*channel)
            })
            .collect();
        for channel in done {
            self.sound_fade.remove(&channel);
        }
    }

    /// `SetPOVShake`'s offsets decay toward zero at their own rate, the way `sithCamera_Update`
    /// subtracts `delta * dt` from every component and clamps at 0 — so a shot's recoil is a kick
    /// that recovers rather than an offset that sticks.
    fn decay_shake(&mut self, dt: f32) {
        let steps = [self.shake_pos_rate * dt, self.shake_angle_rate * dt];
        for (offset, step) in [(&mut self.shake_pos, steps[0]), (&mut self.shake_angle, steps[1])] {
            if step == 0.0 {
                continue;
            }
            for v in offset.iter_mut() {
                *v = if *v < 0.0 {
                    (*v + step).min(0.0)
                } else if *v > 0.0 {
                    (*v - step).max(0.0)
                } else {
                    0.0
                };
            }
        }
    }

    /// `SurfaceLightAnim` and a fading `SetSectorLight`: both ramp a light term onto the level's
    /// baked intensity and then settle, so a door's room brightens once and stays bright.
    fn advance_lights(&mut self, dt: f32) {
        let mut changed = false;
        for surface in self.surfaces.iter_mut() {
            if surface.light_rate == 0.0 {
                continue;
            }
            surface.light += surface.light_rate * dt;
            // Walking past the target means arriving; a rate is one-shot, like the engine's anim
            // records (`field_44`/`field_40`), which free themselves when the value reaches its end.
            let arrived = if surface.light_rate > 0.0 {
                surface.light >= surface.light_end
            } else {
                surface.light <= surface.light_end
            };
            if arrived {
                surface.light = surface.light_end;
                surface.light_rate = 0.0;
            }
            changed = true;
        }
        for sector in self.sectors.iter_mut() {
            if sector.light_rate == 0.0 {
                continue;
            }
            sector.light += sector.light_rate * dt;
            let arrived = if sector.light_rate > 0.0 {
                sector.light >= sector.light_end
            } else {
                sector.light <= sector.light_end
            };
            if arrived {
                sector.light = sector.light_end;
                sector.light_rate = 0.0;
            }
            changed = true;
        }
        if changed {
            self.surface_epoch += 1;
        }
    }

    /// `sithUpdate` (`0x4c4c00`) for a runtime with no player body: the time advance, the
    /// events, the surface tick and `sithThing_Update`, in the engine's order. The app runs the
    /// same slots through [`Self::sith_update_head`], the AI, [`Self::sith_surface_tick`] and
    /// [`Self::sith_thing_update`]; this form is the headless one the checks and tests drive.
    ///
    /// `geometry` is the caller's collider view, which the bodies' moves (and an AI's `CanWalk`)
    /// run against when it is given; `Runtime::solids` stands in otherwise.
    ///
    /// `ThingMotion::moved` and `ThingMotion::turned` name every thing whose transform changed;
    /// `blocked` and `arrived` are the script events this step already queued. A renderer can
    /// rewrite just those transforms instead of the whole level's.
    pub fn step(
        &mut self,
        geometry: Option<&crate::collide::Solids>,
        dt: f32,
    ) -> ThingMotion {
        self.sith_update_head(dt);
        self.sith_surface_tick(dt);
        let mut pass = self.thing_update_begin(geometry, dt);
        for index in 0..self.things.len() {
            self.sith_thing_update_one(&mut pass, geometry, index);
        }
        self.thing_update_end(pass)
    }

    /// `sithUpdate`'s (`0x4c4c00:24-31`) slots before the AI: the time advance, the sound mixer
    /// and the events. The port's event queue is the bin cadence and the item respawn countdown;
    /// the camera's shake decay (`sithCamera_Update`, which the engine runs from the render path)
    /// rides here too.
    pub fn sith_update_head(&mut self, dt: f32) {
        self.time += dt;
        self.advance_sound_fades(dt);
        // `sithInventory_SendFire`: an active bin with a delay sends its own cog a `fire` every delay,
        // which is what makes a held force power keep doing its thing. Re-armed while the bin stays in
        // `bin_active`, so `DeactivateBin` ends the loop.
        if !self.bin_due.is_empty() {
            let mut still: Vec<(f32, i32, i32, usize, f32)> = Vec::with_capacity(self.bin_due.len());
            for (due, bin, cog, player, delay) in std::mem::take(&mut self.bin_due) {
                if !self.bin_active.contains_key(&bin) {
                    continue;
                }
                if self.time >= due {
                    self.messages
                        .push(("fire".to_string(), Some(player), Some(cog), [bin as f64, 0.0]));
                    // `binWait = now + activationDelaySecs` in the engine: the cadence repeats.
                    still.push((self.time + delay.max(0.01), bin, cog, player, delay));
                } else {
                    still.push((due, bin, cog, player, delay));
                }
            }
            self.bin_due = still;
        }
        // Powerups with `respawn=` come back. The countdown runs in the same step as
        // everything else; a thing restored this way is alive and full-health again.
        if !self.respawning.is_empty() {
            let mut back: Vec<usize> = Vec::new();
            self.respawning.retain_mut(|(index, left)| {
                *left -= dt;
                if *left <= 0.0 {
                    back.push(*index);
                    false
                } else {
                    true
                }
            });
            for index in back {
                if let Some(thing) = self.things.get_mut(index) {
                    thing.destroyed = false;
                    let template = self.templates.get(index).cloned().unwrap_or_default();
                    thing.health = self.template_health.get(&template).copied().unwrap_or(1.0);
                }
                self.respawned.push(index);
            }
        }
        self.decay_shake(dt);
    }

    /// `sithSurface_Tick`'s slot (`0x4c4c00:34`): surface and material cels, scrolling, sector and
    /// thing light ramps. shortcut: the port's own advances stand in for `sithSurface_Tick`'s
    /// internals; transliterate that function when a surface mechanic disagrees.
    pub fn sith_surface_tick(&mut self, dt: f32) {
        self.decay_dynamic_tint_unowned(dt);
        self.advance_surface_cels();
        self.advance_material_cels();
        self.bump_scroll_epoch();
        self.advance_lights(dt);
        self.advance_thing_lights(dt);
    }

    /// `sithThing_Update` (`0x4cc1e0`) with the player body: every thing in index order, and at
    /// the local player's own index its physics and move through `player` (the app's
    /// [`Self::step_player`] call). The runtime is released around `player`, which runs scripts
    /// synchronously at its contacts.
    pub fn sith_thing_update(
        world: &World,
        geometry: Option<&crate::collide::Solids>,
        dt: f32,
        mut player: impl FnMut(),
    ) -> ThingMotion {
        let (mut pass, count) = {
            let mut rt = world.borrow_mut();
            let pass = rt.thing_update_begin(geometry, dt);
            let count = rt.things.len();
            (pass, count)
        };
        for index in 0..count {
            world.borrow_mut().sith_thing_update_one(&mut pass, geometry, index);
            if Some(index) == pass.player_at {
                // `sithPhysics_UpdateThing` and `sithThing_UpdateMove` for the player's body.
                player();
            }
        }
        world.borrow_mut().thing_update_end(pass)
    }

    /// The per-frame tables `sithThing_Update`'s loop reads, and the one-time load attach.
    fn thing_update_begin(&mut self, geometry: Option<&crate::collide::Solids>, dt: f32) -> ThingPass {
        let mut moved = std::mem::take(&mut self.respawned);
        // `ApplyForce(thing, v)`: the impulse the force scripts apply. `force_pull.cog` gives a pulled
        // item `30` units/s toward the player with gravity cleared (`ClearPhysicsFlags(thing, 0x1)`);
        // the impulse is folded into the thing's velocity once, which is what makes it fly in.
        //
        // This replaced an invented continuous drag: `jkSetTarget` is only the *highlight* the reticle
        // shows, and the real movement is this one impulse at the moment the power is released
        // (`force_pull.cog`'s `deactivated`, §16.5).
        if !self.force.is_empty() {
            for (index, impulse) in std::mem::take(&mut self.force) {
                if let Some(thing) = self.things.get_mut(index) {
                    for (slot, add) in thing.vel.iter_mut().zip(impulse) {
                        *slot += add;
                    }
                    moved.push(index);
                }
            }
        }
        // `sithPhysics.c`: a body in a `SITH_SECTOR_USETHRUST` sector gets `thrust * dt` added to its
        // velocity each step, exactly like gravity. Level 21's core tube lifts what stands in it.
        let thrust: Vec<[f32; 3]> = self.sectors.iter().map(|s| s.thrust).collect();
        // The player is **not** a generic mover: `step_player` integrates the body and consumes a
        // scripted velocity (`force_jump.cog`'s `SetThingVel`) into `player.vy`. Letting both loops run
        // meant the generic one moved the *thing*, hit the level, zeroed the velocity and rewound the
        // position, so the jump's impulse was spent before the walker ever saw it.
        let player_at = self.local_player();
        // Per-body radius, sampled **before** the mutable iteration: the seat borrows the
        // collider, which the loop below forbids.
        let body_radius: Vec<f32> = (0..self.things.len())
            .map(|index| {
                let Some(name) = self.templates.get(index) else {
                    return 0.065;
                };
                // Template names in the shipped data are already lowercase, so the common path is a
                // borrowed lookup; only a mixed-case name pays for the lowercased key.
                self.template_hit
                    .get(name)
                    .or_else(|| self.template_hit.get(&name.to_ascii_lowercase()))
                    .map(|(radius, _)| *radius)
                    .unwrap_or(0.065)
            })
            .collect();
        let stored = if geometry.is_none() { self.solids.clone() } else { None };
        // `sithThing_Initialize` attaches a placed physics body to the floor it stands over at
        // load (`FindFloor(thing, 1)`, the forced reach), so an actor keeps the level's authored
        // height instead of sagging to its sphere contact first. The pass runs once, on the first
        // step that has a collider.
        let seat_geometry = geometry.or(stored.as_deref());
        let mut seated_load = false;
        if !self.seated_load && let Some(solids) = seat_geometry {
            seated_load = true;
            for i in 0..self.things.len() {
                if Some(i) == player_at
                    || !matches!(self.types.get(i).copied(), Some(2 | 3))
                    || self.things[i].phys_flags & 0x1 == 0
                    || self.mass.get(&i).copied().unwrap_or(0.0) == 0.0
                    || self.things[i].attach.is_some()
                    || self.things[i].destroyed
                {
                    continue;
                }
                let t = &self.things[i];
                let radius = body_radius[i];
                let height = self.thing_height(Some(i), radius);
                let Some(sector) = usize::try_from(t.sector)
                    .ok()
                    .filter(|sector| *sector < solids.sectors.len())
                    .or_else(|| solids.sector_at(t.pos, 0.1))
                else {
                    continue;
                };
                let mut state = crate::collide::MoveState {
                    pos: t.pos, sector, vel: t.vel, remainder: [0.0; 3],
                    phys_flags: t.phys_flags, kind: self.types.get(i).copied().unwrap_or(0) as u32,
                    move_type: 1, collide: 1, flags: t.flags, attach: t.floor_attach,
                    attach_flags: t.attach_flags, water_surface: None, owner: None,
                };
                let previous = state.attach;
                solids.find_floor_for(&mut state, radius, height, self.gates(), true, Some(i), self.thing_up(i));
                self.floor_transition(&mut state, previous, Some(i), true);
                let anchor = state.attach.and_then(|a| a.thing)
                    .and_then(|target| self.floor_local_anchor(target.thing, state.pos));
                let thing = &mut self.things[i];
                thing.floor_attach = state.attach;
                thing.attach_flags = state.attach_flags;
                thing.phys_flags = state.phys_flags;
                thing.vel = state.vel;
                if let Some(anchor) = anchor { thing.attach_offset = anchor; }
            }
        }
        // Latch, never reset: the load-time attach is once (`sithThing_Initialize`). Assigning
        // the per-tick flag here cleared it every other tick and re-ran the creation attach.
        self.seated_load |= seated_load;
        ThingPass {
            dt,
            player_at,
            body_radius,
            thrust,
            stored,
            moved,
            turned: Vec::new(),
            blocked: Vec::new(),
            arrived: Vec::new(),
        }
    }

    /// One thing's turn in `sithThing_Update` (`0x4cc1e0:24-93`), in the engine's slot order.
    fn sith_thing_update_one(
        &mut self,
        pass: &mut ThingPass,
        geometry: Option<&crate::collide::Solids>,
        i: usize,
    ) {
        let dt = pass.dt;
        // 1. The lifetime countdown (`0x4cc1e0:24-61`): `timer=`/`SetLifeLeft` reaching zero
        // destroys the thing. Without this an impact spark lives forever, and a corpse never goes
        // away.
        if let Some(left) = self.life_left.get_mut(&i) {
            *left -= dt;
            if *left <= 0.0 {
                self.life_left.remove(&i);
                self.life_total.remove(&i);
                // A projectile whose `timer=` ran out never landed: it is a miss in the chronology.
                self.note_shot_lost(i);
                if let Some(thing) = self.things.get_mut(i) {
                    thing.destroyed = true;
                }
            }
        }
        // 2. `sithCog_UpdateThingTimer` (thingflags `0x60000`, `0x4cc1e0:64-66`): a passed
        // `SetThingTimer` deadline sends `timer` (4) to the thing's own scripts, which is how
        // `03_thinglight.cog` rotates its colours; `SetThingPulse` sends `pulse` every interval.
        if self.thing_timer.get(&i).is_some_and(|at| *at <= self.time) {
            self.thing_timer.remove(&i);
            for instance in self.thing_instances(i) {
                self.pending_messages.push((instance, "timer", [0.0; 2], None, None));
            }
        }
        if let Some(thing) = self.things.get_mut(i)
            && thing.pulse > 0.0
            && thing.pulse_at <= self.time
        {
            thing.pulse_at = self.time + thing.pulse;
            for instance in self.thing_instances(i) {
                self.pending_messages.push((instance, "pulse", [0.0; 2], None, None));
            }
        }
        // 3. The control arms (`0x4cc1e0:67-76`): `sithAI_Tick` for an AI thing. shortcut:
        // `sithExplosion_Update` and `sithParticle_Update` have no port yet.
        let ai_geometry = geometry.or(pass.stored.as_deref());
        self.sith_ai_tick(i, dt, ai_geometry);
        // 4. The type arms (`0x4cc1e0:77-86`).
        match self.types.get(i).copied() {
            Some(3) => self.sith_weapon_update(i, dt),
            _ if Some(i) == pass.player_at => self.sith_player_update(dt),
            // shortcut: `sithActor_Update` (drowning) has no port yet (#111).
            _ => {}
        }
        // 5. `sithPhysics_UpdateThing` (move=physics) or the frame path (move=path), then 6.
        // `sithThing_UpdateMove` (`0x4cc4c0`) inside the physics body's move.
        self.sith_thing_motion(pass, geometry, i);
        // 7. The puppet slot (`sithPuppet_UpdatePuppet`, `0x4cc1e0:93`): the thing-track `PlayKey`.
        // shortcut: the actors' animation input is still `animate_actors`' in the app.
        self.sith_puppet_thing_key(pass, i);
    }

    /// `sithWeapon_Update`'s decay, one step's worth: damage falls by `rate * dt`, floored
    /// at `mindamage`. A weapon whose typeflags lack `SITH_WF_DAMAGE_DECAY` (0x4000) keeps
    /// its damage. shortcut: the weapon's own movement arms are not ported.
    fn sith_weapon_update(&mut self, index: usize, dt: f32) {
        let Some(thing) = self.things.get_mut(index) else { return };
        if thing.decay > 0.0 && thing.damage > thing.min_damage {
            thing.damage = (thing.damage - thing.decay * dt).max(thing.min_damage);
        }
    }

    /// `sithPlayer_Update`'s slot for the local player: the dynamic tint decay and the force mana
    /// regeneration.
    fn sith_player_update(&mut self, dt: f32) {
        self.decay_dynamic_tint(dt);
        // Force mana regenerates — but only for a Jedi that has a pool. `kyle.cog`'s class script
        // carries the rule (`Set Mana to full`: `SetInv(player, 14, GetInv(player, 20) * 50)`), so a
        // rank-0 player has no mana to regenerate; retail's level-2 save carries no bin 14 for exactly
        // that reason, while the port used to credit a pool out of nothing. A pool a script or a save
        // already gave the player still regenerates whatever the rank.
        let rank = self.inv.get(&crate::force::RANK).copied().unwrap_or(0.0);
        let has_pool = self.inv.get(&14).copied().unwrap_or(0.0) > 0.0;
        if rank > 0.0 || has_pool {
            // `force_heal.cog` spends `20 * rank` of bin 14; the *rate* is calibrated (§16.5) because
            // the engine's own tick lives in `jkPlayer.c`, which this source tree does not carry.
            if let Some(mana) = self.inv.get_mut(&14) {
                *mana = (*mana + self.mana_regen * dt).min(self.mana_max);
            }
        }
    }

    /// The player's tint and mana when the runtime has no player thing to update them: a test
    /// runtime or a level with no `walkplayer`. The engine has no such case; the port's state is
    /// global, so it still advances.
    fn decay_dynamic_tint_unowned(&mut self, dt: f32) {
        if self.local_player().is_none() {
            self.sith_player_update(dt);
        }
    }

    /// `sithPhysics_UpdateThing` or the frame path, and the move.
    fn sith_thing_motion(
        &mut self,
        pass: &mut ThingPass,
        geometry: Option<&crate::collide::Solids>,
        i: usize,
    ) {
        let dt = pass.dt;
        let seat_geometry = geometry.or(pass.stored.as_deref());
        let seat = if Some(i) == pass.player_at { None } else {
            seat_geometry.and_then(|solids| self.move_gravity_body(solids, i, dt, pass.body_radius[i], &pass.thrust))
        };
        let moved = &mut pass.moved;
        let turned = &mut pass.turned;
        let t = &mut self.things[i];
        if Some(i) == pass.player_at {
            // Path moves still run — a script can walk the player along frames — but the body's
            // velocity is the walker's to integrate, and `t.vel` is left for it to consume.
            let rotating = t.rot_vel != [0.0; 3];
            let _ = t.step(dt);
            if rotating {
                turned.push(i);
            }
            self.sith_track_thing_pivot(pass, i);
            return;
        }
        // MoveThing already ran the body's sweep and FindFloor tail.
        if let Some((state, flat, crossed, anchor)) = seat {
            let attach_flipped = t.floor_attach.is_some() != state.attach.is_some();
            let fell = (state.pos[2] - t.pos[2]).abs() > 1e-9;
            t.pos = state.pos;
            t.sector = state.sector as i32;
            t.vel = state.vel;
            t.floor_attach = state.attach;
            t.attach_flags = state.attach_flags;
            t.phys_flags = state.phys_flags;
            if let Some(anchor) = anchor { t.attach_offset = anchor; }
            let rotating = t.rot_vel != [0.0; 3];
            let path_moved = t.step(dt);
            if rotating {
                turned.push(i);
            }
            // `moved` must also see a pure vertical fall inside one sector: `flat` is the
            // horizontal travel and the path arm only covers frame paths, so a dropping
            // body would keep a stale drawn transform until it landed.
            if flat > 0.0 || fell || crossed || attach_flipped || path_moved {
                moved.push(i);
            }
            self.sith_track_thing_pivot(pass, i);
            return;
        }
        let thrust = &pass.thrust;
        if let Some(drive) = usize::try_from(t.sector).ok().and_then(|s| thrust.get(s)).copied()
            && drive != [0.0; 3]
            // `sithPhysics_UpdateThingPhysics`'s thrust arm (`0x4f6270`) skips a body whose
            // physflags carry `0x400000` — the flag `_powerup` ships (`physflags=0x400000`), which
            // is why m5's four weapon pickups stay put in the sectors a script gives an updraft
            // instead of climbing to z 89.3. The player is excluded above: `step_player` owns it.
            && t.phys_flags & 0x400000 == 0
            && !t.destroyed
        {
            for k in 0..3 {
                t.vel[k] += drive[k] * dt;
            }
        }
        // A projectile flies straight; a door or lift follows its frame path. A thing
        // is one or the other, so both are checked and a projectile never has frames.
        let previous = t.pos;
        let flying = t.vel != [0.0; 3];
        if flying {
            for k in 0..3 {
                t.pos[k] += t.vel[k] * dt;
            }
        }
        let path_moved = t.step(dt);
        if t.rot_vel != [0.0; 3] {
            turned.push(i);
        }
        // A thing that runs into the level stops and sends `blocked`; nothing here moves a
        // projectile out of the world (the renderer's own bounds test removes those).
        if let Some(solids) = &self.solids
            && (flying || path_moved)
            && !t.destroyed
        {
            // ponytail: position/segment tests against the sector's own planes and faces, not
            // `collide::sweep`, so a thing crosses a shut adjoin (a closed force field) that
            // stops the player. The engine's `SearchForSurfaceCollisions` treats it as solid for
            // every body; putting projectiles and actors on the swept path is the upgrade.
            // The sector **tracks the move**: a bolt crosses several rooms per frame and the
            // engine enters each one (`sithThing_EnterSector`) as the move lands, while a stale
            // sector had the resolver pushing against a room the thing was not in (level 1's
            // door fight: a bolt "in" sector 210 pushed out by a sector-339 wall 0.37 short of
            // the body, while the LOS test called the same line clear).
            let probe = if self.types.get(i).copied() == Some(3) { 0.02 } else { 0.4 };
            let sector = solids
                .sector_at(t.pos, probe)
                .unwrap_or(t.sector.max(0) as usize);
            if sector as i32 != t.sector {
                t.sector = sector as i32;
            }
            // A projectile is not a 0.4-radius body: the engine collides it by its template's
            // own `movesize` (`+bryarbolt` is 0.001), and the fixed 0.4 made every bolt stop on
            // a doorway jamb half a unit short of its target — level 1's door and catwalk fights
            // never landed a shot because of it. Doors and lifts keep the old reach.
            let radius = if self.types.get(i).copied() == Some(3) {
                self.templates
                    .get(i)
                    .map(|name| name.to_ascii_lowercase())
                    .and_then(|name| self.template_hit.get(&name).map(|(r, _)| *r))
                    .unwrap_or(0.02)
                    .max(0.02)
            } else {
                0.4
            };
            let (resolved, hit) = solids.resolve_stepped(sector, t.pos, radius);
            // A push-out is one signal; a fast mover can also step clean through a plane between
            // two frames, so "the new position is in no sector" counts too. That is the swept
            // test a position-only resolver cannot make.
            let escaped = solids.sector_at(t.pos, 0.1).is_none();
            // A **projectile** that reaches a surface dies there: the engine's
            // `sithThing_Update` explodes the weapon and removes it, while a stopped-but-alive
            // bolt froze mid-air (its `vel` zeroed, its damage live) and could never hit again.
            // A `_debris` body is `type=weapon` but rubble, not a shot: it has to land on the
            // vertical stop below instead of dying on the first surface it brushes (round 246's
            // gate). Keying on the gravity bit was wrong: shipped projectiles fall too
            // (`+grenade2 physflags=0x225`, `08tiebomb`), so the template family is the test.
            let projectile = self.types.get(i).copied() == Some(3)
                && !self.templates.get(i).is_some_and(|template| {
                    self.template_debris
                        .contains(&template.to_ascii_lowercase())
                });
            let mut lost = false;
            let mut stopped = false;
            if hit && (resolved[0] - t.pos[0]).abs() + (resolved[1] - t.pos[1]).abs() > 0.01 {
                if std::env::var_os("BK_AI_TRACE").is_some() && projectile {
                    eprintln!("ai: shot {i} hit a surface at {:?} (push to {:?}, sector {sector})", t.pos, resolved);
                }
                if projectile {
                    // A **projectile** is a ray, not a body. A plane push that runs *along* the
                    // flight is the containment resolver sliding it out of a thin sector, not a
                    // wall (level 1's door fight: +0.02 in +y while the bolt climbed at +2.09);
                    // only a push that opposes the velocity is a surface in front of it.
                    let along = (resolved[0] - t.pos[0]) * t.vel[0]
                        + (resolved[1] - t.pos[1]) * t.vel[1]
                        + (resolved[2] - t.pos[2]) * t.vel[2];
                    if along >= 0.0 {
                        self.sith_track_thing_pivot(pass, i);
                        return;
                    }
                    t.destroyed = true;
                    lost = true;
                } else {
                    t.pos[0] = resolved[0];
                    t.pos[1] = resolved[1];
                    t.vel = [0.0; 3];
                    t.target = None;
                    stopped = true;
                }
            } else if escaped || solids.segment_blocked(previous, t.pos) {
                if std::env::var_os("BK_AI_TRACE").is_some() && projectile {
                    // Which face stopped it: a muzzle inside a jamb and a bolt aimed into a wall
                    // look identical in the position alone.
                    eprintln!(
                        "ai: shot {i} stopped from {:?} to {:?} (escaped {escaped}, blocker {:?})",
                        previous,
                        t.pos,
                        solids.segment_blocked_at(previous, t.pos)
                    );
                }
                if projectile {
                    t.destroyed = true;
                    lost = true;
                } else {
                    // Put it back where it last made sense and stop it: the segment test is what
                    // catches a mover that stepped clean over a wall between two frames.
                    t.pos = previous;
                    t.vel = [0.0; 3];
                    t.target = None;
                    stopped = true;
                }
            }
            if lost {
                // A projectile that dies on geometry is a miss in the encounter chronology.
                self.note_shot_lost(i);
                self.sith_track_thing_pivot(pass, i);
                return;
            }
            if stopped {
                pass.blocked.push(i);
                for instance in self.thing_instances(i) {
                    self.pending_messages.push((instance, "blocked", [0.0; 2], None, None));
                }
            }
        }
        if path_moved || flying {
            pass.moved.push(i);
        }
        self.sith_track_thing_pivot(pass, i);
    }

    /// The frame path's turn and arrival, in the path slot: `RotatePivot`'s pivot turn, then the
    /// `arrived` a finished frame move sends to the cogs linked to the thing.
    fn sith_track_thing_pivot(&mut self, pass: &mut ThingPass, index: usize) {
        let dt = pass.dt;
        // `RotatePivot`: a thing turning about the pivot its frame names, a whole turn of `seconds`.
        // The pose is a function of progress from the pose it started at, so a long frame cannot
        // drift the way an accumulator would.
        if let Some(spin) = self.things[index].spin.clone() {
            let done = (spin.done + dt).min(spin.duration);
            let t = (done / spin.duration.max(1e-4)).clamp(0.0, 1.0);
            let angles = [
                spin.angles[0] * t,
                spin.angles[1] * t,
                spin.angles[2] * t,
            ];
            // A positive yaw turns +Y toward -X, which is this world's own convention (`yaw_axes` in
            // the renderer). Pitch is degrees above the horizon. `ponytail:` pitch and roll turn the
            // *thing* but not the arm it swings on; every shipped `RotatePivot` pivots on yaw.
            let turn = Quat::from_rotation_z(angles[1].to_radians());
            let d = Vec3::from(spin.start_pos) - Vec3::from(spin.centre);
            let orbited = Vec3::from(spin.centre) + turn * d;
            if let Some(thing) = self.things.get_mut(index) {
                thing.pos = [orbited.x, orbited.y, orbited.z];
                thing.pitch = spin.start_angles[0] + angles[0];
                thing.yaw = spin.start_angles[1] + angles[1];
                thing.roll = spin.start_angles[2] + angles[2];
                thing.spin = if done >= spin.duration { None } else { Some(PivotSpin { done, ..spin }) };
            }
            if !pass.moved.contains(&index) {
                pass.moved.push(index);
            }
            if !pass.turned.contains(&index) {
                pass.turned.push(index);
            }
        }
        if let Some(thing) = self.things.get_mut(index)
            && thing.arrived
        {
            thing.arrived = false;
            pass.arrived.push(index);
            for instance in self.thing_instances(index) {
                self.pending_messages.push((instance, "arrived", [0.0; 2], None, None));
            }
        }
    }

    /// `PlayKey` on a **thing** (a script camera's `MAcamera.key`) moves its pose: the key's single
    /// node carries the dolly, relative to the pose the key started from. Level 14's opening key walks
    /// its `droidcam` from (39.47, 11.38, 35.15) to about (39.65, -6.6, 28.4) — onto Kyle — where the
    /// retail shot looks down at him; without it the view stays at the placed pose. Actors' `PlayKey`
    /// names a puppet track, whose nodes pose the *model*, so only a key whose single node is a camera
    /// box is a thing track. `ponytail:` a mesh-name test, not the engine's root-track rule in
    /// `sithPuppet_UpdateThingMove`; the upgrade path is to read that rule and drop the name test.
    fn sith_puppet_thing_key(&mut self, pass: &mut ThingPass, thing: usize) {
        let Some((name, _flags, start)) = self.play_key.get(&thing).cloned() else { return };
        let Some(key) = self.keys.get(&name) else { return };
        if key.nodes.len() != 1 {
            return;
        }
        let node = &key.nodes[0];
        if !node.mesh.to_ascii_lowercase().contains("camera") {
            return;
        }
        let Some(base) = self.key_base.get(&thing).copied() else { return };
        let Some(pose) = node.at(start as f32 + self.time * key.fps) else { return };
        if std::env::var_os("BK_KEY_TRACE").is_some() {
            println!(
                "key: thing {thing} {name} start {start} time {:.3} fps {} frame {:.2}",
                self.time,
                key.fps,
                start as f32 + self.time * key.fps
            );
        }
        let Some(t) = self.things.get_mut(thing) else { return };
        let (sin, cos) = base.1.to_radians().sin_cos();
        t.pos = [
            base.0[0] + pose.pos[0] * cos - pose.pos[1] * sin,
            base.0[1] + pose.pos[0] * sin + pose.pos[1] * cos,
            base.0[2] + pose.pos[2],
        ];
        t.yaw = base.1 + pose.rot[1];
        t.pitch = base.2 + pose.rot[0];
        t.roll = base.3 + pose.rot[2];
        if std::env::var_os("BK_KEY_TRACE").is_some() {
            println!("key: thing {thing} pos now {:?}", t.pos);
        }
        pass.moved.push(thing);
    }

    /// After `sithThing_Update`'s loop: the attached things follow their parents. shortcut: a
    /// remainder — the engine pushes riders inside the parent's `MoveThing`.
    fn thing_update_end(&mut self, pass: ThingPass) -> ThingMotion {
        let ThingPass { mut moved, turned, blocked, arrived, .. } = pass;
        self.update_attachments(&mut moved);
        self.things_moved += moved.len();
        ThingMotion {
            moved,
            turned,
            blocked,
            arrived,
        }
    }

    /// Place every attached thing at `parent.position + parent.rotation * offset`, and carry the
    /// parent's facing, which is what makes an attached thing *ride* rather than float.
    ///
    /// `ponytail:` one level deep and in index order, so a chain of attachments resolves only when
    /// the parent has a lower index — the engine walks a child list instead. No shipped script
    /// attaches a thing to a thing that is itself attached.
    fn update_attachments(&mut self, moved: &mut Vec<usize>) {
        for index in 0..self.things.len() {
            let Some(parent) = self.things[index].attach else { continue };
            let Some(from) = self.things.get(parent).map(|p| (p.pos, p.yaw, p.sector)) else {
                continue;
            };
            let offset = self.things[index].attach_offset;
            // The parent's own rotation, in this world's convention: a positive yaw turns +Y toward
            // -X (see `yaw_axes` in the renderer), which is `from_rotation_z(+yaw)`.
            let turn = Quat::from_rotation_z(from.1.to_radians());
            let orbited = Vec3::from(from.0) + turn * Vec3::from(offset);
            let next = [orbited.x, orbited.y, orbited.z];
            let sector = from.2;
            if let Some(thing) = self.things.get_mut(index) {
                thing.pos = next;
                thing.yaw = from.1;
                thing.sector = sector;
            }
            if !moved.contains(&index) {
                moved.push(index);
            }
        }
    }

    /// The switched flags, for the solver: what `SetAdjoinFlags`/`ClearAdjoinFlags` and
    /// `SetSectorAdjoins`/`SetSectorFlags` have done to the world so far.
    pub fn gates(&self) -> Gates<'_> {
        Gates { sectors: &self.sector_flags, adjoins: &self.adjoin_flags }
    }

    /// Whether this module implements a verb, so the interpreter routes it here
    /// rather than to its own builtins. Kept beside `call` so both lists are read
    /// together.
    pub fn handles(verb: &str) -> bool {
        matches!(
            verb.to_ascii_lowercase().as_str(),
            "getthingpos"
                | "setthingpos"
                | "getthingrot"
                | "getthinglvec"
                | "setthingrot"
                | "setthinglook"
                | "setthingpulse"
                | "getthingsector"
                | "setthingsector"
                | "getthingflags"
                | "aisetmode"
                | "aiclearmode"
                | "aisetmovespeed"
                | "aisetmovepos"
                | "aisetmovething"
                | "aisetlookpos"
                | "aisetmoveframe"
                | "aisetlookframe"
                | "getmastercog"
                | "setmastercog"
                | "getgoalframe"
                | "clearthingflags"
                | "ismulti"
                | "isserver"
                | "getnumplayersinteam"
                | "getteamscore"
                | "getplayerscore"
                | "getplayerteam"
                | "setplayerscore"
                | "setplayerteam"
                | "getmultimodeflags"
                | "setmultimodeflags"
                | "clearmultimodeflags"
                | "sendtrigger"
                | "playkey"
                | "stopanim"
                | "jumptoframe"
                | "setmountwait"
                | "jkclearflags"
                | "setfacegeomode"
                | "getfacegeomode"
                | "setfirewait"
                | "setgoalflags"
                | "cleargoalflags"
                | "setthingflags"
                | "getthinghealth"
                | "gethealth"
                | "getleveltime"
                | "isthingvisible"
                | "getthingmass"
                | "setthingmass"
                | "getthinguserdata"
                | "setthinguserdata"
                | "setthingtimer"
                | "setthinghealth"
                | "getthingtemplate"
                | "getcurframe"
                | "ismoving"
                | "isthingmoving"
                | "movetoframe"
                | "stopthing"
                | "setsectoradjoins"
                | "getsectoradjoins"
                | "setadjoinflags"
                | "clearadjoinflags"
                | "setsectorflags"
                | "setsectorlight"
                | "setsectortint"
                | "setcolormap"
                | "setsectorcolormap"
                | "enableirmode"
                | "disableirmode"
                | "setmapmodeflags"
                | "getmapmodeflags"
                | "clearmapmodeflags"
                | "getsectorflags"
                | "playsoundthing"
                | "playsoundpos"
                | "playsoundlocal"
                | "playsound"
                | "playsong"
                | "setmusicvol"
                | "jkendlevel"
                | "endlevel"
                | "stopsong"
                | "stopmusic"
                | "settimer"
                | "setpulse"
                | "settimerx"
                | "sleep"
                | "getinv"
                | "setinv"
                | "changeinv"
                | "createthingatposnr"
                | "createthingnr"
                | "createbackpack"
                | "pickupbackpack"
                | "clearinv"
                | "vectoradd"
                | "vectoradd2"
                | "vectorsub"
                | "vector_sub"
                | "vectornorm"
                | "vectorscale"
                | "vectordot"
                | "vectordist"
                | "vectorx"
                | "vectory"
                | "vectorz"
                | "getlocalplayerthing"
                | "getdifficulty"
                | "isinvactivated"
                | "jksettarget"
                | "jkendtarget"
                | "jksettargetcolors"
                | "firstthinginview"
                | "nextthinginview"
                | "haslos"
                | "thingviewdot"
                | "applyforce"
                | "setactorflags"
                | "clearactorflags"
                | "getactorflags"
                | "aigetmode"
                | "getthingsignature"
                | "bittest"
                | "skilltarget"
                | "getactorweapon"
                | "attachthingtothing"
                | "attachthingtothingex"
                | "detachthing"
                | "rotatepivot"
                | "getthingparent"
                | "setthingattachflags"
                | "aisetfiretarget"
                | "aiflee"
                | "aisetclass"
                | "setactorweapon"
                | "amputatejoint"
                | "playmode"
                | "getlifeleft"
                | "setlifeleft"
                | "getitemflags"
                | "getparam"
                | "getcurweapon"
                | "getthingtype"
                | "firstthinginsector"
                | "nextthinginsector"
                | "sectoradjoins"
                | "jkprintunistring"
                | "creatething"
                | "damagething"
                | "surfacecenter"
                | "setwallcel"
                | "getwallcel"
                | "slidewall"
                | "surfaceanim"
                | "getsurfaceanim"
                | "stopsurfaceanim"
                | "sectorlight"
                | "getsectorlight"
                | "surfacelightanim"
                | "setcamerafocus"
                | "setcurrentcamera"
                | "cyclecamera"
                | "getcameramode"
                | "getprimaryfocus"
                | "getsecondaryfocus"
                | "setpovshake"
                | "materialanim"
                | "slidehorizonsky"
                | "slideceilingsky"
                | "sectorthrust"
                | "destroything"
                | "settimerex"
                | "getsurfacecenter"
                | "killtimerex"
                | "waitforstop"
                | "stopsound"
                | "createthingatpos"
                | "teleportthing"
                | "setrespawnmask"
                | "fireprojectile"
                | "syncthingpos"
                | "syncsurface"
                | "syncthingscale"
                | "syncthinglook"
                | "pathmovepause"
                | "setsectorthrust"
                | "sendmessageex"
                | "loadtemplate"
                | "setfacelightmode"
                | "pathmoveresume"
                | "activateweapon"
                | "deactivateweapon"
                | "changefirerate"
                | "setautoswitch"
                | "getautoswitch"
                | "setautopickup"
                | "getautopickup"
                | "jksetsuperflags"
                | "jkclearsuperflags"
                | "jkgetsuperflags"
                | "getkeylen"
                | "jksetweaponmesh"
                | "setweaponmesh"
                | "jksetpovmodel"
                | "jkplaypovkey"
                | "jkstoppovkey"
                | "setarmedmode"
                | "getcurweaponmode"
                | "setcurweapon"
                | "setmajormode"
                | "getmajormode"
                | "getcurrentcamera"
                | "getthingclasscog"
                | "randvec"
                | "getthingthrust"
                | "jkenablesaber"
                | "jkdisablesaber"
                | "jkgetsabercam"
                | "jksetwaggle"
                | "stopkey"
                | "loadsound"
                | "jkstringclear"
                | "jkstringconcatint"
                | "jkstringconcatflex"
                | "jkstringconcatspace"
                | "jkstringconcatunistring"
                | "jkstringconcatstring"
                | "jkstringconcatasciistring"
                | "jkstringconcatvector"
                | "jkstringconcatplayername"
                | "jkstringoutput"
                | "syncscores"
                | "autosavegame"
                | "setthingmodel"
                | "newcoloreffect"
                | "modifycoloreffect"
                | "freecoloreffect"
                | "setthingrotvel"
                | "autoselectweapon"
                | "jkgetchoice"
                | "jksetsaberinfo"
                | "getthinggeomode"
                | "setthingcurgeomode"
                | "changesoundvol"
                | "changesoundpitch"
                | "sectorsound"
                | "thinglight"
                | "setthinglight"
                | "thinglightanim"
                | "setsurfaceflags"
                | "capturething"
                | "takeitem"
                | "sendmessage"
                | "getinvmax"
                | "setinvavailable"
                | "adddynamictint"
                | "addtint"
                | "selectweapon"
                | "setcurinvweapon"
                | "getcurinvweapon"
                | "assignweapon"
                | "setthingvel"
                | "getthingvel"
                | "vectorset"
                | "setactorextraspeed"
                | "healthing"
                | "activatebin"
                | "deactivatebin"
                | "setinvactivated"
                | "setbinwait"
                | "getattachflags"
                | "getphysicsflags"
                | "setphysicsflags"
                | "clearphysicsflags"
                | "jkgetflags"
                | "jksetflags"
                | "jksetpersuasioninfo"
                | "getsoundlen"
        )
    }

    /// `sithAIAwareness_ProcessEvent` (`0x004f2c30`): record the event in its own sector's slot, then
    /// walk the adjoin graph, subtracting each adjoin's attenuation and stopping under the engine's
    /// floor. Each sector block (`0x8553a8 + sector * 100`) has **three slots** — strength at `+1/+2/+3`,
    /// position at `+4..+6` and source at `+0x16` — indexed by the event's third argument (the kind:
    /// `ProcessEvent`'s `ai[0x14]`), and a slot is replaced only when the new event is stronger. That is
    /// exactly what `sithAICmd_Listen` scans, in slot order 2 → 1 → 0 (`iVar9` from 2 down), so the
    /// records here are keyed by sector and kind.
    pub fn transmit_awareness(
        &mut self,
        sector: i32,
        kind: u8,
        pos: [f32; 3],
        strength: f32,
        source: Option<usize>,
    ) {
        let mut stack = vec![(sector, strength)];
        let mut written: Vec<i32> = Vec::new();
        let mut visited = 0usize;
        while let Some((at, strength)) = stack.pop() {
            visited += 1;
            if visited > 4096 {
                break; // ponytail: a breadth guard; the strength floor ends real propagation first.
            }
            if at < 0 || at as usize >= self.sector_adjoins.len() {
                continue;
            }
            // `ProcessEvent`: `if (stored_strength < new_strength) { store }` — a weaker event does
            // not replace the one already there, and does not propagate past it either.
            let stronger = self
                .sector_awareness
                .get(&(at, kind))
                .is_none_or(|(stored, _, _, _)| *stored < strength);
            if !stronger {
                continue;
            }
            if !written.contains(&at) {
                written.push(at);
            }
            self.sector_awareness
                .insert((at, kind), (strength, pos, self.time, source));
            for (next, cost) in &self.sector_adjoins[at as usize] {
                let next_strength = strength - cost;
                if next_strength > AI_AWARE_FLOOR {
                    stack.push((*next, next_strength));
                }
            }
        }
        // Fade the records that are past the window, the way the engine's own timer does.
        self.sector_awareness
            .retain(|_, (_, _, at, _)| self.time - *at < AI_AWARE_SECONDS);
        // The per-frame sweep after `ProcessEvent` in the engine's own update (`0x4f2b60`): the tick
        // counter `DAT_008553b4` is bumped, every `ProcessEvent` call stamps its sector's slot with
        // it, and then the level's thing table is walked — a thing whose sector's slot carries the
        // current stamp gets **AI event 2** with argument 0. That is the one event the `0x1000`
        // one-shot latch lets through (it clears the latch doing so), so this is what re-arms
        // `dodge`'s shot path after the first dodge. The engine reaches each thing at most once per
        // tick, which is the same guard here.
        for at in written {
            let things: Vec<usize> = self
                .things
                .iter()
                .enumerate()
                .filter(|(index, thing)| !thing.destroyed && thing.sector == at && {
                    self.awareness_event_at.get(index) != Some(&self.time)
                })
                .map(|(index, _)| index)
                .collect();
            for index in things {
                self.awareness_event_at.insert(index, self.time);
                let _ = self.emit_ai_event(index, 2, None);
            }
        }
    }

    /// What an actor standing in `sector` on `side` would read as the last known enemy position: the
    /// freshest event in the sector's ring that is still inside the window.
    pub fn awareness_at(&self, sector: i32) -> Option<[f32; 3]> {
        // The listener's own order, kind 2 then 1 then 0, and the first record still inside the window
        // answers — the engine has no "newest across kinds" reader.
        [2u8, 1, 0].into_iter().find_map(|kind| {
            let (_, pos, at, _) = self.sector_awareness.get(&(sector, kind))?;
            (self.time - *at < AI_AWARE_SECONDS).then_some(*pos)
        })
    }

    /// One random row of an actor's own `.snd` class for a sound-mode event — what
    /// `sithSoundClass_PlayModeRandom` (`0x4e6b70`) picks: `class->+0x20 + mode*4` is the mode's list
    /// and a random entry is played. The class comes from the template's `soundclass=`.
    ///
    /// The draw is the engine's `rand()`, like the rest of a script's stream: talk, fear and hurt
    /// picks consume it in retail too, so keeping them here is what holds the AI's fire cadence.
    /// The AI's own shot sound is the one pick retail never makes — see [`Self::shot_fire_wav`].
    fn chatter_wav(&mut self, index: usize, mode: &str) -> Option<String> {
        let template = self.templates.get(index)?.to_ascii_lowercase();
        let class = self.template_soundclass.get(&template)?.clone();
        let rows = self.sound_classes.get(&class)?.get(mode)?;
        let pick = (crate::cog::pseudo_random() * rows.len() as f64) as usize;
        let wav = rows.get(pick.min(rows.len().saturating_sub(1)))?.0.clone();
        self.last_sound.insert((index, mode.to_string()), wav.clone());
        Some(wav)
    }

    /// The `fire1` row an AI shot plays. Retail's AI shot names no wav, so the class row stands in;
    /// the pick must **not** come from `cog::pseudo_random` — retail never draws there, and a draw
    /// would shift the AI fire delays that share the stream (issue #13).
    fn shot_fire_wav(&mut self, index: usize) -> Option<String> {
        let template = self.templates.get(index)?.to_ascii_lowercase();
        let class = self.template_soundclass.get(&template)?.clone();
        let rows = self.sound_classes.get(&class)?.get("fire1")?;
        self.sound_rng = self
            .sound_rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let pick = ((self.sound_rng >> 33) as usize) % rows.len().max(1);
        rows.get(pick.min(rows.len().saturating_sub(1))).map(|(wav, _)| wav.clone())
    }

    /// The row a hurt of `kind` would play for this actor, so a check can name it without queueing
    /// anything: the row the actor's own class last played for the damage kind's mode.
    pub fn hurt_sound_wav(&self, index: usize, kind: u32) -> Option<String> {
        let mode = Self::hurt_mode(kind);
        if let Some(wav) = self.last_sound.get(&(index, mode.to_string())) {
            return Some(wav.clone());
        }
        let template = self.templates.get(index)?.to_ascii_lowercase();
        let class = self.template_soundclass.get(&template)?;
        self.sound_classes
            .get(class)?
            .get(mode)?

            .first()
            .map(|(wav, _)| wav.clone())
    }

    /// `sithAI_SetMoveThing` (`0x4eac90`): unless the jump lockout (`ai + 0x28c`) is still in the
    /// future **and** the actor is already moving, store the speed argument as movespeed
    /// (`ai + 0x1bc`), the move position (`ai + 0x1a0`), play sound mode 5 (`PlayModeFirst`,
    /// "moving") and set the moving flag `0x1`.
    fn ai_set_move_thing(&mut self, actor: usize, point: [f32; 3], speed: f32) {
        let flags = self.ai_flags.get(&actor).copied().unwrap_or(0);
        if self.time >= self.ai_move_lock.get(&actor).copied().unwrap_or(0.0) || flags & 0x1 == 0 {
            self.ai_move_speed.insert(actor, speed);
            self.ai_move.insert(actor, point);
            if let Some(at) = self.things.get(actor).map(|t| t.pos) {
                self.queue_first_mode_sound(actor, "moving", at, 1.0);
            }
            *self.ai_flags.entry(actor).or_insert(0) |= 0x1;
        }
    }

    /// The cog verbs' call (`0x500cd0`/`0x500da0`/`0x500e00`): only an AI-controlled thing, at
    /// its own current movespeed.
    fn ai_set_move_thing_cog(&mut self, actor: usize, point: [f32; 3]) {
        if self.ai.contains_key(&actor) {
            let speed = self.ai_move_speed.get(&actor).copied().unwrap_or(AI_MOVE_SPEED);
            self.ai_set_move_thing(actor, point, speed);
        }
    }

    /// `sithAI_Jump` (`0x4eacf0`): a move at speed **2.0** (`0x40000000`) toward the stored
    /// destination, puppet play mode `0x1b`, and — when the puppet cannot drive the body —
    /// `sithPlayerActions_JumpWithVel` (`0x4fc270`), which adds `jumpspeed` straight up. The
    /// 2000 ms lockout and the moving flag close it.
    fn ai_jump_move(&mut self, actor: usize, pos: [f32; 3]) {
        let template = self
            .templates
            .get(actor)
            .cloned()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let jumpspeed = self
            .template_jumpspeed
            .get(&template)
            .copied()
            .unwrap_or(1.5);
        self.ai_move_speed.insert(actor, 2.0);
        self.ai_move.insert(actor, pos);
        self.play_mode.insert(actor, 0x1b);
        // shortcut: the port's puppet records the mode but has no keyframes to move the body, so
        // the engine's JumpWithVel fallback runs unconditionally; drop this when puppets animate.
        if let Some(thing) = self.things.get_mut(actor) {
            thing.vel[2] += jumpspeed;
        }
        self.ai_move_lock.insert(actor, self.time + 2.0);
        *self.ai_flags.entry(actor).or_insert(0) |= 0x1;
    }

    /// `sithAI_SetLookFrame` (`0x4eab80`): the one way an actor starts turning. The look vector is
    /// `point - position`, normalised; the head pitch is clamped into the template's
    /// `minheadpitch`/`maxheadpitch` (the engine's `SetHeadPYR` for a puppet thing); the z is
    /// flattened and the vector renormalised; flag `0x8` arms the blend in `ai_turn_blend`.
    fn ai_set_look_frame(&mut self, actor: usize, point: [f32; 3]) {
        let Some(pos) = self.things.get(actor).map(|t| t.pos) else {
            return;
        };
        let d = [point[0] - pos[0], point[1] - pos[1], point[2] - pos[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len == 0.0 {
            return;
        }
        let template = self
            .templates
            .get(actor)
            .cloned()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let min = self
            .template_minheadpitch
            .get(&template)
            .copied()
            .unwrap_or(-80.0);
        let max = self
            .template_maxheadpitch
            .get(&template)
            .copied()
            .unwrap_or(80.0);
        // The head moves only while the actor word's bit `0x1` is set (`*(byte *)(iVar6 + 0x130)
        // & 1`, `0x4eab80`) — the engine leaves the pitch alone otherwise.
        if self.actor_flags.get(&actor).copied().unwrap_or(0) & 0x1 != 0 {
            if let Some(thing) = self.things.get_mut(actor) {
                thing.head_pitch = (d[2] / len).asin().to_degrees().clamp(min, max);
            }
        }
        let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
        // Straight up/down has no flat direction: the engine's zeroed z leaves a zero vector and
        // the blend holds the body's facing rather than dividing by zero.
        if flat > 1e-6 {
            self.ai_look_dir.insert(actor, [d[0] / flat, d[1] / flat]);
            *self.ai_flags.entry(actor).or_insert(0) |= 0x8;
        }
    }

    /// `sithAI_Tick`'s flag-`0x8` arm: run the blend once per ticked actor, before the instinct
    /// pass of the frame (the engine runs it in the thing update, ahead of `sithAI_Process`).
    fn ai_turn_blend(&mut self, actor: usize, dt: f32) {
        if self.ai_flags.get(&actor).copied().unwrap_or(0) & 0x8 == 0 {
            return;
        }
        let Some(look) = self.ai_look_dir.get(&actor).copied() else {
            return;
        };
        let Some(yaw) = self.things.get(actor).map(|t| t.yaw) else {
            return;
        };
        let template = self
            .templates
            .get(actor)
            .cloned()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let rotthrust = self
            .template_rotthrust
            .get(&template)
            .copied()
            .unwrap_or(180.0);
        let (forward, done) = turn_blend(forward_of_yaw(yaw), look, rotthrust, dt);
        if done {
            *self.ai_flags.entry(actor).or_insert(0) &= !0x8;
        } else if let Some(thing) = self.things.get_mut(actor) {
            thing.yaw = yaw_of_forward(forward);
        }
    }

    /// Queue the random row of the actor's own class for a sound mode at `at` — what
    /// `sithSoundClass_PlayModeRandom` plays, through the port's sound-request pipeline. Shared by
    /// `talk` (its moods), `sensedanger` (fear) and the hurt sound.
    fn queue_mode_sound(&mut self, index: usize, mode: &str, at: [f32; 3]) {
        self.queue_mode_sound_at(index, mode, at, 1.0);
    }

    /// The same, with the volume `sithSoundClass_PlayMode(thing, mode, volume)` takes: the caller's
    /// volume is the sound's base, and `play_requested_sounds` still owns the fade and the rear pan.
    fn queue_mode_sound_at(&mut self, index: usize, mode: &str, at: [f32; 3], volume: f32) {
        let Some(wav) = self.chatter_wav(index, mode) else { return };
        self.queue_sound_class_entry(index, wav, at, volume, "sithSoundClass_PlayModeRandom (0x4e6b70)");
    }

    /// `sithSoundClass_PlayModeFirstEx` (`0x4e6af0`): the head entry, without drawing RNG.
    fn queue_first_mode_sound(&mut self, index: usize, mode: &str, at: [f32; 3], volume: f32) {
        let Some(wav) = self.templates.get(index)
            .and_then(|name| self.template_soundclass.get(&name.to_ascii_lowercase()))
            .and_then(|class| self.sound_classes.get(class))
            .and_then(|modes| modes.get(mode))
            .and_then(|rows| rows.first())
            .map(|(wav, _)| wav.clone())
        else { return };
        self.queue_sound_class_entry(index, wav, at, volume, "sithSoundClass_PlayModeFirstEx (0x4e6af0)");
    }

    fn queue_sound_class_entry(&mut self, index: usize, wav: String, at: [f32; 3], volume: f32, source: &'static str) {
        // `sithSoundMixer_PlaySoundThing` (`0x4dba60`) ends with
        // `CreateTransmittingEvent(sector, pos, 0, volume * _DAT_00522f80, thing)`: a sound played on a
        // thing is a **kind-0** awareness event at `volume * 0.6` (the constant read out of the EXE's
        // `.rdata`). That is how a shout or a hurt cry carries through a room.
        if let Some(sector) = self.things.get(index).map(|t| t.sector) {
            self.transmit_awareness(
                sector,
                0,
                at,
                (volume * AI_AWARE_SOUND_SCALE).max(0.0),
                Some(index),
            );
        }
        let channel = self.sound_volume.len();
        self.sound_volume.push(volume);
        self.sound_pitch.push(1.0);
        self.sounds.push(SoundRequest {
            name: wav,
            volume,
            pos: Some(at),
            channel,
            pitch: 1.0,
            source,
        });
    }
    /// `dodge`'s damage path (`0x50b360`, event 1): an actor whose class carries the instinct and is
    /// not already walking (AI flag bit `0x1` clear) steps a little **away** from whoever hit it.
    /// The handler computes `point = self - arg0 * normalize(attacker - self)` and hands it to
    /// `sithAI_SetMoveThing(ai, point, 2.5)` — the same `ai + 0x93` destination the port's `ai_move`
    /// is — then sets its own 1000 ms cooldown. `arg0` is the first number after the class line's
    /// weight (`dodge 0.1, 0.1` for a grunt, `0.3, 0.1` for a boss). Returns whether it reacted, which
    /// is what stops `sithAI_EmitEvent`'s dispatch.
    fn dodge_from_damage(&mut self, actor: usize, attacker: usize) -> bool {
        if self.ai_flags.get(&actor).copied().unwrap_or(0) & 0x1 != 0 {
            return false;
        }
        let Some(step) = self
            .ai
            .get(&actor)
            .and_then(|class| class.instinct("Dodge"))
            .and_then(|dodge| dodge.args.first().copied())
        else {
            return false;
        };
        let step = step as f32;
        if step == 0.0 {
            return false;
        }
        let (Some(from), Some(to)) = (
            self.things.get(actor).map(|t| t.pos),
            self.things.get(attacker).map(|t| t.pos),
        ) else {
            return false;
        };
        // The engine steps along the full 3D `norm(attacker - self) * -fltArg[1]`; there is no
        // cooldown clock of the handler's own — the moving bit `0x1` is the gate, and the 1000 ms
        // `ai + 0xa3` lockout the handler writes only refuses a move for a body already moving.
        let dir = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if len < 1e-4 {
            return false;
        }
        let point = [
            from[0] - dir[0] / len * step,
            from[1] - dir[1] / len * step,
            from[2] - dir[2] / len * step,
        ];
        self.ai_set_move_thing(actor, point, 2.5);
        self.ai_move_lock.insert(actor, self.time + 1.0);
        if std::env::var_os("BK_AI_TRACE").is_some() {
            eprintln!("ai: {actor} dodges {step:.2} away from thing {attacker} to {point:?}");
        }
        // The handler returns 0 (`0x50b360`), so the damage event keeps dispatching.
        false
    }

    /// `sensedanger`'s damage path (`0x50b9b0`, event 1): an **awake** actor (`0x4`) that has not
    /// yet been alerted (`0x800` clear) plays its `surprise` sound (mode `0x57`, seventh of the mood
    /// range 75–87), takes `sithThing_GetThingParent(attacker)` as its target — the shooter, for a
    /// projectile — and swaps the awake bit for the alert bit (`flags & !0x4 | 0x800`). Unlike the
    /// scheduler path it raises **no** awareness event: that is the on-sight branch's.
    /// `sensedanger`'s event-2 arm (`0x50b9ff`): an actor that has been hurt but is not alert yet
    /// (`0x4` set, `0x800` clear) hears something in its own sector and, if the record's source is a
    /// visible thing within its `heardist`, takes that thing as its target, raises the alert word and
    /// re-broadcasts the event at **4.0** (`0x40800000`).
    ///
    /// The gates, read off the disassembly: the instinct's own weight must beat the record's strength
    /// (that is why a gunshot usually silences this arm — its awareness is stronger than a 0.3 weight),
    /// the sector's **kind-1** slot must name a source, and `sithAI_CheckSightThing` (`0x4eb090`) must
    /// not answer 1 or 3. That call passes the class's `heardist` (`class + 0x18`) with a `-1.0`
    /// cone, so the check is a range plus a clear line, not a view cone. Two substitutions:
    /// the cloak rules the helper also applies are not modelled, and the arm's sound (mode `0x4f`) has
    /// no name in the port's table, so it is not played.
    fn sensedanger_from_awareness(&mut self, actor: usize, weight: f32) -> bool {
        let flags = self.ai_flags.get(&actor).copied().unwrap_or(0);
        if flags & 0x4 == 0 || flags & 0x800 != 0 {
            return false;
        }
        let Some(sector) = self.things.get(actor).map(|t| t.sector) else {
            return false;
        };
        let Some(&(strength, _, _, source)) = self.sector_awareness.get(&(sector, 1)) else {
            return false;
        };
        if weight <= strength {
            return false;
        }
        let Some(source) = source.filter(|s| *s != actor) else {
            return false;
        };
        let (Some(from), Some(source_pos), Some(heardist)) = (
            self.things.get(actor).map(|t| t.pos),
            self.things.get(source).map(|t| (t.pos, t.destroyed)).filter(|(_, gone)| !gone).map(|(p, _)| p),
            self.ai.get(&actor).map(|class| class.heardist),
        ) else {
            return false;
        };
        let (dx, dy, dz) = (
            source_pos[0] - from[0],
            source_pos[1] - from[1],
            source_pos[2] - from[2],
        );
        let reach = (dx * dx + dy * dy + dz * dz).sqrt();
        if reach > heardist.max(0.0) {
            return false;
        }
        if self
            .solids
            .as_ref()
            .is_some_and(|solids| solids.segment_blocked(from, source_pos))
        {
            return false;
        }
        self.ai_flags.insert(actor, (flags & !0x4) | 0x800);
        self.remember(actor, source_pos, "surprise");
        if self
            .ai
            .get(&actor)
            .is_some_and(|c| c.instinct("Withdraw").is_some())
        {
            self.ai_withdraw.entry(actor).or_insert((source, None));
        }
        // `0x4f2a90(ai->sector, thing.pos, 1, 0x40800000, thing)`: the arm re-broadcasts the event
        // at **4.0**, which is what pulls the rest of the room out of `sleep`.
        self.transmit_awareness(sector, 1, from, 4.0, Some(source));
        if std::env::var_os("BK_AI_TRACE").is_some() {
            eprintln!("ai: {actor} heard thing {source} in sector {sector} and alerts");
        }
        true
    }

    fn sensedanger_from_damage(&mut self, actor: usize, attacker: usize) -> bool {
        let flags = self.ai_flags.get(&actor).copied().unwrap_or(0);
        if flags & 0x4 == 0 || flags & 0x800 != 0 {
            return false;
        }
        if self
            .ai
            .get(&actor)
            .is_none_or(|class| class.instinct("SenseDanger").is_none())
        {
            return false;
        }
        let target = self
            .things
            .get(attacker)
            .and_then(|t| t.owner)
            .unwrap_or(attacker);
        let (Some(at), Some(target_pos)) = (
            self.things.get(actor).map(|t| t.pos),
            self.things.get(target).map(|t| t.pos),
        ) else {
            return false;
        };
        self.ai_flags.insert(actor, (flags & !0x4) | 0x800);
        // `ai[0x70]` is the target thing (`withdraw`'s slot); the port's chase reads the target's
        // last-seen position.
        self.remember(actor, target_pos, "surprise");
        if self
            .ai
            .get(&actor)
            .is_some_and(|c| c.instinct("Withdraw").is_some())
        {
            self.ai_withdraw.entry(actor).or_insert((target, None));
        }
        self.queue_mode_sound(actor, "surprise", at);
        if std::env::var_os("BK_AI_TRACE").is_some() {
            eprintln!("ai: {actor} is surprised by thing {attacker} (target {target})");
        }
        true
    }

    /// `sithAICmd_Startup` (`0x5091b0`) registers every sithAI instinct with three words —
    /// `sithAI_RegisterInstinct(name, handler, accept, reject, mask)` (`0x4ea0a0`) — which the
    /// class loader copies into each instinct entry (`accept` at `+0x2c`, `reject` at `+0x30`, the
    /// event mask at `+0x34`). The scheduler gates a run on `(accept & mode) != 0 &&
    /// (reject & mode) == 0`; the event dispatch wakes every instinct whose `mask` carries the
    /// event bit, in class-file order. An accept of 0 (`dodge`, `jump`, `listen`, `returnhome`)
    /// means the engine's scheduler **never** runs the handler: it is event-only. `specialattack`
    /// is a jk-side command, not in the sithAI registry; its contact mask is the port's own
    /// measured word (the `aistatus` row reads `4`).
    fn instinct_registry(name: &str) -> (u32, u32, u32) {
        match name.to_ascii_lowercase().as_str() {
            "listen" => (0, 0, 0x7),
            "lookfortarget" => (0x204, 0, 0),
            "primaryfire" => (2, 0, 0x100),
            "follow" => (2, 0x800, 0xe00),
            "turretfire" => (2, 0x800, 0x100),
            "opendoors" => (2, 0, 0),
            "jump" => (0, 0, 0x604),
            "randomturn" => (4, 0, 0),
            "roam" => (4, 0, 0),
            "flee" => (0x800, 0, 0xf05),
            "sensedanger" => (4, 0x800, 0x7),
            "hitandrun" => (0xc00, 0, 0),
            "retreat" => (2, 0x800, 0),
            "circlestrafe" => (2, 0x800, 0),
            "blindfire" => (2, 0xc00, 0),
            "returnhome" => (0, 0, 0x900),
            "lobfire" => (2, 0, 0x100),
            "talk" => (0xffff, 0, 0),
            "crouch" => (2, 0, 0x100),
            "withdraw" => (0x800, 0, 0xf05),
            "dodge" => (0, 0, 0x1003),
            "specialattack" => (0, 0, 4),
            // The jk-side commands (`ForcePowers`, `SaberFighting`, ...) are registered by a
            // different startup and keep the port's own clocks until they are ported.
            _ => (0, 0, 0),
        }
    }

    /// `sithAI_Tick` (`0x4ea5e0`), the AI control arm of `sithThing_Update`: only a type-2 thing
    /// whose health (`+0x134`) is above `_DAT_00523050` (0.0) and that has an AI block. Flag `0x8`
    /// runs the turn blend (`0x4ea630`), flag `0x1` the move arm (`0x4ea890`).
    fn sith_ai_tick(&mut self, index: usize, dt: f32, solids: Option<&crate::collide::Solids>) {
        if self.types.get(index).copied() != Some(2)
            || !self.ai.contains_key(&index)
            || self.things.get(index).is_none_or(|t| t.destroyed || t.health <= 0.0)
        {
            return;
        }
        if self.ai_flags.get(&index).copied().unwrap_or(0) & 0x8 != 0 {
            self.ai_turn_blend(index, dt);
        }
        if self.ai_flags.get(&index).copied().unwrap_or(0) & 0x1 != 0 {
            self.sith_ai_msgarrived_target(index, dt, solids);
        }
    }

    /// `sithAI_idk_msgarrived_target` (`0x4ea890`): the move arm. It pushes the body's velocity
    /// (`+0x1d4`) toward the move position (`ai + 0x1a0`) by `movespeed (ai + 0x1bc) × maxthrust
    /// (+0x148) × dt`; physics and `sithThing_UpdateMove` then carry the body.
    ///
    /// - Returns at once on AI flag `0x1000` or typeflag `0x40000`.
    /// - The push's z is `_DAT_00523050` (0.0) unless the sector is underwater (`& 2`) or the thing
    ///   flies (physflag `0x2000`).
    /// - An attached walker (AI flag `0x40` clear) with a non-zero velocity asks
    ///   `sithAI_CanWalk` at `pos + vel × dt`; a refusal zeroes the velocity and emits `0x400`.
    /// - A flyer without typeflag `0x40` stops when the step's sector
    ///   (`sithCollision_FindSectorInRadius`) is underwater, or always under thingflag
    ///   `0x2000000`; the stop then subtracts `_DAT_0052305c` (-0.5) from the z velocity.
    /// - Arrival is the distance to the move position, taken before the push, at or under
    ///   `moveSize` (`+0x2c`): zero velocity, clear AI flag `0x1`, cog message `0xb` (`arrived`)
    ///   and AI event `0x800`. shortcut: `sithSoundClass_StopMode(thing, 5)` has no port — the
    ///   sound pipeline cannot stop a playing class mode yet.
    fn sith_ai_msgarrived_target(&mut self, index: usize, dt: f32, solids: Option<&crate::collide::Solids>) {
        let flags = self.ai_flags.get(&index).copied().unwrap_or(0);
        let template = self.templates.get(index).map(|n| n.to_ascii_lowercase()).unwrap_or_default();
        let typeflags = self.template_typeflags.get(&template).copied().unwrap_or(0);
        if flags & 0x1000 != 0 || typeflags & 0x40000 != 0 {
            return;
        }
        let goal = self.ai_move.get(&index).copied().unwrap_or_default();
        let maxthrust = self.template_maxthrust.get(&template).copied().unwrap_or(0.0);
        let push = self.ai_move_speed.get(&index).copied().unwrap_or(AI_MOVE_SPEED) * maxthrust * dt;
        let move_size = self.template_hit.get(&template).map_or(0.065, |(radius, _)| *radius);
        let t = &self.things[index];
        let (pos, sector, phys, thing_flags, attached) = (t.pos, t.sector, t.phys_flags, t.flags, t.attach_flags != 0);
        let mut dir = [goal[0] - pos[0], goal[1] - pos[1], goal[2] - pos[2]];
        let distance = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if distance > 0.0 {
            dir = dir.map(|v| v / distance);
        }
        let sector_flags = |s: i32| usize::try_from(s).ok().and_then(|s| self.sector_flags.get(s)).copied().unwrap_or(0);
        let flyer = phys & 0x2000 != 0;
        let mut step = dir.map(|v| v * push);
        if sector_flags(sector) & crate::collide::SECTOR_UNDERWATER == 0 && !flyer {
            step[2] = 0.0;
        }
        let mut vel = self.things[index].vel;
        for k in 0..3 {
            vel[k] += step[k];
        }
        let ahead = [0, 1, 2].map(|k| vel[k] * dt + pos[k]);
        if flyer {
            if typeflags & 0x40 == 0 {
                let stop = thing_flags & 0x2000000 != 0
                    || solids.zip(usize::try_from(sector).ok()).is_some_and(|(solids, sector)| {
                        let mut to = ahead;
                        let found = solids.find_sector_in_radius(sector, pos, &mut to, 0.0, self.gates());
                        self.sector_flags.get(found).is_some_and(|f| f & crate::collide::SECTOR_UNDERWATER != 0)
                    });
                if stop {
                    vel = [0.0, 0.0, 0.5]; // 0 - _DAT_0052305c (-0.5)
                }
            }
        } else if flags & 0x40 == 0 && attached && vel != [0.0; 3] && self.sith_ai_can_walk(index, ahead, solids) == 0 {
            self.things[index].vel = [0.0; 3];
            let _ = self.emit_ai_event(index, 0x400, None);
            return;
        }
        if distance <= move_size {
            vel = [0.0; 3];
            *self.ai_flags.entry(index).or_insert(0) &= !0x1;
            self.things[index].vel = vel;
            self.sith_cog_thing_send_message(index, None, "arrived");
            let _ = self.emit_ai_event(index, 0x800, None);
            return;
        }
        self.things[index].vel = vel;
    }

    /// `sithAI_CanWalk` (`0x4eb4b0`): `sithCollision_FindSectorInRadius` from the body to `dest`
    /// (radius 0, which clamps `dest` onto the first wall), then the explicit-sector form
    /// (`0x4eb640`) sweeps a `moveSize × 0.25` sphere down `GetThingHeight + maxstep` (class
    /// `+0x10`, default 0.5) with flags `0x2002`. 0 refuses the step; 1 and 2 allow it. Without
    /// level geometry nothing refuses.
    fn sith_ai_can_walk(&self, index: usize, dest: [f32; 3], solids: Option<&crate::collide::Solids>) -> u32 {
        let Some(solids) = solids else { return 1 };
        let t = &self.things[index];
        let Some(sector) = usize::try_from(t.sector).ok().filter(|s| *s < solids.sectors.len()) else { return 0 };
        let template = self.templates.get(index).map(|n| n.to_ascii_lowercase()).unwrap_or_default();
        let move_size = self.template_hit.get(&template).map_or(0.065, |(radius, _)| *radius);
        let mut dest = dest;
        let sector = solids.find_sector_in_radius(sector, t.pos, &mut dest, 0.0, self.gates());
        let height = self.thing_height(Some(index), move_size);
        let maxstep = self.ai.get(&index).map_or(0.5, |class| class.maxstep);
        let name = self.templates.get(index);
        let mover = crate::collide::MoveState {
            pos: t.pos, sector, vel: t.vel, remainder: [0.0; 3], phys_flags: t.phys_flags,
            kind: 2, move_type: 1, collide: name.and_then(|n| self.template_collide.get(n)).copied().unwrap_or(1),
            flags: t.flags, attach: t.floor_attach, attach_flags: t.attach_flags, owner: t.owner,
            water_surface: None,
        };
        solids.ai_can_walk_in_sector(
            sector, &mover, index, dest, height + maxstep, move_size * 0.25, self.gates(),
            |thing| self.things.get(thing).map_or(0, |t| t.flags),
        )
    }

    /// `sithAI_EmitEvent` (`0x4e9f80`): raise one AI **event** on one actor. The engine's outer
    /// gates come first — the thing is not `0x202`-flagged (`SITH_TF` says no AI), it is alive, the AI
    /// is not switched off (`0x2000`), and the `0x1000` one-shot latch lets only event 2 through and
    /// clears itself doing so. The latch's **setter** is not in this build: nothing in the executable
    /// writes `ai+8 |= 0x1000` (a full scan finds no such store) and no shipped script writes it
    /// through `jkSetFlags` (those pass `0x5`, `0x8`, `0x15`, `0x20` and `0x200`), so the gate is
    /// effectively clear and the awareness sweep's event 2 is what keeps it clear. Then every instinct
    /// whose class line enables the event's mask is
    /// called **in file order**, stopping at the first handler that returns non-zero.
    ///
    /// Every emit site in the executable is accounted for: `sithActor_DamageActor` raises 1 (this
    /// build calls it from `damage_typed_from`), `sithWeapon_WeaponFireProjectile` raises `0x1000`
    /// (called from `fire_with_spread`), `sithAI_InstinctUpdate`/the two AI-flag helpers raise the
    /// `0x100` cascade, the track-target helper raises `0x400`/`0x800`, and the per-frame awareness
    /// sweep (`0x4f2b60`) raises **event 2** on every thing in a sector whose slot ProcessEvent just
    /// stamped (`transmit_awareness` raises it here). Event 4 comes from the actor/player contact
    /// test. The handlers' event-2 arms are still unported, so the sweep's only effect today is to
    /// clear the `0x1000` one-shot latch.
    pub fn emit_ai_event(&mut self, actor: usize, event: u32, source: Option<usize>) -> bool {
        self.emit_ai_event_from(actor, event, source, 0)
    }

    /// [`Runtime::emit_ai_event`] with the handlers' last argument: for the `0x100` mode-change
    /// event it is the mode word before the change (`returnhome` reads its awake bit).
    fn emit_ai_event_from(&mut self, actor: usize, mut event: u32, source: Option<usize>, mut old_word: u32) -> bool {
        // `sithAI_EmitEvent` (`0x4e9f80`) loops: dispatch, then — when a handler changed the mode
        // word — re-dispatch as the 0x100 mode-change event with the old word as its argument, until
        // the word stops changing. `acted` reports whether any handler in the chain returned
        // non-zero (the engine returns void; the port's callers and tests read it).
        let mut acted_any = false;
        loop {
            let Some(thing) = self.things.get(actor) else {
                return acted_any;
            };
            // The outer gates, in engine order: a live non-`0x202` thing, the global AI switch, the
            // `0x2000` sleep word, then the `0x1000` one-shot latch — which only event 2 passes and
            // which passing clears. The latch is **set** by `sithAI_Create`'s `0x1004` seed, so a
            // placed actor is dormant until the awareness sweep's event 2 reaches it.
            if thing.flags & 0x202 != 0 || thing.health <= 0.0 {
                return acted_any;
            }
            if self.ai_disabled_global {
                return acted_any;
            }
            let flags = self.ai_flags.get(&actor).copied().unwrap_or(0);
            if flags & 0x2000 != 0 {
                return acted_any;
            }
            if flags & 0x1000 != 0 {
                if event != 2 {
                    return acted_any;
                }
                self.ai_flags.insert(actor, flags & !0x1000);
            }
            // Event 0x100 tells the actor's own cogs (`ThingSendMessageEx(thing, 0, 0x18, 256.0)`).
            if event == 0x100 {
                self.send_thing_message(actor, "aievent", [256.0, 0.0]);
            }
            let Some(class) = self.ai.get(&actor).cloned() else {
                return acted_any;
            };
            let mut acted = false;
            for instinct in &class.instincts {
                if Self::instinct_registry(&instinct.name).2 & event == 0 {
                    continue;
                }
                acted = match (instinct.name.to_ascii_lowercase().as_str(), event) {
                    ("dodge", 1) => {
                        source.is_some_and(|attacker| self.dodge_from_damage(actor, attacker))
                    }
                    ("dodge", 0x1000) => {
                        let shot = source.and_then(|s| self.things.get(s).map(|t| t.pos));
                        shot.is_some_and(|shot| self.dodge_from_shot(actor, shot))
                    }
                    ("sensedanger", 1) => {
                        source.is_some_and(|attacker| self.sensedanger_from_damage(actor, attacker))
                    }
                    ("sensedanger", 2) => self.sensedanger_from_awareness(actor, instinct.weight),
                    // `dodge`'s event-2 arm (`0x50b5c1`) is dead content, not unported: it demands the
                    // awareness source be a **weapon thing** (`[source+0xc] == 3`, `SITH_THING_WEAPON`)
                    // whose `[source+0x10]` is 1, and every shipped event records the thing that made the
                    // noise (an actor or the player), not the projectile.
                    ("dodge", 2) => false,
                    ("jump", 0x400) => self.jump_from_obstacle(actor, instinct.weight),
                    ("specialattack", 4) => source.is_some_and(|target| {
                        // `sithAICmd_SpecialAttack` event 4: damage the collided player with
                        // `DAMAGE_SABER` at `fltArg[8]` (Maw 10000, Pic 2000), then rearm.
                        let damage = instinct.args.get(7).copied().unwrap_or(0.0);
                        if damage <= 0.0 || target == actor {
                            return false;
                        }
                        self.damage_typed(target as i64, damage, DAMAGE_SABER);
                        self.ai_special.insert(actor, self.time + 1.0);
                        true
                    }),
                    // `lobfire`'s event-`0x100` branch (`0x509cd0`): holster or draw the weapon with
                    // the awake bit, then re-arm in 1000 ms (the scheduler's default arm). Its
                    // scheduler branch fires the lobbed shot itself — the fire path is #93's region.
                    ("lobfire", 0x100) => {
                        let flags = self.ai_flags.get(&actor).copied().unwrap_or(0);
                        self.armed_mode.insert(actor, u32::from(flags & 0x200 != 0));
                        false
                    }
                    // `returnhome` (`0x50bd70`) event `0x100`: an actor that just woke (awake bit set
                    // now, clear in the old word) walks home — `SetMoveThing(ai + 0x93, 1.0)` — and
                    // looks there. Returns 0.
                    ("returnhome", 0x100) => {
                        let flags = self.ai_flags.get(&actor).copied().unwrap_or(0);
                        if flags & 0x4 != 0
                            && old_word & 0x4 == 0
                            && let Some(home) = self.ai_home.get(&actor).copied()
                        {
                            self.ai_set_move_thing(actor, home, 1.0);
                            self.ai_set_look_frame(actor, home);
                        }
                        false
                    }
                    // `returnhome` event `0x800` (arrived): while awake, look along the placed facing
                    // (`ai + 0x96`, the thing's look vector `sithAI_Create` copied). Returns 0.
                    ("returnhome", 0x800) => {
                        if self.ai_flags.get(&actor).copied().unwrap_or(0) & 0x4 != 0
                            && let (Some(look), Some(pos)) =
                                (self.ai_home_look.get(&actor).copied(), self.things.get(actor).map(|t| t.pos))
                        {
                            self.ai_set_look_frame(actor, [pos[0] + look[0], pos[1] + look[1], pos[2] + look[2]]);
                        }
                        false
                    }
                    // primaryfire/turretfire/crouch also carry the 0x100 bit; their event arms are
                    // not ported (#91/#93).
                    _ => false,
                };
                if acted {
                    break;
                }
            }
            acted_any |= acted;
            if self.ai_flags.get(&actor).copied().unwrap_or(0) == flags {
                return acted_any;
            }
            // The mode word changed under the dispatch: cascade with the word it changed from.
            old_word = flags;
            event = 0x100;
        }
    }


    /// `sithAI_Process` (`0x4e9e20`): the per-slot gates before an instinct pass. The thing gates
    /// (`0x202` flags, health above `_DAT_0052304c` = 0.0, read from the shipped image) and the
    /// mode word's `0x3000` family — the shipped scripts' `0x2000` sleep switch **and** the
    /// `0x1000` one-shot latch `sithAI_Create` seeds — are checked by the caller inside the tick's
    /// actor loop; what remains here is the pass deadline (`ai + 0x190 <= now`), which holds the
    /// whole slot until the earliest armed instinct of the modes the word accepts comes due.
    fn ai_process(&mut self, index: usize, player: usize, dt: f32, ctx: &AiCtx) {
        if self
            .ai_next_due
            .get(&index)
            .is_some_and(|due| *due > self.time)
        {
            return;
        }
        self.ai_instinct_update(index, player, ctx);
    }

    /// `sithAI_InstinctUpdate` (`0x4e9e90`): one pass over the actor's instincts in class-file
    /// order. An instinct runs only when the registry's accept word intersects the AI mode word,
    /// its reject word does not, and the instance's own clock is due; the clock is armed at
    /// `now + 1000 ms` **before** the handler runs (the handler may re-arm itself — `talk` at its
    /// own first number, `lookfortarget` at its class interval). A handler that returns non-zero
    /// after the mode word changed raises the `0x100` mode-change cascade and **restarts the scan
    /// from the first instinct**. The pass writes the slot's next deadline (`ai + 0x190`) as the
    /// earliest armed clock of the accepted instincts, `now + 5000` at the most.
    fn ai_instinct_update(&mut self, index: usize, player: usize, ctx: &AiCtx) {
        let Some(count) = self.ai.get(&index).map(|class| class.instincts.len()) else {
            return;
        };
        self.ai_inst.entry(index).or_default().resize(count, 0.0);
        let mut next_pass = self.time + 5.0;
        'scan: loop {
            let Some(class) = self.ai.get(&index).cloned() else {
                return;
            };
            let flags_at_scan = self.ai_flags.get(&index).copied().unwrap_or(0);
            for i in 0..count {
                let (accept, reject, _) = Self::instinct_registry(&class.instincts[i].name);
                let flags = self.ai_flags.get(&index).copied().unwrap_or(0);
                if accept & flags == 0 || reject & flags != 0 {
                    continue;
                }
                if self.ai_inst[&index][i] <= self.time {
                    // Arm before the call, exactly as the engine does.
                    self.ai_inst.get_mut(&index).unwrap()[i] = self.time + 1.0;
                    let instinct = class.instincts[i].clone();
                    let acted = self.ai_instinct_handler(index, i, &instinct, player, ctx);
                    if acted
                        && self.ai_flags.get(&index).copied().unwrap_or(0) != flags_at_scan
                    {
                        let _ = self.emit_ai_event_from(index, 0x100, None, flags_at_scan);
                        continue 'scan;
                    }
                }
                let armed = self.ai_inst[&index][i];
                if armed < next_pass {
                    next_pass = armed;
                }
            }
            self.ai_next_due.insert(index, next_pass);
            return;
        }
    }

    /// One instinct's handler, dispatched by class-line name. Handlers whose engine bodies are
    /// ported elsewhere (the fire arms, #93; the perception chain, #91) or not yet answer `false`
    /// and keep the scheduler's default `now + 1000 ms` re-arm. The engine's `0x50*` handler
    /// addresses are on each arm.
    fn ai_instinct_handler(
        &mut self,
        index: usize,
        slot: usize,
        instinct: &crate::ai::Instinct,
        player: usize,
        ctx: &AiCtx,
    ) -> bool {
        // Thin wrapper so the arms read `rearm(self, t)`; the body is `ai_rearm`'s.
        let rearm = |rt: &mut Self, next: f32| rt.ai_rearm(index, slot, next);
        match instinct.name.to_ascii_lowercase().as_str() {
            // `talk` (`0x50be20`): idle chatter on its own clock — the handler re-arms at
            // `now + fltArg[0]` before it decides anything, rolls fltArg[1], and picks the mood
            // from the mode word and the health fraction. Always returns 0.
            "talk" => {
                rearm(
                    self,
                    self.time + instinct.weight / 1000.0,
                );
                let chance = instinct.args.first().copied().unwrap_or(0.0) as f64;
                let max_health = self
                    .template_max_health
                    .get(&ctx.template)
                    .copied()
                    .unwrap_or(ctx.health)
                    .max(1.0);
                if crate::cog::pseudo_random() < chance
                    && let Some(mode) = talk_mode(
                        self.ai_flags.get(&index).copied().unwrap_or(0),
                        ctx.health / max_health,
                    )
                {
                    self.queue_mode_sound(index, mode, ctx.from);
                }
                false
            }
            // `lookfortarget` (`0x50aa80`, accept `0x204`): see `sithAICmd_LookForTarget`.
            "lookfortarget" => self.sith_ai_cmd_look_for_target(index, slot, instinct, player, ctx),
            // `sensedanger` (`0x50b9b0`, accept 4 reject 0x800): the scheduler branch looks the
            // player over once a second while awake and not yet alerted — with **no** random roll:
            // the handler's only gate past the mode word is `fltArg[1] != 0` (the class line's
            // second number; `0.00`, most shipped lines, disables it). A sighted player alerts:
            // `flags & ~4 | 0x800`, the fear sound, a kind-1 awareness event at 3.0, and the
            // player in the target slot.
            "sensedanger" => {
                let enabled = instinct.args.first().copied().unwrap_or(0.0);
                if enabled == 0.0 || self.player_dead || !ctx.sighted {
                    return false;
                }
                let flags = self.ai_flags.get(&index).copied().unwrap_or(0);
                self.ai_flags.insert(index, (flags & !0x4) | 0x800);
                self.remember(index, ctx.target, "sensedanger");
                if self
                    .ai
                    .get(&index)
                    .is_some_and(|c| c.instinct("Withdraw").is_some())
                {
                    self.ai_withdraw.entry(index).or_insert((player, None));
                }
                self.queue_mode_sound(index, "fear", ctx.from);
                self.transmit_awareness(ctx.sector, 1, ctx.from, AI_AWARE_ALERT, Some(index));
                if std::env::var_os("BK_AI_TRACE").is_some() {
                    eprintln!(
                        "ai: {index} senses the danger and alerts sector {} from {:?}",
                        ctx.sector, ctx.from
                    );
                }
                true
            }
            // `roam` (`0x50b830`): re-arm at `now + intArg[0]`; while **not** searching
            // (`flags & 2 == 0`) turn the body's look vector (`+0x94`) by a random yaw in
            // `[-180, 180)` (`(rand × _DAT_0052329c − 0.5) × 360`) and pick a point along it:
            // `fltArg[1] <= 0` walks up to `−fltArg[1] × rand` from the body, otherwise up to
            // `fltArg[1] × rand` from home (`ai + 0x93`). `SetLookFrame` and `SetMoveThing(1.0)`
            // there. Returns 0.
            "roam" => {
                rearm(self, self.time + (instinct.weight as i32) as f32 / 1000.0);
                if self.ai_flags.get(&index).copied().unwrap_or(0) & 0x2 != 0 {
                    return false;
                }
                let range = instinct.args.first().copied().unwrap_or(0.0) as f32;
                let turn = (crate::cog::pseudo_random() as f32 - 0.5) * 360.0;
                let [dx, dy] = forward_of_yaw(ctx.yaw + turn);
                let (base, reach) = if range <= 0.0 {
                    (ctx.from, -range * crate::cog::pseudo_random() as f32)
                } else {
                    let home = self.ai_home.get(&index).copied().unwrap_or(ctx.from);
                    (home, crate::cog::pseudo_random() as f32 * range)
                };
                let point = [base[0] + dx * reach, base[1] + dy * reach, base[2]];
                self.ai_set_look_frame(index, point);
                self.ai_set_move_thing(index, point, 1.0);
                false
            }
            // `randomturn` (`0x50b700`): needs the awake bit (its registry word is 4); picks a
            // random direction every class interval (5000 ms when the first number is 0).
            "randomturn" => {
                rearm(
                    self,
                    self.time + if instinct.weight > 0.0 {
                        instinct.weight / 1000.0
                    } else {
                        5.0
                    },
                );
                self.ai_randomturn
                    .insert(index, ctx.yaw + crate::cog::pseudo_random() as f32 * 360.0);
                true
            }
            // `opendoors` (`0x50abc0`): see `sithAICmd_OpenDoors`.
            "opendoors" => self.sith_ai_cmd_open_doors(index, ctx),
            // `hitandrun` (`0x50bb90`, accept `0xc00`): see `sithAICmd_HitAndRun`.
            "hitandrun" => self.sith_ai_cmd_hit_and_run(index, slot, instinct, player),
            // `lobfire` (`0x509cd0`, accept 2): see `sithAICmd_LobFire` — the scheduler branch
            // fires the lobbed shot itself.
            "lobfire" => self.sith_ai_cmd_lobfire(index, slot, instinct, player, ctx),
            // `retreat` (`0x50bc60`, accept 2 reject 0x800): when health falls under
            // `fltArg[0] x max` and a `fltArg[1]` roll passes, set the alert bit and remember the
            // player in the target slot (the port's `ai_withdraw` walk stands in for the
            // `withdraw` handler's move).
            "retreat" => {
                if self.ai_withdraw.contains_key(&index) {
                    return false;
                }
                let max_health = self
                    .template_max_health
                    .get(&ctx.template)
                    .copied()
                    .unwrap_or(0.0);
                let fraction = instinct.weight;
                let chance = instinct.args.first().copied().unwrap_or(0.0) as f64;
                if max_health > 0.0
                    && ctx.health < fraction * max_health
                    && self.things.get(player).is_some_and(|t| !t.destroyed)
                    && crate::cog::pseudo_random() < chance
                {
                    let flags = self.ai_flags.entry(index).or_insert(0);
                    *flags |= 0x800;
                    self.ai_withdraw.insert(index, (player, None));
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Re-arm one instinct's own clock — the engine handler's `inst + 4` write.
    fn ai_rearm(&mut self, index: usize, slot: usize, next: f32) {
        if let Some(clock) = self.ai_inst.get_mut(&index).and_then(|s| s.get_mut(slot)) {
            *clock = next;
        }
    }

    /// `sithAICmd_OpenDoors` (`0x50abc0`): while the moving bit `0x1` is set, run the player's own
    /// activate action on the actor's body. Returns 0 — the engine's `now + 1000` re-arm is the
    /// same value the scheduler's default arm already wrote, so the handler adds nothing.
    fn sith_ai_cmd_open_doors(&mut self, index: usize, ctx: &AiCtx) -> bool {
        if self.ai_flags.get(&index).copied().unwrap_or(0) & 0x1 == 0 {
            return false;
        }
        let reach = self.activate_reach(index);
        let dir = [-ctx.yaw.to_radians().sin(), ctx.yaw.to_radians().cos(), 0.0];
        self.sith_player_actions_activate(index, ctx.from, dir, reach, ctx.sector);
        false
    }

    /// `FUN_0050aa80` (`sithAICmd_LookForTarget`, accept `0x204`): the awake gates, then one branch
    /// per mode word. The engine's outer gate `DAT_008ee66c & 0x200` is the console's
    /// target-search toggle (`sithCommand_DebugMode` case 6), never set in play; the port has no
    /// console, so it reads clear.
    ///
    /// **A** (`0x200` clear, `0x4` set) — the scan. Re-arm at `now + arg0` and take the player
    /// into the target slot (`ai + 0x74`) first; if the player is not `0x202`-flagged, refresh
    /// sight: `check_sight_thing` (`0x4eb090`) is the whole sight test and the detect roll rides
    /// the same refresh (`CanDetectSightThing` inside `sub_4EAD60`). Visible: the word becomes
    /// `(flags & !4) | 0x232`, the `alert` spot sound (`PlayModeRandom 0x4c` in the soundclass
    /// mode table, base `0x54b408`), the kind-0 awareness broadcast at strength 3.0
    /// (`0x40400000`), the player into the fire-target slot (`ai + 0x85`), return 1. The
    /// `PlayModeFirst(thing, 2)` puppet track on a find has no port puppet to play. Not visible
    /// and `arg0 == _DAT_00523294` (0.0, read from the shipped image): the engine rewrites the
    /// class argument to 500, so the next scan comes in half a second.
    ///
    /// **B** (`0x200` set) — the memory-decay wake: `arg1 != 0 && last_seen + arg1 < now` writes
    /// `(flags & 0xfffff9dd) | 4` and levels the head (`SetHeadPYR` with the zero vector at
    /// `0x522af8`), return 1.
    fn sith_ai_cmd_look_for_target(
        &mut self,
        index: usize,
        slot: usize,
        instinct: &crate::ai::Instinct,
        player: usize,
        ctx: &AiCtx,
    ) -> bool {
        if std::env::var_os("BK_NO_SCAN").is_some() {
            return false;
        }
        let flags = self.ai_flags.get(&index).copied().unwrap_or(0);
        if flags & 0x200 == 0 {
            if flags & 0x4 == 0 {
                return false;
            }
            self.ai_rearm(index, slot, self.time + instinct.weight / 1000.0);
            if !self
                .things
                .get(player)
                .is_some_and(|t| t.flags & 0x202 == 0 && !t.destroyed)
            {
                return false;
            }
            let Some(class) = self.ai.get(&index).cloned() else {
                return false;
            };
            let sight =
                self.check_sight_thing(index, ctx.eye, player, class.fovcos(), class.sightdist);
            if std::env::var("BK_AI_TRACE").is_ok_and(|v| v == "2") {
                eprintln!(
                    "ai: {index} `{}` scan at {:.2} s sight {sight} flags {flags:#x}",
                    ctx.template, self.time
                );
            }
            // `sub_4EAD60` rolls the detect chance only when the target was **not** already
            // visible (`if (local_4 != 0)`); a held target is never lost to a bad roll.
            if sight == 0 && (!self.ai_detected.contains(&index) || self.can_detect(index, player)) {
                self.ai_detected.insert(index);
                self.remember(index, ctx.target, "lookfortarget");
                self.ai_flags
                    .insert(index, (flags & 0xffff_fffb) | 0x232);
                self.queue_mode_sound(index, "alert", ctx.from);
                if let Some(sector) = self.things.get(player).map(|t| t.sector) {
                    self.transmit_awareness(sector, 0, ctx.from, AI_AWARE_SIGHT, Some(player));
                }
                self.ai_fire_target.insert(index, player);
                if std::env::var_os("BK_AI_TRACE").is_some() {
                    eprintln!(
                        "ai: {index} `{}` LookForTarget finds the body at {:?}",
                        ctx.template, ctx.target
                    );
                }
                return true;
            }
            // `arg0 == 0` (`_DAT_00523294` = 0.0): the engine rewrites the class's own float
            // argument to 500 (`0x43fa0000`, `param_2 + 0xc`) — a persistent class edit, not a
            // one-off re-arm — and the next pass runs at the rewritten interval.
            if instinct.weight == 0.0
                && let Some(class) = self.ai.get_mut(&index)
                && let Some(first) = class.instincts.get_mut(slot)
                && first.weight == 0.0
            {
                first.weight = 500.0;
                self.ai_rearm(index, slot, self.time + 0.5);
            }
            return false;
        }
        let decay = instinct.args.first().copied().unwrap_or(0.0) / 1000.0;
        if decay > 0.0
            && self.time
                >= self.ai_last_seen_at.get(&index).copied().unwrap_or(0.0) + decay
        {
            self.ai_flags.insert(index, (flags & 0xffff_f9dd) | 0x4);
            if let Some(thing) = self.things.get_mut(index) {
                thing.head_pitch = 0.0;
            }
            return true;
        }
        false
    }

    /// `sithAICmd_HitAndRun` (`0x50bb90`, accept `0xc00`): the attack-then-withdraw state machine,
    /// read straight off the decompile.
    ///
    /// - `0x800` set: the break-off is over — clear it and return 1 (the re-engage).
    /// - `0x400` set (a shot went out; `sithAICmd_PrimaryFire` sets it on every committed fire):
    ///   `acc -= _DAT_005232f4` with the constant **−1000.0** read from the shipped image — one
    ///   more second of firing per pass. `fltArg[0] < acc`: reset the accumulator,
    ///   `flags |= 0x800`, hand the current target to the withdraw walk (`ai + 0x1c0 =
    ///   ai + 0x1d0`; the port's stand-in is the `ai_withdraw` walk at the handler's own
    ///   deadline), re-arm at `now + fltArg[1]`, return 1. Otherwise re-arm at `now + 1000` and
    ///   return that (non-zero: the engine's handler signals "acted", though the word did not
    ///   change, so no cascade follows).
    /// - Neither bit: reset the accumulator, return 0.
    fn sith_ai_cmd_hit_and_run(
        &mut self,
        index: usize,
        slot: usize,
        instinct: &crate::ai::Instinct,
        player: usize,
    ) -> bool {
        let flags = self.ai_flags.get(&index).copied().unwrap_or(0);
        if flags & 0x800 != 0 {
            self.ai_flags.insert(index, flags & !0x800);
            return true;
        }
        if flags & 0x400 != 0 {
            let acc = self.ai_hitrun.entry(index).or_insert(0.0);
            *acc -= -1000.0;
            if instinct.weight < *acc {
                *acc = 0.0;
                self.ai_flags.insert(index, flags | 0x800);
                let break_off = instinct.args.first().copied().unwrap_or(3000.0) / 1000.0;
                self.ai_withdraw.insert(index, (player, Some(self.time + break_off)));
                self.ai_rearm(index, slot, self.time + break_off);
                return true;
            }
            self.ai_rearm(index, slot, self.time + 1.0);
            return true;
        }
        self.ai_hitrun.insert(index, 0.0);
        self.ai_rearm(index, slot, self.time + 1.0);
        false
    }

    /// `FUN_00509cd0` (`sithAICmd_LobFire`, accept 2, event mask `0x100`) — the handler wiring #93
    /// left out. The scheduler branch (event 0) is the fire path: with the player `0x202`-flagged
    /// (gone) it stops the fight — the `flee` sound (`PlayModeRandom 0x52`) when a shot is still
    /// owed (`0x400`), the puppet land track (`PlayMode 0x25`, no port puppet), and
    /// `flags = (flags & 0xfffff9dd) | 4`. Otherwise `sithAI_FireWeapon` with `fl = 2` — the lob,
    /// #93's arithmetic — with `minD/maxD/dotTol/err` from the class's numbers 3-6 and a weapon-2
    /// roll (`rand()/32768 < fltArg[5]`). Fired: `flags |= 0x400`, and both the fire lockout
    /// `ai + 0xa2` and the re-arm land at `now + arg0`. Refused (or not visible: the engine's
    /// `ai[0x7d] != 0` gate inside `FireWeapon`): a look frame at the player, `flags |= 0x400`,
    /// re-arm at `now + 500`. Every path returns 0 — this handler never raises the cascade. The
    /// event-`0x100` branch (holster/draw with the awake bit, `sithPuppet_SetArmedMode`) lives in
    /// `emit_ai_event`'s dispatch.
    fn sith_ai_cmd_lobfire(
        &mut self,
        index: usize,
        slot: usize,
        instinct: &crate::ai::Instinct,
        player: usize,
        ctx: &AiCtx,
    ) -> bool {
        let flags = self.ai_flags.get(&index).copied().unwrap_or(0);
        let player_gone = !self
            .things
            .get(player)
            .is_some_and(|t| t.flags & 0x202 == 0 && !t.destroyed);
        if player_gone {
            if flags & 0x400 != 0 {
                self.queue_mode_sound(index, "flee", ctx.from);
            }
            self.ai_flags.insert(index, (flags & 0xffff_f9dd) | 0x4);
            return true;
        }
        let refused = !ctx.sighted || {
            let accuracy = self.ai.get(&index).map(|c| c.accuracy);
            let mode = u32::from(
                crate::cog::pseudo_random() < instinct.args.get(4).copied().unwrap_or(0.0) as f64,
            );
            matches!(
                self.sith_ai_fire_weapon(
                    index,
                    player,
                    ctx.target,
                    instinct.args.get(1).copied().unwrap_or(0.0),
                    instinct.args.get(2).copied().unwrap_or(f32::MAX),
                    instinct.args.first().copied().unwrap_or(0.5),
                    instinct.args.get(3).copied().unwrap_or(0.0),
                    mode,
                    2,
                    accuracy,
                ),
                FireOutcome::Refused
            )
        };
        if refused {
            self.ai_set_look_frame(index, ctx.target);
            *self.ai_flags.entry(index).or_insert(0) |= 0x400;
            self.ai_rearm(index, slot, self.time + 0.5);
            return false;
        }
        *self.ai_flags.entry(index).or_insert(0) |= 0x400;
        let at = self.time + instinct.weight / 1000.0;
        self.ai_fire_lock.insert(index, at);
        self.ai_rearm(index, slot, at);
        self.last_shot.insert(index, self.time);
        false
    }
    /// The nearest actor a shot from `from` along `dir` would sweep into within `range` — the port's
    /// stand-in for the engine's `sithCollision_SearchForCollisions` in the player-fire pre-sweep. The
    /// engine collides the shot by its own `movesize` against the actor's body cylinder; this uses the
    /// actor's own `movesize` radius against the shot's line, with a wall check on the segment.
    fn first_actor_on_path(&self, from: [f32; 3], dir: [f32; 3], range: f32) -> Option<usize> {
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if len < 1e-4 {
            return None;
        }
        let unit = [dir[0] / len, dir[1] / len, dir[2] / len];
        let mut best: Option<(f32, usize)> = None;
        for (index, thing) in self.things.iter().enumerate() {
            if thing.destroyed || self.types.get(index).copied() != Some(2) {
                continue;
            }
            let d = [
                thing.pos[0] - from[0],
                thing.pos[1] - from[1],
                thing.pos[2] - from[2],
            ];
            let along = d[0] * unit[0] + d[1] * unit[1] + d[2] * unit[2];
            if along <= 0.0 || along > range {
                continue;
            }
            let miss = [
                d[0] - unit[0] * along,
                d[1] - unit[1] * along,
                d[2] - unit[2] * along,
            ];
            let radius = self
                .templates
                .get(index)
                .map(|name| name.to_ascii_lowercase())
                .and_then(|name| self.template_hit.get(&name))
                .map(|(r, _)| *r)
                .unwrap_or(0.065);
            if (miss[0] * miss[0] + miss[1] * miss[1] + miss[2] * miss[2]).sqrt() > radius {
                continue;
            }
            let at = [
                from[0] + unit[0] * along,
                from[1] + unit[1] * along,
                from[2] + unit[2] * along,
            ];
            if self
                .solids
                .as_ref()
                .is_some_and(|solids| solids.segment_blocked(from, at))
            {
                continue;
            }
            if best.is_none_or(|(was, _)| along < was) {
                best = Some((along, index));
            }
        }
        best.map(|(_, index)| index)
    }

    /// `dodge`'s shot-passing path (`0x50b360`, event `0x1000`): the player's pre-swept shot found the
    /// actor, and the actor **sees the shot** — the handler runs `sithAI_CheckSightThing(actor,
    /// actor->pos, projectile, class sightdist, class fov)` and only then reacts. It rotates the
    /// direction to the shot by a random **45-90 degrees** (`rand * 45 + 45`, negated on a second roll
    /// against 0.5 — the constants are the EXE's `_DAT_005232b0` 45, `_DAT_005232b4` -45 and
    /// `_DAT_005232a0` 0.5) and steps `arg0` units along the **negated** rotated direction: a sideways
    /// dodge, not a retreat.
    fn dodge_from_shot(&mut self, actor: usize, shot: [f32; 3]) -> bool {
        if self.ai_flags.get(&actor).copied().unwrap_or(0) & 0x200 == 0 {
            return false;
        }
        let Some(step) = self
            .ai
            .get(&actor)
            .and_then(|class| class.instinct("Dodge"))
            .and_then(|dodge| dodge.args.first().copied())
        else {
            return false;
        };
        let step = step as f32;
        if step == 0.0 {
            return false;
        }
        let Some((from, yaw)) = self.things.get(actor).map(|t| (t.pos, t.yaw)) else {
            return false;
        };
        if !self
            .ai
            .get(&actor)
            .is_some_and(|class| class.sees(from, yaw, shot))
        {
            return false;
        }
        if self
            .solids
            .as_ref()
            .is_some_and(|solids| solids.segment_blocked(from, shot))
        {
            return false;
        }
        let (dx, dy) = (shot[0] - from[0], shot[1] - from[1]);
        let flat = (dx * dx + dy * dy).sqrt();
        if flat < 1e-4 {
            // The shot is on the actor: there is no direction to step around.
            return false;
        }
        let mut angle = crate::cog::pseudo_random() as f32 * 45.0 + 45.0;
        if (crate::cog::pseudo_random() as f32) < 0.5 {
            angle = -angle;
        }
        let (sin, cos) = angle.to_radians().sin_cos();
        let dir = [dx / flat, dy / flat];
        let rotated = [dir[0] * cos - dir[1] * sin, dir[0] * sin + dir[1] * cos];
        let point = [from[0] - rotated[0] * step, from[1] - rotated[1] * step, from[2]];
        self.ai_set_move_thing(actor, point, 2.5);
        if std::env::var_os("BK_AI_TRACE").is_some() {
            eprintln!("ai: {actor} dodges the shot from {shot:?} sideways to {point:?}");
        }
        true
    }

    /// `jump` (`0x50abf0`), the event-`0x400` arm — the only arm content can reach. The engine's
    /// steering helper `sithAI_idk_msgarrived_target` raises `0x400` when a **moving** actor's way ahead
    /// is not walkable (it clears the steering first), and the registry word from the `aistatus` dump is
    /// `mask 0x604`: events `4` and `0x200` are registered but never emitted, so those arms are dead
    /// content like `dodge`'s event 2.
    ///
    /// The handler's gates, read from the decomp: a non-null `thing + 0x34` (the port reads that as
    /// "attached"), the AI's moving bit `0x1`, and "already travelling toward the target" (`0.02 <
    /// dot(dir, vel)`, `_DAT_005232e8`). Then it adds `dir * 0.1` (`_DAT_005232e4`) to the velocity along
    /// the **direction to the target** and sets the look frame at the target
    /// (`sithAI_SetLookFrame(ai, target, 1.0)`). The two class numbers are the leap distance (`3.0` in
    /// every shipped line, `args[1]`) and the handler's own next-check delay (the line's first number).
    ///
    /// The engine also tests its walkability helper at the leap point (`FUN_004eb4b0(ai, point, 0)`,
    /// [`Self::sith_ai_can_walk`]) — a leap must land on ground.
    fn jump_from_obstacle(&mut self, actor: usize, weight: f32) -> bool {
        let Some(t) = self.things.get(actor) else {
            return false;
        };
        if t.attach.is_some() || self.ai_flags.get(&actor).copied().unwrap_or(0) & 0x1 == 0 {
            return false;
        }
        let Some(dist) = self
            .ai
            .get(&actor)
            .and_then(|class| class.instinct("Jump"))
            .and_then(|jump| jump.args.get(1).copied())
        else {
            return false;
        };
        let due = self.ai_jump.get(&actor).copied().unwrap_or(0.0);
        if self.time < due {
            return false;
        }
        let (from, vel, goal) = (t.pos, t.vel, self.ai_move.get(&actor).copied());
        let Some(goal) = goal else {
            return false;
        };
        let (dx, dy) = (goal[0] - from[0], goal[1] - from[1]);
        let flat = (dx * dx + dy * dy).sqrt();
        if flat < 1e-4 {
            return false;
        }
        let dir = [dx / flat, dy / flat];
        if dir[0] * vel[0] + dir[1] * vel[1] > 0.02 {
            return false;
        }
        let point = [from[0] + dir[0] * dist as f32, from[1] + dir[1] * dist as f32, from[2]];
        // `FUN_004eb4b0(ai, point, 0)`: the engine tests its **walkability helper** at the leap point,
        // so the leap is taken only onto ground the body can stand on — a clear line is not the
        // question a leap asks.
        if self.sith_ai_can_walk(actor, point, self.solids.as_deref()) == 0 {
            return false;
        }
        self.ai_jump
            .insert(actor, self.time + weight / 1000.0);
        if let Some(t) = self.things.get_mut(actor) {
            // `_DAT_005232e4` = 0.1: the engine's velocities are level units a second (`thing + 0x234`
            // is `vel * dt`), so this is a real but small unstick toward the target.
            t.vel[0] += dir[0] * 0.1;
            t.vel[1] += dir[1] * 0.1;
        }
        // `FUN_0050abf0`'s event-`0x400` arm ends in `sithAI_Jump(ai, ai + 0x68, 1.0)`: the move
        // destination it already holds, taken at speed 2.0 with the puppet's `0x1b` jump play, the
        // `jumpspeed` hop and the 2000 ms lockout. The yaw snap the port had here is not engine
        // behaviour — the handler never sets a look frame, so the body keeps its facing.
        self.ai_jump_move(actor, goal);
        if std::env::var_os("BK_AI_TRACE").is_some() {
            eprintln!("ai: {actor} is blocked and nudges toward {goal:?} (jump)");
        }
        true
    }

    /// `sithAI_CanDetectSightThing` (`0x4ec140`): the **chance** that `actor` registers `target` on
    /// one sight check, from distance, the watcher's awake bit and the target's motion, pose and flags.
    /// Line of sight is a different question (`Ai::sees` plus the wall cast) — this one answers "did it
    /// notice". The engine's constants are read out of the executable's `.rdata`:
    ///
    /// - beyond `2.0` units (`_DAT_00523078`), a watcher whose AI flags lack `0x200` (the awake bit
    ///   `talk`/`sensedanger` gate on) starts at 0.5 instead of 1.0;
    /// - the range falloff is `0.1` per unit (`_DAT_00523084`), capped at `0.6` (`_DAT_00523088`);
    /// - a physics target (`move=physics`) that is perfectly still multiplies 0.5 (`_DAT_00523070`),
    ///   and one that is crouched (`physflags & 0x10000`) multiplies 0.75 (`_DAT_00523090`);
    /// - a target whose `thingflags` carry `0x80`, or a watcher whose `thingflags` carry `0x800`,
    ///   multiplies 0.05 (`_DAT_00523094`), the same value the final clamp floors at (cap 1.0,
    ///   `_DAT_00523068`).
    ///
    /// - the darkness term (round 314): the **target's sector** (`thing + 0x5c`) carries the level's
    ///   baked light at `+4`, and under 0.5 the chance scales by `light + 0.2` (`_DAT_0052308c` is
    ///   -0.2) unless the watcher's thingflags carry `0x2000000`. That field is the level's
    ///   `AMBIENT LIGHT` (`Runtime::sector_ambient_light`); `EXTRA LIGHT` belongs to the POV pass instead.
    ///
    /// - the block's own gate (round 317): a target whose `thingflags` carry `0x4`, or whose saber is
    ///   **on** (`thing + 0x2a8` bit 1 — the bit `jkSaber_Draw` and `jkPlayer_DrawPov` read), skips the
    ///   falloff, darkness and pose terms; the blade's glow is what gives the body away.
    pub fn detect_chance(&self, actor: usize, target: usize) -> f32 {
        let (Some(watcher), Some(seen)) = (self.things.get(actor), self.things.get(target)) else {
            return 1.0;
        };
        if !matches!(self.types.get(target).copied(), Some(2) | Some(10)) {
            return 1.0;
        }
        let (dx, dy, dz) = (
            seen.pos[0] - watcher.pos[0],
            seen.pos[1] - watcher.pos[1],
            seen.pos[2] - watcher.pos[2],
        );
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
        let mut chance = 1.0f32;
        if distance >= DETECT_NEAR {
            if self.ai_flags.get(&actor).copied().unwrap_or(0) & 0x200 == 0 {
                chance = 0.5;
            }
            // The decomp's inner gate: `seen->flags & 4` (the target's own invisibility bit) and
            // `seen->flags2a8 & 1`, which `jkSaber_Draw`/`jkPlayer_DrawPov` read as the player's
            // **saber on**. A lit blade skips the range falloff, the darkness term and the pose
            // terms: the glowing blade is what gives the body away. The hidden/flag terms below run
            // either way, exactly as they sit outside that block in the decomp.
            let sabre_lit = self.saber_shown(target);
            if seen.flags & 0x4 == 0 && !sabre_lit {
                let falloff =
                    ((distance - DETECT_NEAR) * DETECT_FALLOFF).clamp(0.0, DETECT_MAX_FALLOFF);
                chance *= 1.0 - falloff;
                // The darkness term: the engine reads the *target's sector* (`thing + 0x5c`) light at
                // `+4` and, under 0.5, scales the chance by `light + 0.2` (`_DAT_0052308c` is -0.2) -
                // unless the watcher's thingflags carry `0x2000000`. A missing sector (a fixture with
                // no level) reads as lit.
                let dark = self
                    .sector_ambient_light
                    .get(seen.sector.max(0) as usize)
                    .copied()
                    .unwrap_or(1.0);
                if watcher.flags & 0x2000000 == 0 && dark < 0.5 {
                    chance *= dark + 0.2;
                }
                let physics = self
                    .templates
                    .get(target)
                    .map(|template| template.to_ascii_lowercase())
                    .and_then(|template| self.template_move.get(&template))
                    .is_some_and(|m| m.eq_ignore_ascii_case("physics"));
                if physics {
                    if seen.phys_flags & 0x10000 != 0 {
                        chance *= DETECT_CROUCH;
                    }
                    if seen.vel == [0.0; 3] {
                        chance *= DETECT_STILL;
                    }
                }
            }
        }
        if seen.flags & 0x80 != 0 && watcher.flags & 0x8000 == 0 {
            chance *= DETECT_UNSEEN;
        }
        if watcher.flags & 0x800 != 0 {
            chance *= DETECT_UNSEEN;
        }
        chance.clamp(DETECT_UNSEEN, 1.0)
    }

    /// One roll of [`Self::detect_chance`] — `sithAI_CanDetectSightThing`'s `rand/32768` test.
    /// Record that `actor` last had `target` in view, naming the handler that did it under
    /// `BK_AI_TRACE`. The remembered point is what `BlindFire` and the chase walk to, so an insert
    /// from an unexpected handler is visible instead of inferred from the shots that follow it.
    fn remember(&mut self, actor: usize, target: [f32; 3], why: &str) {
        if std::env::var_os("BK_AI_TRACE").is_some() {
            eprintln!("ai: {actor} remembers the body at {target:?} ({why})");
        }
        self.ai_last_seen.insert(actor, target);
        self.ai_last_seen_at.insert(actor, self.time);
    }

    pub fn can_detect(&self, actor: usize, target: usize) -> bool {
        crate::cog::pseudo_random() < self.detect_chance(actor, target) as f64
    }

    /// `sithAI_CheckSightThing` (`0x4eb090`): the engine's one sight test, transliterated.
    /// Returns its code: `0` visible, `1` out of range, `2` outside the class's cone, `3` no line
    /// (or no target).
    ///
    /// - The distance is the **3D** eye→target length with the target's radius removed, and the
    ///   range gate `sightdist < dist − radius` removes it a **second** time, so a body whose edge
    ///   is inside `sightdist` stays visible out to `sightdist + 2·radius` of centre.
    /// - The cone is the sideways-component test ([`crate::ai::cone_holds`]) over the observer's
    ///   yaw frame: `lvec = (−sin yaw, cos yaw)`, `rvec = (cos yaw, sin yaw)`.
    /// - The engine's cloak rules (`thingflags & 0x2000000` on either side return 3) are not
    ///   modelled — the same ledger entry as `detect_chance`'s dark-sector rule.
    /// - The engine's collision sweep (flags `0x102`) stops on surfaces and on type-2/7 things and
    ///   counts the target itself as visible; the port's stand-in is the `segment_blocked` +
    ///   `segment_thing` pair, which covers surfaces and placed solid things but not other actors'
    ///   bodies.
    pub fn check_sight_thing(
        &self,
        observer: usize,
        eye: [f32; 3],
        target: usize,
        fovcos: f32,
        sightdist: f32,
    ) -> u8 {
        let Some(t) = self.things.get(target) else { return 3; };
        let tpos = t.pos;
        let radius = self
            .templates
            .get(target)
            .and_then(|name| self.template_hit.get(&name.to_ascii_lowercase()))
            .map(|(r, _)| *r)
            .unwrap_or(0.065);
        let d = [tpos[0] - eye[0], tpos[1] - eye[1], tpos[2] - eye[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let dist = (len - radius).max(0.0);
        // `param_5 < fVar9 - r` on a `fVar9` that already removed the radius: twice.
        if sightdist < dist - radius {
            return 1;
        }
        if fovcos > -1.0 && len > 1e-4 {
            let yaw = self.things.get(observer).map(|t| t.yaw).unwrap_or(0.0);
            let (sin, cos) = yaw.to_radians().sin_cos();
            if !crate::ai::cone_holds(
                fovcos,
                [-sin, cos, 0.0],
                [cos, sin, 0.0],
                [d[0] / len, d[1] / len, d[2] / len],
            ) {
                return 2;
            }
        }
        if self.solids.as_ref().is_some_and(|s| {
            s.segment_blocked(eye, tpos) || s.segment_thing(eye, tpos)
        }) {
            return 3;
        }
        0
    }

    /// `sithAI_FirstThingInView` (`0x4eb790`): collect the things inside a frustum, in sector
    /// walk order, up to `max_count` of them. The engine's caller is `sithWeapon_GetAimOrient`
    /// (`0x4d5c60`, the aim assist) — the AI scan itself goes through
    /// `Runtime::check_sight_thing` instead.
    ///
    /// `hfov`/`vfov` are **total** cone degrees; the engine stores their limits as
    /// `sin(fov · 0.5)` (`stdMath_SinCos(90 − fov · 0.5)` reads the cosine — `0x523070 = 0.5`,
    /// `0x523074 = 90` in the shipped image). A negative fov refuses the whole call, as in the
    /// engine.
    pub fn first_thing_in_view(
        &self,
        start_sector: usize,
        eye: [f32; 3],
        yaw: f32,
        hfov: f32,
        vfov: f32,
        max_count: usize,
        type_mask: u32,
        max_dist: f32,
    ) -> Vec<usize> {
        if !(hfov >= 0.0 && vfov >= 0.0) {
            return Vec::new();
        }
        let (sin, cos) = yaw.to_radians().sin_cos();
        let frame = ([cos, sin, 0.0], [-sin, cos, 0.0], [0.0, 0.0, 1.0]);
        let mut by_sector = std::collections::BTreeMap::new();
        for (index, thing) in self.things.iter().enumerate() {
            if !thing.destroyed {
                by_sector
                    .entry(thing.sector.max(0) as usize)
                    .or_insert_with(Vec::new)
                    .push(index);
            }
        }
        let mut out = Vec::new();
        let mut visited = std::collections::BTreeSet::new();
        self.get_things_in_view(
            start_sector, eye, frame, hfov, vfov, max_count, type_mask, max_dist, 0.0,
            &mut by_sector, &mut visited, &mut out,
        );
        out
    }

    /// `sithAI_GetThingsInView` (`0x4ebe80`): one sector's things inside the frustum, then a
    /// recursion through the sector's passable adjoins, at most `0x80` sectors per call (the
    /// engine's per-render-tick visit cap — one call is one tick, so a visit set is the same
    /// rule).
    ///
    /// A thing qualifies when its type's bit is in `type_mask`, its `thingflags` carry none of
    /// `0x80202`, and `|uvec·d| ≤ vlim`, `|rvec·d| ≤ hlim`, `lvec·d ≥ 0` for the normalized
    /// eye→thing direction — note the **vertical** limit is the same kind of sideways test as
    /// the horizontal one, over the frame's up row.
    ///
    /// The engine also admits an opening whose surface has no geometry, a scrolled surface flag,
    /// or a transparent material (`iVar4 == 0 || surface+0x1c == 0 || surface+0x18&2 ||
    /// texture+0x1c&1`); those three tests are not mapped to port data yet — the port recurses
    /// through every adjoin whose flag word still has `0x1` and that faces the look direction
    /// (`lvec·outward_normal > 0`; the engine's normals point into the sector, so its test reads
    /// `< 0`).
    fn get_things_in_view(
        &self,
        sector: usize,
        eye: [f32; 3],
        frame: ([f32; 3], [f32; 3], [f32; 3]),
        hfov: f32,
        vfov: f32,
        max_count: usize,
        type_mask: u32,
        max_dist: f32,
        depth: f32,
        by_sector: &mut std::collections::BTreeMap<usize, Vec<usize>>,
        visited: &mut std::collections::BTreeSet<usize>,
        out: &mut Vec<usize>,
    ) {
        if visited.len() >= 0x80 || !visited.insert(sector) {
            return;
        }
        let (rvec, lvec, uvec) = frame;
        let hlim = (hfov * 0.5).to_radians().sin();
        let vlim = (vfov * 0.5).to_radians().sin();
        if let Some(things) = by_sector.remove(&sector) {
            for index in things {
                if out.len() >= max_count {
                    return;
                }
                let ty = self.types.get(index).copied().unwrap_or(0) as u32;
                if type_mask & (1 << (ty & 0x1f)) == 0 {
                    continue;
                }
                let Some(thing) = self.things.get(index) else { continue };
                if thing.flags & 0x80202 != 0 {
                    continue;
                }
                let d = [
                    thing.pos[0] - eye[0],
                    thing.pos[1] - eye[1],
                    thing.pos[2] - eye[2],
                ];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if len <= 1e-4 {
                    out.push(index);
                    continue;
                }
                let dir = [d[0] / len, d[1] / len, d[2] / len];
                let up = uvec[0] * dir[0] + uvec[1] * dir[1] + uvec[2] * dir[2];
                let right = rvec[0] * dir[0] + rvec[1] * dir[1] + rvec[2] * dir[2];
                let look = lvec[0] * dir[0] + lvec[1] * dir[1] + lvec[2] * dir[2];
                if up <= vlim && up >= -vlim && right <= hlim && right >= -hlim && look >= 0.0 {
                    out.push(index);
                }
            }
        }
        if depth <= max_dist
            && let Some(solids) = self.solids.as_ref()
            && let Some(portals) = solids.portals.get(sector)
        {
            for portal in portals {
                let visible = self
                    .adjoin_flags
                    .get(portal.link)
                    .map_or(crate::collide::ADJOIN_VISIBLE, |f| *f)
                    & crate::collide::ADJOIN_VISIBLE
                    != 0;
                let ahead = lvec[0] * portal.normal[0]
                    + lvec[1] * portal.normal[1]
                    + lvec[2] * portal.normal[2];
                if visible && ahead > 0.0 {
                    let step = self.adjoin_depth.get(portal.link).copied().unwrap_or(0.0);
                    self.get_things_in_view(
                        portal.adjoin, eye, frame, hfov, vfov, max_count, type_mask, max_dist,
                        depth + step, by_sector, visited, out,
                    );
                }
            }
        }
    }

    /// Adopt the committed capture's AI phase at the encounter's origin clock.
    ///
    /// The engine's `rand()` state (`0x0054F750`) is the capture's own word, so every AI roll after
    /// this point continues the captured stream; each actor's flag word and scheduled deadlines are
    /// the values the engine held at that instant, converted to the runtime's clock relative to the
    /// origin. An actor whose shot the engine had not scheduled (`fire_ms` 0 or -1) keeps whatever
    /// wait it had: the capture says nothing is pending, and inventing a shot would be a guess.
    ///
    /// `next_pass_ms` lands on `ai_detect_at`, the port's own instinct-pass clock: the engine's
    /// `sithAI_InstinctUpdate` refreshes sight inside a pass, which is why the port already rolls the
    /// sight test on that clock instead of every frame.
    ///
    /// A captured deadline of 0 is **due now**, not "never": `sithAI_Process` calls
    /// `sithAI_InstinctUpdate` when `instance + 0x190 <= now`, and the update always writes a fresh
    /// deadline (`now + 5000` at the earliest), so a captured 0 is a fresh instance whose first pass
    /// is due on the next tick. Seeding the clocks from the row's own offset keeps that.
    pub fn seed_capture_phase(
        &mut self,
        rng: u32,
        origin_ms: i64,
        phases: &[(usize, CapturedPhase)],
    ) {
        crate::cog::set_random_seed(rng);
        for (index, phase) in phases {
            let offset = |ms: i64| ((ms - origin_ms) as f32 / 1000.0).max(0.0);
            let due = |ms: i64, now: f32| now + offset(ms);
            if self.types.get(*index).copied() != Some(2) {
                continue;
            }
            self.ai_flags.insert(*index, phase.flags);
            if phase.fire_ms > 0
                && let Some(body) = self.things.get_mut(*index)
            {
                body.ai_wait = offset(phase.fire_ms);
            }
            // `acquire_next_ms` is the `lookfortarget` instinct's own clock; `next_pass_ms` is the
            // slot's whole-AI deadline (`ai + 0x190`). Both seed the scheduler's slots.
            if let Some(class) = self.ai.get(index) {
                let count = class.instincts.len();
                let slots = self.ai_inst.entry(*index).or_default();
                slots.resize(count, 0.0);
                if let Some(slot) = class
                    .instincts
                    .iter()
                    .position(|i| i.name.eq_ignore_ascii_case("LookForTarget"))
                {
                    if let Some(clock) = slots.get_mut(slot) {
                        *clock = due(phase.acquire_ms, self.time);
                    }
                }
            }
            self.ai_next_due
                .insert(*index, due(phase.next_pass_ms, self.time));
            self.ai_detect_at.insert(*index, due(phase.next_pass_ms, self.time));
        }
    }

    /// One AI tick: every actor that can see the player fires its own `weapon=`.
    ///
    /// The engine runs an instinct scheduler over the `.ai` file's weights; this build uses the
    /// fields whose meaning is not in doubt — `sightdist=`, `fov=`, and `PrimaryFire`'s own shot
    /// delay — and no scheduler. Returns how many shots were fired.
    pub fn ai_tick(&mut self, player: usize, dt: f32) -> usize {
        let Some(target) = self.things.get(player).map(|t| t.pos) else { return 0 };
        // `BK_AI_ROSTER=1`: what stands near the body and which of them the port has given an AI
        // class. An actor outside `self.ai` is never ticked at all, so this is the first thing to
        // look at when a room fires nothing. Debug only.
        if std::env::var_os("BK_AI_ROSTER").is_some() {
            static ROSTER: std::sync::Once = std::sync::Once::new();
            let mut near: Vec<(f32, usize)> = self
                .things
                .iter()
                .enumerate()
                .filter(|(index, thing)| {
                    !thing.destroyed && self.types.get(*index).copied() == Some(2)
                })
                .map(|(index, thing)| {
                    let d = (thing.pos[0] - target[0]).powi(2) + (thing.pos[1] - target[1]).powi(2);
                    ((d + (thing.pos[2] - target[2]).powi(2)).sqrt(), index)
                })
                .filter(|(d, _)| *d <= 8.0)
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            ROSTER.call_once(|| {
                for (distance, index) in near {
                    let template = self.templates.get(index).cloned().unwrap_or_default();
                    let pos = self.things[index].pos;
                    let yaw = self.things[index].yaw;
                    let class = self.ai.get(&index);
                    let in_cone = class.is_some_and(|ai| ai.sees(pos, yaw, target));
                    let los = self.solids.as_ref().is_none_or(|s| !s.segment_blocked(pos, target));
                    let bearing = (target[1] - pos[1]).atan2(target[0] - pos[0]).to_degrees();
                    let mut off = (bearing - yaw).abs() % 360.0;
                    if off > 180.0 {
                        off = 360.0 - off;
                    }
                    eprintln!(
                        "roster: {index} `{template}` d {distance:.2} cone {in_cone} los {los} yaw {yaw:.0} bearing {bearing:.0} off {off:.0} ai {}",
                        if class.is_some() { "yes" } else { "NO" }
                    );
                }
            });
        }
        let mut fired = 0;
        let mut shooters: Vec<usize> = Vec::new();

        // Every actor with an instinct class: `sithAI_Create` (`0x4e9cb0`) gives a thing an AI
        // block only when its `aiclass=` loaded, and the AI verbs gate on that block.
        let actors: Vec<usize> = self.ai.keys().copied().collect();
        for index in actors {
            let ai = self.ai.get(&index).cloned();
            let ai_template = self.templates.get(index).cloned().unwrap_or_default();
            // `sithAI_Process` (`0x4e9e20`) gates one AI slot before its instinct pass: the thing is
            // not `0x202`-flagged, its health is above `_DAT_0052304c` (0.0, read from the shipped
            // image), and the AI mode word carries none of the **`0x3000` family** — the shipped
            // scripts' `0x2000` sleep switch (`00_aisleep.cog` "aisetmode(enemy, 0x2000)") **and**
            // the `0x1000` one-shot latch `sithAI_Create` (`0x4e9cb0`) seeds with its `0x1004`, which
            // only the awareness sweep's event 2 clears: a placed actor runs no instinct at all until
            // something in its sector makes a noise or a spotter broadcasts the player. The pass
            // deadline (`ai + 0x190`) is checked inside `ai_process`.
            if let Some(thing) = self.things.get(index)
                && (thing.flags & 0x202 != 0 || thing.health <= 0.0)
            {
                continue;
            }
            if self.ai_flags.get(&index).copied().unwrap_or(0) & 0x3000 != 0 {
                if let Some(thing) = self.things.get_mut(index) {
                    thing.ai_target = None;
                }
                continue;
            }
            // `sithAI_Tick` (`0x4ea5e0`) — the turn blend and the move arm — runs in the thing's own
            // update slot (`sith_ai_tick`), after this scheduler pass.
            let Some(thing) = self.things.get_mut(index) else { continue };
            if thing.destroyed || self.types.get(index).copied() != Some(2) {
                continue;
            }
            // The firing cadence gates the *shot*, not the legs: an actor that has just
            // fired keeps walking, which is what makes it close the distance instead of
            // stopping for a second every shot.
            thing.ai_wait = (thing.ai_wait - dt).max(0.0);
            let ready = thing.ai_wait <= 0.0;
            let (from, yaw, health) = (thing.pos, thing.yaw, thing.health);
            // `sithAI_sub_4EAD60` builds the sight eye from the thing's model offset, above the
            // body's origin. An actor standing on a table, a crate or a balcony therefore casts its
            // sight ray from above that surface, and the ray leaves the solid thing it stands on;
            // the port's ray started *on* the surface, its leaf was skipped by the solid test, and
            // level 1's upstairs Greedo saw the player straight through the balcony floor and
            // turned while retail's actor never moved.
            let eye = {
                let radius = self
                    .template_hit
                    .get(&ai_template.to_ascii_lowercase())
                    .map(|(radius, _)| *radius)
                    .unwrap_or(0.065);
                [from[0], from[1], from[2] + radius + 0.01]
            };
            let mut want_yaw = yaw;
            let look = self.ai_look.get(&index).copied();
            // `AISetLookPos` is a standing turn: it decides facing whenever the legs are not carrying
            // the actor somewhere, which is how `00_AIPatrol.cog` reverses at the end of each leg.
            let look_yaw = look.map(|l| {
                (-(l[0] - from[0])).atan2(l[1] - from[1]).to_degrees()
            });
            // No instinct class means no sight and no instincts: a class-less actor still walks a
            // destination a script gave it, but it never spots the player on its own.
            // Sight is `sithAI_CheckSightThing` (`0x4eb090`, `Runtime::check_sight_thing`) from
            // the eye `sithAI_sub_4EAD60` raises above the body: the engine's one test covers the
            // range (the target's radius removed twice), the class's sideways-component cone, and
            // the collision ray — where the port's old pair tested `Ai::sees` from the body origin
            // and only then the two segment tests.
            let ai_on = !self.ai_cleared.contains(&index);
            let sighted = ai_on
                && ai.as_ref().is_some_and(|ai| {
                    self.check_sight_thing(index, eye, player, ai.fovcos(), ai.sightdist) == 0
                });
            // `sithAI_CanDetectSightThing` (`0x4ec140`): a clear line is not the same as noticing. The
            // engine rolls its detection chance whenever it refreshes a target; the port rolls it when
            // the actor first registers the player and keeps the deterministic line test afterwards, so
            // an actor cannot lose a target it has already engaged (ledger). A target inside two units
            // (or one with `thingflags & 0x4`) passes at 1.0, so this only delays ranged acquisition.
            let mut sighted = sighted;
            if sighted && !self.ai_detected.contains(&index) {
                // One roll per instinct pass, not per frame: the engine's `sithAI_InstinctUpdate`
                // (`0x4e9e90`) arms each instinct's own clock at `now + 1000` before calling it, and
                // the sight refresh lives inside an instinct. A per-frame roll turned a 50 % chance
                // into "sees you immediately", which is what made level 3's tuskens fire inside a
                // second while retail's series records no damage at all.
                let due = self
                    .ai_detect_at
                    .get(&index)
                    .copied()
                    .unwrap_or(f32::NEG_INFINITY);
                if self.time >= due {
                    self.ai_detect_at.insert(index, self.time + 1.0);
                    if self.can_detect(index, player) {
                        self.ai_detected.insert(index);
                        self.remember(index, target, "sight");
                        // `FUN_0050aa80`'s branch A — the engine's acquisition path — writes
                        // `(flags & 0xffff_fffb) | 0x232` (clear awake, set engaged/searching) and
                        // broadcasts `CreateTransmittingEvent(player->sector, actor->pos, kind 0,
                        // 3.0, player)`. The engaged bit makes it once per acquisition, not per frame.
                        let flags = self.ai_flags.entry(index).or_insert(0);
                        // Only a class that owns the instinct runs its handler, so only those alert
                        // the neighbourhood; the sight refresh for everyone else stays silent.
                        let broadcasts = ai
                            .as_ref()
                            .is_some_and(|class| class.instinct("LookForTarget").is_some());
                        if broadcasts && *flags & 0x200 == 0 {
                            *flags = (*flags & 0xffff_fffb) | 0x232;
                            if let Some(sector) = self.things.get(player).map(|t| t.sector) {
                                self.transmit_awareness(
                                    sector,
                                    0,
                                    from,
                                    AI_AWARE_SIGHT,
                                    Some(player),
                                );
                            }
                        }
                    } else {
                        sighted = false;
                    }
                } else {
                    sighted = false;
                }
            }
            // `sithAI_Process` -> `sithAI_InstinctUpdate`: the instinct pass runs after the sight
            // preamble (its handlers read the same eye/sight facts) and before the frame's motion
            // half — the engine's own order (decisions in the scheduler, motion in `sithAI_Tick`).
            let ctx = AiCtx {
                from,
                yaw,
                sector: self.things.get(index).map(|t| t.sector).unwrap_or(-1),
                eye,
                target,
                sighted,
                health,
                template: ai_template.clone(),
            };
            if ai.is_some() {
                self.ai_process(index, player, dt, &ctx);
            }
            // `forcepowers` (`0x40ff40`): the handler's own clock is `fltArg[0]`; while the target is
            // visible (`ai + 0x1f4 == 0`) and the ability cooldown (`ai + 0x288`) has passed, it
            // rolls one number against the class line's three chances and sends `user0` (id 27) to
            // the actor's **own** scripts with `(target thing id, ability 1..3)`. The shipped
            // `darkjedi_*.cog` handlers do the rest — force persuasion, blinding, grip — so this is
            // the engine-side half only, exactly as `ThingSendMessageEx` (`0x4dfaf0`) is.
            if let Some(force) = ai.as_ref().and_then(|class| class.instinct("ForcePowers")).cloned() {
                let due = self.ai_force.get(&index).copied().unwrap_or(self.time);
                if self.time >= due {
                    self.ai_force
                        .insert(index, self.time + (force.weight / 1000.0).max(0.0));
                    let cooling = self.ai_force_cool.get(&index).copied().unwrap_or(0.0);
                    if sighted && self.time >= cooling {
                        let reach = ((target[0] - from[0]).powi(2)
                            + (target[1] - from[1]).powi(2)
                            + (target[2] - from[2]).powi(2))
                        .sqrt();
                        // `sithAI_CheckSightThing` reports the gap after the target's own body
                        // radius, clamped at zero (`param_3[0xc]`).
                        let radius = self
                            .templates
                            .get(player)
                            .and_then(|name| self.template_hit.get(&name.to_ascii_lowercase()))
                            .map(|(radius, _)| *radius)
                            .unwrap_or(0.065);
                        let distance = (reach - radius).max(0.0);
                        if let Some((power, cooldown)) = force_power(
                            &force.args,
                            distance,
                            crate::cog::pseudo_random(),
                        ) {
                            // `ThingSendMessageEx(thing, 0, user0, target_id, power, 0, 0)`: the
                            // receiver's `GetParam(0)` is the victim, `GetParam(1)` the power.
                            self.send_thing_message(
                                index,
                                "user0",
                                [player as f64, f64::from(power)],
                            );
                            self.ai_force_cool
                                .insert(index, self.time + (cooldown / 1000.0).max(0.0));
                            if std::env::var_os("BK_AI_TRACE").is_some() {
                                eprintln!(
                                    "ai: {index} forcepowers ability {power} at {distance:.2} (cooldown {cooldown:.0} ms)"
                                );
                            }
                        }
                    }
                }
            }
            // `saberfighting` (`0x40fa40`): a dark Jedi whose blade the class's own cog installed
            // (`jkSetSaberInfo`) swings when the target is inside the class line's first range.
            // The handler rearms every 500 ms, rolls `fltArg[0]` for a swing, and picks one of the
            // four `(range, damage, cooldown)` entries; the shipped classes fill the first two. The
            // blade's damage is `sithThing_DamageThing` with `DAMAGE_SABER` (0x10), so it rides the
            // same path a player swing does. What is not modelled: the target-typeflag saber-lock
            // branch, `jkSaber_Enable`'s unrecovered arguments, and the puppet track/sound ids (see
            // the ledger).
            if let Some(saber) = ai.as_ref().and_then(|class| class.instinct("SaberFighting")).cloned() {
                let due = self.ai_saber.get(&index).copied().unwrap_or(self.time);
                if self.time >= due {
                    self.ai_saber.insert(index, self.time + 0.5);
                    let cooling = self.ai_saber_cool.get(&index).copied().unwrap_or(0.0);
                    if sighted && self.time >= cooling && self.saber.contains_key(&index) {
                        let reach = ((target[0] - from[0]).powi(2)
                            + (target[1] - from[1]).powi(2)
                            + (target[2] - from[2]).powi(2))
                        .sqrt();
                        let radius = self
                            .templates
                            .get(player)
                            .and_then(|name| self.template_hit.get(&name.to_ascii_lowercase()))
                            .map(|(radius, _)| *radius)
                            .unwrap_or(0.065);
                        let distance = (reach - radius).max(0.0);
                        if let Some((damage, mode, cooldown)) = saber_attack(
                            &saber.args,
                            distance,
                            crate::cog::pseudo_random(),
                            crate::cog::pseudo_random(),
                        ) {
                            self.damage_typed(player as i64, damage, DAMAGE_SABER);
                            *self.ai_flags.entry(index).or_insert(0) |= 0x400;
                            self.play_mode.insert(index, mode);
                            self.ai_saber_cool
                                .insert(index, self.time + (cooldown / 1000.0).max(0.0));
                            if std::env::var_os("BK_AI_TRACE").is_some() {
                                eprintln!(
                                    "ai: {index} saberfighting {damage:.0} damage at {distance:.2} (cooldown {cooldown:.0} ms)"
                                );
                            }
                        } else {
                            *self.ai_flags.entry(index).or_insert(0) &= !0x400;
                        }
                    }
                }
            }
            if sighted && std::env::var_os("BK_AI_TRACE").is_some() {
                let (sx, sy) = (target[0] - from[0], target[1] - from[1]);
                let range = (sx * sx + sy * sy).sqrt().max(1e-4);
                let (sin, cos) = yaw.to_radians().sin_cos();
                eprintln!(
                    "ai: sees {index} range {range:.2} dot {:.2} ready {ready} yaw {yaw:.1} from {from:?} target {target:?}",
                    (cos * sy - sin * sx) / range
                );
            }
            // `AISetFireTarget`: what this actor shoots at, when a script names it. That replaces the
            // player as the thing it has in sight — `00_AITurnCoat.cog` turns an actor on a victim —
            // so it is computed here, before the out-of-sight early return below.
            let aim = self
                .ai_fire_target
                .get(&index)
                .and_then(|t| self.things.get(*t).map(|t| t.pos))
                .filter(|at| self.solids.as_ref().is_none_or(|s| !s.segment_blocked(from, *at)));
            // The withdraw mode's own bookkeeping: **the mode ends when the remembered target is
            // gone or its deadline passes** (`FUN_0050b150` clears on `ai[0x70] == 0`; `hitandrun`'s
            // break-off has a real deadline). A destroyed thing is a null pointer in the engine: it
            // frees the record.
            if let Some((from, until)) = self.ai_withdraw.get(&index).copied()
                && (self
                    .things
                    .get(from)
                    .is_none_or(|target| target.destroyed)
                    || until.is_some_and(|u| self.time >= u))
            {
                self.ai_withdraw.remove(&index);
            }
            // A scripted flee lasts the class's own `Flee` instinct duration (shipped: 1-2 s); the
            // engine's handler clears the mode at that deadline and wakes the actor again, so the port
            // drops the state when it passes rather than running from a script order forever.
            if self
                .ai_flee_until
                .get(&index)
                .is_some_and(|until| self.time >= *until)
            {
                self.ai_flee.remove(&index);
                self.ai_flee_until.remove(&index);
            }
            let fleeing = self
                .ai_flee
                .get(&index)
                .and_then(|t| self.things.get(*t).map(|t| t.pos))
                .or_else(|| {
                    // Only the **`Withdraw` handler** walks the body away: `Retreat` merely writes the
                    // mode and the target into `ai[0x70]`, so a class that ships `Retreat` alone arms a
                    // withdrawal nobody acts on — exactly as in the engine, where the instinct list is
                    // what schedules the handler.
                    if !ai.as_ref().is_some_and(|c| c.instinct("Withdraw").is_some()) {
                        return None;
                    }
                    self.ai_withdraw
                        .get(&index)
                        .and_then(|(from, _)| self.things.get(*from).map(|t| t.pos))
                });
            // Movement is not decided here: a move order is `sithAI_SetMoveThing` (`0x4eac90`) from a
            // script verb or a ported handler, and `sithAI_Tick`'s move arm (`sith_ai_tick`) pushes
            // the body's velocity through physics in the thing's own update slot.
            let scripted = (self.ai_flags.get(&index).copied().unwrap_or(0) & 0x1 != 0)
                .then(|| self.ai_move.get(&index).copied())
                .flatten();
            let engaged = self.ai_detected.contains(&index);
            let remembered = self.ai_last_seen.contains_key(&index);
            if scripted.is_none() && !(sighted && engaged) && aim.is_none() && !remembered && fleeing.is_none() {
                // Out of sight and nothing remembered: the actor stands where it is, turning to a
                // scripted look target — or, with `randomturn`, to the direction its scheduler slot
                // last picked (`0x50b700`: a random direction every class interval).
                let idle_yaw = look_yaw.or_else(|| self.ai_randomturn.get(&index).copied());
                if let Some(thing) = self.things.get_mut(index) {
                    thing.ai_target = None;
                }
                if let Some(yaw) = idle_yaw {
                    let dir = forward_of_yaw(yaw);
                    self.ai_set_look_frame(index, [from[0] + dir[0], from[1] + dir[1], from[2]]);
                }
                continue;
            }
            // A fleeing actor does not shoot (`sithAI_Flee`'s mode); the run itself belongs to the
            // unported `withdraw` handler (`0x50b150`).
            if fleeing.is_some() {
                continue;
            }
            if let Some(thing) = self.things.get_mut(index) {
                thing.ai_target = scripted;
            }
            // `sithActor_UpdateAimJoints`: a registered target wins the facing; without one a
            // scripted look stands. A bare line of sight is not a target (round 307).
            if sighted && engaged {
                want_yaw = (-(target[0] - from[0])).atan2(target[1] - from[1]).to_degrees();
            } else if let Some(yaw) = look_yaw {
                want_yaw = yaw;
            }
            // The instinct that fires. An actor whose class has no `PrimaryFire` never
            // shoots, however close the player is, and a scripted walk does not shoot at what it
            // cannot see. A scripted fire target replaces the player as what it shoots at.
            //
            // The numbers are the `.ai` file's own (`sithAICmd_PrimaryFire`):
            //   fltArg[0] shot delay in ms      fltArg[1] minDot   fltArg[2] maxDistToFire
            //   fltArg[3] aim error fraction    fltArg[4] minDist  fltArg[8] burst count
            //   intArg[9] ms between burst shots
            // so a `peddefault.ai` gunner and a `rydefault.ai` one fire at different rates, from
            // different ranges and with different accuracy — as the shipped files say.
            // The frame's desired facing becomes the look frame — `sithAI_SetLookFrame`, the one
            // way an engine actor starts turning — and the blend at the top of the next tick eases
            // the body toward it. A look is only set while the body is not already facing there, so
            // the flag `0x8` clears once the blend finishes and stays clear until the desire moves
            // again, the way the engine's event-driven handlers leave it.
            let delta = (want_yaw - yaw).rem_euclid(360.0);
            if delta.min(360.0 - delta) > 0.6 {
                let dir = forward_of_yaw(want_yaw);
                self.ai_set_look_frame(index, [from[0] + dir[0], from[1] + dir[1], from[2]]);
            }
            let mut shooting_at = aim.unwrap_or(target);
            // An engine actor has a *target* only once the detection roll registered it
            // (`sithAI_CheckSightThing` -> `sithAI_CanDetectSightThing`), and `PrimaryFire` fires at
            // that target. Firing on the bare line test made level 1's `reeyeesbhgun` shoot the player
            // inside the first second from a view where retail's own series shows no shot at all
            // (round 307); the shot's awareness event then walked the whole cantina.
            let can_see = if aim.is_some() { true } else { sighted && engaged };
            // `BK_AI_TRACE=2`: name the gate that stops an in-sight actor from firing. The engine's
            // `sithAICmd_PrimaryFire` has exactly three besides readiness — the instinct itself, its
            // range, and its `minDot` facing test — so one line per refusal is enough to tell an AI
            // that is asleep from one that is merely still turning.
            let fire_trace = std::env::var("BK_AI_TRACE").is_ok_and(|v| v == "2");
            // The puppet-deferred shot (`sithAI_FireWeapon`'s `0x20000` branch) leaves with its
            // gates skipped — the stored `fl` carries the engine's own bit 8. The engine fires it
            // when the fire track reaches its marker; the port's stand-in is this next pass,
            // before any readiness or sight gate, like the marker itself.
            if let Some((fl, err, mode, dtarget)) = self.ai_deferred_fire.remove(&index)
                && let Some(dpos) = self.things.get(dtarget).map(|t| t.pos)
            {
                let accuracy = ai.as_ref().map(|c| c.accuracy);
                if !matches!(
                    self.sith_ai_fire_weapon(index, dtarget, dpos, 0.0, f32::MAX, -1.0, err, mode, fl, accuracy),
                    FireOutcome::Refused
                ) {
                    self.last_shot.insert(index, self.time);
                    fired += 1;
                }
            }

            // `sithAICmd_BlindFire` (`0x509b30`): an actor that has lost sight but still remembers where
            // the body was fires at **that** point — the engine tests the remembered spot against the
            // class's cos limit and a fixed 10-unit reach, rolls the instinct's chance, and fires with
            // its spread. The port reuses this same firing path with the remembered position as the
            // target and the `BlindFire` arguments, so an actor that is out of sight is not silent.
            let blind = !can_see
                && ready
                && ai.as_ref().is_some_and(|class| class.instinct("BlindFire").is_some())
                && self.ai_last_seen.contains_key(&index);
            if !ready || (!can_see && !blind) {
                if fire_trace && can_see {
                    eprintln!("ai: {index} ready {ready} not firing yet");
                }
                continue;
            }
            if blind {
                shooting_at = self.ai_last_seen.get(&index).copied().unwrap_or(shooting_at);
            }
            let instinct = if blind {
                "BlindFire"
            } else if ai.as_ref().and_then(|class| class.instinct("PrimaryFire")).is_none()
                && ai.as_ref().is_some_and(|class| class.instinct("TurretFire").is_some())
            {
                "TurretFire"
            } else {
                "PrimaryFire"
            };
            // `turretfire` (`0x50a0f0`): a turret's class carries **no `PrimaryFire`** (the shipped
            // `turdefault.ai` has only `TurretFire`), so a turret that falls back to this path would
            // never fire at all. The handler aims the articulated head and fires the weapon from it;
            // the body never moves. The port fires from the body, with the class's own `sightdist` as
            // the reach, no facing gate and no burst — the ±160-degree head sweep the handler applies
            // is a joint articulation the port cannot reproduce, so that is on the ledger, not hidden.
            let turret_shot = instinct == "TurretFire";
            // A damaged turret's head sweep, applied to the shot's direction below (`FUN_0050a0f0`
            // re-rolls ±arg2/±arg3 on every handler call and fires along that offset).
            let mut turret_sweep: Option<(f32, f32)> = None;
            let Some(primary) = ai.as_ref().and_then(|ai| ai.instinct(instinct)).cloned() else {
                if fire_trace {
                    eprintln!("ai: {index} `{ai_template}` has no {instinct} instinct");
                }
                continue;
            };
            // The parser stores the first number as `weight`; the engine reads the whole list as
            // `fltArg[0..9]`, so `fltArg[k]` is `weight` for k 0 and `args[k - 1]` after that.
            let arg = |k: usize| {
                if k == 0 {
                    primary.weight as f32
                } else {
                    primary.args.get(k - 1).copied().unwrap_or(0.0) as f32
                }
            };
            let (delay_ms, min_dot, max_dist, error, min_dist) = if blind {
                // Every shipped line is `750, 0.80, 0, 0.8, 0.0, 0.30`: delay, chance, unused cos
                // limit, minimum distance, spread. The reach is the engine's fixed 10 units, and
                // `0x509b30` scales the spread by `fltArg[5] / dist` before `RandomFireVector`.
                (arg(0), arg(2), 10.0, arg(5), arg(3))
            } else if turret_shot {
                // `turretfire`'s shipped line is `750, 75, 160, 160, 5.0, 0.20, 500, 10000, 0`: delay,
                // then the aim-head sweep (±arg2 yaw, ±arg3 pitch), the damaged-health fraction
                // (arg5 = 0.20), the damaged cadence (arg6 = 500 ms) and the self-destruct delay
                // (arg7 = 10 s). The port has no articulated head, so a healthy turret fires from the
                // body, uses the class's `sightdist` as the reach and no facing gate (fov 360).
                let max_health = self
                    .template_max_health
                    .get(&ai_template)
                    .copied()
                    .unwrap_or(health)
                    .max(1.0);
                let damaged = arg(5) > 0.0 && health < max_health * arg(5);
                if damaged {
                    // `FUN_0050a0f0`: the first damaged call arms `now + arg7`; once that passes the
                    // handler kills the turret (`sithActor_KillActor(thing, thing, 2)`) and aborts.
                    let kill_s = arg(7) * 0.001;
                    if kill_s > 0.0 {
                        let armed = *self.ai_turret_down.entry(index).or_insert(self.time + kill_s);
                        if self.time >= armed {
                            self.damage_typed_from(index as i64, health.max(1.0) + 1.0, 0, 0);
                            continue;
                        }
                        turret_sweep = Some((arg(2).max(0.0), arg(3).max(0.0)));
                    }
                    (arg(6).max(1.0), 0.0, ai.as_ref().map(|a| a.sightdist).unwrap_or(5.0), 0.0, 0.0)
                } else {
                    (arg(0), 0.0, ai.as_ref().map(|a| a.sightdist).unwrap_or(5.0), 0.0, 0.0)
                }
            } else {
                (arg(0), arg(1), arg(2), arg(3), arg(4))
            };
            let (burst, burst_ms) = if blind || turret_shot {
                (1.0, 0.0)
            } else {
                (arg(8), arg(9))
            };
            // A damaged turret's head sweep, applied to the shot's direction: `FUN_0050a0f0` re-rolls
            // ±arg2 yaw and ±arg3 pitch on every handler call and fires along that offset.
            if let Some((sweep_yaw, sweep_pitch)) = turret_sweep {
                let ry = (crate::cog::pseudo_random() as f32 * 2.0 - 1.0) * sweep_yaw;
                let rp = (crate::cog::pseudo_random() as f32 * 2.0 - 1.0) * sweep_pitch;
                let (dx, dy, dz) = (
                    shooting_at[0] - from[0],
                    shooting_at[1] - from[1],
                    shooting_at[2] - from[2],
                );
                let bearing = dy.atan2(dx) + ry.to_radians();
                let pitch = dz.atan2((dx * dx + dy * dy).sqrt()) + rp.to_radians();
                let reach = (dx * dx + dy * dy + dz * dz).sqrt().max(1e-3);
                shooting_at = [
                    from[0] + bearing.cos() * pitch.cos() * reach,
                    from[1] + bearing.sin() * pitch.cos() * reach,
                    from[2] + pitch.sin() * reach,
                ];
            }
            // `BlindFire`'s own cadence: the handler rolls the chance **once per call**, and a failed
            // roll sets its next call to now + **1000 ms** (a success schedules its own interval,
            // `class+0x4c` = 750 ms shipped). The port re-rolled at the shot slot's cadence, which
            // rolls more often than the engine does; the wait is written here so the retry is the
            // engine's.
            // A chance of 0 never fires (the engine's `chance < rand` test), so the roll is not
            // skipped for it; a chance of 1 always does.
            if blind && crate::cog::pseudo_random() > arg(1) as f64 {
                if let Some(thing) = self.things.get_mut(index) {
                    thing.ai_wait = 1.0;
                }
                continue;
            }
            let (sx, sy) = (shooting_at[0] - from[0], shooting_at[1] - from[1]);
            // A script that names a fire target is telling the actor to shoot *that*, so it turns to
            // it first — `00_AITurnCoat.cog` turns an actor on a victim behind it. The engine's
            // `sithAICmd_PrimaryFire` sets a **look frame**, so the body eases onto the victim
            // instead of snapping; the facing gate below still reads the pre-turn yaw.
            if aim.is_some() {
                self.ai_set_look_frame(index, shooting_at);
            }
            // `sithAICmd_PrimaryFire` (`0x509e40`) rolls `fltArg[6]` for the lead flag and
            // `fltArg[7]` for the second weapon, in that order and each only when its argument is
            // non-zero, before `sithAI_FireWeapon` runs (which draws the accuracy gate next).
            // The range, facing and visibility gates themselves are `sithAI_FireWeapon`'s own.
            let mut fl = 0u32;
            let mut mode = 0u32;
            if !blind && !turret_shot {
                if arg(6) != 0.0 && crate::cog::pseudo_random() <= arg(6) as f64 {
                    fl |= 1;
                }
                if arg(7) != 0.0 && crate::cog::pseudo_random() <= arg(7) as f64 {
                    mode = 1;
                }
            }
            // `BK_AI_TRACE=2`: when the shot actually leaves, so an acquisition delay can be measured
            // against a level's spawn (the ledger's spawn-combat row).
            if fire_trace {
                eprintln!(
                    "ai: {index} fires {instinct} at ({sx:.2},{sy:.2}) fl {fl} mode {mode} (time {:.2} s)",
                    self.time
                );
            }
            // What the shot is aimed at: the scripted `AISetFireTarget` thing when one is named,
            // otherwise the player. That is the engine's own target, not the aim point.
            let target_thing = self.ai_fire_target.get(&index).copied().unwrap_or(player);
            // `BlindFire`'s inline path (`0x509b30`) scales `fltArg[5]` by the remembered
            // distance before `RandomFireVector`, and has no accuracy gate of its own.
            let err = if blind {
                let dz = shooting_at[2] - from[2];
                let len = (sx * sx + sy * sy + dz * dz).sqrt();
                if len > 1e-6 {
                    error / len
                } else {
                    0.0
                }
            } else {
                error
            };
            let accuracy = if blind { None } else { ai.as_ref().map(|c| c.accuracy) };
            // The engine's handler fires `sithAI_FireWeapon` synchronously and schedules its
            // cadence only on a fired (or deferred) shot; a refusal retries in 250 ms (`0xfa`).
            let outcome = self.sith_ai_fire_weapon(
                index,
                target_thing,
                shooting_at,
                min_dist,
                max_dist,
                min_dot,
                err,
                mode,
                fl,
                accuracy,
            );
            let committed = !matches!(outcome, FireOutcome::Refused);
            if committed {
                if let FireOutcome::Shot(shot) = outcome {
                    shooters.push(index);
                    // `sithCog_ThingSendMessage(shooter, projectile, 0x24)` — the `fired`
                    // message, delivered with the new thing next frame.
                    self.send_thing_message(index, "fired", [shot as f64, 0.0]);
                }
                fired += 1;
                // `sithAICmd_PrimaryFire` (`0x509e40`) sets the AI mode bit `0x400` when
                // `FireWeapon` returns fired — the bit `HitAndRun` (`0x50bb90`) counts on.
                *self.ai_flags.entry(index).or_insert(0) |= 0x400;
                // The renderer plays a puppet's `fire` track off `last_shot`: the engine's
                // `sithPuppet_UpdateThingMove` picks the action track while the actor shoots.
                self.last_shot.insert(index, self.time);
                // #92: the engine's fire path sets a **look frame** (`sithAI_SetLookFrame`), never
                // the yaw — the body blends onto the target while the bolt flies at the AI's aim.
                self.ai_set_look_frame(index, shooting_at);
            } else if let Some(thing) = self.things.get_mut(index) {
                thing.ai_wait = 0.25;
            }
            if committed && let Some(thing) = self.things.get_mut(index) {
                // `sithActor_UpdateAimJoints`: the aim points at the target in **three** dimensions.
                // Without the elevation an actor shoots level at a player standing on a ledge and the
                // bolt passes underneath — the bug the player had until round 109.
                let dz = shooting_at[2] - from[2];
                let flat = (sx * sx + sy * sy).sqrt().max(1e-4);
                let elevation = dz.atan2(flat).to_degrees();
                thing.pitch = elevation.clamp(-60.0, 60.0);
                // `sithAI_SetLookFrame`: the head pitch is clamped by the actor's own
                // `minheadpitch`/`maxheadpitch` (degrees in the shipped templates).
                let min = self
                    .templates
                    .get(index)
                    .and_then(|name| self.template_minheadpitch.get(name))
                    .copied()
                    .unwrap_or(-80.0);
                let max = self
                    .templates
                    .get(index)
                    .and_then(|name| self.template_maxheadpitch.get(name))
                    .copied()
                    .unwrap_or(80.0);
                thing.head_pitch = elevation.clamp(min, max);
                // `sithAICmd_PrimaryFire`: a burst of `fltArg[8]` shots `intArg[9]` apart, then the
                // shot delay jittered by ±20% (`(1.2 - 0.2 * frand) * fltArg[0]`).
                if thing.ai_burst > 0.0 {
                    thing.ai_burst -= 1.0;
                    thing.ai_wait = burst_ms / 1000.0;
                } else {
                    thing.ai_burst = (burst - 1.0).max(0.0);
                    // `sithAICmd_PrimaryFire` jitters the shot delay by ±20% (the ledger's figure).
                    // The asymmetric `1.2 - 0.2 * frand` averaged 1.1 and put the door gunner's
                    // cadence at ~825 ms, where retail's fixture measures ~750 ms.
                    let jitter = 1.0 - 0.2 + 0.4 * crate::cog::pseudo_random() as f32;
                    thing.ai_wait = delay_ms / 1000.0 * jitter;
                }
            }
        }

        // `sithAIAwareness_CreateTransmittingEvent` (`0x4f2a90`) + `ProcessEvent` (`0x4f2c30`):
        // firing raises an event at the shooter with strength 4.0, and the engine's propagation
        // carries it through the adjoin graph for the shooter's own side. The record is the
        // shooter's position, which is what makes reinforcements converge on gunfire.
        for shooter in shooters {
            let Some(thing) = self.things.get(shooter) else { continue };
            let (sector, pos) = (thing.sector, thing.pos);
            // `sithWeapon_WeaponFire` (`0x4d44f0`) and `FireProjectile` (`0x4d5f20`) raise kind **1**
            // at strength **2.0** (`0x40800000`) with the weapon/thing as the source. Every shooter
            // raises it — the player has no `.ai` class, and requiring one meant no actor ever heard
            // the player's gunfire.
            self.transmit_awareness(sector, 1, pos, AI_AWARE_SHOT, Some(shooter));
            // `sithWeapon_Update` (`0x4d3530`) raises kind **2** at strength **2.0**
            // (`0x40000000`) while the weapon's firing flag (`0x40000`) is up, every eighth engine
            // tick. The port's shots stand in for that stream — the record is strength-keyed, so the
            // engine's decimation would only repeat the same value.
            self.transmit_awareness(sector, 2, pos, AI_AWARE_WEAPON, Some(shooter));
        }
        fired
    }

    /// `sithTime_Advance` (`0x4dd640`): advance once before movement, not per move sub-step.
    /// Keep the fractional remainder of a pinned port frame; the engine clock reads whole ms.
    pub fn advance_game_clock(&mut self, dt: f32) {
        self.game_clock += std::time::Duration::from_secs_f32(dt.clamp(0.01, 0.5));
    }

    /// Advance the player body from one frame's input through attached/unattached physics and
    /// `MoveThing`, releasing the runtime for synchronous contact scripts. Seating, scrolling carry,
    /// thrust, the water state, run/walk scaling, drag, the fixed sub-step count, target assembly, jump,
    /// velocity feedback, fall damage and fall death live here; the caller supplies only the intent.
    ///
    /// The final pose is published onto the local player's thing, so camera, HUD, scripts and
    /// projectiles read one body. `teleport_player` and `respawn_player` are the only other writers.
    #[allow(clippy::too_many_arguments)]
    pub fn step_player(
        world: &World,
        body: &mut PlayerBody,
        solids: &crate::collide::Solids,
        input: PlayerInput,
        rules: &crate::Rules,
        dt: f32,
        dispatch: impl FnMut(),
    ) -> bool {
        // A script can move the body: `force_jump.cog`'s `SetThingVel(player, ...)` lands on the
        // player thing, and the walker consumes it as a one-shot impulse.
        {
            let rt = world.borrow();
            if let Some(index) = rt.local_player() {
                let vel = rt.things[index].vel;
                if vel.map(f32::to_bits) != body.published_vel.map(f32::to_bits) {
                    body.vel = [vel[0], vel[1]];
                    body.vy = vel[2];
                }
                if let Some((published, sector)) = body.published_pose {
                    let thing = &rt.things[index];
                    let mut pos = body.pos.to_array();
                    for axis in 0..3 {
                        if thing.pos[axis].to_bits() != published[axis].to_bits() {
                            pos[axis] = if axis == 2 { thing.pos[axis] - body.eye } else { thing.pos[axis] };
                            body.attach = None;
                        }
                    }
                    body.pos = Vec3::from(pos);
                    if body.attach.is_some() {
                        body.attach = thing.floor_attach;
                    }
                    if thing.sector != sector as i32 && let Ok(sector) = usize::try_from(thing.sector) {
                        body.sector = sector;
                    }
                }
            }
        }
        world.borrow_mut().publish_player(body);
        let radius = rules.radius;
        // No seat before the move. `sithCommand_Warp` (`0x4eea40`) sets the position and leaves it, and
        // `sithPhysics_FindFloor` (`0x4f5550`) only ever lands a body through the move's sweep. The
        // port's old seat compared the feet with a floor plane's perpendicular foot, which on a sloped
        // face (`06abarons`' pool bowl, surface 342) lifted a sphere resting on the slope out of its
        // sector, and every later move then failed `IsSphereInSector` and was undone (`walk5_spawn`).
        let [along, across] = rules.speed(
            input.forward,
            input.strafe,
            input.multiplier,
            input.crouch,
            body.water,
        );
        // A force-speed term adds to `maxThrust` in the engine, so it scales both axes.
        let force = if input.extra_speed > 0.0 {
            rules.extra_walk(input.extra_speed) / rules.walk
        } else {
            1.0
        };
        let target = walk_target(along * force, across * force, input.yaw);
        let drag = if body.grounded { rules.drag } else { rules.air_drag };
        let steps = integrate_walk(&mut body.vel, target, drag, dt, &mut body.step_rollover);
        let height = {
            let rt = world.borrow();
            rt.thing_height(rt.local_player(), radius)
        };
        let thrusting = along != 0.0 || across != 0.0;
        // `SetSectorThrust` is half of a conveyor: `sithPhysics.c` adds `sector->thrust * dt` to a
        // body's velocity exactly like gravity, so an updraft column lifts what stands in it —
        // level 21's core tube is `'0.0 0.0 1.0'` at 4 to 5, level 10's power core pushes the body out.
        let thrust = world.borrow()
            .sectors
            .get(body.sector)
            .map(|s| s.thrust)
            .unwrap_or([0.0; 3]);
        if thrust != [0.0; 3] && std::env::var_os("BK_SCROLL_TRACE").is_some() {
            println!("thrust: sector {} accelerates the body at {thrust:?} units/s^2", body.sector);
        }
        let mut state = world.borrow().player_move_state(body, body.sector, body.pos.to_array(),
            [body.vel[0], body.vel[1], body.vy]);
        let dispatch = std::cell::RefCell::new(dispatch);
        // `MoveThing` pushes riders with search flag 0x40 (0x4e7950:61-77). The runtime has
        // already advanced the parent's path, so move the rider by its changed anchor now.
        if let Some(previous) = state.attach.filter(|a| a.thing.is_some()) {
            let anchor = body.floor_anchor.filter(|(index, _)| Some(*index) == previous.thing.map(|t| t.thing))
                .map(|(_, anchor)| anchor);
            let ride = world.borrow().floor_rider_motion(solids, previous, state.pos, anchor);
            if let Some((attach, carry)) = ride {
                state.attach = Some(attach);
                let carry = if state.attach_flags & 8 == 0 { carry } else { [0.0; 3] };
                solids.move_thing_with(
                    &mut state, carry, radius, 0x40,
                    |state, dir, distance, radius, flags, search| {
                        solids.search_for_collisions(state.sector, Some(state), state.pos, dir,
                            distance, radius, world.borrow().gates(), flags, search);
                    },
                    |state, surface, far| Self::player_crossing(world, state, body.eye, surface, far, &mut *dispatch.borrow_mut()),
                    |state| solids.inside_sector_with_gates(state.sector, state.pos, 0.0, world.borrow().gates()),
                    |_| {},
                    |state, item| Self::sith_item_player_touch(world, solids, state, body.eye, item, &mut *dispatch.borrow_mut()),
                    |state, hit, crease| {
                        let mut dispatch = dispatch.borrow_mut();
                        let mover = world.borrow().local_player();
                        let mut rt = ContactRuntime::Player { world, eye: body.eye, dispatch: &mut *dispatch };
                        Self::solid_contact(solids, &mut rt, mover, state, hit, crease)
                    },
                );
            } else {
                Self::player_detach_floor(world, &mut state, body.eye, &mut *dispatch.borrow_mut());
            }
        }
        // A jump inherits the surface or parent's velocity on detach (0x4ce380).
        if input.jump && body.grounded {
            state.vel[2] = rules.jump_velocity(input.crouch);
            Self::player_detach_floor(world, &mut state, body.eye, &mut *dispatch.borrow_mut());
            body.grounded = false;
        }
        let gravity = body.gravity;
        state.vel[0] += thrust[0] * dt;
        state.vel[1] += thrust[1] * dt;
        state.vel[2] += thrust[2] * dt;
        let mut pos = state.pos;
        let mut achieved_len: f32 = 0.0;
        let mut sector = state.sector;
        let mut vy = state.vel[2];
        let mut landed = false;
        let mut crossed = false;
        let mut step_vel = state.vel;
        let wet = |s: usize| world.borrow().sector_flags.get(s).is_some_and(|f| f & crate::collide::SECTOR_UNDERWATER != 0);
        let mut surface = body.water_surface;
        for _ in 0..steps {
            const SUB: f32 = 1.0 / 50.0;
            // Each sub-step moves by the projected velocity, the way the engine re-copies the
            // accumulated `0x234` into the move: a deflection inside one sub-step steers the
            // next, instead of every sub-step re-pushing into the wall at the frame's input.
            let was_wet = wet(sector);
            // `sithPhysics_UpdateUnderwaterThingPhysics` (`0x4f6d80`), vertical only: gravity is scaled
            // by `buoyancy` (`thing+0x224`), which `walkplayer` never sets and `sithThing_Reset`
            // (`0x4cd190`) zeroes, so it is no force at all. The velocity takes `airdrag * 4`
            // (`0x5231b4`) through `ApplyDrag` (`0x4f5d50`, factor capped at 1, `0x523194`). With
            // physflag `0x100000` and no upward thrust of the body's own (the walker has no swim keys),
            // a rise slower than `dt * 0.2` (`0x5231bc`) is dropped and the body is lifted toward
            // 0.01 (`0x5231b8`) under the surface by at most `dt * 0.2`.
            // shortcut: the horizontal half (drag, `thrust * 0.6`) stays the walker's halved speed.
            // `sithPhysics_UpdateThing`'s routing (`0x4f5900:22-29`) tests the attach first: an
            // attached body keeps the floor hold even under water (it wades), and only an
            // unattached one takes the underwater model.
            let held = (was_wet && state.attach.is_none()).then(|| {
                vy -= vy * (SUB * rules.air_drag * 4.0).min(1.0);
                if vy.abs() <= 1e-5 {
                    vy = 0.0;
                }
                let mut rise = vy * SUB;
                if let Some(depth) = surface {
                    let cap = SUB * 0.2;
                    if rise > 0.0 && rise < cap {
                        rise = 0.0;
                    }
                    let below = depth - 0.01;
                    if below > 0.0 {
                        rise += below.min(cap);
                    }
                }
                let held = vy;
                vy = rise / SUB;
                step_vel[2] = vy;
                held
            });
            state.vel = step_vel;
            state.water_surface = surface;
            let submerged = was_wet && state.attach.is_none();
            let displacement = crate::collide::Solids::update_thing_physics(
                &mut state, SUB, radius, height, if submerged { 0.0 } else { gravity },
            );
            let prepared_vz = state.vel[2];
            let start = state.pos;
            let previous = state.sector;
            let action = world.borrow().update_move(&state, displacement, SUB);
            let mut find_floor = action == crate::collide::UpdateMove::FindFloor;
            let (delta, move_flags) = match action {
                crate::collide::UpdateMove::Move { delta, flags } => (delta, flags),
                _ => ([0.0; 3], 0),
            };
            let traveled = solids.move_thing_with(
                &mut state, delta, radius, move_flags,
                |state, dir, distance, radius, flags, search| {
                    let rt = world.borrow();
                    if let Some(index) = rt.local_player() { search.exclude_thing(index); }
                    solids.search_for_collisions(
                        state.sector, Some(state), state.pos, dir, distance, radius,
                        Gates { sectors: &rt.sector_flags, adjoins: &rt.adjoin_flags },
                        flags, search,
                    );
                },
                |state, surface, far| {
                    Self::player_crossing(world, state, body.eye, surface, far, &mut *dispatch.borrow_mut());
                },
                |state| {
                    let rt = world.borrow();
                    solids.inside_sector_with_gates(
                        state.sector, state.pos, 0.0,
                        Gates { sectors: &rt.sector_flags, adjoins: &rt.adjoin_flags },
                    )
                },
                |_| { find_floor = true; },
                |state, item| {
                    Self::sith_item_player_touch(world, solids, state, body.eye, item, &mut *dispatch.borrow_mut());
                },
                |state, hit, crease| {
                    let mut dispatch = dispatch.borrow_mut();
                    let mover = world.borrow().local_player();
                    let mut rt = ContactRuntime::Player { world, eye: body.eye, dispatch: &mut *dispatch };
                    Self::solid_contact(solids, &mut rt, mover, state, hit, crease)
                },
            );
            let step_landed = find_floor
                && Self::player_find_floor(world, solids, &mut state, radius, height, body.eye, &mut *dispatch.borrow_mut());
            if std::env::var_os("BK_STEP_TRACE").is_some() {
                let kind = match &state.attach {
                    Some(a) => if a.thing.is_some() { "thing" } else { "surface" },
                    None => "none",
                };
                eprintln!(
                    "step: z {:.4} vz {:.4} traveled {:.4} attach {kind} landed {step_landed} wet {was_wet}",
                    state.pos[2], state.vel[2], traveled
                );
            }
            let len = ((state.pos[0] - start[0]).powi(2) + (state.pos[1] - start[1]).powi(2)).sqrt();
            let cross = state.sector != previous;
            pos = state.pos;
            sector = state.sector;
            step_vel = state.vel;
            body.attach = state.attach;
            vy = step_vel[2];
            if let Some(held) = held {
                vy = if step_landed { 0.0 } else if vy.to_bits() == prepared_vz.to_bits() { held } else { vy };
            }
            // `sithThing_EnterWater` (`0x4cd370`) damps velocity synchronously in `player_crossing`,
            // before the entered handler sees it; do not damp it again from sub-step endpoints.
            step_vel[2] = vy;
            // `sithCollision_MoveThing` (`0x4e7950`) ends a physics body's move with `FindFloor`
            // (`0x4f5550`); under water that is the water branch, which sets or clears the
            // surface hold inside the tail callback above (`thing+0x48` keeps its last answer
            // between moves).
            surface = state.water_surface;
            achieved_len = achieved_len.max(len);
            crossed |= cross;
            landed |= step_landed;
        }
        body.water_surface = surface;
        // Surface carry joins the displacement, not the relative physics velocity (0x4cc4c0:33-38).
        body.vel = [step_vel[0] - thrust[0] * dt, step_vel[1] - thrust[1] * dt];
        vy = step_vel[2];
        body.vy = vy;
        body.sector = sector;
        body.pos = Vec3::from(pos);
        body.attach = state.attach;
        body.floor_anchor = body.attach.and_then(|a| a.thing).and_then(|parent| {
            world.borrow().floor_local_anchor(parent.thing, state.pos).map(|anchor| (parent.thing, anchor))
        });
        body.last_move = achieved_len;
        body.grounded = body.attach.is_some() || landed || body.vy == 0.0;
        // The engine's fall death (`sithPlayer.c:251`).
        if !body.fall_killed
            && crate::fall_death_rule(
                -body.vy / crate::physics_scale().max(1.0).sqrt(),
                world.borrow().sector_flags.get(body.sector).copied().unwrap_or(0),
            )
        {
            body.fall_killed = true;
            body.fall_fade = 1.0;
            println!("fall death: the player fell out of the level; the palette starts to fade");
        }
        if body.fall_killed {
            body.fall_fade -= dt * 0.7;
            if body.fall_fade <= 0.0 {
                body.fall_killed = false;
                body.fall_fade = 1.0;
                let mut rt = world.borrow_mut();
                // Self-inflicted, like `CollideHurt`'s `DamageThing(thing, thing, ..)`; 0 would name thing 0.
                let source = rt.local_player().map_or(-1, |index| index as i64);
                rt.hurt_player_fall(1000.0, source);
            }
        }
        // Water halves movement: the engine reads the surface under the feet (`SITH_SURFACE_WATER`
        // 0x20000 / `VERYDEEPWATER` 0x100000) and the sector's own `SITH_SECTOR_UNDERWATER` — the
        // bit `SetSectorFlags(sector, 2)` sets for the shipped "passthru layer" water. It is read
        // after the move, so the next step's speed is the room the body is actually in.
        let feet = [body.pos.x, body.pos.y, body.pos.z - radius + 0.01];
        {
            let rt = world.borrow();
            body.water = rt.in_water(solids, body.sector, feet, &rt.sector_flags);
        }
        if std::env::var_os("BK_WALK_TRACE").is_some() {
            eprintln!(
                "walk trace: water {} sector {} (gate flags {:#x})",
                body.water,
                body.sector,
                world.borrow().sector_flags.get(body.sector).copied().unwrap_or(0),
            );
        }
        {
            let mut rt = world.borrow_mut();
            if let Some(index) = rt.local_player() {
                let thing = &mut rt.things[index];
                body.yaw = thing.yaw.to_radians();
                body.pitch = thing.pitch.to_radians();
                thing.vel = [body.vel[0], body.vel[1], body.vy];
                body.published_vel = thing.vel;
                thing.flags = state.flags;
                thing.phys_flags = state.phys_flags;
                thing.floor_attach = state.attach;
                thing.attach_flags = state.attach_flags;
            }
            rt.publish_player(body);
            body.published_pose = rt.local_player().map(|i| (rt.things[i].pos, body.sector));
        }
        crossed
    }

    fn player_move_state(
        &self, body: &PlayerBody, sector: usize, pos: [f32; 3], vel: [f32; 3],
    ) -> crate::collide::MoveState {
        let player = self.local_player();
        let thing = player.and_then(|i| self.things.get(i));
        let name = player.and_then(|i| self.templates.get(i));
        crate::collide::MoveState {
            pos, sector, vel, remainder: [0.0; 3], attach: body.attach,
            water_surface: body.water_surface,
            attach_flags: body.attach.map_or(0, |a| {
                thing.map_or(0, |t| t.attach_flags & 8) | if a.surface.is_some() { 1 } else { 2 }
            }),
            kind: player.map_or(10, |i| self.types.get(i).copied().unwrap_or(0) as u32),
            move_type: name.map_or(1, |n| match self.template_move.get(n).map(String::as_str) {
                Some("physics") => 1, Some("path") => 2, _ => 0,
            }),
            collide: name.map_or(1, |n| self.template_collide.get(n).copied().unwrap_or(0)),
            phys_flags: thing.map_or(0x4a4f, |t| t.phys_flags),
            flags: thing.map_or(0, |t| t.flags),
            owner: None,
        }
    }

    /// The generic gravity tick's `sithPhysics_UpdateThing` -> `sithCollision_MoveThing`.
    fn move_gravity_body(
        &mut self, solids: &crate::collide::Solids, index: usize, dt: f32,
        radius: f32, thrust: &[[f32; 3]],
    ) -> Option<(crate::collide::MoveState, f32, bool, Option<[f32; 3]>)> {
        let t = &self.things[index];
        let kind = self.types.get(index).copied()? as u32;
        let name = self.templates.get(index)?;
        if !matches!(kind, 2..=4) || t.phys_flags & 1 == 0 || t.destroyed || t.attach.is_some()
            || self.mass.get(&index).copied().unwrap_or(0.0) == 0.0
            || self.template_move.get(name).is_some_and(|m| m != "physics")
        {
            return None;
        }
        let sector = usize::try_from(t.sector).ok().filter(|s| *s < solids.sectors.len())
            .or_else(|| solids.sector_at(t.pos, 0.1))?;
        let start = t.pos;
        let height = self.thing_height(Some(index), radius);
        let gravity = if t.phys_flags & 0x40000 != 0 { self.gravity * 0.5 } else { self.gravity };
        let (surf_drag, static_drag) = self.template_drag.get(name).copied().unwrap_or((0.0, 0.0));
        let mut state = crate::collide::MoveState {
            pos: start, sector, vel: t.vel, remainder: [0.0; 3], phys_flags: t.phys_flags,
            kind, move_type: 1, collide: self.template_collide.get(name).copied().unwrap_or(1),
            flags: t.flags, attach: t.floor_attach, attach_flags: t.attach_flags, owner: t.owner,
            water_surface: None,
        };
        // `sithPhysics_UpdateThingPhysics` (`0x4f6270`): the sector drive joins velocity before
        // the move, except under physflag 0x400000.
        if t.phys_flags & 0x400000 == 0 && let Some(drive) = thrust.get(sector) {
            for (v, drive) in state.vel.iter_mut().zip(drive) { *v += drive * dt; }
        }
        let previous = state.attach;
        let mut carry = [0.0; 3];
        if let Some(attach) = previous.filter(|a| a.thing.is_some()) {
            if let Some((attach, displacement)) = self.floor_rider_motion(solids, attach, start, Some(t.attach_offset)) {
                state.attach = Some(attach);
                if state.attach_flags & 8 == 0 { carry = displacement; }
            } else {
                state.attach = None;
                state.attach_flags = 0;
                self.floor_transition(&mut state, previous, Some(index), false);
            }
        }
        if carry != [0.0; 3] {
            self.move_gravity_body_sweep(solids, index, &mut state, carry, radius, height, 0x40);
        }
        let thrust_sector = thrust.get(sector).is_some_and(|drive| *drive != [0.0; 3]);
        crate::collide::Solids::apply_attached_drag(&mut state, surf_drag, static_drag, thrust_sector, dt);
        let displacement = crate::collide::Solids::update_thing_physics(&mut state, dt, radius, height, gravity);
        match self.update_move(&state, displacement, dt) {
            crate::collide::UpdateMove::FindFloor => self.update_floor_attach(solids, index, &mut state, radius, height),
            crate::collide::UpdateMove::Move { delta, flags } => {
                self.move_gravity_body_sweep(solids, index, &mut state, delta, radius, height, flags);
            }
            crate::collide::UpdateMove::Hold => {}
        }
        if std::env::var_os("BK_SEAT_TRACE").is_some() {
            eprintln!("seat: thing {index} z {:.4} vz {:.4} attach {}", state.pos[2], state.vel[2], state.attach.is_some());
        }
        let flat = (state.pos[0] - start[0]).hypot(state.pos[1] - start[1]);
        let crossed = state.sector != sector;
        let anchor = state.attach.and_then(|a| a.thing)
            .and_then(|target| self.floor_local_anchor(target.thing, state.pos));
        Some((state, flat, crossed, anchor))
    }

    fn move_gravity_body_sweep(
        &mut self, solids: &crate::collide::Solids, index: usize, state: &mut crate::collide::MoveState,
        delta: [f32; 3], radius: f32, height: f32, flags: u32,
    ) {
        let rt = std::cell::RefCell::new(self);
        solids.move_thing_with(
            state, delta, radius, flags,
            |state, dir, distance, radius, flags, search| {
                search.exclude_thing(index);
                solids.search_for_collisions(
                    state.sector, Some(state), state.pos, dir, distance, radius, rt.borrow().gates(), flags, search,
                );
            },
            |state, _, far| {
                if state.sector != far && rt.borrow().sector_flags.get(far).is_some_and(|f| f & crate::collide::SECTOR_UNDERWATER != 0) {
                    state.vel[2] *= 0.25; // `sithThing_EnterWater` (0x4cd370, 0x522e8c).
                }
                state.sector = far;
            },
            |state| solids.inside_sector_with_gates(state.sector, state.pos, 0.0, rt.borrow().gates()),
            |state| rt.borrow_mut().update_floor_attach(solids, index, state, radius, height),
            |_, _| {},
            |state, hit, crease| {
                // Specialized weapon surface/thing callbacks retain their earlier scope.
                if state.kind == 3 { return solids.probe_contact(state, hit, crease); }
                Self::solid_contact(solids, &mut ContactRuntime::Tick(&mut rt.borrow_mut()), Some(index), state, hit, crease)
            },
        );
    }

    /// `sithCollision_MoveThing`'s closing `sithPhysics_FindFloor(thing, 0)` (`0x4e7950:279-294`)
    /// for a non-player body, then the attach change it causes.
    fn update_floor_attach(
        &mut self, solids: &crate::collide::Solids, index: usize, state: &mut crate::collide::MoveState,
        radius: f32, height: f32,
    ) {
        let previous = state.attach;
        solids.find_floor_for(state, radius, height, self.gates(), false, Some(index), self.thing_up(index));
        self.floor_transition(state, previous, Some(index), false);
    }

    fn solid_contact(
        solids: &crate::collide::Solids, rt: &mut ContactRuntime<'_>, mover: Option<usize>,
        state: &mut crate::collide::MoveState, hit: crate::collide::Sweep, crease: &mut Option<[f32; 3]>,
    ) -> bool {
        if let Some(target) = hit.thing {
            // Weapon pairs have dedicated handlers; preserve the generic gravity sweep's stop.
            if state.kind == 3 || rt.with(|rt| rt.types.get(target.thing) == Some(&3)) {
                return solids.probe_contact(state, hit, crease);
            }
            Self::thing_collision_handler(rt, mover, state, hit, target.thing, crease)
        } else {
            Self::handle_thing_hit_surface(rt, mover, state, hit, crease)
        }
    }

    /// `sithCollision_HandleThingHitSurface` (`0x4e8b40`): respond first, then `touched`, then
    /// the impact sound. Suppress the inline charge only when the surface's 0x80 bit is set.
    fn handle_thing_hit_surface(
        rt: &mut ContactRuntime<'_>, mover: Option<usize>, state: &mut crate::collide::MoveState,
        hit: crate::collide::Sweep, crease: &mut Option<[f32; 3]>,
    ) -> bool {
        if state.move_type != 1 { return false; }
        let [x, y, z] = hit.normal;
        let speed = -(state.vel[0] * x + state.vel[2] * z + state.vel[1] * y);
        let flags = rt.with(|rt| rt.surface_flags.get(hit.surface).copied().unwrap_or(0));
        if !crate::collide::Solids::collide_hurt(state, hit.normal, hit.distance, crease, flags & 0x80 != 0, |state, damage| {
            Self::charge_contact(rt, mover, state, damage);
        }) {
            return false;
        }
        if flags & SITH_SURFACE_COG_LINKED != 0 && state.flags & 0x100 == 0 {
            let sent = rt.with(|rt| {
                let now = (rt.time * 1000.0) as u32;
                let last = rt.surface_touched_ms.entry(hit.surface).or_default();
                if last.wrapping_add(500) <= now {
                    *last = now;
                    rt.sith_cog_surface_send_message(hit.surface, mover, "touched") != 0
                } else {
                    false
                }
            });
            if sent { rt.dispatch(state); }
        }
        let metal = rt.with(|rt| rt.surface_flags.get(hit.surface).is_some_and(|f| f & 0x10000 != 0));
        Self::contact_impact_sound(rt, mover, state, speed, metal);
        true
    }

    /// `sithCollision_ThingCollisionHandler` (`0x4e8c50`), in mover/hit order. The pair table's
    /// swapped entries swap the callback arguments back before running this response.
    fn thing_collision_handler(
        rt: &mut ContactRuntime<'_>, mover: Option<usize>, state: &mut crate::collide::MoveState,
        hit: crate::collide::Sweep, target: usize, crease: &mut Option<[f32; 3]>,
    ) -> bool {
        if let Some(mover) = mover && state.flags & 0x100 == 0 {
            if state.flags & 0x400 != 0 {
                let sent = rt.with(|rt| rt.sith_cog_thing_send_message(mover, Some(target), "touched"));
                if sent != 0 { rt.dispatch(state); }
            }
            let sent = rt.with(|rt| {
                if rt.things.get(target).is_some_and(|t| t.flags & 0x400 != 0) {
                    rt.sith_cog_thing_send_message(target, Some(mover), "touched")
                } else { 0 }
            });
            if sent != 0 { rt.dispatch(state); }
        }

        let (mover_mass, target_mass, target_vel, target_phys, target_flags, target_kind) = rt.with(|rt| {
            let mover_mass = mover.map_or(1.0, |i| rt.mass.get(&i).copied().unwrap_or(0.0));
            let physics = rt.templates.get(target)
                .is_some_and(|n| rt.template_move.get(n).is_some_and(|m| m == "physics"));
            let target_mass = if physics { rt.mass.get(&target).copied().unwrap_or(0.0) } else { 0.0 };
            let (vel, phys, flags) = rt.things.get(target).map_or(([0.0; 3], 0, 0), |t| (t.vel, t.phys_flags, t.flags));
            (mover_mass, target_mass, vel, phys, flags, rt.types.get(target).copied())
        });
        // Path movers still use the separate path update, not this MoveThing sweep.
        if state.move_type != 1 || mover_mass == 0.0 { return true; }
        let [x, y, z] = hit.normal;
        let speed = -(state.vel[0] * x + state.vel[1] * y + state.vel[2] * z);
        if target_mass != 0.0 {
            let mut relative = speed + target_vel[0] * x + target_vel[1] * y + target_vel[2] * z;
            if relative.abs() <= 1e-5 || relative <= 0.0 { return false; }
            // `0x523024` scales once for each body without the bounce flag.
            if state.phys_flags & 0x20 == 0 { relative *= 0.5; }
            if target_phys & 0x20 == 0 { relative *= 0.5; }
            let product = mover_mass * target_mass;
            let ratio = (product + product) / (mover_mass + target_mass);
            let force = hit.normal.map(|n| relative * n * ratio);
            rt.with(|rt| {
                Self::sith_physics_apply_force(mover_mass, force, &mut state.vel, &mut state.phys_flags, |vel| {
                    let parent = state.attach.and_then(|a| a.thing).map(|hit| hit.thing)
                        .or_else(|| mover.and_then(|i| rt.things[i].attach));
                    rt.detach_support(mover, state.attach.and_then(|a| a.surface), parent, state.flags, Some(vel));
                    state.attach = None;
                    state.attach_flags = 0;
                    if let Some(mover) = mover {
                        rt.things[mover].attach = None;
                        rt.things[mover].floor_attach = None;
                        rt.things[mover].attach_flags = 0;
                    }
                });
                // The force acts on the live target before CollideHurt reads the mover's velocity.
                let mut vel = target_vel;
                let mut phys = target_phys;
                Self::sith_physics_apply_force(target_mass, force.map(|f| -f), &mut vel, &mut phys, |vel| {
                    let t = &rt.things[target];
                    let surface = t.floor_attach.and_then(|a| a.surface);
                    let parent = t.floor_attach.and_then(|a| a.thing).map(|hit| hit.thing).or(t.attach);
                    let flags = t.flags;
                    rt.detach_support(Some(target), surface, parent, flags, Some(vel));
                    let t = &mut rt.things[target];
                    t.floor_attach = None;
                    t.attach = None;
                    t.attach_flags = 0;
                });
                rt.things[target].vel = vel;
                rt.things[target].phys_flags = phys;
            });
        }
        let accepted = crate::collide::Solids::collide_hurt(state, hit.normal, hit.distance, crease, false, |state, damage| {
            Self::charge_contact(rt, mover, state, damage);
        });
        if accepted {
            if target_mass == 0.0 {
                Self::contact_impact_sound(rt, mover, state, speed, target_flags & 0x400000 != 0);
            }
            // Actor pairs wrap this handler (`0x4ed210`) and emit AI event 4 on acceptance.
            rt.with(|rt| {
                if state.kind == 2 && let Some(mover) = mover {
                    rt.emit_ai_event(mover, 4, Some(target));
                }
                if target_kind == Some(2) { rt.emit_ai_event(target, 4, mover); }
            });
        }
        accepted
    }

    /// `sithPhysics_ApplyForce` (`0x4f59b0`) for the physics bodies admitted by the pair handler.
    fn sith_physics_apply_force(
        mass: f32, force: [f32; 3], vel: &mut [f32; 3], flags: &mut u32, detach: impl FnOnce(&mut [f32; 3]),
    ) {
        if mass <= 0.0 { return; } // 0x523180
        let inverse = 1.0 / mass; // 0x523194
        if force[2] * inverse > 0.5 { detach(vel); } // 0x523198
        for (v, f) in vel.iter_mut().zip(force) { *v += f * inverse; }
        *flags |= 0x8000;
    }

    /// `sithThing_DetachThing` (`0x4ce380`): restore support velocity and send `exited`.
    fn detach_support(
        &mut self, sender: Option<usize>, surface: Option<usize>, parent: Option<usize>,
        flags: u32, vel: Option<&mut [f32; 3]>,
    ) {
        if let Some(vel) = vel {
            let carry = if let Some(parent) = parent { self.thing_support_velocity(parent) }
                else { Vec3::from(surface.map_or([0.0; 3], |id| self.surface_scroll(id))) };
            *vel = (Vec3::from(*vel) + carry).to_array();
        }
        if flags & 0x100 != 0 { return; }
        if let Some(parent) = parent {
            if self.things.get(parent).is_some_and(|t| t.flags & 0x400 != 0) {
                self.sith_cog_thing_send_message(parent, sender, "exited");
            }
        } else if let Some(surface) = surface {
            let surface_flags = self.surface_flags.get(surface).copied().unwrap_or(0);
            if surface_flags & SITH_SURFACE_COG_LINKED != 0 {
                self.sith_cog_surface_send_message(surface, sender, "exited");
            }
        }
    }

    /// `sithCollision_CollideHurt`'s inline charge (`0x4e9090:56-60`): the `hitdamaged` mode
    /// sound (`0x35`) and `sithThing_DamageThing(thing, thing, damage, 0x40)`, applied at the
    /// contact pose before the move continues.
    fn charge_contact(
        rt: &mut ContactRuntime<'_>, mover: Option<usize>, state: &mut crate::collide::MoveState, damage: f32,
    ) {
        let Some(mover) = mover else { return };
        let at = rt.position(state);
        rt.with(|rt| {
            let thing = &mut rt.things[mover];
            thing.pos = at;
            thing.sector = state.sector as i32;
            thing.vel = state.vel;
            rt.queue_mode_sound(mover, "hitdamaged", at); // `0x35`, table `0x54b40c`
            if state.kind == 10 {
                rt.hurt_player_fall(damage, mover as i64);
            } else if state.kind == 2 {
                rt.damage_typed_from(mover as i64, damage, 0x40, mover as i64);
            }
        });
        rt.dispatch(state);
    }

    /// The impact sound shared by `sithCollision_HandleThingHitSurface` (`0x4e8b40:29-38`) and
    /// `sithCollision_ThingCollisionHandler` (`0x4e8c50:87-95`): above 0.15 (`0x523020`) of
    /// approach speed, mode `hitmetal` (`0x2f`) or `hithard` (`0x2e`).
    fn contact_impact_sound(
        rt: &mut ContactRuntime<'_>, mover: Option<usize>, state: &crate::collide::MoveState, speed: f32, metal: bool,
    ) {
        if speed > 0.15 && let Some(mover) = mover { // 0x523020
            // `0x2e`/`0x2f`, table `0x54b40c`; 0x523014 caps the gain.
            let mode = if metal { "hitmetal" } else { "hithard" };
            let at = rt.position(state);
            rt.with(|rt| rt.queue_first_mode_sound(mover, mode, at, speed.min(1.0)));
        }
    }

    /// Publish a canonical contact once, then preserve untouched body coordinates bit-for-bit.
    fn player_move_phase(
        world: &World, state: &mut crate::collide::MoveState, eye: f32, dispatch: &mut impl FnMut(),
    ) {
        let published = [state.pos[0], state.pos[1], state.pos[2] + eye];
        let Some(index) = world.borrow().local_player() else { return };
        {
            let mut rt = world.borrow_mut();
            let thing = &mut rt.things[index];
            thing.pos = published;
            thing.sector = state.sector as i32;
            thing.vel = state.vel;
            thing.flags = state.flags;
            thing.phys_flags = state.phys_flags;
            thing.floor_attach = state.attach;
            thing.attach_flags = state.attach_flags;
        }
        dispatch();
        let rt = world.borrow();
        let thing = &rt.things[index];
        let teleported = thing.pos.map(f32::to_bits) != published.map(f32::to_bits);
        for axis in 0..3 {
            if thing.pos[axis].to_bits() != published[axis].to_bits() {
                state.pos[axis] = if axis == 2 { thing.pos[axis] - eye } else { thing.pos[axis] };
            }
        }
        if let Ok(sector) = usize::try_from(thing.sector) {
            state.sector = sector;
        }
        state.vel = thing.vel;
        state.flags = thing.flags;
        state.phys_flags = thing.phys_flags;
        state.attach = if teleported { None } else { thing.floor_attach };
        state.attach_flags = if teleported { 0 } else { thing.attach_flags };
        state.kind = rt.types.get(index).copied().unwrap_or(0) as u32;
    }

    /// `0x4fbe10`: player flag, item-to-player LOS, strict millisecond deadline, send, stamp.
    fn sith_item_player_touch(
        world: &World, solids: &crate::collide::Solids, state: &mut crate::collide::MoveState,
        eye: f32, item: usize, dispatch: &mut impl FnMut(),
    ) {
        if state.flags & 0x100 != 0 {
            return;
        }
        let sent = {
            let mut rt = world.borrow_mut();
            let Some(player) = rt.local_player() else { return };
            let Some(thing) = rt.things.get(item) else { return };
            if thing.destroyed || thing.flags & 0x80002 != 0 || rt.types.get(item) != Some(&5) {
                return;
            }
            let Ok(sector) = usize::try_from(thing.sector) else { return };
            if !solids.item_has_los(sector, thing.pos, state.pos, rt.gates())
                || thing.next_touch_ms >= rt.game_clock.as_millis() as u32
            {
                return;
            }
            rt.sith_cog_item_send_message(item, player, "touched")
        };
        if sent {
            Self::player_move_phase(world, state, eye, dispatch);
        }
        let mut rt = world.borrow_mut();
        let now = rt.game_clock.as_millis() as u32;
        rt.things[item].next_touch_ms = now.wrapping_add(500);
    }

    /// `MoveThing` calls this at the contact, not at the frame's final endpoint.
    fn player_crossing(
        world: &World, state: &mut crate::collide::MoveState, eye: f32,
        surface: Option<usize>, far: usize, dispatch: &mut impl FnMut(),
    ) {
        let player = world.borrow().local_player();
        if let Some(surface) = surface {
            {
                let mut rt = world.borrow_mut();
                if rt.surface_flags.get(surface).is_some_and(|f| f & SITH_SURFACE_COG_LINKED != 0) {
                    rt.sith_cog_surface_send_message(surface, player, "crossed");
                }
            }
            Self::player_move_phase(world, state, eye, dispatch);
        }
        if state.sector == far {
            return;
        }
        {
            let mut rt = world.borrow_mut();
            if state.flags & 0x80100 == 0
                && rt.sector_flags.get(state.sector).is_some_and(|f| f & 4 != 0)
            {
                rt.sith_cog_sector_send_message(state.sector, player, "exited");
            }
        }
        let exit_position = state.pos.map(f32::to_bits);
        // ExitSector may warp the body and skip unlinking. SetSector still calls EnterSector.
        Self::player_move_phase(world, state, eye, dispatch);
        if state.pos.map(f32::to_bits) == exit_position && let Some(player) = player {
            world.borrow_mut().things[player].sector = -1;
        }
        state.sector = far;
        let mut damp = false;
        {
            let mut rt = world.borrow_mut();
            if let Some(player) = player {
                rt.things[player].sector = far as i32;
            }
            let wet = rt.sector_flags.get(far).is_some_and(|f| f & crate::collide::SECTOR_UNDERWATER != 0);
            if wet && state.move_type == 1 {
                let keep = player.and_then(|i| rt.things.get(i)).is_some_and(|t| t.attach_flags & 8 != 0);
                if !keep {
                    state.attach = None;
                    if let Some(i) = player {
                        rt.things[i].attach = None;
                        rt.things[i].attach_flags = 0;
                    }
                }
            }
            let was_wet = state.flags & 0x2000000 != 0;
            if wet != was_wet {
                if wet {
                    state.flags |= 0x2000000;
                } else {
                    state.flags &= !0x2000000;
                }
                if let Some(player) = player {
                    let animated = rt.templates.get(player).is_some_and(|n| rt.template_puppet.contains(n));
                    if animated {
                        rt.movement_mode.insert(player, u32::from(wet));
                    }
                }
                let destroy = state.flags & if wet { 0x8000000 } else { 0x10000000 } != 0;
                if (!wet || !destroy) && let Some(player) = player {
                    let mode = if wet {
                        if state.move_type != 1 || state.vel[2] <= -1.0 { "enterwater" } else { "enterwaterslow" }
                    } else {
                        let speed = (state.vel[0] * state.vel[0] + state.vel[2] * state.vel[2] + state.vel[1] * state.vel[1]).sqrt();
                        if state.move_type == 1 && speed < 1.0 { "exitwaterslow" } else { "exitwater" }
                    };
                    rt.queue_mode_sound(player, mode, [state.pos[0], state.pos[1], state.pos[2] + eye]);
                }
                if destroy {
                    state.flags |= 2;
                    if state.flags & 0x400 != 0 && state.flags & 0x100 == 0
                        && let Some(player) = player
                    {
                        rt.sith_cog_thing_send_message(player, None, "removed");
                    }
                } else {
                    if state.flags & 0x20000000 != 0 && state.flags & 0x100 == 0
                        && let Some(player) = player
                    {
                        let identity = SourceSender {
                            source_type: THING_SOURCE, source: player as i32, sender_type: 0,
                            sender: i32::from(wet),
                        };
                        let scripts = [rt.thing_class_cog.get(&player).copied(), rt.thing_capture_cog.get(&player).copied()];
                        for instance in scripts.into_iter().flatten() {
                            rt.pending_messages.push((instance as usize, "splash", [0.0; 2], Some(0), Some(identity)));
                        }
                    }
                    damp = wet;
                }
            }
        }
        Self::player_move_phase(world, state, eye, dispatch);
        if damp && state.move_type == 1 {
            state.vel[2] *= 0.25;
        }
        {
            let mut rt = world.borrow_mut();
            if state.flags & 0x80100 == 0
                && rt.sector_flags.get(state.sector).is_some_and(|f| f & 4 != 0)
            {
                rt.sith_cog_sector_send_message(state.sector, player, "entered");
            }
        }
        Self::player_move_phase(world, state, eye, dispatch);
    }

    /// Is a body standing in water at `feet`? Two rules, both the engine's: the surface under the
    /// feet carries `SITH_SURFACE_WATER` (0x20000) or `VERYDEEPWATER` (0x100000), **or** the
    /// sector carries `SITH_SECTOR_UNDERWATER` (0x2) — `sithPhysics.c:43` tests the sector.
    pub fn in_water(
        &self,
        solids: &crate::collide::Solids,
        sector: usize,
        feet: [f32; 3],
        sector_flags: &[u32],
    ) -> bool {
        solids
            .floor_surface(sector, feet)
            .and_then(|(_, surface)| self.surface_flags.get(surface))
            .is_some_and(|flags| flags & (0x2_0000 | 0x10_0000) != 0)
            || sector_flags
                .get(sector)
                .is_some_and(|flags| flags & crate::collide::SECTOR_UNDERWATER != 0)
    }

    /// The local player's thing, when the level has one and the runtime holds it.
    fn local_player(&self) -> Option<usize> {
        self.templates
            .iter()
            .position(|t| t.eq_ignore_ascii_case("walkplayer"))
            .filter(|index| *index < self.things.len())
    }

    /// `sithPhysics_GetThingHeight` (`0x4f6210`) for a placed thing (or the `walkplayer` template
    /// when `index` is `None`): the template's `height=` when set, else the model insert. Template
    /// keys are lowercase, so a mixed-case placed name retries lowercased.
    fn thing_height(&self, index: Option<usize>, radius: f32) -> f32 {
        let name = index.and_then(|i| self.templates.get(i)).map_or("walkplayer", String::as_str);
        let insert = index.map_or_else(|| template_insert(name)[2], |i| self.things[i].insert[2]);
        let explicit = self.template_height.get(name).copied().or_else(|| {
            if name.bytes().any(|b| b.is_ascii_uppercase()) {
                self.template_height.get(&name.to_ascii_lowercase()).copied()
            } else {
                None
            }
        });
        crate::collide::get_thing_height(explicit.unwrap_or(0.0), insert, radius)
    }

    fn floor_parent_move(&self, index: usize) -> u32 {
        match self.templates.get(index).and_then(|name| self.template_move.get(name)).map(String::as_str) {
            Some("physics") => 1,
            Some("path") => 2,
            _ => 0,
        }
    }

    /// `sithThing_UpdateMove` (`0x4cc4c0`) for one mover: the attached surface's scroll
    /// (`sithSurface_DetachThing`) and the attached thing's move type come from the world.
    fn update_move(
        &self, state: &crate::collide::MoveState, displacement: [f32; 3], dt: f32,
    ) -> crate::collide::UpdateMove {
        let scroll = state.attach.and_then(|a| a.surface).map_or([0.0; 3], |surface| self.surface_scroll(surface));
        let parent = state.attach.and_then(|a| a.thing).map(|target| self.floor_parent_move(target.thing));
        crate::collide::Solids::update_move(state, displacement, scroll, dt, parent)
    }

    /// `0x4ce050:71-74`: inverse ortho rotation of the rider's offset.
    fn floor_local_anchor(&self, index: usize, pos: [f32; 3]) -> Option<[f32; 3]> {
        let thing = self.things.get(index)?;
        let relative = Vec3::from(pos) - Vec3::from(thing.pos);
        Some([0, 1, 2].map(|axis| {
            let mut basis = [0.0; 3];
            basis[axis] = 1.0;
            relative.dot(Vec3::from(rotate_by_angles(basis, thing.pitch, thing.yaw, thing.roll)))
        }))
    }

    /// The rider half of a thing-face attach: `sithThing_AttachThingToThingFace` (`0x4ce050:71-74`)
    /// stores the rider's offset in the parent's frame ([`Runtime::floor_local_anchor`]); this
    /// returns the face attach and the displacement that restores that offset after the parent
    /// moved, or `None` once the parent is gone, flagged `0x80002`, or has no face.
    fn floor_rider_motion(
        &self, solids: &crate::collide::Solids, previous: crate::collide::Attach,
        pos: [f32; 3], anchor: Option<[f32; 3]>,
    ) -> Option<(crate::collide::Attach, [f32; 3])> {
        let target = previous.thing?;
        let parent = self.things.get(target.thing).filter(|t| !t.destroyed && t.flags & 0x80002 == 0)?;
        let attach = solids.thing_face_attach(target)?;
        let carry = anchor.map_or([0.0; 3], |offset| {
            (Vec3::from(parent.pos) + Vec3::from(rotate_by_angles(offset, parent.pitch, parent.yaw, parent.roll))
                - Vec3::from(pos)).to_array()
        });
        Some((attach, carry))
    }

    /// `sithSurface_DetachThing` (0x4f0a30): the live scroll, only while the surface is scrolling.
    fn surface_scroll(&self, surface: usize) -> [f32; 3] {
        if self.surface_flags.get(surface).is_some_and(|flags| flags & 0x800 != 0)
            && let Some(surface) = self.surfaces.get(surface)
        {
            return surface.slide;
        }
        [0.0; 3]
    }

    fn floor_velocity(&self, attach: crate::collide::Attach) -> Vec3 {
        if let Some(surface) = attach.surface {
            return Vec3::from(self.surface_scroll(surface));
        }
        let Some(target) = attach.thing else { return Vec3::ZERO };
        self.thing_support_velocity(target.thing)
    }

    fn thing_support_velocity(&self, index: usize) -> Vec3 {
        let Some(parent) = self.things.get(index) else { return Vec3::ZERO };
        match self.floor_parent_move(index) {
            1 => Vec3::from(parent.vel),
            2 if !parent.paused => {
                // The port's frame rate is frames/second; the engine stores path direction × speed.
                let Some((target, speed)) = parent.target else { return Vec3::ZERO };
                let segment = if target > parent.frame { parent.frame.floor() } else { parent.frame.ceil() - 1.0 };
                let i = segment.max(0.0) as usize;
                let Some((a, b)) = parent.frames.get(i).zip(parent.frames.get(i + 1)) else { return Vec3::ZERO };
                Vec3::new(b[0] - a[0], b[1] - a[1], b[2] - a[2]) * (speed * (target - parent.frame).signum())
            }
            _ => Vec3::ZERO,
        }
    }

    /// The player's `sithPhysics_FindFloor(thing, 0)` at the end of `sithCollision_MoveThing`
    /// (`0x4e7950:279-294`), then the attach change, contact publish and landing it causes.
    fn player_find_floor(
        world: &World, solids: &crate::collide::Solids, state: &mut crate::collide::MoveState,
        radius: f32, height: f32, eye: f32, dispatch: &mut impl FnMut(),
    ) -> bool {
        let previous = state.attach;
        let mut rt = world.borrow_mut();
        let player = rt.local_player();
        let up = player.map_or([0.0, 0.0, 1.0], |i| rt.thing_up(i));
        solids.find_floor_for(state, radius, height, rt.gates(), false, player, up);
        let landed = previous.is_none() && state.attach.is_some();
        let changed = rt.floor_transition(state, previous, player, true);
        drop(rt);
        if changed {
            Self::player_move_phase(world, state, eye, dispatch);
            if landed && let (Some(index), Some(attach)) = (player, state.attach) {
                world.borrow_mut().floor_landing(index, state, attach);
            }
        }
        landed && state.attach.is_some()
    }

    fn player_detach_floor(
        world: &World, state: &mut crate::collide::MoveState, eye: f32, dispatch: &mut impl FnMut(),
    ) {
        let Some(previous) = state.attach.take() else { return };
        state.attach_flags = 0;
        {
            let mut rt = world.borrow_mut();
            let player = rt.local_player();
            rt.floor_transition(state, Some(previous), player, false);
        }
        Self::player_move_phase(world, state, eye, dispatch);
    }

    fn thing_up(&self, index: usize) -> [f32; 3] {
        let thing = &self.things[index];
        rotate_by_angles([0.0, 0.0, 1.0], thing.pitch, thing.yaw, thing.roll)
    }

    /// `0x4ce380`, `0x4cde80`, `0x4ce050`: change support velocity, send exit/entry, then land.
    fn floor_transition(
        &mut self, state: &mut crate::collide::MoveState, previous: Option<crate::collide::Attach>,
        sender: Option<usize>, force: bool,
    ) -> bool {
        let identity = |a: crate::collide::Attach| (a.surface, a.thing);
        if previous.map(identity) == state.attach.map(identity) { return false; }
        if let Some(old) = previous {
            self.detach_support(sender, old.surface, old.thing.map(|t| t.thing), state.flags,
                (state.move_type == 1).then_some(&mut state.vel));
        }
        let Some(new) = state.attach else { return true };
        if state.move_type == 1 {
            state.vel = (Vec3::from(state.vel) - self.floor_velocity(new)).to_array();
        }
        self.sith_thing_attach_thing_to_surface(previous.and_then(|a| a.surface), new.surface, state.flags, sender);
        if let Some(target) = new.thing
            && state.flags & 0x80100 == 0
            && self.things.get(target.thing).is_some_and(|t| t.flags & 0x400 != 0)
        {
            self.sith_cog_thing_send_message(target.thing, sender, "entered");
        }
        if !force && previous.is_none() && let Some(index) = sender {
            self.floor_landing(index, state, new);
        }
        true
    }

    /// `sithCollision_FallHurt` (0x4e9550), called on a fresh, unforced attach.
    fn floor_landing(&mut self, index: usize, state: &crate::collide::MoveState, attach: crate::collide::Attach) {
        let impact = -Vec3::from(state.vel).dot(Vec3::from(attach.normal));
        // `_DAT_00522e94` = 2.5, read from JK.EXE. Sound names come from the 0x54b40c table.
        if impact > 2.5 {
            let damage = crate::fall_damage(impact);
            if damage > 0.0 {
                self.queue_mode_sound(index, "hitdamaged", state.pos);
                let left = self.damage_typed_from(index as i64, damage, DAMAGE_FALL, index as i64);
                if state.kind == 10 && left <= 0.0 { self.player_dead = true; }
            }
            self.queue_mode_sound(index, "landhurt", state.pos);
        }
        let mode = if let Some(surface) = attach.surface {
            let flags = self.surface_flags.get(surface).copied().unwrap_or(0);
            if flags & 0x10000 != 0 { "landmetal" }
            else if flags & 0x20000 != 0 { "landwater" }
            else if flags & 0x40000 != 0 { "landpuddle" }
            else if flags & 0x180000 != 0 { "landearth" }
            else { "landhard" }
        } else {
            let flags = attach.thing.and_then(|a| self.things.get(a.thing)).map_or(0, |t| t.flags);
            if flags & 0x400000 != 0 { "landmetal" }
            else if flags & 0x800000 != 0 { "landearth" }
            else { "landhard" }
        };
        self.queue_mode_sound(index, mode, state.pos);
        if attach.surface.is_some() && state.move_type == 1 && state.phys_flags & 0x10000 == 0
            && self.templates.get(index).is_some_and(|n| self.template_puppet.contains(n))
        {
            self.play_mode.insert(index, 0x14);
        }
    }

    /// Write the body's final pose onto the local player's thing: the eye point, facing, aim and
    /// sector. Scripts, projectiles and the renderer read this one body.
    fn publish_player(&mut self, body: &PlayerBody) {
        let Some(index) = self.local_player() else { return };
        let thing = &mut self.things[index];
        thing.pos = [body.pos.x, body.pos.y, body.pos.z + body.eye];
        thing.yaw = body.yaw.to_degrees();
        thing.pitch = body.pitch.to_degrees();
        thing.sector = body.sector as i32;
    }

    /// The damage a landing or a fall death deals to the player's own health. A landing is a
    /// **fall**, not an impact: `sithActor_DamageActor` exempts `SITH_DAMAGE_FALL` (0x40) from
    /// `SITH_AF_INVULNERABLE`, which is why the level-end freeze does not save a body that drops
    /// off a ledge.
    fn hurt_player_fall(&mut self, damage: f32, source: i64) {
        let Some(index) = self.local_player() else { return };
        let left = self.damage_typed_from(index as i64, damage, DAMAGE_FALL, source);
        if left <= 0.0 {
            self.player_dead = true;
        }
        println!("fall: {damage:.0} damage, {left:.0} health left");
    }

    /// `sithPlayer_Update`: the dynamic tint falls by 0.4 per second on every channel, never below
    /// zero. The damage flash and the `AddDynamicTint` verb are the only things that raise it.
    fn decay_dynamic_tint(&mut self, dt: f32) {
        for channel in &mut self.tint {
            *channel = (*channel - dt * TINT_DECAY).max(0.0);
        }
    }

    /// [`Self::fire`] with the actor's aim error: `sithAICmd_PrimaryFire` passes
    /// `percentageErrorInAim` and `sithAI_RandomFireVector` (0x4eb920) jitters the aim **vector** by
    /// that fraction — `((rand/32768) - 0.5) * error` on each component, then normalized. A missed
    /// shot is a shot that flies wide, which is what makes a firefight survivable.
    pub fn fire_spread(&mut self, shooter: i64, template: &str, error: f32) -> Option<usize> {
        self.fire_with_spread(shooter, template, error, None)
    }

    /// Fire `template` from the shooter: spawn it just in front of the shooter's eye and give it the
    /// template's own `vel=`, rotated into the shooter's facing.
    ///
    /// The engine's projectile is a `SITH_THING_WEAPON` created by `CreateThing` from the weapon
    /// template, launched along the barrel; the speed in the shipped templates is `vel=(0/4/0)` for
    /// `+bryarbolt`, i.e. 4 units per second down the model's +Y.
    pub fn fire(&mut self, shooter: i64, template: &str) -> Option<usize> {
        self.fire_with_spread(shooter, template, 0.0, None)
    }

    /// `sithAI_RandomFireVector` (0x4eb920): add `((rand/32768) - 0.5) * error` to each component of
    /// the aim direction and renormalize. `FUN_00512d00` is the engine's `rand()` and `DAT_0052307c` is
    /// 1/32768, so the jitter is ±error/2 per axis — a small cone, not the fraction-of-a-right-angle
    /// yaw offset the port used to apply (0.2 spread that way was ±18 degrees, and it left pitch
    /// untouched, so an AI gunner missed far more than retail's does at close range).
    fn jitter_aim(forward: [f32; 3], error: f32) -> [f32; 3] {
        let speed = (forward[0] * forward[0] + forward[1] * forward[1] + forward[2] * forward[2])
            .sqrt()
            .max(1e-4);
        let mut unit = [forward[0] / speed, forward[1] / speed, forward[2] / speed];
        for channel in &mut unit {
            *channel += (crate::cog::pseudo_random() as f32 - 0.5) * error;
        }
        let len = (unit[0] * unit[0] + unit[1] * unit[1] + unit[2] * unit[2]).sqrt().max(1e-4);
        [unit[0] / len * speed, unit[1] / len * speed, unit[2] / len * speed]
    }

    /// The engine's fire point `ai + 0x82`: `pos + orient * thing[0x17c]`
    /// (`sithAI_sub_4EAD60` `0x4ead60`). A template without `fireoffset=` transforms the zero
    /// vector and the bolt leaves the body's origin, as the engine's does; the player's
    /// one-unit-ahead fallback in `fire_toward` is a player-path stand-in, not this rule.
    fn sith_ai_fire_point(&self, index: usize) -> [f32; 3] {
        let Some(t) = self.things.get(index) else { return [0.0; 3] };
        let offset = self
            .templates
            .get(index)
            .and_then(|name| self.template_fire_offset.get(&name.to_ascii_lowercase()))
            .copied()
            .unwrap_or([0.0; 3]);
        let (s, c) = t.yaw.to_radians().sin_cos();
        let (sp, cp) = t.pitch.to_radians().sin_cos();
        let lvec = [-s * cp, c * cp, sp];
        let rvec = [c, s, 0.0];
        let uvec = [s * sp, -c * sp, cp];
        [
            t.pos[0] + lvec[0] * offset[1] + rvec[0] * offset[0] + uvec[0] * offset[2],
            t.pos[1] + lvec[1] * offset[1] + rvec[1] * offset[0] + uvec[1] * offset[2],
            t.pos[2] + lvec[2] * offset[1] + rvec[2] * offset[0] + uvec[2] * offset[2],
        ]
    }

    /// `sithAI_FireWeapon` (`0x4eba10`), transliterated. The port's fire block calls this with the
    /// instinct's own numbers after its readiness and sight gates (the engine's `ai[0x7d] == 0`
    /// visibility check is the port's `can_see`, and `ai[0xa2]` is `ai_fire_lock`).
    ///
    /// The gates, in the engine's order: the lockout, `minD <= ai[0x7c] <= maxD` on the
    /// radius-subtracted 3D eye distance (`CheckSightThing` `0x4eb090` writes
    /// `max(len - r, 0)`), and the facing `|rvec . dir| <= 1 - dotTol` with **no behind test**.
    /// A thing flagged `0x20000` (a puppet) defers instead: the engine stores `fl|8`, the error
    /// and the mode, and plays the fire track. The fired direction is the sight line off the
    /// fire point, then the lead (`fl & 1`), the lob (`fl & 2`) and the accuracy-gated jitter in
    /// the engine's order. `accuracy = None` skips the gate, which is the engine's `BlindFire`
    /// inline path (`0x509b30` calls `RandomFireVector` directly).
    fn sith_ai_fire_weapon(
        &mut self,
        index: usize,
        target: usize,
        target_pos: [f32; 3],
        min_d: f32,
        max_d: f32,
        dot_tol: f32,
        err: f32,
        mode: u32,
        fl: u32,
        accuracy: Option<f32>,
    ) -> FireOutcome {
        let Some(t) = self.things.get(index) else { return FireOutcome::Refused };
        // `sithAI_FireWeapon`'s puppet branch tests the **actor word** (`thing + 0x130`,
        // `puVar1[0x4c] & 0x20000`), not the thing's own flags — since #90's split that word is
        // `actor_flags`.
        let (yaw, flags) = (t.yaw, self.actor_flags.get(&index).copied().unwrap_or(0));
        let weapon = self
            .templates
            .get(index)
            .and_then(|name| {
                let name = name.to_ascii_lowercase();
                let w = if mode == 0 {
                    self.template_weapon.get(&name)
                } else {
                    self.template_weapon2.get(&name).or(self.template_weapon.get(&name))
                };
                w.cloned()
            })
            // The engine's weapon is `thing + 0x54/0x55`; a thing with none set refuses.
            .unwrap_or_default();
        if weapon.is_empty() {
            return FireOutcome::Refused;
        }
        let eye = self.sith_ai_fire_point(index);
        let d = [target_pos[0] - eye[0], target_pos[1] - eye[1], target_pos[2] - eye[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len < 1e-6 {
            return FireOutcome::Refused;
        }
        let mut dir = [d[0] / len, d[1] / len, d[2] / len];
        // The aim vector as the engine stores it (`ai + 0x79`): the lob reads this original,
        // never the lead-adjusted direction.
        let aim = dir;
        let radius = self
            .templates
            .get(target)
            .and_then(|name| self.template_hit.get(&name.to_ascii_lowercase()))
            .map(|(r, _)| *r)
            .unwrap_or(0.065);
        let dist = (len - radius).max(0.0);
        let speed = self
            .template_vel
            .get(&weapon)
            .copied()
            .map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt())
            .unwrap_or(0.0);
        if fl & 8 == 0 {
            if self.ai_fire_lock.get(&index).is_some_and(|at| *at > self.time) {
                return FireOutcome::Refused;
            }
            if dist < min_d || max_d < dist {
                if std::env::var_os("BK_AI_TRACE").is_some() {
                    eprintln!("ai: {index} fire range {dist:.2} outside [{min_d:.2}, {max_d:.2}]");
                }
                return FireOutcome::Refused;
            }
            let (s, c) = yaw.to_radians().sin_cos();
            if (c * dir[0] + s * dir[1]).abs() > 1.0 - dot_tol {
                if std::env::var_os("BK_AI_TRACE").is_some() {
                    eprintln!(
                        "ai: {index} fire facing |rvec.dir| {:.2} > {:.2}",
                        (c * dir[0] + s * dir[1]).abs(),
                        1.0 - dot_tol
                    );
                }
                return FireOutcome::Refused;
            }
            if flags & 0x20000 != 0 {
                self.ai_deferred_fire
                    .insert(index, (fl | 8, err, mode, target));
                self.play_mode
                    .insert(index, if mode == 0 { 8 } else { 0x12 } as i32);
                return FireOutcome::Deferred;
            }
        }
        // The lead (`p7 & 1`): a target of thing type 1 that is moving is led by its velocity
        // plus the bolt's flight time along the current line, accepted only while the adjusted
        // line stays within `dot > 0.5` of the original.
        if fl & 1 != 0 && speed > 0.0 {
            // The engine reads the target's physics velocity (`thing + 0x1d4`). An AI actor's is
            // the record its move arm pushes. shortcut: the player's `ThingState::vel` is not that
            // record (the walker integrates its own body), so a player target is not led; read the
            // walker's velocity here when the player body moves into `ThingState`.
            let is_actor = target == 0
                || self
                .templates
                .get(target)
                .and_then(|name| self.template_types.get(&name.to_ascii_lowercase()))
                .is_some_and(|ty| *ty == 1)
                || self.templates.get(target).is_none();
            let vel = if self.ai.contains_key(&target) {
                self.things.get(target).map_or([0.0; 3], |t| [t.vel[0], t.vel[1], 0.0])
            } else {
                [0.0; 3]
            };
            if is_actor && vel != [0.0; 3] {
                let cand = [
                    vel[0] + dir[0] * speed,
                    vel[1] + dir[1] * speed,
                    vel[2] + dir[2] * speed,
                ];
                let clen = (cand[0] * cand[0] + cand[1] * cand[1] + cand[2] * cand[2]).sqrt();
                if clen > 1e-6 {
                    let cand = [cand[0] / clen, cand[1] / clen, cand[2] / clen];
                    if cand[0] * dir[0] + cand[1] * dir[1] + cand[2] * dir[2] > 0.5 {
                        dir = cand;
                    }
                }
            }
        }
        let mut speed_scale = 1.0;
        if fl & 2 != 0 && speed > 0.0 {
            // The lob: scale by the weapon speed, lift z by the gravity arc for that flight
            // time, renormalise, and hand `WeaponFire` the magnitude as its speed parameter.
            // The engine lobs from the **original aim vector** (`param_1[0x79..0x7b]`), not the
            // lead-adjusted `dir` — the two only differ when both `fl & 1` and `fl & 2` are set.
            let mut v = [aim[0] * speed, aim[1] * speed, aim[2] * speed];
            v[2] += (dist / speed) * 0.5 * self.gravity;
            let vlen = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            if vlen > 1e-6 {
                dir = [v[0] / vlen, v[1] / vlen, v[2] / vlen];
                speed_scale = vlen / speed;
            }
        }
        if err != 0.0 && dist != 0.0 {
            // The accuracy gate: the engine draws one number even when the shot stays clean,
            // and jitters only when `class.accuracy < rand()/32768`.
            let jitter = match accuracy {
                Some(acc) => (acc as f64) < crate::cog::pseudo_random(),
                None => true,
            };
            if jitter {
                dir = Self::jitter_aim(dir, err);
            }
        }
        let aim = AiAim { dir, speed_scale, origin: eye };
        match self.fire_toward(index as i64, &weapon, 0.0, None, Some(target), Some(aim)) {
            Some(shot) => FireOutcome::Shot(shot),
            None => FireOutcome::Refused,
        }
    }

    fn fire_with_spread(
        &mut self,
        shooter: i64,
        template: &str,
        error: f32,
        sound: Option<&str>,
    ) -> Option<usize> {
        self.fire_toward(shooter, template, error, sound, None, None)
    }

    /// [`Self::fire_with_spread`] with the thing the shooter has as a target (the AI's own target,
    /// recorded in the encounter chronology). A shot nothing names — the player's weapon, a scripted
    /// `FireProjectile` — passes `None`.

    fn fire_toward(
        &mut self,
        shooter: i64,
        template: &str,
        error: f32,
        sound: Option<&str>,
        target: Option<usize>,
        aim: Option<AiAim>,
    ) -> Option<usize> {
        // Every shot the runtime spawns, counted where it is actually spawned. A check that samples
        // live things misses a bolt that dies between two frames, and at 10 fps that is most of them.
        self.shots_fired += 1;
        let template = template.to_ascii_lowercase();
        let src = self.thing(shooter)?;
        let (pos, yaw, sector) = (src.pos, src.yaw, src.sector);
        let speed = self.template_vel.get(&template).copied().unwrap_or([0.0; 3]);
        // The template's velocity is in the thing's own frame: forward is +Y there.
        //
        // The shot flies along the shooter's whole **orientation**, not just its yaw: the engine's
        // `senderOrient` carries pitch too (an actor's `headPYR` on top of it). Yaw alone meant every
        // shot left level, however the player was aiming.
        let pitch = src.pitch;
        let (sin, cos) = yaw.to_radians().sin_cos();
        // The engine's basis is `forward = (-sin yaw, cos yaw)`, `right = (cos yaw, sin yaw)`; the
        // template velocity is in the shooter's own frame (`x` right, `y` forward).
        let flat = [speed[0] * cos - speed[1] * sin, speed[0] * sin + speed[1] * cos];
        // Pitch tilts that horizontal aim up and down and the template's own z rides along, so a shot
        // aimed 30 degrees up leaves with `speed * sin(30)` in z. `rotate_by_angles` is the engine's
        // `rdMatrix_Build34`, whose yaw starts at +Y; this world's camera yaw starts at +X, so the
        // rotation is written out here rather than reused.
        let forward = match aim {
            // `sithAI_FireWeapon` (`0x4eba10`) already applied its lead, lob and accuracy rules to
            // this direction off the eye; the yaw/pitch build below is the player's own aim.
            Some(a) => {
                let mag =
                    (speed[0] * speed[0] + speed[1] * speed[1] + speed[2] * speed[2]).sqrt();
                [
                    a.dir[0] * mag * a.speed_scale,
                    a.dir[1] * mag * a.speed_scale,
                    a.dir[2] * mag * a.speed_scale,
                ]
            }
            None => {
                let (sp, cp) = pitch.to_radians().sin_cos();
                let mut f = [
                    flat[0] * cp,
                    flat[1] * cp,
                    (flat[0] * flat[0] + flat[1] * flat[1]).sqrt() * sp + speed[2] * cp,
                ];
                if error > 0.0 {
                    f = Self::jitter_aim(f, error);
                }
                f
            }
        };
        let health = self.template_health.get(&template).copied().unwrap_or(1.0);
        let kind = self.template_types.get(&template).copied().unwrap_or(3);
        // The shot starts at full damage and loses it in flight when the weapon decays.
        let damage = self.template_damage.get(&template).copied().unwrap_or(0.0);
        if self.encounter_events_enabled {
            self.encounter_events.push(EncounterEvent::Shot {
                t: self.time,
                owner: usize::try_from(shooter).unwrap_or(usize::MAX),
                template: template.clone(),
                pos,
                target,
            });
        }
        if std::env::var_os("BK_HIT_TRACE").is_some() {
            // The shooter's body and the shot's heading, so a trace can say who hit whom: a bolt's
            // origin alone cannot tell a point-blank friendly hit from a wide shot.
            eprintln!(
                "fire: {template} dmg {damage:.1} vel {forward:?} kind {kind} owner {shooter} from {pos:?} yaw {yaw:.1} pitch {pitch:.1}"
            );
        }
        let key = template.clone();
        // The engine's muzzle is the template's `fireoffset=`, in level units and the shooter's frame
        // (x lateral, y forward, z up). A template that names no offset falls back to a unit ahead of
        // the eye.
        let muzzle = if let Some(a) = aim {
            // The engine's `ai + 0x82`, built by `sithAI_sub_4EAD60` (`0x4ead60`) as
            // `pos + orient * thing[0x17c]` — the same point the sight direction came from.
            a.origin
        } else {
            let len = (forward[0] * forward[0] + forward[1] * forward[1]).sqrt().max(1e-4);
            let ahead = [forward[0] / len, forward[1] / len];
            // The offset belongs to the **shooter's** template (`reeyeesgun`, `greedo`):
            // `fireoffset=` is where that gun leaves that body, not where the bolt is born. The
            // bolt templates carry none, so reading theirs silently used the fallback.
            let shooter_template = usize::try_from(shooter)
                .ok()
                .and_then(|i| self.templates.get(i))
                .map(|t| t.to_ascii_lowercase());
            let offset = shooter_template
                .as_ref()
                .and_then(|t| self.template_fire_offset.get(t))
                .copied();
            match offset {
                // The real offset, even when it sits inside the shooter's body: `impacts` skips
                // the owner, so nothing needs the old "push it out" distance.
                //
                // `rdMatrix_TransformVector34(offset, orient)` — the offset rides the shooter's
                // whole oriented basis, pitch included: `lvec` forward, `rvec` right, `uvec` up.
                Some([x, y, z]) => {
                    let (sp, cp) = pitch.to_radians().sin_cos();
                    let lvec = [-sin * cp, cos * cp, sp];
                    let rvec = [cos, sin, 0.0];
                    let uvec = [sin * sp, -cos * sp, cp];
                    [
                        pos[0] + lvec[0] * y + rvec[0] * x + uvec[0] * z,
                        pos[1] + lvec[1] * y + rvec[1] * x + uvec[1] * z,
                        pos[2] + lvec[2] * y + rvec[2] * x + uvec[2] * z,
                    ]
                }
                // `sithWeapon_FireProjectile` (`0x4d5f20`) spawns at the **weapon thing's** position,
                // adding the offset only when it is not the sentinel — and a shipper's weapon thing sits
                // in the actor's hand. This build has no separate weapon thing, so the fallback is a
                // stand-in: one unit ahead of the body at `z + 0.1`. It is what `--hit` is calibrated on
                // and it is wrong on a stair (level 3's tuskens fire into the next riser); the ledger's
                // *AI projectile spawn* row carries the engine rule and this measurement.
                _ => [
                    pos[0] + ahead[0] * 1.0,
                    pos[1] + ahead[1] * 1.0,
                    pos[2] + 0.1,
                ],
            }
        };
        // `sithWeapon_WeaponFireProjectile` (`0x4d45a0`) creates the projectile at the shooter's own
        // position and then **sweeps** it to the muzzle point: `MoveThing(dir = muzzle - pos,
        // len = |muzzle - pos|)` collides on the way, so a wall between the body and the muzzle stops
        // the bolt on the shooter's side instead of placing it beyond the wall. The port jumped
        // straight to the muzzle, which let a bolt begin inside a wall (round 308 measured ten of the
        // door capture's shots dying 0.07 units out on `HAS_COLLISION` faces).
        let mut spawn = muzzle;
        if let Some(solids) = self.solids.as_ref()
            && let Some((_, at)) = solids.segment_blocked_at(pos, muzzle)
        {
            spawn = at;
            if std::env::var_os("BK_AI_TRACE").is_some() {
                eprintln!("ai: the muzzle of `{template}` is behind a wall; the shot starts at {at:?}");
            }
        }
        self.things.push(ThingState {
            pos: spawn,
            next_touch_ms: 0,
            insert: [0.0; 3],
            owner: usize::try_from(shooter).ok(),
            yaw,
            pitch: 0.0,
            roll: 0.0,
            head_pitch: 0.0,
            sector,
            // A shot carries its template's `thingflags=`; the pre-sweep above read the same word.
            flags: self.template_thingflags.get(&key).copied().unwrap_or(0),
            health,
            frame: 0.0,
            target: None,
            frames: Vec::new(),
            destroyed: false,
            vel: forward,
            damage,
            // A shot carries its template's `physflags=` too: `_weapon`'s `0x200` has no gravity bit,
            // so a bolt still flies straight, while a `_debris`-style thrown thing falls.
            phys_flags: self.template_phys_flags.get(&key).copied().unwrap_or(0),
            jk_flags: 0,
            attach: None,
            floor_attach: None,
            attach_offset: [0.0; 3],
            attach_flags: 0,
            spin: None,
            rot_vel: [0.0; 3],
            paused: false,
            pulse: 0.0,
            pulse_at: 0.0,
            light: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
            extra_speed: 0.0,
            ai_target: None,
            ai_wait: 0.0,
            ai_burst: 0.0,
            arrived: false,
            min_damage: self.template_min_damage.get(&key).copied().unwrap_or(damage),
            decay: if self.template_decay.contains(&key) {
                self.template_rate.get(&key).copied().unwrap_or(0.0)
            } else {
                0.0
            },
        });
        // The projectile's typeflags decide whether the engine runs its player pre-sweep; read before
        // the template String moves into `self.templates`.
        let pre_sweep = self.template_typeflags.get(&template).copied().unwrap_or(0) & 0x200000 != 0;
        self.templates.push(template);
        self.types.push(kind);
        let index = self.things.len() - 1;
        if self.encounter_events_enabled {
            // The projectile's own index now exists; the hit or miss that ends it reports this
            // launch time and target.
            self.encounter_shot_time.insert(index, (self.time, target));
        }
        self.arm_life(index);
        // `sithWeapon_WeaponFireProjectile`'s **player-only pre-sweep**: when the shooter is the local
        // player and the projectile's `typeflags` (the `SITH_WF_*` word, `thing + 0x130`, which
        // `sithWeapon_Update` also reads) carry `0x200000` (`+bryarbolt` ships
        // `typeflags=0x20440d`), the engine sweeps the shot's first five units and raises AI event
        // `0x1000` with the projectile on the actor it found — `dodge`'s shot-passing path. The sweep
        // runs once the projectile exists, exactly as the engine's does.
        if pre_sweep
            && usize::try_from(shooter).ok() == Some(self.player_thing())
            && let Some(actor) = self.first_actor_on_path(muzzle, forward, 5.0)
        {
            self.emit_ai_event(actor, 0x1000, Some(index));
        }
        // `sithWeapon_WeaponFireProjectile` (`0x4d45a0`) plays the sound the caller named through
        // `sithSoundMixer_PlaySoundThing(sound, shooter, 1.0, …)` (`0x4dba60`) — but only when one is
        // named: `weap_bryar.cog` passes `pistol-1.wav`, while the AI's own shot names none, so an
        // actor's class `fire1` row stands in (`gr.snd`: `pistol-1.wav` at 1.0–4.0 units).
        self.queue_shot_sound(shooter, pos, sound);
        Some(index)
    }

    /// The wav a shot plays, queued the way `sithWeapon_WeaponFireProjectile` plays it: on the
    /// shooter, at volume 1.0, so `play_requested_sounds` applies the class's own fade radius.
    fn queue_shot_sound(&mut self, shooter: i64, at: [f32; 3], sound: Option<&str>) {
        let named = sound.map(str::trim).filter(|name| !name.is_empty());
        let wav = match named {
            Some(name) => name.to_ascii_lowercase(),
            None => {
                let Some(index) = usize::try_from(shooter).ok() else { return };
                let Some(wav) = self.shot_fire_wav(index) else { return };
                wav
            }
        };
        let channel = self.sound_volume.len();
        self.sound_volume.push(1.0);
        self.sound_pitch.push(1.0);
        self.sounds.push(SoundRequest {
            name: wav,
            volume: 1.0,
            pos: Some(at),
            channel,
            pitch: 1.0,
            source: "sithWeapon_WeaponFireProjectile (0x4d45a0) -> sithSoundMixer_PlaySoundThing (0x4dba60)",
        });
    }

    /// `TakeItem(item, player)`: the item leaves the world and the player's inventory bin
    /// gains its count.
    ///
    /// The engine's `sithItem_SetItemTaken` applies the item's own inventory handler; the
    /// bin and count come from the item template's `inv=`/`count=` when it has them. When
    /// it does not — `bactatank` leaves both to its cog's `taken:` label — the bin falls
    /// back to the template's position in the template table, which is a stand-in, not the
    /// engine's number.
    pub fn take_item(&mut self, item: i64, player: i64) -> Option<(i32, f32)> {
        let index = usize::try_from(item).ok()?;
        let template = self.templates.get(index)?.to_ascii_lowercase();
        if self.types.get(index).copied() != Some(5) && self.types.get(index).copied() != Some(3) {
            return None;
        }
        let bin = self.template_inv.get(&template).copied().unwrap_or_else(|| {
            self.template_bins.get(&template).copied().unwrap_or((index as i32) + 1)
        });
        let count = self.template_count.get(&template).copied().unwrap_or(1.0);
        if let Some(thing) = self.thing_mut(item) {
            thing.destroyed = true;
        }
        if self.thing(player).is_some() {
            let held = self.inv.get(&bin).copied().unwrap_or(0.0) + count;
            self.set_bin(bin, held);
        }
        self.note_taken(index, &template);
        Some((bin, count))
    }

    /// Bookkeeping after a thing leaves the world: a weapon becomes the player's, and a
    /// powerup with `respawn=` starts counting down.
    fn note_taken(&mut self, index: usize, template: &str) {
        // A weapon becomes the player's when its template is a **projectile** (one with `damage=`):
        // a pistol item like `+bryarpistol` fires `+bryarbolt`, so equipping the item itself would
        // leave the player holding something that does no damage. Placed items carry a `+` prefix
        // that the level's template table does not, so both spellings are tried.
        let bare = template.strip_prefix('+').unwrap_or(template);
        if self.types.get(index).copied() == Some(3)
            && (self.template_damage.contains_key(template)
                || self.template_damage.contains_key(bare))
        {
            self.weapon = Some(template.to_string());
        }
        if let Some(secs) = self
            .template_respawn
            .get(template)
            .or_else(|| self.template_respawn.get(bare))
            .copied()
        {
            self.respawning.push((index, secs));
        }
    }

    /// The weapon the player fires: what a pickup gave them, else the level's default.
    pub fn current_weapon(&self) -> String {
        self.weapon.clone().unwrap_or_else(|| self.default_weapon.clone())
    }

    /// Whether `weapon` is the lightsaber. The engine's saber is bin 10 and its blade is a thing of
    /// its own, so a template name is as close as this build gets to the question.
    pub fn is_saber(weapon: &str) -> bool {
        weapon.to_ascii_lowercase().contains("sab")
    }

    /// How long a swing takes. `jkSaber.c`'s timing lives in the swing keyframes, which this build does
    /// not play, so one number stands in for the whole animation.
    pub const SABER_SWING_WAIT: f32 = 0.45;
    /// Damage of one swing. The engine's own numbers are per blade section against the victim's
    /// `saberDamage`; that table is not recovered, so this is one value with the same order as a bolt.
    pub const SABER_DAMAGE: f32 = 35.0;

    /// One saber swing: everything inside the blade's swept cylinder takes `DAMAGE_SABER`.
    ///
    /// `jkSaber.c` keeps the blade as a polyline of `length` from the hilt with a `base_rad` and a
    /// `tip_rad`, and a swing traces it against the world. This build has the same numbers from
    /// `jkSetSaberInfo`, in **model** units (a `.3do` node carries a 10x scale, which is why a
    /// doorway is 0.30 level units and a body 0.60), so the blade is `length * 10` level units long.
    ///
    /// Returns the things it hit.
    pub fn saber_swing(&mut self, player: usize) -> Vec<usize> {
        if self.time < self.saber_wait {
            return Vec::new();
        }
        let Some(blade) = self.saber.get(&player).cloned() else { return Vec::new() };
        let Some(src) = self.things.get(player) else { return Vec::new() };
        let (from, yaw) = (src.pos, src.yaw);
        let reach = (blade.length * 10.0).max(0.4);
        let radius = (blade.base_rad * 10.0).max(0.05) + 0.3;
        let (sin, cos) = yaw.to_radians().sin_cos();
        let mut hit = Vec::new();
        for (index, thing) in self.things.iter().enumerate() {
            // The player's own body is not a target, nor is anything already gone or an item: a swing
            // goes through a health pack without smashing it.
            if index == player || thing.destroyed {
                continue;
            }
            if self.types.get(index).copied() == Some(5) {
                continue;
            }
            let (px, py) = (thing.pos[0] - from[0], thing.pos[1] - from[1]);
            let along = (px * cos + py * sin).clamp(0.0, reach);
            let (dx, dy) = (px - cos * along, py - sin * along);
            let dz = thing.pos[2] - from[2];
            if (dx * dx + dy * dy + dz * dz).sqrt() <= radius {
                hit.push(index);
            }
        }
        self.saber_wait = self.time + Self::SABER_SWING_WAIT;
        for &index in &hit {
            self.damage_typed(index as i64, Self::SABER_DAMAGE, DAMAGE_SABER);
        }
        hit
    }

    /// A shot from the player, refused while `SetFireWait`'s cooldown is running. The engine arms
    /// `nextFire` on the shot rather than measuring from the click, so the wait starts when it fires.
    pub fn player_fire(&mut self, index: usize, weapon: &str) -> Option<usize> {
        // `sithWeapon_secMountWait`: the weapon has to finish mounting before it fires
        // (`sithWeapon_ProcessWeaponControls` returns before the fire path while the mount is pending).
        // Every shipped caller passes 0 (`SetMountWait(player, 0)` in each level's `endlevel.cog`
        // startup), which cancels the wait rather than imposing one.
        if self.time < self.mount_wait_until || self.time < self.next_fire {
            return None;
        }
        let shot = self.fire(index as i64, weapon)?;
        self.next_fire = self.time + self.fire_wait.max(0.0);
        Some(shot)
    }

    /// The goal counters `SetGoalFlags`/`ClearGoalFlags` write: bins from `SITHBIN_GOAL00` (100), the
    /// same map `SetInv`/`GetInv` use, so a script reads back exactly what another one set.
    pub fn goal_flags(&mut self, bin: i32, flags: u32, clear: bool) {
        let entry = self.inv.entry(GOAL_BIN + bin).or_insert(0.0);
        let value = *entry as u32;
        *entry = (if clear { value & !flags } else { value | flags }) as f32;
    }

    /// The engine's inventory bins (`enum SITHBIN`, `types_enums.h`). Force powers are bins
    /// like any other: 21 force jump, 22 speed, 23 seeing, 24 pull, 25 healing,
    /// 26 persuasion, and 14 is the force mana pool itself.
    pub fn bin_name(bin: i32) -> &'static str {
        match bin {
            0 => "none",
            1 => "fists",
            2 => "bryar pistol",
            3 => "stormtrooper rifle",
            4 => "thermal detonator",
            5 => "tusken prod",
            6 => "repeater",
            7 => "rail detonator",
            8 => "sequencer charge",
            9 => "concussion rifle",
            10 => "lightsaber",
            11 => "energy",
            12 => "power",
            13 => "battery",
            14 => "force mana",
            15 => "rail charges",
            16 => "new stars",
            17 => "spend stars",
            18 => "choice",
            19 => "force defense",
            20 => "jedi rank",
            21 => "force jump",
            22 => "force speed",
            23 => "force seeing",
            24 => "force pull",
            25 => "force healing",
            26 => "force persuasion",
            _ => "unknown",
        }
    }

    /// Which projectile template a bin arms, if the level has one this build can recognise.
    ///
    /// The engine keeps this in `weaponData[bin]` (a template path per bin). That table is not in the
    /// level, so the faithful mapping is each weapon template's own `inv=` (tried first, below); a
    /// `ponytail:` name-hint list is the **fallback** for a level whose templates omit `inv=`.
    pub fn weapon_for_bin(&self, bin: i32) -> Option<String> {
        // The faithful rule first: a weapon is a template the engine types as `SITH_THING_WEAPON`
        // (3) that names this bin in its own `inv=`. That is the engine's own mapping — `weaponData`
        // is not in the level text, but every shipped weapon template carries `inv=` and the port
        // already reads it into `template_inv`, so this needs no name guessing and covers every
        // weapon the level defines rather than the handful a hint list happened to name.
        let by_inv = self
            .template_inv
            .iter()
            .filter(|(_, owner)| **owner == bin)
            .map(|(name, _)| name)
            .filter(|name| {
                self.template_types.get(*name).copied() == Some(3)
                    && self.template_damage.get(*name).copied().unwrap_or(0.0) > 0.0
                    && !["exp", "cloud", "saber", "force_"].iter().any(|bad| name.contains(bad))
            })
            .min()
            .cloned();
        if by_inv.is_some() {
            return by_inv;
        }
        // Fallback for a level whose weapon templates carry no `inv=`: name hints. Exact-ish, not
        // bare substrings — a "st" hint happily matched `+concblast2`, and a "saber" hint matched the
        // `+force_saber` particle effect.
        let hints: &[&str] = match bin {
            2 => &["bryar"],
            3 => &["stlaser", "stormtrooper"],
            4 => &["thermal"],
            6 => &["repeater"],
            9 => &["concussion", "conclaser"],
            10 => &["sabre", "saber"],
            _ => return None,
        };
        // Only a thing the engine would treat as a weapon: type 3, with damage, and not one
        // of the effect templates that also carry a `damage=`.
        self.template_types
            .iter()
            .filter(|(name, kind)| {
                **kind == 3
                    && self.template_damage.get(*name).copied().unwrap_or(0.0) > 0.0
                    && !["exp", "cloud", "saber", "force_"].iter().any(|bad| name.contains(bad))
                    && hints.iter().any(|hint| name.contains(hint))
            })
            .map(|(name, _)| name.clone())
            .next()
    }

    /// `AutoSelectWeapon(player, kind)`: pick the weapon bin the player should be holding and arm it.
    ///
    /// `sithWeapon_AutoSelect` asks **each weapon cog** for a score (`SITH_MESSAGE_AUTOSELECT`) and
    /// takes the highest. The item cogs are instantiated now (`load_item_cogs`, `items.dat`), but the
    /// cog VMs are owned by the message-delivery systems (`deliver_*_messages`), not by this runtime,
    /// so the query cannot be made from here — the score is the weapon's own `damage=`, the one
    /// engine-side number that ranks the shipped weapons roughly the way the cogs' hand-tuned
    /// constants do. `kind` (0..2 in the engine) is accepted and ignored: the cogs use it to answer
    /// differently per slot, and the port asks nothing.
    ///
    /// ponytail: a damage ranking, not `pow_*.cog`'s `autoselect:` scores. Clearing an enemy with a
    /// rifle equipped and then a concussion rifle is the shipped difference; a per-cog score is what
    /// makes the choice contextual, and that arrives with inventory cogs.
    pub fn auto_select_weapon(&mut self, kind: i32) -> i32 {
        if !(0..=2).contains(&kind) {
            return -1;
        }
        let mut best: Option<(i32, f32)> = None;
        for bin in 1..=26 {
            if self.inv.get(&bin).copied().unwrap_or(0.0) <= 0.0 {
                continue;
            }
            let Some(template) = self.weapon_for_bin(bin) else { continue };
            let score = self.template_damage.get(&template.to_ascii_lowercase()).copied().unwrap_or(0.0);
            if score <= 0.0 {
                continue;
            }
            // Ties keep the armed bin, so picking up the same class of weapon does not shuffle the
            // player's hands for no reason.
            if best.is_none_or(|(bin_now, score_now)| {
                score > score_now || (score == score_now && bin == self.cur_bin && bin_now != bin)
            }) {
                best = Some((bin, score));
            }
        }
        match best {
            Some((bin, _)) => {
                self.select_bin(bin);
                bin
            }
            None => -1,
        }
    }

    /// `jkPlayer_NextWeapon` / `jkPlayer_PrevWeapon`: step through the weapon bins the player is
    /// carrying. The engine keeps a "weapons I own" mask and walks it; a count in `inv` is the same
    /// set here, and `weapon_for_bin` says which of those are weapons at all.
    ///
    /// `step` is +1 or -1. Returns the bin armed after the move, or -1 when no weapon is carried.
    pub fn cycle_weapon(&mut self, step: i32) -> i32 {
        if step == 0 {
            return -1;
        }
        let mut own: Vec<i32> = (1..=26)
            .filter(|bin| self.inv.get(bin).copied().unwrap_or(0.0) > 0.0)
            .filter(|bin| self.weapon_for_bin(*bin).is_some())
            .collect();
        if own.is_empty() {
            return -1;
        }
        own.sort_unstable();
        let next = match own.iter().position(|bin| *bin == self.cur_bin) {
            Some(at) => {
                let index = (at as i32 + step.signum()).rem_euclid(own.len() as i32);
                own[index as usize]
            }
            None => own[0],
        };
        self.select_bin(next);
        next
    }

    /// `SetCurInvWeapon` / `SelectWeapon`: the bin the player has armed, and — when this
    /// build can tell which template that bin means — the projectile it fires.
    pub fn select_bin(&mut self, bin: i32) -> bool {
        if !(0..=26).contains(&bin) {
            return false;
        }
        // Arming is a pair of messages, not a field: the engine's `sithInventory_ActivateBin` sends
        // `activated` to the bin's item cog, and the weapon cog's own `activated:` label is what
        // calls `ActivateWeapon` (`cog_weap_bryar.cog:105`). Nothing in this build did that, so
        // `weapon_cog` stayed None after a switch and the trigger did nothing.
        let previous = self.cur_bin;
        if previous != bin
            && let Some(cog) = self.bin_cog_index.get(&previous).copied()
        {
            self.messages
                .push(("deactivated".to_string(), Some(self.player_thing()), Some(cog), [0.0; 2]));
        }
        self.cur_bin = bin;
        if let Some(weapon) = self.weapon_for_bin(bin) {
            self.weapon = Some(weapon);
        }
        if previous != bin
            && let Some(cog) = self.bin_cog_index.get(&bin).copied()
        {
            let player = self.player_thing();
            // `selected:` runs the mount animation and sets the first-person model; `activated:`
            // arms the trigger.
            for message in ["selected", "activated"] {
                self.messages
                    .push((message.to_string(), Some(player), Some(cog), [0.0; 2]));
            }
        }
        true
    }


    /// Every (shot, target, carried damage) impact this frame, from `first_created` onward.
    ///
    /// The scan lives here rather than in the Bevy system so the combat path can be checked
    /// without running the app: `--hit` drives it directly. Placed things are targets too — the
    /// old version skipped everything below `first_created`, which made every actor the level
    /// placed immune and left only run-time `CreateThing` actors killable.
    pub fn impacts(&self, first_created: usize, dt: f32) -> Vec<(usize, usize, f32)> {
        let mut impacts = Vec::new();
        let player_thing = self
            .templates
            .iter()
            .position(|t| t.eq_ignore_ascii_case("walkplayer"));
        for index in first_created..self.things.len() {
            let shot = &self.things[index];
            if shot.destroyed || shot.damage <= 0.0 {
                continue;
            }
            for (other, thing) in self.things.iter().enumerate() {
                let armed = self.types.get(other).copied() == Some(2) || Some(other) == player_thing;
                // A shot never hits the body that fired it: the engine's projectile carries its
                // owner and skips it. Distance alone cannot do this — the shipped `fireoffset=` puts
                // the muzzle *inside* the shooter's body radius (`greedo` 0.55 ahead, body 0.6 plus
                // the bolt's 0.15), so "spawn it a unit ahead" only ever papered over this check.
                if shot.owner == Some(other) {
                    continue;
                }
                if !armed || other == index || thing.destroyed {
                    continue;
                }

                if self.shot_hit_fraction(index, other, dt).is_some() {
                    impacts.push((index, other, shot.damage));
                    break;
                }
            }
        }
        impacts
    }

    /// The thing the player is looking at: nearest thing inside a narrow cone around the eye's facing
    /// and within pull range. The engine's own query is in `jkPlayer.c` (`jkPlayer_Update` fills
    /// `playerThing->lastObjectHit`), so the cone and range here are calibrated (§16.5).
    pub fn aimed_thing(&self) -> Option<usize> {
        let player = self
            .templates
            .iter()
            .position(|t| t.eq_ignore_ascii_case("walkplayer"))?;
        let at = self.things.get(player)?.pos;
        let yaw = self.things[player].yaw.to_radians();
        let forward = [yaw.cos(), yaw.sin()];
        let mut best: Option<(usize, f32)> = None;
        for (index, thing) in self.things.iter().enumerate() {
            if index == player || thing.destroyed {
                continue;
            }
            let (dx, dy) = (thing.pos[0] - at[0], thing.pos[1] - at[1]);
            let flat = (dx * dx + dy * dy).sqrt();
            if flat < 0.1 || flat > PULL_RANGE {
                continue;
            }
            // `dot / |d|` is the cosine of the angle off the facing; 0.94 is about 20 degrees.
            let aim = (dx * forward[0] + dy * forward[1]) / flat;
            if aim < 0.94 {
                continue;
            }
            if best.is_none_or(|(_, nearest)| flat < nearest) {
                best = Some((index, flat));
            }
        }
        best.map(|(index, _)| index)
    }

    /// Spend one round from an ammo bin: `false` when the bin is empty, which is when the weapon
    /// cannot fire.
    ///
    /// The bin comes from the weapon's own powerup cog (`pow_bryar.cog` declares `ammobin(Int)`, and
    /// `--ammo` resolves the player's to 11). The *rate* is one round per shot, calibrated: the
    /// engine's firing tick is in `sithWeapon.c`/`jkPlayer.c`, which this source tree does not carry.
    pub fn spend(&mut self, bin: i32) -> bool {
        let left = self.inv.entry(bin).or_insert(0.0);
        if *left < 1.0 {
            return false;
        }
        *left -= 1.0;
        true
    }


    /// Set force seeing's mode. `EnableIRMode`/`DisableIRMode` are the script's own calls.
    pub fn set_ir_mode(&mut self, on: bool) {
        self.ir_mode = on;
    }

    /// Whether sector `other` is in the use action's collision search from `sector`.
    ///
    /// `sithCollision_SearchForCollisions` (`0x4e8160`) adds a sector to the sweep only when the
    /// sphere reaches one of its adjoins, and `sithPlayerActions_Activate` (`0x4fc0a0`) calls it with
    /// a 0.025 radius (`0x3ccccccd`) along the **look**, so a body standing in the doorway reaches the
    /// leaf's stub sector and a plate deeper in the next room is not in the search.
    fn in_use_search(&self, sector: i32, other: i32, eye: [f32; 3], dir: [f32; 3], reach: f32) -> bool {
        if other == sector {
            return true;
        }
        let (Ok(sector), Ok(other)) = (usize::try_from(sector), usize::try_from(other)) else {
            return false;
        };
        self.solids
            .as_deref()
            .is_some_and(|solids| solids.in_search_chain(sector, other, eye, dir, reach, 0.025))
    }

    /// `sithPlayerActions_Activate`'s reach for one body: `movesize + 0.1`
    /// (`*(float *)(thing + 0x2c) - _DAT_00523200`, and the constant at `0x523200` is -0.1). The
    /// player's use key calls the same rule. A template without a `movesize` falls back to the
    /// engine's 0.06 actor body.
    pub fn activate_reach(&self, index: usize) -> f32 {
        self.templates
            .get(index)
            .and_then(|name| {
                self.template_hit
                    .get(name)
                    .or_else(|| self.template_hit.get(&name.to_ascii_lowercase()))
            })
            .map(|(radius, _)| *radius)
            .unwrap_or(0.06)
            + 0.1
    }

    /// `sithCog_SurfaceSendMessage` (`0x4dfed0`): the three-argument form the world uses, i.e.
    /// `sithCog_SurfaceSendMessageEx(surface, sender, msg, 0, 0, 0, 0)`.
    pub fn sith_cog_surface_send_message(
        &mut self,
        surface: usize,
        sender: Option<usize>,
        msg: &'static str,
    ) -> usize {
        self.sith_cog_send_message_ex(SURFACE_SOURCE, surface, sender, msg)
    }



    /// `sithThing_AttachThingToSurface` (`0x4cde80`): the message half of the attach. The engine
    /// sends the surface's linked cog `entered` (id 6) when the new attach surface differs from the
    /// previous one (its early return keeps a same-surface re-attach silent), the thing's flags are
    /// clear of `0x80100`, and the surface's own surfflags carry `SITH_SURFACE_COG_LINKED` (2).
    /// The send runs regardless of the no-landing-effects flag, so a creation attach fires too.
    pub fn sith_thing_attach_thing_to_surface(
        &mut self,
        prev: Option<usize>,
        new: Option<usize>,
        thing_flags: u32,
        sender: Option<usize>,
    ) {
        if thing_flags & 0x80100 != 0 {
            return;
        }
        if let Some(surface) = new
            && Some(surface) != prev
            && self.surface_flags.get(surface).is_some_and(|f| f & SITH_SURFACE_COG_LINKED != 0)
        {
            if std::env::var_os("BK_ATTACH_TRACE").is_some() {
                eprintln!("attach: thing {sender:?} entered surface {surface} (was {prev:?})");
            }
            self.sith_cog_surface_send_message(surface, sender, "entered");
        }
    }

    /// `sithCog_ThingSendMessage` (`0x4dfac0`): `sithCog_ThingSendMessageEx(thing, sender, msg, 0,
    /// 0, 0, 0)`. Every link to the thing hears it, plus the thing's own two script slots; the port
    /// has the link tables only (`wire_links`).
    pub fn sith_cog_thing_send_message(
        &mut self,
        thing: usize,
        sender: Option<usize>,
        msg: &'static str,
    ) -> usize {
        self.sith_cog_send_message_ex(THING_SOURCE, thing, sender, msg)
    }

    /// `sithCog_ThingSendMessageEx` (`0x4dfaf0`) for the item's `touched`/`taken` paths:
    /// class COG, capture COG, then the links whose mask admits the player.
    fn sith_cog_item_send_message(&mut self, item: usize, player: usize, msg: &'static str) -> bool {
        let identity = Some(SourceSender::new(THING_SOURCE, item as i32, Some(player)));
        let class = self.thing_class_cog.get(&item).map(|&i| i as usize);
        let capture = self.thing_capture_cog.get(&item).map(|&i| i as usize);
        let before = self.pending_messages.len();
        for instance in [class, capture].into_iter().flatten() {
            self.pending_messages.push((instance, msg, [0.0; 2], Some(0), identity));
        }
        for &(instance, link, target) in &self.thing_links {
            // Placed class instances are also registered as link 0 for the existing message bus.
            if target == item && !(link == 0 && Some(instance) == class)
                && self.link_hears(THING_SOURCE, instance, link, item, Some(player))
            {
                self.pending_messages.push((instance, msg, [0.0; 2], Some(link as i64), identity));
            }
        }
        self.pending_messages.len() != before
    }

    /// `sithCog_SectorSendMessage` (`0x4dfd60`): `sithCog_SectorSendMessageEx(sector, sender, msg,
    /// 0, 0, 0, 0)`.
    pub fn sith_cog_sector_send_message(
        &mut self,
        sector: usize,
        sender: Option<usize>,
        msg: &'static str,
    ) -> usize {
        self.sith_cog_send_message_ex(SECTOR_SOURCE, sector, sender, msg)
    }

    /// `sithCog_SendMessageEx` (`0x4dedc0`), the queue every world sender writes.
    ///
    /// The three senders differ only in their `sourceType` and the index they name, and each
    /// matching link carries its own `param0` — the **link id** that `GetSenderId()` answers — and
    /// its mask bit. The port's link tables have no mask, so a link hears every message sent to its
    /// target (`wire_links` records that deviation).
    fn sith_cog_send_message_ex(
        &mut self,
        source_type: i32,
        source: usize,
        sender: Option<usize>,
        msg: &'static str,
    ) -> usize {
        // The identity the engine passes down (`SourceSender`): its `sender`/`sender_type` are what
        // `GetSenderRef`/`GetSenderType` answer inside the handler.
        let identity = SourceSender::new(source_type, source as i32, sender);
        let links = match source_type {
            THING_SOURCE => self.thing_links.clone(),
            SECTOR_SOURCE => self.sector_links.clone(),
            _ => self.surface_links.clone(),
        };
        let mut sent = 0;
        for (instance, link, linked) in links {
            if linked != source || !self.link_hears(source_type, instance, link, source, sender) {
                continue;
            }
            self.pending_messages
                .push((instance, msg, [0.0; 2], Some(link as i64), Some(identity)));
            sent += 1;
        }
        sent
    }



    /// Which cog instances a level's own `cogs` section links to its things, surfaces and sectors.
    ///
    /// A level's `cogs` rows carry one value per non-local symbol, and the script's `symbols` section
    /// says what kind each one is: `surface 12`, `thing 5`, `sector 3` are indices into the level. The
    /// engine calls this `sithCog_LinkCogToSurface` / `_ToThing` / `_ToSector`; the port had the three
    /// tables declared, initialised empty and read, with nothing ever writing to them, so no message
    /// from the world reached a script. Fills them once, in table order.
    ///
    /// A `nolink` symbol is not linked (`sithCog_LinkCog`, `0x4df610`, skips link id -1), and a
    /// symbol's `mask=` is recorded in `link_masks` for `link_hears`.
    pub fn wire_links(
        &mut self,
        cogs: &[crate::jkl::CogInstance],
        scripts: &std::collections::HashMap<String, crate::cog::Cog>,
    ) {
        if !(self.thing_links.is_empty() && self.surface_links.is_empty() && self.sector_links.is_empty()) {
            return;
        }
        for (index, inst) in cogs.iter().enumerate() {
            let Some(script) = scripts.get(&inst.script) else { continue };
            let mut values = inst.values.iter();
            for symbol in &script.symbols {
                if symbol.local || symbol.kind == crate::cog::SymKind::Message {
                    continue;
                }
                let Some(raw) = values.next() else { break };
                let Ok(target) = raw.trim().parse::<usize>() else { continue };
                if symbol.nolink {
                    continue;
                }
                let link = symbol.linkid.unwrap_or(0) as i32;
                let source_type = match symbol.kind {
                    crate::cog::SymKind::Thing => THING_SOURCE,
                    crate::cog::SymKind::Surface => SURFACE_SOURCE,
                    _ => SECTOR_SOURCE,
                };
                if symbol.mask != 0x401 {
                    self.link_masks.insert((source_type, index, link, target), symbol.mask);
                }
                match symbol.kind {
                    crate::cog::SymKind::Thing => {
                        // `sithCog_LinkCog` (`0x4df610`) sets `SITH_TF_CAPTURED` (0x400) on the thing as
                        // it records the link, and that is the bit `sithRender_RenderThing` (`0x4c8070`)
                        // and `sithPlayerActions_Activate` (`0x4fc0a0`) test before a thing can answer.
                        if let Some(thing) = self.things.get_mut(target) {
                            thing.flags |= TF_CAPTURED;
                        }
                        self.thing_links.push((index, link, target));
                    }
                    crate::cog::SymKind::Surface => {
                        // `sithCog_LinkCog` (`0x4df610`): a surface link sets `SITH_SURFACE_COG_LINKED`
                        // (2) in the surface's own surfflags — the word `AttachThingToSurface`'s
                        // `entered` send tests. The shipped data usually carries the bit already; the
                        // OR is the engine's rule, not an observation of the JKL.
                        if let Some(flags) = self.surface_flags.get_mut(target) {
                            *flags |= SITH_SURFACE_COG_LINKED;
                        }
                        self.surface_links.push((index, link, target));
                    }
                    crate::cog::SymKind::Sector => self.sector_links.push((index, link, target)),
                    _ => {}
                }
            }
        }
    }

    /// The mask test in `sithCog_SectorSendMessageEx` (`0x4dfd90`) and its thing and surface twins:
    /// a link hears a message only when its mask has the sender's type bit (`1 << type`), or bit 0
    /// when the message has no sender.
    fn link_hears(&self, source_type: i32, instance: usize, link: i32, target: usize, sender: Option<usize>) -> bool {
        let bit = sender.map_or(1, |s| 1u32 << (self.types.get(s).copied().unwrap_or(0) as u32 & 31));
        let mask = self.link_masks.get(&(source_type, instance, link, target)).copied().unwrap_or(0x401);
        mask & bit != 0
    }

    /// `sithPlayerActions_Activate` (`0x4fc0a0`): the use key — for the player, and for an actor's
    /// `opendoors` instinct, which runs this same function on the actor's own body.
    ///
    /// The order is the engine's:
    ///
    /// 1. the 250 ms wait (`DAT_008554bc + 0xfa <= clock`) is stamped **before** the sweep, so a press
    ///    that finds nothing still starts it; with no master cog (`DAT_00832628 == 0`) there is no wait;
    /// 2. the eye and the look come off the body: its position (`+0xb8`), its orientation (`+0x88`)
    ///    pre-rotated by its own PYR (`+0x15c`) for an actor or the player, plus the `+0x168` eye
    ///    offset. The caller passes the port's equivalents because the player's `PlayerBody` carries
    ///    the eye height and the pitch that a `ThingState` does not;
    /// 3. `sithCollision_FindSectorInRadius(sector, eye, eye, 0)` (`0x4e71b0`) answers the sector it
    ///    is given for a body whose eye is inside it, and that sector is the sweep's start;
    /// 4. `sithCollision_SearchForCollisions(sector, body, eye, look, moveSize - (-0.1), 0.025, 2)`
    ///    (`0x4e8160`) sweeps a 0.025 sphere `moveSize + 0.1` along the look. Its collisions pop
    ///    nearest-first and the **first one that can answer ends the search**: a thing whose type is
    ///    neither a weapon (3) nor an item (5) and whose flags carry `SITH_TF_COGLINKED` (0x400) gets
    ///    message id 1 (`activated`) through `sithCog_ThingSendMessage(thing, body, 1)`; a surface
    ///    whose `+0x10` bit 2 is set gets it through `sithCog_SurfaceSendMessage(surface, body, 1)`.
    ///    Both halves rank together, because the engine pops one stack.
    ///
    /// `ponytail:` the hits are the port's `ThingSolid`/surface-centre geometry (a surface has no
    /// polygon at runtime), only linked targets are tested (the engine skips an unlinked hit too),
    /// and the looked-at test stands in for the sweep's direction — the reach is the engine's own.
    pub fn sith_player_actions_activate(
        &mut self,
        body: usize,
        eye: [f32; 3],
        dir: [f32; 3],
        reach: f32,
        sector: i32,
    ) -> Option<UseHit> {
        if self.master_cog.is_some() && self.time < self.last_activate + 0.25 {
            return None;
        }
        self.last_activate = self.time;
        let trace = std::env::var_os("BK_USE_TRACE").is_some();
        // `sithCog_LinkCog` (`0x4df610`) sets `surface + 0x10 |= 2` and `thing |= 0x400` as it records
        // a link, so the link tables are that flag: a scribed target is a candidate, nothing else is.
        let mut stack: Vec<(f32, UseHit)> = Vec::new();
        for (_, _, surface) in self.surface_links.clone() {
            let owner = self.surface_sector.get(surface).copied().unwrap_or(-1);
            if !self.in_use_search(sector, owner, eye, dir, reach) {
                continue;
            }
            let Some(centre) = self.surfaces.get(surface).map(|s| s.center) else { continue };
            let (dx, dy, dz) = (centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]);
            let flat = (dx * dx + dy * dy).sqrt();
            let dist = (flat * flat + dz * dz).sqrt();
            let dot = if flat > 0.01 {
                (dx / flat) * dir[0] + (dy / flat) * dir[1]
            } else {
                1.0
            };
            if trace {
                println!(
                    "use-candidate: surface {surface} sector {owner} centre {centre:?} dist {dist:.2} dot {dot:.2}"
                );
            }
            if dist <= reach && dot >= 0.3 {
                stack.push((dist, UseHit::Surface(surface)));
            }
        }
        for (_, _, target) in self.thing_links.clone() {
            // The engine's activate **skips a weapon (type 3) and an item (type 5)** hit: those are
            // fired or picked up, not used.
            if matches!(self.types.get(target).copied(), Some(3) | Some(5)) {
                continue;
            }
            let Some(thing) = self.things.get(target) else { continue };
            let (pos, thing_sector, flags) = (thing.pos, thing.sector, thing.flags);
            if flags & TF_CAPTURED == 0 || !self.in_use_search(sector, thing_sector, eye, dir, reach) {
                continue;
            }
            let (dx, dy, dz) = (pos[0] - eye[0], pos[1] - eye[1], pos[2] - eye[2]);
            let flat = (dx * dx + dy * dy).sqrt();
            let dist = (flat * flat + dz * dz).sqrt();
            let dot = if flat > 0.01 {
                (dx / flat) * dir[0] + (dy / flat) * dir[1]
            } else {
                1.0
            };
            if trace {
                println!(
                    "use-thing: thing {target} sector {thing_sector} (want {sector}) pos {pos:?} dist {dist:.2} dot {dot:.2}"
                );
            }
            if dist <= reach && dot >= 0.3 {
                stack.push((dist, UseHit::Thing(target)));
            }
        }
        stack.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (_, hit) = *stack.first()?;
        match hit {
            // The message id is 1 (`activated`), which resolves through the cog's message table; the
            // sender is the body, so `GetSenderRef()` names the thing that used it while
            // `GetSenderId()` stays the link id (`00_doorswitch.cog` branches on 1 and 2).
            UseHit::Thing(thing) => {
                self.sith_cog_thing_send_message(thing, Some(body), "activate");
            }
            UseHit::Surface(surface) => {
                self.sith_cog_surface_send_message(surface, Some(body), "activate");
            }
        }
        Some(hit)
    }

    /// Which instances are linked to `thing`, so a damage or death can message them.
    pub fn thing_instances(&self, thing: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .thing_links
            .iter()
            .filter(|(_, _, target)| *target == thing)
            .map(|(instance, _, _)| *instance)
            .collect();
        if let Some(instance) = self.thing_capture_cog.get(&thing).map(|i| *i as usize)
            && !out.contains(&instance)
        {
            out.push(instance);
        }
        out
    }

    /// `sithCog_ThingSendMessageEx` addressed at a thing's own scripts (`0x4dfaf0`). The engine
    /// messages `thing + 0x29c` and `thing + 0x2a0` (the thing's two script slots) plus every
    /// subscriber whose mask matches; for a placed `thing x` symbol that is the instance the cog
    /// section binds to it, which is what `thing_instances` resolves here. The sender id is the
    /// thing's own id (`thing + 4` in the engine), and both message parameters ride along for the
    /// receiver's `GetParam(0)`/`GetParam(1)`.
    ///
    /// Returns how many instances were queued.
    pub fn send_thing_message(
        &mut self,
        thing: usize,
        label: &'static str,
        params: [f64; 2],
    ) -> usize {
        let instances = self.thing_instances(thing);
        for instance in &instances {
            self.pending_messages
                .push((*instance, label, params, Some(thing as i64), None));
        }
        instances.len()
    }

    /// Which cog instances a move from one sector to another messages.
    ///
    /// `sithThing_EnterSector` sends `entered` to the sector's links and `sithThing_LeaveSector`
    /// sends `exited` (§16.10), each with the moving thing as the sender, so a link's mask decides
    /// whether that body's type is heard (`link_hears`).
    pub fn sector_messages(
        &self,
        old: Option<usize>,
        new: Option<usize>,
        thing: Option<usize>,
    ) -> Vec<(usize, &'static str, i32)> {
        // `sithThing_EnterSector` (`0x4cd2c0`) is narrower than "every link hears it": it messages the
        // sector only when the sector's byte at `+0x34` carries bit `4` (`SITH_SECTOR_COGLINKED`) and
        // the thing's flags carry none of `0x80100` — a sector the level's scripts watch, entered by a
        // body they care about. Sending to every linked sector ran `06_endpart1.cog`'s goal sequence
        // from a walk leg whose spawn merely *sat* in the sector, and the sequence's own `sleep`s
        // ended the level mid-capture.
        if thing.is_some_and(|t| self.things.get(t).is_some_and(|s| s.flags & 0x80100 != 0)) {
            return Vec::new();
        }
        let watched = |sector: usize| {
            self.sector_flags.get(sector).is_some_and(|flags| flags & 0x04 != 0)
        };
        let mut out = Vec::new();
        for (instance, link, sector) in &self.sector_links {
            if !watched(*sector) || !self.link_hears(SECTOR_SOURCE, *instance, *link, *sector, thing) {
                continue;
            }
            if Some(*sector) == new {
                // `sithThing_EnterSector` sends `entered` with the **link id as the sender id**:
                // `00_SecretAreas.cog`'s handler is `flags = flags | GetSenderId()`, so a message
                // with sender 0 loses the bit (round 198: retail `flags` 0 -> 2, the port stayed 0).
                out.push((*instance, "entered", *link));
            } else if Some(*sector) == old {
                out.push((*instance, "exited", *link));
            }
        }
        out
    }

    /// `sithRender_RenderThing` (`0x4c8070`): the first time the renderer draws a **thing**.
    ///
    /// `SITH_TF_INCAMFOV` (`0x100000`) is latched, and a thing whose flags carry `SITH_TF_CAPTURED`
    /// (`0x400` — the bit `sithCog_LinkCog`, `0x4df610`, sets when a cog is placed on it) is sent
    /// message id 9 (`sighted`) through `sithCog_ThingSendMessage(thing, 0, 9)`: sender 0, i.e. the
    /// world, which is what `GetSenderId()` reads as 0. Level 1's goal is exactly this —
    /// `01_goal0.cog 204` puts the script on 8t88, so *seeing him* completes "Find 8t88 before he
    /// escapes Nar Shaddaa". The same pass wakes a sleeping actor in the engine, which this port has no
    /// sleep state for.
    pub fn thing_sighted(&mut self, thing: usize) -> usize {
        let Some(t) = self.things.get_mut(thing) else { return 0 };
        if t.flags & TF_INCAMFOV != 0 {
            return 0;
        }
        t.flags |= TF_INCAMFOV;
        if t.flags & TF_CAPTURED == 0 {
            return 0;
        }
        self.sith_cog_thing_send_message(thing, None, "sighted")
    }

    /// `sithRender_BuildVisibleSectorList` (`0x4c6650`): the first time the renderer draws a sector.
    ///
    /// `SITH_SECTOR_SEEN` (`0x4000`) is latched, and a sector whose flags carry
    /// `SITH_SECTOR_COGLINKED` (`0x4` — the bit `sithCog_LinkCog` sets from a `sector` symbol) is sent
    /// message id 9 (`sighted`) through `sithCog_SectorSendMessage(sector, 0, 9)`, again from the
    /// world. That is the other half of level 1's goal: `01_goal0.cog` also waits on the room.
    pub fn sector_sighted(&mut self, sector: usize) -> usize {
        let Some(flags) = self.sector_flags.get_mut(sector) else { return 0 };
        if *flags & SECTOR_SEEN != 0 {
            return 0;
        }
        *flags |= SECTOR_SEEN;
        if *flags & SECTOR_COGLINKED == 0 {
            return 0;
        }
        self.sith_cog_sector_send_message(sector, None, "sighted")
    }

    /// The next track in the current range, wrapping to the range's start: what `stdMci_Play(from,
    /// to)` does when a track ends. Returns the new track, or `None` when nothing is playing.
    pub fn advance_music(&mut self) -> Option<i32> {
        let (from, to, start) = self.music?;
        let next = if start < to { start + 1 } else { from };
        self.music = Some((from, to, next));
        Some(next)
    }

    /// Give a thing the life its template's `timer=` asks for, if it has one.
    ///
    /// `THINGPARAM_TIMER` (`sithThing.c:1777`) stores `lifeLeftSec * 1000` in `msecLifeLeft`, and
    /// `sithThing_Update` counts it down and destroys the thing at zero. Every created thing gets
    /// this: an impact spark ships `timer=0.5` and a corpse's script calls `SetLifeLeft(_, 30)`,
    /// so without it both would stay in the world forever.
    fn arm_life(&mut self, index: usize) {
        let Some(template) = self.templates.get(index).map(|t| t.to_ascii_lowercase()) else { return };
        if let Some(secs) = self.template_timer.get(&template).copied() {
            self.life_left.insert(index, secs);
            self.life_total.insert(index, secs);
        }
    }

    /// The cel a sprite thing draws, out of `cels`.
    ///
    /// `sithExplosion_CreateThing` animates an explosion's sprite only when its template's
    /// `typeflags` carry `SITHEXPLOSION_FLAG_ANIMATED_SPRITE` (0x1), and at
    /// `num_texinfo / msecLifeLeft * 1000` frames per second — one cel per `life / cels` seconds,
    /// so frame `k` is up for the k-th `life / cels` slice of the thing's life. `+laserhit` is
    /// `typeflags=0x33 timer=0.5 sprite=bryx.spr`, i.e. eight frames over half a second.
    pub fn sprite_cel(&self, index: usize, cels: u32) -> u32 {
        let template = self.templates.get(index).map(|t| t.to_ascii_lowercase());
        if cels < 2 || !template.is_some_and(|t| self.template_sprite_anim.contains(&t)) {
            return 0;
        }
        let Some(total) = self.life_total.get(&index).copied().filter(|t| *t > 0.0) else {
            return 0;
        };
        let left = self.life_left.get(&index).copied().unwrap_or(0.0);
        let progress = ((total - left) / total).clamp(0.0, 1.0);
        ((progress * cels as f32) as u32).min(cels - 1)
    }

    /// What a projectile leaves behind: the `explode=` template, spawned where it landed.
    ///
    /// `sithExplosion` creates the template at the impact point and that thing's own `damage=`
    /// plus `size=` decide who is hurt, which is why an explosion is a thing and not a special
    /// case. Returns the new thing's index. A weapon with no `explode=` leaves nothing.
    pub fn explode(&mut self, projectile: usize) -> Option<usize> {
        let template = self.templates.get(projectile)?.clone().to_ascii_lowercase();
        let explode = self.template_explode.get(&template)?.clone();
        let at = self.things.get(projectile)?.pos;
        let sector = self.things.get(projectile)?.sector;
        let health = self.template_health.get(&explode).copied().unwrap_or(1.0);
        let damage = self.template_damage.get(&explode).copied().unwrap_or(0.0);
        let kind = self.template_types.get(&explode).copied().unwrap_or(3);
        self.things.push(ThingState {
            pos: at,
            next_touch_ms: 0,
            insert: [0.0; 3],
            owner: None,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            head_pitch: 0.0,
            sector,
            flags: 0,
            health,
            frame: 0.0,
            target: None,
            frames: Vec::new(),
            destroyed: false,
            vel: [0.0; 3],
            damage,
            min_damage: damage,
            decay: 0.0,
            // An explosion thing starts with its template's `physflags=` like any created thing.
            phys_flags: self.template_phys_flags.get(&explode).copied().unwrap_or(0),
            jk_flags: 0,
            attach: None,
            floor_attach: None,
            attach_offset: [0.0; 3],
            attach_flags: 0,
            spin: None,
            rot_vel: [0.0; 3],
            paused: false,
            pulse: 0.0,
            pulse_at: 0.0,
            light: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
            extra_speed: 0.0,
            ai_target: None,
            ai_wait: 0.0,
            ai_burst: 0.0,
            arrived: false,
        });
        let index = self.things.len() - 1;
        self.templates.push(explode.clone());
        self.types.push(kind);
        self.arm_life(index);
        // `sithExplosion_MakeBlast` (`0x4fb920`) raises kind **1** at strength **3.0** (`0x40400000`)
        // at the blast, with the blast thing as the source — a grenade is heard from further than a
        // rifle shot.
        self.transmit_awareness(sector, 1, at, AI_AWARE_BLAST, Some(index));
        Some(index)
    }

    /// Everything an explosion at `at` hurts: actors and the player inside its body radius.
    pub fn explode_hits(&self, explosion: usize) -> Vec<usize> {
        let Some(blast) = self.things.get(explosion) else { return Vec::new() };
        let template = self.templates.get(explosion).cloned().unwrap_or_default().to_ascii_lowercase();
        let (radius, height) = self.template_body.get(&template).copied().unwrap_or((1.0, 2.0));
        let player = self.player_thing();
        let mut out = Vec::new();
        for (index, thing) in self.things.iter().enumerate() {
            let armed = self.types.get(index).copied() == Some(2) || index == player;
            if !armed || index == explosion || thing.destroyed {
                continue;
            }
            let (dx, dy) = (thing.pos[0] - blast.pos[0], thing.pos[1] - blast.pos[1]);
            if (dx * dx + dy * dy).sqrt() > radius {
                continue;
            }
            if thing.pos[2] + height < blast.pos[2] || thing.pos[2] > blast.pos[2] + height {
                continue;
            }
            out.push(index);
        }
        out
    }

    /// Damage the **player**: `sithActor_DamageActor`'s subtraction, with the shield bar standing
    /// in front of health the way the shipped `item_shields.cog` does it.
    ///
    /// The engine does not absorb shield-first in C: it broadcasts `damaged` to the player's
    /// inventory item cogs whose `items.dat` flags carry `0x10` (`sithInventory_BroadcastMessage`,
    /// `0x4d24a0`) and applies the amount the handler returns. Bin 60's script (`item_shields.cog`)
    /// owns the bar: it spends the whole hit on it, stops a hit while the bar is above 50, and lets
    /// health take a share that grows as the bar falls (see [`Self::shield_script`], which is that
    /// script's arithmetic). The runtime's bar is a field rather than a VM, so the script's branch is
    /// ported here; the draw and sound half of the script is not (ledger).
    pub fn damage_player(&mut self, amount: f32, kind: u32) -> (f32, f32) {
        let player = self.player_thing();
        // The player goes through the same guard as any actor (`sithActor_DamageActor` takes the
        // player branch too), so a scripted invulnerability covers bolts as well as everything else.
        if self.invulnerable(player, kind) {
            if std::env::var_os("BK_HIT_TRACE").is_some() {
                eprintln!(
                    "hit: {amount:.0} ignored -- player thing {player} is invulnerable (flags {:#x})",
                    self.actor_flags.get(&player).copied().unwrap_or(0)
                );
            }
            return (
                self.player_shields,
                self.things.get(player).map(|t| t.health).unwrap_or(0.0),
            );
        }
        // `sithActor_DamageActor` tints the local player's palette on damage that lands, before the
        // shields are spent: its call is `sithPlayer_AddDynamicTint(damage * 0.04, 0, 0)`.
        add_dynamic_tint(&mut self.tint, [amount * DAMAGE_TINT, 0.0, 0.0]);
        let before_shields = self.player_shields;
        let before_health = self.things.get(player).map(|t| t.health).unwrap_or(0.0);
        let before = before_shields + before_health;
        // `item_shields.cog` has branches for energy (2), fire (4) and saber (16) only; any other
        // kind reaches health with the bar untouched, exactly as the script does.
        let mut left = amount;
        if self.player_shields > 0.0
            && matches!(kind, DAMAGE_ENERGY | DAMAGE_FIRE | DAMAGE_SABER)
        {
            let (health_part, shields_left) =
                Self::shield_script(kind, self.player_shields, amount);
            self.player_shields = shields_left;
            left = health_part;
        }
        if let Some(thing) = self.things.get_mut(player) {
            thing.health = (thing.health - left).max(0.0);
            if thing.health <= 0.0 {
                thing.destroyed = true;
                self.player_dead = true;
            }
        }
        if self.encounter_events_enabled {
            let health = self.things.get(player).map(|t| t.health).unwrap_or(0.0);
            // The retail capture writes the amount the bars actually lost, net of a recharge, so
            // the row is the same number on both sides. The split is what one hit took from each
            // bar; health clamps at zero, so the delta is measured against the clamped value.
            let amount = before - (self.player_shields + health);
            self.encounter_events.push(EncounterEvent::Hit {
                t: self.time,
                amount,
                shields: self.player_shields,
                health,
                health_lost: before_health - health,
                shields_lost: before_shields - self.player_shields,
            });
        }
        (self.player_shields, self.things.get(player).map(|t| t.health).unwrap_or(0.0))
    }

    /// `item_shields.cog`'s `damaged` arithmetic for one protected hit: `(health damage, bar after)`.
    ///
    /// Energy and fire share the branch this mirrors, saber halves the hit first. The bar takes the
    /// whole hit; a bar above **50** stops it completely; a hit that crosses 50 loses what it
    /// overshot; and the rest is scaled by `1 - shieldsLeft / 50` on the script's raw `shieldsLeft`.
    /// That factor is **above 1** when the hit spent the bar (the script keeps the negative value),
    /// which is why the door fixture's health loss per hit grows as the bar empties.
    fn shield_script(kind: u32, shields: f32, damage: f32) -> (f32, f32) {
        let damage = if kind == DAMAGE_SABER { damage * 0.5 } else { damage };
        let shields_left = shields - damage;
        let mut health = if shields_left > 50.0 {
            0.0
        } else if shields > 50.0 {
            damage - (shields - 50.0)
        } else {
            damage
        };
        if health > 0.0 {
            health *= 1.0 - shields_left / 50.0;
        }
        // `damage = damage + halfdamage` puts the saber's other half back before it lands.
        if kind == DAMAGE_SABER {
            health += damage;
        }
        // `SetInv(player, 60, shieldsLeft)` clamps at the item's own minimum, 0.
        (health, shields_left.max(0.0))
    }

    /// Move the player's body to `pos` and publish the runtime's own pose at once. The one entry
    /// point for a warp: the level's start, a scripted placement, or a gate pinning the body. It
    /// does not change the facing or the eye height; the caller sets those on the body first.
    pub fn teleport_player(&mut self, body: &mut PlayerBody, pos: [f32; 3], sector: Option<usize>) {
        let previous = body.sector;
        body.pos = Vec3::from(pos);
        body.vy = 0.0;
        body.vel = [0.0; 2];
        body.grounded = true;
        if let Some(sector) = sector {
            body.sector = sector;
        }
        body.attach = None;
        body.published_vel = [0.0; 3];
        self.publish_player(body);
        if let Some(player) = self.local_player() {
            self.things[player].vel = [0.0; 3];
            body.published_pose = Some((self.things[player].pos, body.sector));
        }
        if previous != body.sector && let Some(player) = self.local_player() {
            let messages = self.sector_messages(Some(previous), None, Some(player));
            for (instance, label, link) in messages {
                self.pending_messages.push((instance, label, [0.0; 2], Some(link as i64),
                    Some(SourceSender::new(SECTOR_SOURCE, previous as i32, Some(player)))));
            }
            self.queue_player_sector_entry(player, body.sector);
        }
    }

    /// Explicit placement enters a sector without reconstructing a movement segment.
    pub fn queue_player_sector_entry(&mut self, player: usize, sector: usize) {
        if self.things.get(player).is_some_and(|t| t.flags & 0x80100 == 0)
            && self.sector_flags.get(sector).is_some_and(|f| f & 4 != 0)
        {
            self.sith_cog_sector_send_message(sector, Some(player), "entered");
        }
    }

    /// Put the player back on its feet at the level's own spawn: full health and shields, the
    /// death flag cleared, and the body reset to the spawn pose. The engine reloads or respawns at
    /// the last checkpoint; this build has no checkpoint list yet, so it is the level's
    /// `walkplayer` position.
    pub fn respawn_player(
        &mut self,
        body: &mut PlayerBody,
        spawn: [f32; 3],
        sector: Option<usize>,
    ) -> (f32, f32) {
        let player = self.player_thing();
        let health = self
            .templates
            .get(player)
            .and_then(|name| self.template_health.get(name))
            .copied()
            .unwrap_or(100.0);
        if let Some(thing) = self.things.get_mut(player) {
            thing.health = health;
            thing.destroyed = false;
            thing.vel = [0.0; 3];
        }
        let previous = body.sector;
        self.teleport_player(body, spawn, sector);
        if previous == body.sector {
            self.queue_player_sector_entry(player, body.sector);
        }
        body.fall_killed = false;
        body.fall_fade = 1.0;
        self.player_shields = 100.0;
        self.player_dead = false;
        (self.player_shields, health)
    }

    /// The radius a projectile carries into its hit test: its template's own `movesize=` — the
    /// engine's `sithCollision_MoveThing` sweeps the **moving** thing by its own size, and
    /// `sithIntersect_CheckSphereIntersection` tests `targetRadius + movingRadius`. `+bryarbolt`
    /// (and every bolt that inherits it, including `+elaser`) ships `movesize=0.001`, so the
    /// combined reach is the target's body plus a thousandth of a unit.
    fn shot_radius(&self, shot: usize) -> f32 {
        self.templates
            .get(shot)
            .map(|name| name.to_ascii_lowercase())
            .and_then(|name| self.template_hit.get(&name).map(|(radius, _)| *radius))
            .unwrap_or(0.001)
    }

    /// Whether a shot at `shot` is inside a target's body.
    ///
    /// A thing's position is its **feet**, so testing the distance to it misses a bolt fired at
    /// eye height: the player's eye is about 1.4 units up and a placed actor's origin is on the
    /// floor, so a 0.5-unit point test never landed on anything that was placed in the level.
    /// The body comes from the template's `size=x/y/z` (half-extents) or `radius=`, so the test is
    /// "inside the cylinder the level's own template describes".
    /// `sithCollision_SearchForCollisions` on a projectile: the shot's own **move** is what is
    /// tested, so a fast bolt cannot step over a body between frames. `dt` is the frame it moved in;
    /// zero keeps the sample-at-the-end behaviour the callers of [`Self::shot_hits`] rely on.
    pub fn shot_hits_swept(&self, shot: usize, target: usize, dt: f32) -> bool {
        self.shot_hit_fraction(shot, target, dt).is_some()
    }

    /// Where along this frame's move the shot first touches `target`, as a 0..1 fraction of `dt`;
    /// `None` when it does not. [`Self::shot_hits_swept`] is this test's yes/no form. An
    /// impact-time damage correction keyed on the fraction was measured against the fixture and
    /// overshoots it (`encounter-gate.ps1`), so the damage stays the projectile's own value.
    pub fn shot_hit_fraction(&self, shot: usize, target: usize, dt: f32) -> Option<f32> {
        if dt <= 0.0 {
            return self.shot_hits(shot, target).then_some(1.0);
        }
        let (Some(a), Some(b)) = (self.things.get(shot), self.things.get(target)) else {
            return None;
        };
        let name = self.templates.get(target).cloned().unwrap_or_default().to_ascii_lowercase();
        let (radius, height) = self.template_hit.get(&name).copied().unwrap_or((0.065, 0.18));
        let bolt = self.shot_radius(shot);
        let from = [
            a.pos[0] - a.vel[0] * dt,
            a.pos[1] - a.vel[1] * dt,
            a.pos[2] - a.vel[2] * dt,
        ];
        // The engine sweeps the projectile's whole step (`sithCollision_MoveThing`), so the hit
        // test is the **3D distance between the bolt's step segment and the body's axis
        // segment** — not a plan sweep with an instantaneous z test. A descending (lob) bolt
        // has no plan motion at all, and the plan-sweep form took its `len2 == 0` default (the
        // segment's far endpoint), skipping the body's z-band entirely (#93's encounter
        // regression: bolts passing 0.024 units away with no hit).
        let (t_bolt, t_axis) = closest_segment_parameters(
            from,
            a.pos,
            [b.pos[0], b.pos[1], b.pos[2]],
            [b.pos[0], b.pos[1], b.pos[2] + height],
        );
        let (px, py, pz) = (
            from[0] + (a.pos[0] - from[0]) * t_bolt,
            from[1] + (a.pos[1] - from[1]) * t_bolt,
            from[2] + (a.pos[2] - from[2]) * t_bolt,
        );
        let qz = b.pos[2] + height * t_axis;
        let dist = ((px - b.pos[0]).powi(2) + (py - b.pos[1]).powi(2) + (pz - qz).powi(2)).sqrt();
        if std::env::var_os("BK_SHOT_TRACE").is_some() {
            eprintln!(
                "shot trace: shot {shot} -> target {target} ({name}) from {from:?} to {:?} closest 3d {dist:.3} (limit {:.3}) at bolt t {t_bolt:.2}",
                a.pos,
                radius + bolt,
            );
        }
        (dist <= radius + bolt).then_some(t_bolt)
    }

    pub fn shot_hits(&self, shot: usize, target: usize) -> bool {
        let (Some(a), Some(b)) = (self.things.get(shot), self.things.get(target)) else {
            return false;
        };
        // The target's body, in the engine's own terms: its `movesize` radius and the model height
        // that implies. A missing template keeps the old cylinder so a script-spawned thing is still
        // hittable.
        let name = self.templates.get(target).cloned().unwrap_or_default().to_ascii_lowercase();
        let (radius, height) = self.template_hit.get(&name).copied().unwrap_or((0.065, 0.18));
        // The two spheres touch when the distance is at most the sum of their radii.
        let bolt = self.shot_radius(shot);
        let (dx, dy) = (a.pos[0] - b.pos[0], a.pos[1] - b.pos[1]);
        let flat = (dx * dx + dy * dy).sqrt();
        if flat > radius + bolt {
            return false;
        }
        let low = b.pos[2];
        let high = b.pos[2] + height;
        a.pos[2] >= low - bolt && a.pos[2] <= high + bolt
    }

    /// `DamageThing` as a call, for code that holds no `Value`.
    pub fn damage(&mut self, target: i64, amount: f32) -> f32 {
        self.damage_typed(target, amount, DAMAGE_IMPACT)
    }

    /// Damage of a known **kind**: the engine's word is a bitmask (`SithDamageType`, `types_enums.h`)
    /// — impact 0x1, energy 0x2, fire 0x4, force 0x8, saber 0x10, drown 0x20, fall 0x40.
    ///
    /// `sithActor_DamageActor` (`JK.EXE:004ecc90`) checks `actor->flags & SITH_AF_INVULNERABLE` before
    /// anything else and returns without damage **unless** the type is a fall, so scripted
    /// invulnerability does not save anyone from a long drop. `01_endlevel.cog` sets that flag while it
    /// freezes Kyle for the level-end fade.
    pub fn damage_typed(&mut self, target: i64, amount: f32, kind: u32) -> f32 {
        self.damage_typed_from(target, amount, kind, 0)
    }

    /// Damage with the thing that caused it. `sithActor_DamageActor`'s message carries the attacker
    /// as the message's sender, which is how a `killed:` handler's `GetSourceRef()` tells the player's
    /// kills from the world's: the actor scripts credit alignment bin 72 only when
    /// `player == GetSourceRef()`, so a message without a sender can never credit a kill. `source` is
    /// a thing index, or 0 for "the world did it" (falls, scripts).
    pub fn damage_typed_from(&mut self, target: i64, amount: f32, kind: u32, source: i64) -> f32 {
        let Ok(index) = usize::try_from(target) else { return 0.0 };
        if self.invulnerable(index, kind) {
            return self.things.get(index).map(|t| t.health).unwrap_or(0.0);
        }
        // Actor-versus-actor damage is a tenth: `sithActor_DamageActor` scales it when
        // `GetThingParent(sender)` is an actor without `AF_ACTOR_FULL_DAMAGE` and the victim is an
        // actor too, with drowning and falls exempt. Applied before the AI event, the messages and the
        // subtraction, since the engine's whole function works on the scaled number.
        let mut amount = amount;
        if kind != 0x20
            && kind != DAMAGE_FALL
            && self.types.get(index).copied() == Some(2)
            && source > 0
            && usize::try_from(source).is_ok_and(|attacker| {
                self.types.get(attacker).copied() == Some(2)
                    && self.actor_flags.get(&attacker).copied().unwrap_or(0) & AF_ACTOR_FULL_DAMAGE == 0
            })
        {
            amount *= 0.1;
        }
        // `sithActor_DamageActor` (`0x4ecc90`) raises AI **event 1** with the attacker *before* it
        // subtracts the health (`if (param_2 != param_1 && thing->type == 2)
        // sithAI_EmitEvent(thing->ai, 1, param_2)`); `dodge` steps away and `sensedanger` plays its
        // surprise and turns on whoever hit it.
        if source > 0
            && self.types.get(index).copied() == Some(2)
            && let Ok(attacker) = usize::try_from(source)
            && attacker != index
        {
            self.emit_ai_event(index, 1, Some(attacker));
        }
        // The player's palette flashes red on every hit that lands, whichever path applied it.
        if index == self.player_thing() {
            add_dynamic_tint(&mut self.tint, [amount * DAMAGE_TINT, 0.0, 0.0]);
            // `BK_AI_TRACE`: the HUD comparison (retail's series reads 100/50 undamaged) needs a
            // timestamped record of what actually landed, not just shots spawned.
            if std::env::var_os("BK_AI_TRACE").is_some() {
                let left = self.things.get(index).map(|t| t.health).unwrap_or(0.0);
                eprintln!("ai: player takes {amount:.1} from {source} (time {:.2} s, {left:.0} left)", self.time);
            }
        }
        let mut died = false;
        // A hit flinches: `sithPuppet_SetMode` plays the puppet's `hit` track off a damage message,
        // and 127 of level 1's 128 actors ship one.
        self.last_hit.insert(index, self.time);
        let left = if let Some(t) = self.things.get_mut(index) {
            t.health -= amount;
            if t.health <= 0.0 {
                t.health = 0.0;
                t.destroyed = true;
                died = true;
            }
            t.health
        } else {
            return 0.0;
        };
        // `sithActor_KillActor` (`0x4ecfe0`) raises the awareness event **before** it messages the
        // scripts: kind 0, strength 5.0 (`0x40a00000`), at the victim, with the killer as the source.
        if died {
            if let Some(t) = self.things.get(index) {
                let (sector, at) = (t.sector, t.pos);
                let killer = usize::try_from(source).ok().filter(|k| *k != index);
                self.transmit_awareness(sector, 0, at, AI_AWARE_KILL, killer);
            }
        }
        // `SITH_MESSAGE_DAMAGED` (10) and `SITH_MESSAGE_KILLED` (12) go to the cogs linked to the
        // thing that was hit; the level's scripts wait on both (level 1: `damaged` x4, `killed` x5).
        // The params are the engine's: the amount and the damage-type word, which is what a handler's
        // `GetParam(0)`/`GetParam(1)` read (`force_absorb.cog`, `force_protection.cog`).
        let params = [amount as f64, kind as f64];
        for instance in self.thing_instances(index) {
            self.pending_messages
                .push((instance, "damaged", params, Some(source), None));
            if died {
                self.pending_messages
                    .push((instance, "killed", params, Some(source), None));
            }
        }
        // What a handler's answer has to amend: the engine applies the amount the `damaged` handler
        // returns, not the one it was handed.
        self.damage_applied.insert(index, amount);
        // `sithActor_PlayDamageSoundFx` (`0x4ece90`), the same function's tail: a body that survived the
        // blow plays its own class's row for the damage kind. A lethal blow returns from
        // `sithActor_DamageActor` through `sithActor_KillActor` before this, so no hurt sound.
        if left > 0.0
            && !died
            && amount >= DAMAGE_SOUND_MIN
            && let Some(t) = self.things.get(index)
        {
            let volume = (amount / left * DAMAGE_SOUND_SCALE).clamp(0.0, 1.0);
            if volume >= DAMAGE_SOUND_FLOOR {
                let at = t.pos;
                self.queue_mode_sound_at(index, Self::hurt_mode(kind), at, volume);
            }
        }
        left
    }

    /// Apply a `damaged` handler's answer (`ReturnEx`): the engine's `sithCog_ThingSendMessage`
    /// replaces the amount with the handler's value, so the difference between what was applied and
    /// what the script allows is given back.
    ///
    /// Returns `(applied, allowed)` for a log line, or `None` when the answer changed nothing.
    pub fn amend_damage(&mut self, index: usize, allowed: f32) -> Option<(f32, f32)> {
        let applied = self.damage_applied.remove(&index)?;
        if (allowed - applied).abs() < 1e-4 {
            return None;
        }
        // Only give health back: a handler asking for *more* than was applied is not a rule the
        // engine's path can express (the damage already happened), and the scripts never do it.
        let refund = (applied - allowed).max(0.0);
        if refund > 0.0
            && let Some(t) = self.things.get_mut(index)
        {
            t.health = (t.health + refund).max(0.0);
            // A handler that softens a blow can keep a thing alive; the engine decides death after
            // the handler, so a thing whose health is back above zero is not dead.
            if t.health > 0.0 {
                t.destroyed = false;
            }
        }
        Some((applied, allowed))
    }

    /// An actor's own frame list: `frame=(x/y/z:…)` entries the level places on the thing, indexed from
    /// 0. `AISetMoveFrame`/`AISetLookFrame` walk to one of these, and `MoveToFrame` uses the same list.
    pub fn frame_of(&self, thing: usize, frame: i64) -> Option<[f32; 3]> {
        let frames = self.things.get(thing)?.frames.get(usize::try_from(frame).ok()?)?;
        Some([frames[0], frames[1], frames[2]])
    }

    /// The mode a surface is drawn with: the script's `SetFaceGeoMode` override, else the level's own
    /// `geo` column. Untouched faces are **not** 0 — `09_fuelexplode.cog` reads
    /// `GetFaceGeoMode(GetSenderRef()) == 0` to bail out of a fuel-pipe chain that is already gone, so
    /// answering 0 for every face retired that whole level's set piece.
    pub fn face_geomode_at(&self, surface: usize) -> i32 {
        self.face_geomode
            .get(&surface)
            .copied()
            .unwrap_or_else(|| self.surface_geomode.get(surface).copied().unwrap_or(0))
    }

    /// The `.snd` event a damage kind cries out with, from the engine's canonical mode table: the
    /// pointer array at `0x54b40c` that `sithSoundClass_Startup` fills into its hashtable, entry *n*
    /// = mode `n` (entry `0x57` is `surprise`, which the AI path already used).
    /// `sithActor_PlayDamageSoundFx` switches on the kind word itself, so a combined word (`0x3`)
    /// takes the default `hurtimpact`.
    fn hurt_mode(kind: u32) -> &'static str {
        match kind {
            DAMAGE_ENERGY => "hurtenergy", // mode 0x39
            0x04 => "hurtfire",            // 0x3a, `DAMAGE_FIRE`
            0x08 => "hurtmagic",           // 0x3b, `DAMAGE_FORCE`
            DAMAGE_SABER => "hurtspecial", // 0x3c
            0x20 => "drowning",            // 0x3d, `DAMAGE_DROWN`
            _ => "hurtimpact",             // 0x38, the default arm (impact, fall, combined)
        }
    }

    /// `actor->flags & SITH_AF_INVULNERABLE` with the engine's fall exception.
    fn invulnerable(&self, index: usize, kind: u32) -> bool {
        kind != DAMAGE_FALL
            && self.actor_flags.get(&index).copied().unwrap_or(0) & AF_INVULNERABLE != 0
    }

    /// Report the launch of a shot that just landed on the player. Retail's door fixture can only
    /// observe shots through their impact (its `hit` rows), so the port's `shot` row is the same
    /// shot's launch; the gate compares the two streams nearest-time, a flight apart. A shot the
    /// player's invulnerability swallowed is not reported: retail's fixture has no such row.
    pub fn note_shot_landed(&mut self, shot: usize) {
        if !self.encounter_events_enabled {
            return;
        }
        let Some((t, target)) = self.encounter_shot_time.remove(&shot) else { return };
        let Some(thing) = self.things.get(shot) else { return };
        let owner = thing.owner.unwrap_or(shot);
        let template = self.templates.get(shot).cloned().unwrap_or_default();
        let pos = thing.pos;
        self.encounter_events
            .push(EncounterEvent::ShotLanded { t, owner, template, pos, target });
    }

    /// Report a shot that died without landing on the player — a wall, an actor, or its own `timer=`.
    /// The chronology then says hit or miss for every shot; retail's fixture can only see impacts,
    /// so these rows have no counterpart and the gate does not pair them.
    pub fn note_shot_lost(&mut self, shot: usize) {
        if !self.encounter_events_enabled {
            return;
        }
        let Some((t, target)) = self.encounter_shot_time.remove(&shot) else { return };
        let Some(thing) = self.things.get(shot) else { return };
        let owner = thing.owner.unwrap_or(shot);
        let template = self.templates.get(shot).cloned().unwrap_or_default();
        self.encounter_events.push(EncounterEvent::Miss { t, owner, template, target });
    }

    /// The damage this template deals (`damage=`), for a projectile that hits.
    /// How much damage a *flying* projectile still carries.
    ///
    /// The engine's `sithWeapon_Update` decays it: when the weapon's typeflags carry
    /// `SITH_WF_DAMAGE_DECAY` (0x4000), `damage -= rate * dt`, floored at `mindamage`.
    /// `+bryarbolt` is `damage=30 mindamage=10 rate=15 typeflags=0x20440d`, so a bolt loses
    /// 15 points a second and is down to its floor after (30-10)/15 = 1.33 s of flight —
    /// 5.3 units at its `vel=(0/4/0)`.
    pub fn damage_of(&self, template: &str) -> f32 {
        self.template_damage
            .get(&template.to_ascii_lowercase())
            .copied()
            .unwrap_or(0.0)
    }

    /// The `SithDamageType` a template deals, from its `damageclass=` chain. A template with no
    /// word falls back to impact, the engine's default arm.
    pub fn damage_kind_of(&self, template: &str) -> u32 {
        self.template_damageclass
            .get(&template.to_ascii_lowercase())
            .copied()
            .unwrap_or(DAMAGE_IMPACT)
    }

    /// The kind of hurt a shot thing deals, so a hit uses its own template's `damageclass=`
    /// (energy for `+bryarbolt`, fire for `+grenade2`, impact for `08tiebomb`).
    pub fn shot_damage_kind(&self, shot: usize) -> u32 {
        self.templates
            .get(shot)
            .map(|template| self.damage_kind_of(template))
            .unwrap_or(DAMAGE_IMPACT)
    }

    /// First thing index in `sector`, or -1: `FirstThingInSector`.
    fn first_in_sector(&self, sector: i32) -> i64 {
        self.things
            .iter()
            .position(|t| t.sector == sector)
            .map(|i| i as i64)
            .unwrap_or(-1)
    }

    /// The next thing index in `sector` after `after`, or -1: `NextThingInSector`.
    fn next_in_sector(&self, sector: i32, after: i64) -> i64 {
        let start = usize::try_from(after).map(|a| a + 1).unwrap_or(0);
        self.things
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, t)| t.sector == sector)
            .map(|(i, _)| i as i64)
            .unwrap_or(-1)
    }

    fn surface(&self, i: i64) -> Option<&SurfaceState> {
        usize::try_from(i).ok().and_then(|i| self.surfaces.get(i))
    }

    fn surface_mut(&mut self, i: i64) -> Option<&mut SurfaceState> {
        usize::try_from(i).ok().and_then(|i| self.surfaces.get_mut(i))
    }

    fn thing(&self, i: i64) -> Option<&ThingState> {
        usize::try_from(i).ok().and_then(|i| self.things.get(i))
    }

    fn thing_mut(&mut self, i: i64) -> Option<&mut ThingState> {
        usize::try_from(i).ok().and_then(|i| self.things.get_mut(i))
    }

    fn sector_mut(&mut self, i: i64) -> Option<&mut SectorState> {
        usize::try_from(i).ok().and_then(|i| self.sectors.get_mut(i))
    }

    /// The thing the camera rides: the level's first `walkplayer`, which is the engine's own rule
    /// (`sithPlayer_Initialize` takes the first thing of type PLAYER).
    pub fn player_thing(&self) -> usize {
        self.local_player().unwrap_or(0)
    }

    /// `jkPlayer_GetJediRank` (`0x4074d0`) is `PlayerGetInvItemAmount(0x14)` truncated — the rank is
    /// the force-level bin, and the tally screen draws that many stars.
    pub fn jedi_rank(&self) -> i32 {
        self.inv.get(&20).copied().unwrap_or(0.0) as i32
    }

    /// `(secrets found, secrets in the level)`: bins 70 and 71, which `jkGuiSingleTally_Show`
    /// (`0x417e30`) reads straight out of the inventory.
    pub fn secrets(&self) -> (i32, i32) {
        (
            self.inv.get(&70).copied().unwrap_or(0.0) as i32,
            self.inv.get(&71).copied().unwrap_or(0.0) as i32,
        )
    }

    /// `jkPlayer_CalcAlignment` (`0x405cf0`), read from JK.EXE rather than guessed: the multipliers
    /// are the floats at `0x52206c` (6.25) and `0x522070` (−6.25), the clamp is ±100 (`0x522064`,
    /// `0x522068`), the secrets term is `−20` (`0x522074`) and the ratio scale is 100 (`0x522064`).
    ///
    /// Dark-side powers (bins 30–33) count against the alignment and light-side ones (25–28) for it.
    /// A player who found no secrets is spared the ratio term (`0x522054` is 0.0).
    ///
    /// The **choice comes first**, as it does in the binary: `jkGetChoice` is bin 18, and
    /// `jkPlayer_CalcAlignment` returns `+100` for 1 (the light ending) and `-100` for 2 (the dark
    /// one) before any power or secret arithmetic. The tally screen is the only single-player caller
    /// that passes 0 (include the secrets term); the force screen passes a global instead.
    pub fn alignment(&self) -> f32 {
        match self.inv.get(&18).copied().unwrap_or(0.0) as i32 {
            1 => return 100.0,
            2 => return -100.0,
            _ => {}
        }
        let mut value = 0.0f32;
        for bin in 30..34 {
            value -= self.inv.get(&bin).copied().unwrap_or(0.0) * 6.25;
        }
        for bin in 25..29 {
            value -= self.inv.get(&bin).copied().unwrap_or(0.0) * -6.25;
        }
        let (found, total) = (self.inv.get(&72).copied().unwrap_or(0.0), self.inv.get(&73).copied().unwrap_or(0.0));
        if total <= 0.0 {
            value += 20.0;
        } else {
            value = value - (found / total) * 100.0 + 20.0;
        }
        value.clamp(-100.0, 100.0)
    }

    /// `jkPlayer_CalcAlignment` as the tally calls it: compute the number and **store** it in bin
    /// 75, which is the engine's own last step (`sithPlayer_SetInvItemAmount(0x4b, value)`). The
    /// write is why retail's level-2 autosave carries `75 = 20`: `jkGuiSingleTally_Show`
    /// (`0x417e30`) calls this when the tally draws, at the end of level 1, and the next level's
    /// save carries the result. The port's level end does the same, so the carry matches.
    pub fn calc_alignment(&mut self) -> f32 {
        let value = self.alignment();
        self.set_bin(75, value);
        value
    }

    /// The screen effect the live `color_effects` add up to: `(filter, tint, add, alpha)` for
    /// `ui::Canvas::apply_screen_effect`. An effect fades **in** over its `fade` seconds — the scripts
    /// own shape (`newColorEffect(..., 1.0)` then `FreeColorEffect` when the power ends) — and the
    /// channels combine by multiplying the tints and summing the adds, which is what a stack of
    /// palette effects does at scan-out.
    pub fn screen_effect(&self) -> Option<([bool; 3], [f32; 3], [f32; 3], f32)> {
        let mut filter = [true; 3];
        let mut tint = [1.0f32; 3];
        let mut add = [0.0f32; 3];
        let mut alpha = 0.0f32;
        for effect in self.color_effects.values() {
            // `fade` of 0 means "already fully in".
            let progress = if effect.fade > 0.0 {
                ((self.time - effect.started) / effect.fade).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if progress <= 0.0 {
                continue;
            }
            // An effect whose `tint` is all zero is not tinting at all (`GatherEffects` checks for
            // exactly that), and a channel is full only while every effect keeps it.
            let tinting = effect.tint != [0.0; 3];
            for k in 0..3 {
                filter[k] &= effect.filter[k] != 0;
                if tinting {
                    tint[k] *= effect.tint[k];
                }
                add[k] += effect.add[k] as f32;
            }
            alpha = alpha.max(progress);
        }
        (alpha > 0.0).then_some((filter, tint, add, alpha))
    }

    /// How long a script's line stays in the message area. The engine keeps its own timer per
    /// `jkStringOutput` type; this build shows every line for the same few seconds.
    pub const HUD_MESSAGE_TIME: f32 = 4.0;

    /// The line the HUD should be showing right now, if any.
    pub fn hud_line(&self) -> Option<&str> {
        match &self.hud_message {
            Some((line, until)) if self.time < *until => Some(line),
            _ => None,
        }
    }

    /// Whether this thing's template carries a `weapon=`, which is the engine's own default for the
    /// armed mode: a class that ships a weapon starts with it drawn.
    pub fn thing_has_weapon(&self, thing: usize) -> bool {
        self.templates
            .get(thing)
            .is_some_and(|name| self.template_weapon.contains_key(&name.to_ascii_lowercase()))
    }

    /// Whether `thing` took damage within the last fifth of a second — how long the puppet's `hit`
    /// track has to be started, which `sithPuppet_SetMode` does off a damage message.
    pub fn just_hit(&self, thing: usize) -> bool {
        self.last_hit.get(&thing).is_some_and(|at| self.time - at < 0.2)
    }

    /// Whether `thing` fired within the last quarter second — long enough for the puppet's `fire`
    /// track to have started, which is what the renderer switches to.
    pub fn just_fired(&self, thing: usize) -> bool {
        self.last_shot.get(&thing).is_some_and(|at| self.time - at < 0.25)
    }

    /// Append to the `jkString*` buffer, respecting the engine's 128-character cap.
    fn string_buffer_push(&mut self, text: &str) {
        if self.string_buffer.chars().count() + text.chars().count() < 128 {
            self.string_buffer.push_str(text);
        }
    }

    /// Everything a savegame has to put back: the things, what the player carries, the clock and the
    /// sector state.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            time: self.time,
            things: self
                .things
                .iter()
                .enumerate()
                .map(|(index, thing)| ThingSave {
                    template: self.templates.get(index).cloned().unwrap_or_default(),
                    pos: thing.pos,
                    angles: [thing.pitch, thing.yaw, thing.roll],
                    sector: thing.sector,
                    health: thing.health,
                    flags: thing.flags,
                    jk_flags: thing.jk_flags,
                    destroyed: thing.destroyed,
                    frame: thing.frame,
                })
                .collect(),
            inv: self.inv.clone(),
            weapon: self.weapon.clone(),
            cur_bin: self.cur_bin,
            saber: self.saber.get(&self.player_thing()).cloned(),
            adjoin_flags: self.adjoin_flags.clone(),
            sector_light: self.sectors.iter().map(|s| s.light).collect(),
        }
    }

    /// Put [`Self::snapshot`] back. Things are matched **by index**, which is what a save of one level
    /// into the same level needs; a snapshot taken on another level does not fit and is ignored by
    /// length.
    pub fn restore(&mut self, save: &Snapshot) {
        self.time = save.time;
        self.game_clock = std::time::Duration::try_from_secs_f32(save.time).unwrap_or_default();
        for (index, saved) in save.things.iter().enumerate() {
            let Some(thing) = self.things.get_mut(index) else { break };
            thing.pos = saved.pos;
            thing.pitch = saved.angles[0];
            thing.yaw = saved.angles[1];
            thing.roll = saved.angles[2];
            thing.sector = saved.sector;
            thing.health = saved.health;
            thing.flags = saved.flags;
            thing.jk_flags = saved.jk_flags;
            thing.destroyed = saved.destroyed;
            thing.next_touch_ms = 0;
            thing.frame = saved.frame;
        }
        self.inv = save.inv.clone();
        self.weapon = save.weapon.clone();
        self.cur_bin = save.cur_bin;
        if let Some(blade) = &save.saber {
            let player = self.player_thing();
            self.saber.insert(player, blade.clone());
        }
        if save.adjoin_flags.len() == self.adjoin_flags.len() {
            self.adjoin_flags.copy_from_slice(&save.adjoin_flags);
        }
        for (sector, light) in self.sectors.iter_mut().zip(&save.sector_light) {
            sector.light = *light;
            sector.light_from = *light;
            sector.light_end = *light;
            sector.light_rate = 0.0;
        }
    }

    /// Pull the trigger on the armed weapon cog, honouring its own rate.
    ///
    /// The engine's player firing is `sithInventory_ActivateBin`: the bin's `activationDelaySecs`
    /// (the `fireWait / powerBoost` a cog passes to `ActivateWeapon`) becomes `binWait`, and the
    /// `fire` message only goes out when the clock has passed it. A mashed or held trigger
    /// therefore cannot outpace the script that owns the weapon. Returns whether a message
    /// was queued.
    /// `UseBin(bin)`: switch an inventory item on, which is `sithInventory_ActivateBin`'s message —
    /// the bin's own cog gets `activated` with the player as the source, and *it* decides what the
    /// item does (`cog_item_bacta.cog` heals 30 and spends one). Returns whether a message went out.
    ///
    /// The inventory strip calls this on Enter (`main.rs`'s inventory input), the way retail reaches
    /// it through `jkHudInv`'s screen, and `--use-item` drives it headlessly. (The old marker said
    /// nothing called it from a key; the strip has been selectable since it was added — the round-268
    /// debt sweep caught the stale note.)
    pub fn use_bin(&mut self, bin: i32) -> bool {
        let Some(cog) = self.bin_cog_index.get(&bin).copied() else {
            return false;
        };
        let player = self.player_thing();
        self.messages
            .push(("activated".to_string(), Some(player), Some(cog), [0.0; 2]));
        true
    }

    pub fn try_fire_weapon(&mut self, player: usize) -> bool {
        let Some((cog, rate, _)) = self.weapon_cog else { return false };
        if self.time < self.weapon_ready {
            return false;
        }
        self.messages.push(("fire".to_string(), Some(player), Some(cog), [0.0; 2]));
        self.weapon_ready = self.time + rate.max(0.0);
        true
    }

    /// Write an inventory bin and, when it goes from empty to held, ask for that bin's item script.
    ///
    /// `jkHudInv_ItemDatLoad` gives every weapon bin a `cog=`, and the engine starts that script when
    /// the player acquires the item — the script then owns the weapon's own messages (`autoselect:`,
    /// `selected:`, `activated:`). The runtime cannot build a VM, so it names the script and the app
    /// starts it; `spawned_cogs` keeps a bin that drops to zero and comes back from starting twice.
    /// `sithInventory_SetInventory`'s clamp: the item's own `min`/`max` columns, when the bin has a
    /// row in `misc\items.dat`.
    fn clamp_bin(&self, bin: i32, amount: f32) -> f32 {
        let Some(max) = self.item_max.get(&bin).copied() else { return amount };
        let min = self.item_min.get(&bin).copied().unwrap_or(0.0);
        amount.clamp(min, max)
    }

    pub fn set_bin(&mut self, bin: i32, amount: f32) {
        // `sithInventory_SetInventory` (`0x4d2000`) clamps to the item's own `min`/`max` before it
        // reports the change; a bin the table does not list is left as the script set it.
        let amount = self.clamp_bin(bin, amount);
        let was = self.inv.insert(bin, amount).unwrap_or(0.0);
        if was <= 0.0
            && amount > 0.0
            && let Some(cog) = self.item_cog.get(&bin).cloned()
            && !self.spawned_cogs.contains(&cog)
        {
            self.pending_cogs.push(cog);
        }
    }

    /// `jkMain_EndLevel`'s own grant: give the player the power their level's episode row names.
    ///
    /// The row carries a light bin and a dark bin, and the engine sets the `available` bit on each
    /// one the player's declared side may use (`sithInventory_SetCarries`) as the level is finished.
    /// The port's equivalent of that bit is holding the bin: `set_bin` starts the bin's own `cog=`,
    /// which is what makes the power usable, so a granted power is raised to star one and an
    /// already-held one is left where it is.
    pub fn grant_episode_powers(&mut self, lightpow: i32, darkpow: i32) -> Vec<i32> {
        let choice = self.inv.get(&crate::force::SIDE).copied().unwrap_or(0.0) as i32;
        let bins = crate::force::episode_grants(lightpow, darkpow, choice);
        for bin in &bins {
            let level = self.inv.get(bin).copied().unwrap_or(0.0).max(1.0);
            self.set_bin(*bin, level);
        }
        bins
    }

    /// Spend one force point on `bin`, as the force screen's button does. `max` is the row's own
    /// limit from `items.dat`. Returns whether the point went in.
    pub fn spend_force_point(&mut self, bin: i32, max: i32) -> bool {
        if !crate::force::can_spend(&self.inv, bin, max) {
            return false;
        }
        let pool = (self.inv.get(&crate::force::POINTS).copied().unwrap_or(0.0) - 1.0).max(0.0);
        self.set_bin(crate::force::POINTS, pool);
        let level = self.inv.get(&bin).copied().unwrap_or(0.0) + 1.0;
        self.set_bin(bin, level);
        true
    }

    /// `sithMain_SetEndLevel` (`0x4c4bf0`): `DAT_0082f0a8 = 1`.
    ///
    /// The engine's flag has no clear: once a level's own script says it is over, the main loop leaves
    /// on the next frame and nothing writes the word back. It is the observation a trace can compare
    /// even when the app stays in the level for a capture.
    pub fn sith_main_set_end_level(&mut self) {
        self.end_level_called = true;
        self.level_complete = true;
    }

    /// End of level: `jkMain_EndLevel` lands the finished level's state before the tally draws it.
    ///
    /// **Nothing is granted here.** Retail's own saves prove it: the level-1 load-time autosave and the
    /// level-2 save (written after level 1 was finished and its tally shown) carry **no rank bin** (20)
    /// and **no star bins** (16/17) at all, and the decomp reaches `jkPlayer_SetRank`/`jkPlayer_FixStars`
    /// only from multiplayer's new-character screen — nothing writes the point bins. Single-player force
    /// powers come from the episode table instead (`grant_episode_powers`, e.g. level 3 grants force
    /// speed), which is why the port's earlier one-rank-three-stars award per level was a stand-in and
    /// not a rule. The tally still *reads* the rank and the stars earned (bin `0x10`); in a single-player
    /// run they are zero unless a script writes them, exactly as retail shows.
    pub fn finish_level(&mut self) -> (f32, f32) {
        (
            self.inv.get(&crate::force::RANK).copied().unwrap_or(0.0),
            self.inv.get(&0x10).copied().unwrap_or(0.0),
        )
    }

    /// `jkGuiForce_Show`'s opening move: fold the level's award (bin `0x10`) into the pool (`0x11`).
    ///
    /// The tally draws `0x10` — the stars this level earned — and the first time the player opens the
    /// force screen the engine adds them to the pool it spends from and clears `0x10`. The reconcile
    /// afterwards is `jkPlayer_FixStars`, whose budget (three a rank) the award keeps in step, so it
    /// has nothing to do unless a script or a save left the pool out of step.
    pub fn fold_stars(&mut self) -> f32 {
        let fresh = self.inv.get(&0x10).copied().unwrap_or(0.0);
        if fresh > 0.0 {
            let pool = self.inv.get(&crate::force::POINTS).copied().unwrap_or(0.0) + fresh.round();
            self.set_bin(0x10, 0.0);
            self.set_bin(crate::force::POINTS, pool);
            crate::force::fix_stars(&mut self.inv);
        }
        self.inv.get(&crate::force::POINTS).copied().unwrap_or(0.0)
    }

    /// What the player keeps across a level change.
    /// The engine's `jkPlayer_*` state survives the level load — inventory, the armed bin, the
    /// selected weapon, the saber, the `jkFlags` the force scripts set, and the health and shield
    /// bars. This build rebuilds the whole runtime from the incoming `.jkl`, so those pieces have
    /// to be handed over explicitly instead of coming out of the thing list.
    /// `jkPlayer_GetChoice` (`0x405ce0`): the amount of inventory bin `0x12`, which *is* the
    /// light/dark choice — `1` light, `2` dark (`jkMain_EndLevel` compares the value against both).
    /// Retail's endgame screen grants it; `carry` takes it to the next level, and the `DECIDE` fork
    /// reads it through `jkMain_EndLevel`'s carries rule and its arm.
    pub fn choice(&self) -> i32 {
        self.inv.get(&0x12).copied().unwrap_or(0.0).floor() as i32
    }

    pub fn carry(&self) -> CarryOver {
        let player = self.player_thing();
        CarryOver {
            inv: self.inv.clone(),
            weapon: self.weapon.clone(),
            cur_bin: self.cur_bin,
            saber: self.saber.get(&player).cloned(),
            jk_flags: self.things.get(player).map(|t| t.jk_flags).unwrap_or(0),
            health: self.things.get(player).map(|t| t.health).unwrap_or(0.0),
            shields: self.player_shields,
        }
    }

    /// Put [`Self::carry`]'s state onto this (freshly built) level's player.
    pub fn adopt(&mut self, carry: &CarryOver) {
        self.inv = carry.inv.clone();
        // The goal counters are per level. `sithInventory_ResetInventory` (`0x4d2560`) zeroes the
        // bins whose table flag carries `0x40` at level load, before the level's scripts run;
        // `SetGoalFlags`/`ClearGoalFlags` (`0x4e0c40`/`0x4e0cc0`) then write the new level's own
        // objective state. Retail's level-2 autosave proves it: level 1 finished with goal 0 = 3
        // (`01_endlevel.cog`'s `SetGoalFlags(player, 0, 2)`), and level 2's save reads 1, written by
        // `02_endlevel.cog`'s startup — not the carried 3.
        for bin in 100..=115 {
            self.inv.insert(bin, 0.0);
        }
        self.weapon = carry.weapon.clone();
        self.cur_bin = carry.cur_bin;
        if let Some(blade) = &carry.saber {
            let player = self.player_thing();
            self.saber.insert(player, blade.clone());
        }
        let player = self.player_thing();
        let ceiling = self
            .templates
            .get(player)
            .and_then(|name| self.template_max_health.get(name))
            .copied()
            .unwrap_or(100.0)
            .max(1.0);
        if let Some(thing) = self.things.get_mut(player) {
            thing.jk_flags = carry.jk_flags;
            // The new level's player starts at its template's full health; keep what the old
            // level was carrying instead, but never above this level's own maximum.
            thing.health = carry.health.clamp(0.0, ceiling);
        }
        // `SITHBIN_SHIELDS` caps the bar at 200.
        self.player_shields = carry.shields.clamp(0.0, 200.0);
    }

    /// The thing the view follows, when a script has pointed a camera away from the player.
    ///
    /// `sithCamera_Open`: the engine's seven camera slots, each attached to a **placed camera thing**,
    /// in file order. The scripts address them by slot (`SetCurrentCamera(0)`, `SetCameraFocus(0,
    /// thing)`), and level 11's opening uses exactly that pair before playing `gocamera.key` on the
    /// camera thing, so the mapping has to be the level's own order.
    pub fn camera_thing(&self, slot: usize) -> Option<usize> {
        self.types
            .iter()
            .enumerate()
            .filter(|(_, kind)| **kind == 1)
            .nth(slot)
            .map(|(index, _)| index)
    }

    /// The engine's camera table is seven slots (`sithCamera_g_aCameras[7]`); `SetCameraFocus(slot,
    /// thing)` points one at a thing and `SetCurrentCamera(slot)` picks it. `None` means the player's
    /// own eyes, which is both the default and what a focus on the player means.
    pub fn camera_focus_thing(&self) -> Option<usize> {
        let focus = self.camera_focus.get(self.current_camera).copied().flatten()?;
        (focus != self.player_thing()).then_some(focus)
    }

    /// The yaw the **current script camera** looks along, for movement that is relative to the view.
    ///
    /// `sithControl_Update` reads the controls whether or not a script camera is current, and the
    /// thrust it builds is in the *view's* frame — so during a cutscene "forward" walks along the
    /// camera, not along the body's own facing. The camera of a scripted shot **is** the focus thing,
    /// aimed by its own angles (`SetCameraFocus(slot, camera1)` — level 11's drone flying around the
    /// twins, which is what the user's note describes), and that is exactly how the view system poses
    /// it. `None` when no scripted camera with a non-player focus is current, which is the normal
    /// case: the view and the body share a yaw then.
    /// Whether the player's saber should be drawn: armed by `jkSetSaberInfo` **and** not switched
    /// off by `jkDisableSaber`, which the saber scripts call when the blade goes away.
    pub fn saber_shown(&self, player: usize) -> bool {
        self.saber_enabled && self.saber.contains_key(&player)
    }

    /// What the camera's own shake adds to the view and to the aim, in level units and degrees.
    pub fn camera_shake(&self) -> ([f32; 3], [f32; 3]) {
        (self.shake_pos, self.shake_angle)
    }

    /// `SetSectorLight(sector, value, time)`: set the sector's extra light, fading over `time`
    /// seconds when that is positive (`sithSurface_SetSectorLight` builds the same rate from the
    /// value it is leaving).
    fn set_sector_light(&mut self, index: i64, value: f32, time: f32) {
        if let Some(s) = self.sector_mut(index) {
            s.light_from = s.light;
            s.light_end = value;
            s.light_rate = if time > 0.0 { (value - s.light) / time } else { 0.0 };
            if s.light_rate == 0.0 {
                s.light = value;
            }
            self.surface_epoch += 1;
        }
    }

    /// Run one verb against the world. Argument order follows the shipped scripts; a
    /// verb whose real arity differs simply does nothing useful here, and the
    /// interpreter records it.
    /// `sithThing_AttachThingToThing(child, parent)`: the child rides the parent, keeping the offset
    /// it had at the moment of the call — expressed in the parent's own frame
    /// (`rdMatrix_TransformVectorOrtho34` into `field_4C`), so a turning parent swings the child
    /// around it. A negative parent detaches (`sithThing_DetachThing`).
    fn attach_thing(&mut self, child: i64, parent: i64) {
        let (Ok(child), Ok(parent)) = (usize::try_from(child), usize::try_from(parent)) else {
            if let Ok(child) = usize::try_from(child)
                && let Some(thing) = self.things.get_mut(child)
            {
                thing.attach = None;
                thing.attach_flags = 0;
            }
            return;
        };
        if child == parent || self.things.get(parent).is_none() {
            return;
        }
        let (child_pos, parent_pos, parent_yaw) =
            match (self.things.get(child), self.things.get(parent)) {
                (Some(c), Some(p)) => (c.pos, p.pos, p.yaw),
                _ => return,
            };
        let d = [
            child_pos[0] - parent_pos[0],
            child_pos[1] - parent_pos[1],
            child_pos[2] - parent_pos[2],
        ];
        // Into the parent's frame: the inverse of the rotation `update_attachments` applies.
        let inverse = Quat::from_rotation_z(-parent_yaw.to_radians());
        let local = inverse * Vec3::from(d);
        let offset = [local.x, local.y, local.z];
        if let Some(thing) = self.things.get_mut(child) {
            thing.attach = Some(parent);
            thing.attach_offset = offset;
            thing.attach_flags = 1; // SITH_ATTACH_THING
        }
    }

    pub fn call(&mut self, verb: &str, args: &[Value]) -> Value {
        if std::env::var_os("BK_VERB_TRACE").is_some() {
            // The calling cog's **index** (`--links` names each one), so a trace of several scripts
            // separates: the intro and the boss loops call the same verbs, and a mixed stream cannot
            // say which one is stalled.
            eprintln!("verb [cog {}]: {verb} {args:?}", self.caller_cog);
            if verb.eq_ignore_ascii_case("getthinghealth") {
                for arg in args {
                    if let Value::Num(n) = arg {
                        let at = *n as usize;
                        eprintln!(
                            "verb:   thing {at} = {:?} health {:?}",
                            self.templates.get(at),
                            self.things.get(at).map(|t| t.health)
                        );
                    }
                }
            }
        }
        let num = |i: usize| args.get(i).map(|v| v.num()).unwrap_or(0.0);
        let idx = |i: usize| num(i) as i64;
        // Thing indices are `i64` in a script and `usize` in the tables; a negative index is simply
        // "no such thing", which is what every caller wants.
        let ti = |i: usize| usize::try_from(num(i) as i64).unwrap_or(usize::MAX);
        match verb.to_ascii_lowercase().as_str() {
            // ---- surfaces: the wall/cel verbs the wall-animation scripts drive ----
            "surfacecenter" => match self.surface(idx(0)) {
                Some(s) => Value::Vec([s.center[0] as f64, s.center[1] as f64, s.center[2] as f64]),
                None => Value::Vec([0.0; 3]),
            },
            "enableirmode" => {
                self.set_ir_mode(true);
                Value::Num(1.0)
            }
            "disableirmode" => {
                self.set_ir_mode(false);
                Value::Num(1.0)
            }
            // `GetMapModeFlags`: the word `SetMapModeFlags`/`ClearMapModeFlags` keep. `force_pull` reads
            // it back to know which mode it set.
            "getmapmodeflags" => Value::Num(self.map_mode_flags as f64),
            "setmapmodeflags" => {
                self.map_mode_flags = num(0) as u32;
                Value::Num(self.map_mode_flags as f64)
            }
            "clearmapmodeflags" => {
                self.map_mode_flags = 0;
                Value::Num(0.0)
            }
            "setwallcel" => {
                if let Some(s) = self.surface_mut(idx(0)) {
                    s.cel = num(1) as f32;
                    // The renderer folds a surface's cel into the mesh it already built, so a change
                    // has to ask for a rebuild the way a geomode change does.
                    self.surface_epoch += 1;
                }
                Value::Nil
            }
            "getwallcel" => Value::Num(self.surface(idx(0)).map(|s| s.cel).unwrap_or(0.0) as f64),
            // `SlideWall(surface, u, v)`: offset the texture, which is how a scrolling
            // panel or a conveyor is drawn.
            // `SlideWall(surface, vector, scale)`: `sithCogFunctionSurface_SlideWall` scales the
            // vector by `scale * 0.1`, negates it, and records it as the surface's scroll vector —
            // which the engine uses for **two** things: the texture offset, and the velocity of
            // anything standing on the surface (`sithThing.c` adds `scrollVector * dt` to a thing
            // attached to a scrolling surface). A conveyor is both at once.
            "slidewall" => {
                let vec = args.get(1).map(vec3).unwrap_or([0.0; 3]);
                let scale = num(2) as f32 * 0.1;
                if let Some(s) = self.surface_mut(idx(0)) {
                    s.slide = vec.map(|v| -v * scale);
                    if let Some(flags) = self.surface_flags.get_mut(idx(0) as usize) { *flags |= 0x800; }
                    self.surface_epoch += 1;
                }
                Value::Nil
            }
            // `SurfaceAnim(surface, fps, ...)`: a zero or negative rate means the engine's
            // default 15 fps (`sithCogFunction_SurfaceAnim`).
            "surfaceanim" => {
                let rate = if num(1) > 0.0 { num(1) as f32 } else { 15.0 };
                let now = self.time;
                if let Some(s) = self.surface_mut(idx(0)) {
                    s.anim = rate;
                    // The first cel is due one step from now, which is also how the engine starts
                    // (`field_30 = now + field_34`).
                    s.next = now + 1.0 / rate;
                }
                Value::Num(idx(0) as f64)
            }
            "getsurfaceanim" => Value::Num(self.surface(idx(0)).map(|s| s.anim).unwrap_or(0.0) as f64),
            "stopsurfaceanim" => {
                if let Some(s) = self.surface_mut(idx(0)) {
                    s.anim = 0.0;
                }
                Value::Nil
            }
            "sectorlight" => {
                self.set_sector_light(idx(0), num(1) as f32, args.get(2).map(|v| v.num() as f32).unwrap_or(0.0));
                Value::Nil
            }
            // `SurfaceLightAnim(surface, start, end, fps)`: `sithCogFunctionSurface_SurfaceLightAnim`
            // sets the surface's extra light to `start` and ramps it to `end` over `fps * 0.5`
            // seconds, answering the surface index (or -1 when the arguments make no ramp).
            "surfacelightanim" => {
                let (start, end, fps) = (num(1) as f32, num(2) as f32, num(3) as f32);
                if end < start || fps <= 0.0 {
                    return Value::Num(-1.0);
                }
                let index = idx(0);
                if let Some(s) = self.surface_mut(index) {
                    s.light = start;
                    s.light_end = end;
                    s.light_rate = (end - start) / (fps * 0.5);
                    self.surface_epoch += 1;
                    return Value::Num(index as f64);
                }
                Value::Num(-1.0)
            }
            "getsectorlight" => Value::Num(self.sectors.get(idx(0) as usize).map(|s| s.light).unwrap_or(0.0) as f64),

            "destroything" => {
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.destroyed = true;
                }
                Value::Nil
            }
            // `SetTimerEx(time, id, val, val2)` has the same four values as `SetTimer`;
            // the engine keeps the extra two for the handler's parameters.
            "settimerex" => self.call("settimer", args),
            // The same verbs spelled the other way in different scripts.
            "getsurfacecenter" => self.call("surfacecenter", args),
            "killtimerex" => self.call("killtimer", args),
            // A blocking wait for a thing to finish its frame path. There is no scheduler
            // here to suspend, so it reports the thing's state and lets the script
            // continue; a script that loops on it is caught by the interpreter's budget.
            "waitforstop" => Value::Num(self.thing(idx(0)).map(|t| t.is_moving() as i32).unwrap_or(0) as f64),
            // Sounds are a list here, not handles: stopping one clears the list.
            "stopsound" => {
                self.sounds.clear();
                Value::Nil
            }
            // Redbook music. `PlaySong(starttrack, endtrack, loopto)` names CD track numbers,
            // not files: level 1's `01_SoundTrack.cog` asks for `PlaySong(12, 13, 12)`, which
            // the GOG install ships as `MUSIC\Track12.ogg`. `SetMusicVol` is the fade the
            // script drives from a pulse, and `StopSong` clears it.
            "playsong" => {
                // `sithCogFunctionSound_PlaySong` -> `sithSoundMixer_PlaySong(from, to, num)`: the
                // range is `from..to`, `num` is the track to *start* at, clamped into that range, and
                // a start of 0 or less stops the song. The mixer also resets its music volume to 1.0
                // on every start, which is why a `SetMusicVol` before the song is overwritten.
                let from = args.first().map(|v| v.num() as i32).unwrap_or(0);
                let to = args.get(1).map(|v| v.num() as i32).unwrap_or(from);
                let num = args.get(2).map(|v| v.num() as i32).unwrap_or(from);
                let to = if to <= from { from } else { to };
                // The stop test is on the *raw* song number, before any clamping:
                // `sithCogFunctionSound_PlaySong` checks `trackNum <= 0` and calls `StopSong`, and
                // only the else branch enters the mixer (which clamps). Testing the clamped value
                // instead turned `PlaySong(12, 13, 0)` into "play track 12".
                if num <= 0 {
                    self.music = None;
                } else {
                    self.music = Some((from, to, num.clamp(from, to)));
                    self.music_volume = 1.0;
                }
                Value::Nil
            }
            "setmusicvol" => {
                self.music_volume = (num(0) as f32).clamp(0.0, 1.0);
                Value::Nil
            }
            "stopsong" | "stopmusic" => {
                self.music = None;
                Value::Nil
            }
            // `jkEndLevel(flag)`: `jkCog_RegisterVerbs` (`0x40a110`) registers the verb, and the
            // handler is `sithMain_SetEndLevel` (`0x4c4bf0` — `DAT_0082f0a8 = 1`, unconditional)
            // followed by `jkMain_EndLevel` (`0x402eb0`), whose arm grant is `jk_main_end_level` here.
            // The flag rides to the next-level start (`jkMain_StartNextLevelInEpisode(0, flag)`), which
            // the app does when it swaps the world.
            "jkendlevel" | "endlevel" => {
                if std::env::var_os("BK_LEVEL_TRACE").is_some() {
                    eprintln!("level: `jkEndLevel` from cog {}", self.caller_cog);
                }
                self.sith_main_set_end_level();
                Value::Nil
            }
            // ---- attachments: a thing that rides another thing ----
            //
            // `AttachThingToThing(child, parent)` and its `...Ex` spelling with flags: 20 shipped
            // calls, and they are functional — a crate riding a lift, a catwalk hanging off a ship,
            // the sphere `force_absorb.cog` hangs on the player, and the orbiting camera of
            // `00_cameratricks.cog`. `update_attachments` re-places the child every step.
            "attachthingtothing" | "attachthingtothingex" => {
                self.attach_thing(idx(0), idx(1));
                Value::Nil
            }
            "detachthing" => {
                if let Some(thing) = usize::try_from(idx(0)).ok().and_then(|i| self.things.get_mut(i)) {
                    thing.attach = None;
                    thing.attach_flags = 0;
                }
                Value::Nil
            }
            "getthingparent" => Value::Num(
                usize::try_from(idx(0))
                    .ok()
                    .and_then(|i| self.things.get(i))
                    .and_then(|t| t.attach)
                    .map(|p| p as f64)
                    .unwrap_or(-1.0),
            ),
            "setthingattachflags" => {
                if let Some(thing) = usize::try_from(idx(0)).ok().and_then(|i| self.things.get_mut(i)) {
                    thing.attach_flags = idx(1) as u32;
                }
                Value::Nil
            }
            // `RotatePivot(thing, frame, seconds)`: `sithCogFunctionThing_RotatePivot` takes the
            // *frame* from the thing's `frame=` table — its position is the pivot to turn around and
            // its angles the rotation to turn by — and `sithTrackThing_RotatePivot` sets the rate to
            // `1 / seconds`. A zero `seconds` means 1 second and a negative one turns the other way,
            // which is the engine's own two adjustments. It requires a path thing with that frame
            // loaded; so does this.
            "rotatepivot" => {
                let Some(index) = usize::try_from(idx(0)).ok() else { return Value::Nil };
                let Some(frame) = self
                    .things
                    .get(index)
                    .and_then(|t| usize::try_from(idx(1)).ok().and_then(|f| t.frames.get(f)))
                    .copied()
                else {
                    return Value::Nil;
                };
                let mut angles = [frame[3], frame[4], frame[5]];
                let mut seconds = num(2) as f32;
                if seconds == 0.0 {
                    seconds = 1.0;
                }
                if seconds < 0.0 {
                    angles = [-angles[0], -angles[1], -angles[2]];
                    seconds = -seconds;
                }
                if let Some(thing) = self.things.get_mut(index) {
                    thing.spin = Some(PivotSpin {
                        centre: [frame[0], frame[1], frame[2]],
                        angles,
                        duration: seconds,
                        done: 0.0,
                        start_pos: thing.pos,
                        start_angles: [thing.pitch, thing.yaw, thing.roll],
                    });
                    // The frame path is done with: the engine sets `curframe = -1`.
                    thing.target = None;
                }
                Value::Nil
            }
            // ---- the scripted AI: who an actor shoots, where it runs, which instincts it has ----
            //
            // `AISetFireTarget(actor, thing)`: `sithAI_SetFireTarget` stores the thing on the actor;
            // a negative clears it back to the instincts. `00_AITurnCoat.cog` and `06_drama.cog` are
            // the shipped users — both aim one actor at another.
            "aisetfiretarget" => {
                if let Ok(actor) = usize::try_from(idx(0)) {
                    match usize::try_from(idx(1)) {
                        Ok(target) if self.things.get(target).is_some() => {
                            self.ai_fire_target.insert(actor, target);
                        }
                        _ => {
                            self.ai_fire_target.remove(&actor);
                        }
                    }
                }
                Value::Nil
            }
            // `AIFlee(actor, from)`: the actor keeps its distance from that thing and stops firing —
            // `sithAI_Flee` sets the fleeing mode, which `ai_tick` reads as "walk away, do not shoot".
            // The engine's verb (`0x501420`) only stores the thing and raises AI flag 0x800; the walk is
            // the class's `Flee` instinct, and that instinct **ends on the instinct's third argument's
            // seconds** (shipped lines are `1.0, 5000, 2.0` and `1.5, 10000, 1.0`), so a scripted flee is
            // a burst, not a permanent state. A negative second argument stops the flee now.
            "aiflee" => {
                if let Ok(actor) = usize::try_from(idx(0)) {
                    let from = idx(1);
                    if from < 0 {
                        self.ai_flee.remove(&actor);
                        self.ai_flee_until.remove(&actor);
                    } else if let Ok(from) = usize::try_from(from)
                        && from < self.things.len()
                    {
                        let seconds = self
                            .ai
                            .get(&actor)
                            .and_then(|class| class.instinct("Flee"))
                            .and_then(|flee| flee.args.get(1).copied())
                            .map(|s| s as f32)
                            .unwrap_or(2.0)
                            .max(0.1);
                        self.ai_flee.insert(actor, from);
                        self.ai_flee_until.insert(actor, self.time + seconds);
                    }
                }
                Value::Nil
            }
            // `AISetClass(actor, class)`: the argument is the `.ai` file name the cog bound
            // (`ai picup_ai=picup.ai`), so the actor's instincts are replaced wholesale.
            "aisetclass" => {
                let name = match args.get(1) {
                    Some(Value::Str(s)) => s.trim().to_ascii_lowercase(),
                    _ => String::new(),
                };
                let raw = self
                    .ai_classes
                    .get(&name)
                    .or_else(|| name.strip_suffix(".ai").and_then(|n| self.ai_classes.get(n)))
                    .cloned();
                if let (Ok(actor), Some(raw)) = (usize::try_from(idx(0)), raw) {
                    self.ai.insert(actor, crate::ai::parse(&raw));
                }
                Value::Nil
            }
            // `CreateThingAtPos(template, sector, pos, angles)`: the absolute-position form of
            // `CreateThing`, which is how `force_absorb.cog` hangs its sphere on the player and how
            // every effect script spawns a burst where something happened.
            "createthingatpos" => {
                let template = match args.first() {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => return Value::Num(-1.0),
                };
                let sector = idx(1) as i32;
                let pos = args.get(2).map(vec3).unwrap_or([0.0; 3]);
                let angles = args.get(3).map(vec3).unwrap_or([0.0; 3]);
                Value::Num(self.spawn_at(template, pos, sector, angles) as f64)
            }
            // `TeleportThing(thing, to)`: `sithCogFunctionThing_TeleportThing` detaches the thing, then
            // copies the target's **whole pose** — orientation as well as position — re-homes it in the
            // target's sector and re-grounds it. It is not a position set: the angles come along.
            "teleportthing" => {
                let (Ok(thing), Ok(to)) = (usize::try_from(idx(0)), usize::try_from(idx(1))) else {
                    return Value::Nil;
                };
                let Some(from) =
                    self.things.get(to).map(|t| (t.pos, t.pitch, t.yaw, t.roll, t.sector))
                else {
                    return Value::Nil;
                };
                let previous = self.things.get(thing).and_then(|t| usize::try_from(t.sector).ok());
                if let Some(t) = self.things.get_mut(thing) {
                    t.attach = None;
                    t.attach_flags = 0;
                    t.pos = from.0;
                    t.pitch = from.1;
                    t.yaw = from.2;
                    t.roll = from.3;
                    t.sector = from.4;
                    t.target = None;
                    t.vel = [0.0; 3];
                }
                if let Ok(sector) = usize::try_from(from.4) {
                    if previous != Some(sector) {
                        let messages = self.sector_messages(previous, None, Some(thing));
                        for (instance, label, link) in messages {
                            let old = previous.unwrap_or(sector);
                            self.pending_messages.push((instance, label, [0.0; 2], Some(link as i64),
                                Some(SourceSender::new(SECTOR_SOURCE, old as i32, Some(thing)))));
                        }
                        self.queue_player_sector_entry(thing, sector);
                    }
                }
                Value::Nil
            }
            // `SetRespawnMask(player, mask)`: `sithCogFunctionPlayer_SetRespawnMask` writes the *player's*
            // respawn mask — which items a dying player keeps — not a thing's. Stored so the round trip
            // is real; single-player death does not consult it.
            "setrespawnmask" => {
                self.player_respawn_mask = idx(1) as u32;
                Value::Nil
            }
            // `FireProjectile(thing)`: fire the thing's own `weapon=` template, which is what a scripted
            // turret or an NPC does (`sithCogFunctionThing_FireProjectile`).
            // `FireProjectile(shooter, template, sound, mode, fireOffset, aimError, scale, scaleFlags,
            // autoaimFov, autoaimMaxDist)`: the **template** is the projectile a script names
            // (`projectile=+bryarbolt` in a weapon cog's own symbols), and `aimError` is the spread
            // vector. A script that fires is naming what to fire, not asking for the shooter's default.
            "fireprojectile" => {
                let Some(index) = usize::try_from(idx(0)).ok() else { return Value::Num(-1.0) };
                let named = match args.get(1) {
                    // A named projectile is valid when the level carries that template: `template_damage`
                    // only has entries that declare `damage=` themselves, so a bolt inheriting it from
                    // `_weapon` was refused and the shot silently never happened.
                    Some(Value::Str(name))
                        if self.template_damage.contains_key(&name.to_ascii_lowercase())
                            || self
                                .templates
                                .iter()
                                .any(|t| t.eq_ignore_ascii_case(name)) =>
                    {
                        Some(name.clone())
                    }
                    _ => None,
                };
                let weapon = named.or_else(|| {
                    self.templates
                        .get(index)
                        .and_then(|name| self.template_weapon.get(name).cloned())
                });
                let Some(weapon) = weapon else { return Value::Num(-1.0) };
                let error = args.get(5).map(vec3).map(|v| v[2]).unwrap_or(0.0);
                // `FireProjectile(sender, template, sound, mode, offset, …)`: the sound is played
                // only when the script names one (`weap_bryar.cog` passes `pistol-1.wav`).
                let sound = match args.get(2) {
                    Some(Value::Str(name)) if !name.is_empty() => Some(name.as_str()),
                    _ => None,
                };
                let shot = self.fire_with_spread(index as i64, &weapon, error as f32, sound);
                Value::Num(shot.map(|i| i as f64).unwrap_or(-1.0))
            }
            // `SyncThingPos`/`SyncSurface` and their siblings: multiplayer state pushes. A single-player
            // build has nobody to push to, so they are answered and do nothing — the engine's own arm is
            // empty without `COG_SHOULD_SYNC`.
            "syncthingpos" | "syncsurface" | "syncthingscale" | "syncthinglook" => Value::Nil,
            // `SetSectorThrust(sector, vector, mult)`: `sithCogFunctionSector_SetSectorThrust` stores
            // `mult * vector` on the sector (a zero vector clears it and the `USETHRUST` flag), and
            // `sithPhysics.c` adds `thrust * dt` to a body's velocity *exactly like gravity*. Level
            // 21's core tube is an updraft (`'0.0 0.0 1.0'` at 4-5) and level 10's power core pushes
            // the player out of its chamber.
            "setsectorthrust" | "sectorthrust" => {
                let sector = idx(0);
                let vector = args.get(1).map(vec3).unwrap_or([0.0; 3]);
                let mult = num(2) as f32;
                if let Some(s) = self.sector_mut(sector) {
                    s.thrust = if vector == [0.0; 3] {
                        [0.0; 3]
                    } else {
                        [vector[0] * mult, vector[1] * mult, vector[2] * mult]
                    };
                }
                Value::Nil
            }
            // `PathMovePause(thing)` / `PathMoveResume(thing)`: `sithTrackThing_PathMovePause` sets
            // `trackParams.flags |= 0x80` (only when the thing is actually moving, answering its index
            // or -1) and `...Resume` clears it. Level 9's pipe doors stop in the doorway this way:
            // `09_stoppipedoors.cog` pauses when the player enters the sector and resumes on exit.
            "pathmovepause" => {
                let Some(index) = usize::try_from(idx(0)).ok() else { return Value::Num(-1.0) };
                match self.things.get_mut(index) {
                    Some(t) if t.target.is_some() => {
                        t.paused = true;
                        Value::Num(index as f64)
                    }
                    _ => Value::Num(-1.0),
                }
            }
            "pathmoveresume" => {
                let Some(index) = usize::try_from(idx(0)).ok() else { return Value::Num(0.0) };
                match self.things.get_mut(index) {
                    Some(t) if t.paused => {
                        t.paused = false;
                        Value::Num(1.0)
                    }
                    _ => Value::Num(0.0),
                }
            }
            // `GetThingGeoMode` reads the thing's draw mode and `SetThingCurGeoMode` writes it
            // (`renderData.desiredGeoMode`/`curGeoMode`). 0 is `RD_GEOMODE_NONE`: the thing is not
            // drawn, which is how the dark Jedi scripts take a body off screen.
            "getthinggeomode" => Value::Num(
                usize::try_from(idx(0))
                    .ok()
                    .map(|i| self.thing_geomode.get(&i).copied().unwrap_or(1) as f64)
                    .unwrap_or(0.0),
            ),
            "setthingcurgeomode" => {
                if let Ok(thing) = usize::try_from(idx(0)) {
                    let mode = idx(1) as i32;
                    self.thing_geomode.insert(thing, mode);
                    self.geomode_epoch = self.geomode_epoch.wrapping_add(1);
                }
                Value::Nil
            }
            // `LoadTemplate(name)`: `sithCogFunction_LoadTemplate` answers the template's index, or -1.
            // A cog then hands that value straight to `CreateThing`, and this port's `CreateThing`
            // takes the **name**, so the name is what a found template answers — the handle a script
            // stores (`water = LoadTemplate("+superenerg")` in `10_powercore.cog`) works either way.
            "loadtemplate" => {
                let name = match args.first() {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => String::new(),
                };
                if self.template_types.contains_key(&name) || self.template_health.contains_key(&name) {
                    Value::Str(name)
                } else if let Some((name, _)) = self
                    .template_health
                    .iter()
                    .find(|(known, _)| known.eq_ignore_ascii_case(&name))
                {
                    Value::Str(name.clone())
                } else {
                    Value::Num(-1.0)
                }
            }
            // `SetFaceLightMode(surface, mode)`: `renderData`'s per-face `lightingMode`. Only
            // `RD_LIGHTMODE_FULLYLIT` (0) changes what this build draws — the face is drawn at full
            // brightness, which is how the CTF force fields are made to glow — while `GOURAUD` (3)
            // and the other modes are the bake the port always does.
            "setfacelightmode" => {
                if let Ok(surface) = usize::try_from(idx(0)) {
                    self.face_light_mode.insert(surface, idx(1) as i32);
                    self.geomode_epoch = self.geomode_epoch.wrapping_add(1);
                }
                Value::Nil
            }
            "getfacelightmode" => Value::Num(
                usize::try_from(idx(0))
                    .ok()
                    .map(|i| self.face_light_mode.get(&i).copied().unwrap_or(3) as f64)
                    .unwrap_or(0.0),
            ),
            // `ThingLight(thing, value, time)`: `sithCogFunctionThing_ThingLight` sets the thing's own
            // light outright when `time` is 0 and otherwise ramps it there over `time` seconds
            // (`sithSurface_SetThingLight(pThing, light, time, 0)`). A negative value is ignored, which
            // is the engine's guard.
            "thinglight" => {
                let value = num(1) as f32;
                let time = num(2) as f32;
                if value >= 0.0
                    && let Some(index) = usize::try_from(idx(0)).ok()
                {
                    self.set_thing_light(index, value, time);
                }
                Value::Nil
            }
            // `SetThingLight(thing, value)`: the plain setter, no ramp.
            "setthinglight" => {
                let value = num(1) as f32;
                if value >= 0.0
                    && let Some(index) = usize::try_from(idx(0)).ok()
                {
                    self.set_thing_light(index, value, 0.0);
                }
                Value::Nil
            }
            // `ThingLightAnim(thing, start, end, period)`: from `start` to `end` over `period * 0.5`
            // seconds, answering the record or -1 — `10_powercore.cog` pulses four lights with it and
            // `00_lightanim.cog` is the generic form.
            "thinglightanim" => {
                let (start, end, period) = (num(1) as f32, num(2) as f32, num(3) as f32);
                if end < start || period <= 0.0 {
                    return Value::Num(-1.0);
                }
                let Some(index) = usize::try_from(idx(0)).ok() else { return Value::Num(-1.0) };
                if let Some(thing) = self.things.get_mut(index) {
                    thing.light = start;
                    thing.light_end = end;
                    thing.light_rate = (end - start) / (period * 0.5);
                    return Value::Num(index as f64);
                }
                Value::Num(-1.0)
            }
            // Surface flags decide footstep and landing sounds (`SITH_SURFACE_*`); the
            // renderer does not animate them, so this is recorded as answered only.
            "setsurfaceflags" => Value::Nil,
            // `SendMessage(cog, message)`: `sithCogFunction_SendMessage` pops a **cog index** and a
            // message id, resolves the cog and delivers to that one instance. A script's `cog`
            // symbol is such an index, which is how `ctf_callback` and `GetMasterCOG()` arrive here.
            // The id is carried as `#N` and resolved against the receiving script's `message:` list,
            // as before; a negative cog (an unset master) broadcasts, which is the only delivery the
            // shipped start-of-level scripts have until every level's endlevel startup has run.
            // `SendMessageEx(cog, message, param0, param1, param2, param3)`: the verb at `0x5068a0`
            // pops four flex for `param3..param0`, the message id and the cog, then calls
            // `sithCog_SendMessageEx(cog, message, 9, self, …, param0..param3)`. `03_goals.cog`'s
            // `SendMessageEx(GetMasterCOG(), user0, 2, 0, 0, 0)` is `user0` with `GetParam(0) == 2`.
            // This bus carries `param0`/`param1`.
            "sendmessageex" => {
                let cog = (idx(0) >= 0).then(|| idx(0) as i32);
                let name = match args.get(1) {
                    Some(Value::Str(s)) => s.clone(),
                    Some(value) => format!("#{}", value.num() as i64),
                    None => "message 0".to_string(),
                };
                let sender = (self.caller_cog >= 0).then_some(self.caller_cog as usize);
                let params = [num(2), num(3)];
                self.messages.push((name, sender, cog, params));
                self.messages_sent += 1;
                Value::Nil
            }
            // `SendMessage(cog, message)`: `sithCogFunction_SendMessage` pops a **cog index** and a
            "sendmessage" => {
                let cog = (idx(0) >= 0).then(|| idx(0) as i32);
                let name = match args.get(1) {
                    Some(Value::Str(s)) => s.clone(),
                    Some(value) => {
                        // A number is the message's ordinal in the *receiving* script's
                        // `message:` list, so it is carried as `#N` and resolved per
                        // script by the pump.
                        format!("#{}", value.num() as i64)
                    }
                    None => "message 0".to_string(),
                };
                let sender = (self.caller_cog >= 0).then_some(self.caller_cog as usize);
                self.messages.push((name, sender, cog, [0.0; 2]));
                self.messages_sent += 1;
                Value::Nil
            }
            // ---- the force-power verbs: `force_jump.cog` and friends drive these ----
            //
            // `SetThingVel` is how the shipped force scripts move the player at all: force
            // jump does `upVel = VectorSet(0,0,duration + 2.5)` and adds the current
            // velocity, so the *script* owns the number, not the engine.
            "setthingvel" => {
                let vel = match args.get(1) {
                    Some(Value::Vec(v)) => [v[0] as f32, v[1] as f32, v[2] as f32],
                    _ => [0.0; 3],
                };
                if let Some(thing) = self.thing_mut(idx(0)) {
                    thing.vel = vel;
                }
                Value::Nil
            }
            "getthingvel" => match self.thing(idx(0)) {
                Some(thing) => {
                    Value::Vec([thing.vel[0] as f64, thing.vel[1] as f64, thing.vel[2] as f64])
                }
                None => Value::Vec([0.0; 3]),
            },
            // `ActivateBin(player, bin, seconds?)` starts a power's hold timer;
            // `DeactivateBin` reports how long it was held, which is the jump's duration.
            // `SetActorExtraSpeed(thing, speed)`: force speed. The script sets `0.5 + rank/2`
            // and its `stop_power` sets 0, so this is a value to hold, not a one-shot.
            "setactorextraspeed" => {
                if let Some(thing) = self.thing_mut(idx(0)) {
                    thing.extra_speed = num(1) as f32;
                }
                Value::Nil
            }
            // `HealThing(thing, amount)` — registered as `healthing` in the engine. Actors and
            // the player only, clamped at the template's `maxhealth=`.
            "healthing" => {
                let amount = num(1) as f32;
                let index = usize::try_from(idx(0)).unwrap_or(usize::MAX);
                let max = self
                    .templates
                    .get(index)
                    .map(|name| self.template_max_health.get(name).copied().unwrap_or(f32::MAX))
                    .unwrap_or(f32::MAX);
                if amount > 0.0
                    && matches!(self.types.get(index).copied(), Some(2) | Some(10))
                    && let Some(thing) = self.thing_mut(idx(0))
                {
                    thing.health = (thing.health + amount).min(max);
                }
                Value::Nil
            }
            // `ActivateBin(player, 0, 21)`: the bin is the *third* argument in the shipped
            // force scripts; reading the second stored the hold under bin 0, so
            // `DeactivateBin(player, 21)` found nothing and the jump came out as 0.
            "activatebin" => {
                let bin = if num(2) > 0.0 { idx(2) } else { idx(1) };
                let delay = num(1).max(0.0) as f32;
                self.bin_active.insert(bin as i32, self.time);
                // `sithInventory_ActivateBin`: with a delay the bin's own cog gets a `fire` when the
                // delay is up, and `sithInventory_SendFire` keeps sending it while the bin is active.
                // The cog is the caller, which is how the engine's `SendMessageEx(… SENDERTYPE_SYSTEM)`
                // target works out.
                if delay > 0.0 && self.caller_cog >= 0 {
                    self.bin_due.push((
                        self.time + delay,
                        bin as i32,
                        self.caller_cog,
                        idx(0) as usize,
                        delay,
                    ));
                }
                Value::Nil
            }
            "deactivatebin" => {
                let bin = idx(1) as i32;
                // `DeactivateBin` ends the loop as well as reporting the hold.
                self.bin_due.retain(|(_, b, _, _, _)| *b != bin);
                let held = self.bin_active.remove(&bin).map(|at| self.time - at).unwrap_or(0.0);
                self.last_hold = held;
                Value::Num(held as f64)
            }
            "setinvactivated" => {
                self.inv_activated.insert(idx(1) as i32, idx(2) != 0);
                Value::Nil
            }
            // `jkSetTarget`/`jkEndTarget`: the engine's target *holding* — what a pull or a grip
            // grabs. The script hands over a thing (or nothing, meaning "the sender"), and the drag
            // itself is applied in `step`. `jkSetTargetColors` tints the reticle; recorded, not drawn.
            "jksettarget" => {
                // The script passes the thing it grabbed; with no argument it means "what the
                // player is looking at", which is the engine's own aim query (`aimed_thing`).
                let target = if args.is_empty() { self.aimed_thing() } else { usize::try_from(idx(0)).ok() };
                self.force_target = target.filter(|t| self.things.get(*t).is_some_and(|thing| !thing.destroyed));
                Value::Nil
            }
            "jkendtarget" => {
                self.force_target = None;
                Value::Nil
            }
            "jksettargetcolors" => {
                self.force_target_tint = args.iter().take(3).map(|a| a.num() as f32).collect();
                Value::Nil
            }

            // ---- the targeting and force family, as `force_pull.cog` and `force_grip.cog` use it ----
            //
            // `FirstThingInView(player, angle, dist, mask)` snapshots the things of type `mask` inside
            // a cone of `angle` degrees half-width and `dist` units, and `NextThingInView()` walks
            // that snapshot. The mask is **thing type bits**: pull asks for `0x424` (bits 2, 5 and 10 =
            // actor, item, other player) and grip for `0x404` (actor, other player).
            "firstthinginview" => {
                let from = ti(0);
                let angle = num(1) as f32;
                let dist = num(2) as f32;
                let mask = idx(3) as u32;
                let eye = self.things.get(from).map(|t| t.pos).unwrap_or([0.0; 3]);
                let yaw = self.things.get(from).map(|t| t.yaw).unwrap_or(0.0).to_radians();
                let forward = [yaw.cos(), yaw.sin()];
                let half = (angle * 0.5).to_radians().cos();
                let mut list: Vec<(usize, f32)> = Vec::new();
                for (index, thing) in self.things.iter().enumerate() {
                    if thing.destroyed || index == from {
                        continue;
                    }
                    let kind = self.types.get(index).copied().unwrap_or(0);
                    if kind < 0 || (mask >> kind) & 1 == 0 {
                        continue;
                    }
                    let (dx, dy) = (thing.pos[0] - eye[0], thing.pos[1] - eye[1]);
                    let flat = (dx * dx + dy * dy).sqrt();
                    if flat < 1e-3 || flat > dist {
                        continue;
                    }
                    let dot = (dx * forward[0] + dy * forward[1]) / flat;
                    if dot < half {
                        continue;
                    }
                    list.push((index, dot));
                }
                // Nearest first: the scripts keep the best `ThingViewDot` themselves, so order is
                // only a convenience, but a stable order keeps the tick deterministic.
                list.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                self.view_list = list.into_iter().map(|(i, _)| i).collect();
                self.view_at = 0;
                Value::Num(match self.view_list.first() {
                    Some(index) => {
                        self.view_at = 1;
                        *index as f64
                    }
                    None => -1.0,
                })
            }
            "nextthinginview" => {
                let Some(index) = self.view_list.get(self.view_at).copied() else {
                    return Value::Num(-1.0);
                };
                self.view_at += 1;
                Value::Num(index as f64)
            }
            // `HasLOS(from, to)`: a straight line with nothing solid between. The collider is the
            // same one the walker uses, so a doorway is a clear line and a wall is not.
            "haslos" => {
                let (a, b) = match (self.things.get(ti(0)), self.things.get(ti(1))) {
                    (Some(a), Some(b)) => (a.pos, b.pos),
                    _ => return Value::Num(0.0),
                };
                let clear = match self.solids.as_ref() {
                    Some(solids) => !solids.segment_blocked(a, b),
                    None => true,
                };
                Value::Num(if clear { 1.0 } else { 0.0 })
            }
            // `ThingViewDot(from, to)`: cos of the angle between `from`'s facing and the line to `to`.
            // The scripts keep the largest, which is the thing most centred in the view.
            "thingviewdot" => {
                let (Some(from), Some(to)) = (self.things.get(ti(0)), self.things.get(ti(1))) else {
                    return Value::Num(0.0);
                };
                let yaw = from.yaw.to_radians();
                let (dx, dy) = (to.pos[0] - from.pos[0], to.pos[1] - from.pos[1]);
                let flat = (dx * dx + dy * dy).sqrt();
                if flat < 1e-3 {
                    return Value::Num(0.0);
                }
                Value::Num(((dx * yaw.cos() + dy * yaw.sin()) / flat) as f64)
            }
            // `ApplyForce(thing, v)`: an impulse. The scripts give a pulled item `30` units/s toward the
            // player with gravity cleared, so it flies in; `step` folds this into the thing's velocity.
            "applyforce" => {
                let thing = ti(0);
                let force = [num(1) as f32, num(2) as f32, num(3) as f32];
                let entry = self.force.entry(thing).or_insert([0.0; 3]);
                for (slot, add) in entry.iter_mut().zip(force) {
                    *slot += add;
                }
                Value::Nil
            }
            // `SetActorFlags` writes the **sithActor** word (`actor->flags`, `sithCogFunctionAI.c:237`)
            // — grip's hold, blindness, invulnerability. `AISetMode` is a different engine verb that
            // writes the AI's **own** mode word (`ai + 8`, `FUN_00500f50`): the word every instinct
            // handler tests. The two words were conflated in one map until the flag-word split; the
            // engine's AI bit 0x8 ("turning", `SetLookFrame`) collides with `AF_INVULNERABLE` 0x8, so
            // keeping them apart is what lets the AI word carry the engine's bits 1:1.
            "setactorflags" => {
                let actor = ti(0);
                let mode = idx(1) as u32;
                if mode & AF_INVULNERABLE != 0
                    && std::env::var_os("BK_STACK_TRACE").is_some()
                    && !self.caller_script.is_empty()
                {
                    eprintln!("hit: who set the freeze (cog {} `{}`):\n{}", self.caller_cog, self.caller_script, std::backtrace::Backtrace::force_capture());
                }
                if mode & AF_INVULNERABLE != 0 && std::env::var_os("BK_HIT_TRACE").is_some() {
                    eprintln!(
                        "hit: SetActorFlags({actor}, {mode:#x}) made the player invulnerable (cog {} `{}`)",
                        self.caller_cog,
                        self.caller_script
                    );
                }
                *self.actor_flags.entry(actor).or_insert(0) |= mode;
                Value::Nil
            }
            // `sithCogFunctionAI_AISetMode` (`FUN_00500f50`): `ai flags |= mode`, and when the word
            // changes it raises the scheduler's mode-change event (`sithAI_EmitEvent(ai, 0x100,
            // oldflags)`), which is what wakes the instincts whose registry mask carries 0x100.
            "aisetmode" => {
                let actor = ti(0);
                let mode = idx(1) as u32;
                let old = *self.ai_flags.entry(actor).or_insert(0);
                *self.ai_flags.entry(actor).or_insert(0) |= mode;
                // `aisetmode(enemy, 0x200)` is how `00_aisleep.cog` wakes an actor; the same write
                // puts a script-disabled actor back into the AI (`sithCogFunctionAI.c`). The bit is
                // the engine's *engaged* word (`sithAI_CheckSightThing` writes the same `0x200`), so a
                // script that starts an enemy engaged must latch the target for it. Four sites in the
                // 215 shipped cogs use it (`0x202`: `00_aipatrol` x2, `00_aisleep`; `0x200`: `00_aisleep`).
                if mode & 0x200 != 0 {
                    self.ai_cleared.remove(&actor);
                    self.ai_detected.insert(actor);
                }
                if old != (old | mode) {
                    let _ = self.emit_ai_event_from(actor, 0x100, None, old);
                }
                Value::Nil
            }
            "clearactorflags" => {
                let actor = ti(0);
                let mode = idx(1) as u32;
                if mode & AF_INVULNERABLE != 0 && std::env::var_os("BK_HIT_TRACE").is_some() {
                    eprintln!(
                        "hit: ClearActorFlags({actor}, {mode:#x}) from cog {} `{}` clears the freeze",
                        self.caller_cog,
                        self.caller_script
                    );
                }
                *self.actor_flags.entry(actor).or_insert(0) &= !mode;
                Value::Nil
            }
            "aiclearmode" => {
                let actor = ti(0);
                let mode = idx(1) as u32;
                let old = *self.ai_flags.entry(actor).or_insert(0);
                *self.ai_flags.entry(actor).or_insert(0) &= !mode;
                // `01_initialry.cog` starts with `AIClearMode(joery, 0x606)`: the level's first
                // enemy is walked in by its script and must not acquire or shoot on the way. The
                // engine's tick is switched off by these bits; the port runs the AI regardless,
                // which is why level 1's intro Ry fired at 0.13 s where retail's series shows no
                // shot for its whole 7.4 s (round 307).
                if mode & 0x606 == 0x606 {
                    self.ai_cleared.insert(actor);
                    if std::env::var_os("BK_AI_TRACE").is_some() {
                        eprintln!("ai: AIClearMode switched thing {actor} off (mask {mode:#x})");
                    }
                }
                if old != (old & !mode) {
                    let _ = self.emit_ai_event_from(actor, 0x100, None, old);
                }
                Value::Nil
            }
            "getactorflags" => Value::Num(self.actor_flags.get(&ti(0)).copied().unwrap_or(0) as f64),
            // `AiGetMode(thing)` reads the AI's own flag word (`ai + 8`) — the word `AISetMode`
            // writes and every instinct tests, which is what `actor_*.cog` means by
            // `BitTest(AiGetMode(me), 0x800)`.
            "aigetmode" => Value::Num(self.ai_flags.get(&ti(0)).copied().unwrap_or(0) as f64),
            // `sithCogFunction_GetMasterCog` pushes the master's cog index, or -1 when none is set;
            // `SetMasterCog` stores one and clears it when the index resolves to no cog, which is
            // what `sithCog_GetCogByIndex(-1)` returning NULL does. `GetSelfCog` is the VM's own.
            "getmastercog" => Value::Num(self.master_cog.unwrap_or(-1) as f64),
            "setmastercog" => {
                let cog = idx(0) as i32;
                self.master_cog = (cog >= 0).then_some(cog);
                Value::Nil
            }
            // The engine's arm only forwards to `sithWeapon_SetFireWait` when the thing is the
            // player (`FUN_00507930`), which is what keeps a scripted wait from arming everyone's
            // weapon. The negative case is the engine's own "clear" branch.
            "setfirewait" => {
                if self.types.get(ti(0)).copied() == Some(10) {
                    // A trigger event as well as a cadence: a **saber**'s `fire:` has no projectile to
                    // spawn, it swings and calls `SetFireWait` (11gorc is the saber level), so a check
                    // that counts only projectiles calls a working trigger broken there.
                    self.shots_fired += 1;
                    let wait = num(1) as f32;
                    if wait < 0.0 {
                        self.fire_wait = -1.0;
                        self.next_fire = -1.0;
                    } else {
                        self.fire_wait = wait;
                        self.next_fire = self.time + wait;
                    }
                }
                Value::Nil
            }
            // `AISetMoveSpeed` (`0x500c60`): clamp to `_DAT_00523240`..`_DAT_00523244` (0..2) and
            // store it as the AI's movespeed (`ai + 0x1bc`), for an AI-controlled thing only.
            "aisetmovespeed" => {
                if self.ai.contains_key(&ti(0)) {
                    self.ai_move_speed.insert(ti(0), (num(1) as f32).clamp(0.0, 2.0));
                }
                Value::Nil
            }
            // `AISetMovePos` (`0x500cd0`): `sithAI_SetMoveThing(ai, pos, ai + 0x1bc)` for an
            // AI-controlled thing. `AISetLookPos` keeps the port's stored look point.
            "aisetmovepos" | "aisetlookpos" => {
                let pos = match args.get(1) {
                    Some(Value::Vec(xyz)) => [xyz[0] as f32, xyz[1] as f32, xyz[2] as f32],
                    _ => [0.0; 3],
                };
                if verb.eq_ignore_ascii_case("aisetmovepos") {
                    self.ai_set_move_thing_cog(ti(0), pos);
                } else {
                    self.ai_look.insert(ti(0), pos);
                }
                if std::env::var_os("BK_AI_TRACE").is_some() {
                    println!("ai: {verb} sets thing {} to {pos:?}", ti(0));
                }
                Value::Nil
            }
            // `AISetMoveFrame` (`0x500da0`) / `AISetLookFrame` index the actor's own frame list — the
            // `numframes=`/`frame=(x/y/z:…)` parameters the level places on the thing. The engine
            // refuses a frame past `loadedFrames` (`ai + 0x294`), and this does too.
            "aisetmoveframe" | "aisetlookframe" => {
                let (thing, frame) = (ti(0), idx(1));
                if let Some(at) = self.frame_of(thing, frame) {
                    if verb.eq_ignore_ascii_case("aisetmoveframe") {
                        self.ai_set_move_thing_cog(thing, at);
                    } else {
                        self.ai_look.insert(thing, at);
                    }
                }
                Value::Nil
            }
            // `AISetMoveThing` (`0x500e00`): `sithAI_SetMoveThing` to the thing's position **now**
            // (it is not re-read). shortcut: the engine also makes the thing the AI's target
            // (`ai + 0x214`), which the port's player-only target does not model.
            "aisetmovething" => {
                let (actor, destination) = (ti(0), ti(1));
                if let Some(at) = self.things.get(destination).map(|t| t.pos) {
                    self.ai_set_move_thing_cog(actor, at);
                }
                Value::Nil
            }
            // `SetFaceGeoMode(surface, mode)` / `GetFaceGeoMode(surface)`: `sithSurface_SetGeoMode`.
            // Mode 0 is `GEOMETRY_NONE` — `00_breakingglass.cog` sets it on the glass it shatters and
            // `00_slashablefaces.cog` sets 4 back when the panel is intact — so the renderer has to stop
            // drawing those polygons, not just record the number.
            "setfacegeomode" => {
                let surface = ti(0);
                // Retail keeps `GEOMETRY_NONE` on a face the level does not draw and ignores the
                // argument (`FUN_00500790`: `if (surface->face.material == 0) surface->geoMode = 0`).
                let drawn = self.surface_geomode.get(surface).copied().unwrap_or(0) != 0;
                let mode = if drawn { idx(1) as i32 } else { 0 };
                if self.face_geomode.get(&surface).copied() != Some(mode) {
                    self.face_geomode.insert(surface, mode);
                    self.geomode_epoch = self.geomode_epoch.wrapping_add(1);
                }
                Value::Nil
            }
            "getfacegeomode" => Value::Num(self.face_geomode_at(ti(0)) as f64),
            "setmountwait" => {
                // `sithWeapon_SetMountWait` stores `secWait + gameTime` in one global, and the fire path
                // refuses until that clock passes. `SetMountWait(player, 0)` — what every shipped
                // `endlevel.cog` startup calls — therefore cancels the wait instead of imposing one.
                self.mount_wait_until = self.time + num(1) as f32;
                Value::Nil
            }
            // `PlayKey(thing, key, flags, frame)` — `sithPuppet_PlayKey` swaps a puppet's animation for
            // one of the script's own `keyframe` symbols. Level 1's TIE bomber plays `01VTieB.key` from
            // frame 0x14 the moment the player sees it; `StopAnim` drops back to the puppet's own state
            // track and `JumpToFrame` moves the start of the clip the script already asked for.
            "playkey" => {
                let thing = ti(0);
                let key = args.get(1).map(render).unwrap_or_default();
                if key.is_empty() {
                    self.play_key.remove(&thing);
                } else {
                    let flags = args.get(2).map(|v| v.num() as i32).unwrap_or(0);
                    let frame = args.get(3).map(|v| v.num() as i32).unwrap_or(0);
                    self.play_key.insert(thing, (key.to_ascii_lowercase(), flags, frame));
                    // A key on a **thing** (a script camera's `MAcamera.key`) moves its pose; the key's
                    // node transform is relative to where the thing stood when the key started.
                    if let Some(t) = self.things.get(thing) {
                        self.key_base.insert(thing, (t.pos, t.yaw, t.pitch, t.roll));
                    }
                }
                Value::Num(0.0)
            }
            // `StopKey(thing, handle, blendTime)`: stop the key on that puppet track. This build plays
            // one key per thing, so any handle stops it — the engine's `track` argument only picks among
            // several simultaneous tracks, which is the puppet work.
            "stopkey" => {
                self.play_key.remove(&ti(0));
                Value::Nil
            }
            // `LoadSound(path)`: answer the sound's id, or -1. This build plays sounds by name, so the
            // id is an index into the names it has been given.
            "loadsound" => {
                match args.first() {
                    Some(Value::Str(path)) => {
                        let key = path.to_ascii_lowercase();
                        if !self.loaded_sounds.contains(&key) {
                            self.loaded_sounds.push(key);
                        }
                        Value::Num(self.loaded_sounds.len() as f64)
                    }
                    _ => Value::Num(-1.0),
                }
            }
            "stopanim" => {
                self.play_key.remove(&ti(0));
                Value::Nil
            }
            "jumptoframe" => {
                let thing = ti(0);
                if let Some(entry) = self.play_key.get_mut(&thing) {
                    entry.2 = idx(1) as i32;
                }
                Value::Nil
            }
            // `GetGoalFrame` (`sithCogFunctionThing.c:540`) answers `thing->goalframe`, which
            // `MoveToFrame` sets and `sithTrackThing_Arrivedidk` **resets to 0 the moment the piece
            // arrives** — so a script sees the destination only while the thing is still moving.
            // That is the value `MoveToFrame(elevator, 1 - GetGoalFrame(elevator), speed)` reverses.
            "getgoalframe" => Value::Num(
                self.thing(idx(0)).and_then(|t| t.target).map_or(0.0, |(frame, _)| frame as f64),
            ),
            // `MaterialAnim(material, fps, mode)`: `sithCogFunction_MaterialAnim` builds an animation
            // record whose discrete arm sets `material->curCelNum`, so every surface of that material
            // without a cel of its own animates. The starting cel comes from `mode` (`& 4` -> 2,
            // `& 2` -> 1, else 0) and a non-positive rate is the engine's 15 fps.
            "materialanim" => {
                let fps = if num(1) > 0.0 { num(1) as f32 } else { 15.0 };
                let mode = idx(2) as i32;
                // The argument is a material *name* (`materialanim(animmat, fps, 1)` with
                // `animmat` bound to `09sign.mat` by the level).
                let name = match args.first() {
                    Some(Value::Str(s)) => s.trim().to_ascii_lowercase(),
                    Some(v) => format!("{}", v.num() as i64),
                    None => return Value::Num(-1.0),
                };
                let Some(&material) = self.material_names.get(&name) else {
                    return Value::Num(-1.0);
                };
                let cel = if mode & 4 != 0 {
                    2.0
                } else if mode & 2 != 0 {
                    1.0
                } else {
                    0.0
                };
                let next = self.time + 1.0 / fps;
                self.material_anims.push((material, cel, next, fps));
                self.material_cel.insert(material, cel);
                self.surface_epoch += 1;
                // The engine answers the animation record's index; the material's own index is the
                // handle this port has for the same thing.
                Value::Num(material as f64)
            }
            // ---- the camera slots and POV shake ----
            //
            // `SetCameraFocus(slot, thing)`: `sithCogFunction_SetCameraFocus` bounds the slot to
            // 0..6 and stores the thing; the three-argument spelling carries a secondary focus.
            "setcamerafocus" => {
                let slot = idx(0);
                if (0..7).contains(&slot) {
                    self.camera_focus[slot as usize] = usize::try_from(idx(1)).ok();
                    self.camera_secondary[slot as usize] = usize::try_from(idx(2)).ok();
                }
                Value::Nil
            }
            // `SetCurrentCamera(slot)` (registered as `setcurrentcamera`): which slot the view uses.
            "setcurrentcamera" => {
                let slot = idx(0);
                if (0..7).contains(&slot) {
                    self.current_camera = slot as usize;
                    self.script_camera = true;
                }
                Value::Nil
            }
            "getcameramode" => Value::Num(self.current_camera as f64),
            "getprimaryfocus" => Value::Num(
                self.camera_focus
                    .get(idx(0).max(0) as usize)
                    .copied()
                    .flatten()
                    .map(|t| t as f64)
                    .unwrap_or(-1.0),
            ),
            "getsecondaryfocus" => Value::Num(
                self.camera_secondary
                    .get(idx(0).max(0) as usize)
                    .copied()
                    .flatten()
                    .map(|t| t as f64)
                    .unwrap_or(-1.0),
            ),
            // `SlideHorizonSky(x, y)` / `SlideCeilingSky(x, y)`: `sithSurface_SlideHorizonSky` keeps
            // one world-wide offset per sky kind and the update walks it by this vector, so it is a
            // pair of speeds rather than a per-surface one. `[0]` is the horizon, `[1]` the ceiling.
            "slidehorizonsky" | "slideceilingsky" => {
                let kind = usize::from(verb.eq_ignore_ascii_case("slideceilingsky"));
                self.sky_scroll[kind] = [num(0) as f32, num(1) as f32];
                self.surface_epoch += 1;
                Value::Nil
            }
            // `SetPOVShake(posOffset, angleOffset, posDelta, angleDelta)`: the offset the view takes
            // on, and the rate it walks back to zero at (`sithCamera_SetPOVShake`).
            "setpovshake" => {
                self.shake_pos = args.first().map(vec3).unwrap_or([0.0; 3]);
                self.shake_angle = args.get(1).map(vec3).unwrap_or([0.0; 3]);
                self.shake_pos_rate = num(2) as f32;
                self.shake_angle_rate = num(3) as f32;
                Value::Nil
            }
            // ---- the network verbs, answered for a single-player build ----
            //
            // `sithNet_isMulti` and `sithNet_isServer` are both false in a one-player game, so these are
            // the engine's own answers, not placeholders: no team, no remote player, no score. They are
            // spelled out because the catch-all would answer the same 0 to a getter that must *not* be 0.
            "ismulti" | "isserver" | "getnumplayersinteam" | "getteamscore" | "getplayerscore"
            | "getplayerteam" => Value::Num(0.0),
            "setplayerscore" | "setplayerteam" => Value::Nil,
            "getmultimodeflags" => Value::Num(self.multi_mode_flags as f64),
            "setmultimodeflags" => {
                self.multi_mode_flags |= idx(0) as u32;
                Value::Nil
            }
            "clearmultimodeflags" => {
                self.multi_mode_flags &= !(idx(0) as u32);
                Value::Nil
            }
            // `SendTrigger(thing, type, p0..p3)` (`sithCogFunction.c:1189`) **broadcasts**
            // `SITH_MESSAGE_TRIGGER` (40) when its source is the local player. This port's messages carry
            // labels, not the four parameters, so the broadcast carries the message id alone — which is
            // the part a listener branches on (`message trigger`).
            "sendtrigger" => {
                let sender = (self.caller_cog >= 0).then_some(self.caller_cog as usize);
                self.messages.push(("#40".to_string(), sender, None, [0.0; 2]));
                self.messages_sent += 1;
                Value::Nil
            }
            "getthingsignature" => Value::Num(
                self.signature.get(ti(0)).copied().unwrap_or(0) as f64,
            ),
            // `BitTest(value, mask)`: the shipped call is `BitTest(GetActorFlags(potential), 0x100)`,
            // so the second argument is a mask, not a bit number.
            "bittest" => Value::Num(if (idx(0) as u32) & (idx(1) as u32) != 0 { 1.0 } else { 0.0 }),
            // `SkillTarget(victim, player, bin, rank)`: the engine's per-power effect. For pull (bin 24)
            // it returns the thing that gets thrown — the victim's dropped weapon, or the item itself —
            // and for grip (bin 31) the damage one tick of the hold does. What it returns decides what
            // the scripts do next, so an item always counts as "thrown" and an actor never does: this
            // port does not model disarming.
            // The engine's own shape, from `0x5047e0` (not a named Ghidra procedure, so read from
            // the image with `llvm-objdump`): it takes `(victim, player, bin, rank)`, returns `-1.0`
            // when either thing is null or the **player's** power table (`thing+0x29c`) is empty, and
            // otherwise calls `0x4dedc0(table, skill 0x19, 3, player, 0, victim, 0, bin, rank, 0, 0)`.
            // So grip's rate is a per-skill entry of the player's force table, not the victim's
            // template: this branch is still the port's placeholder until that table is decoded, and
            // it must at least answer the engine's guards rather than indexing a missing thing.
            "skilltarget" => {
                let victim = ti(0);
                let player = ti(1);
                let alive = |i: usize| self.things.get(i).is_some_and(|t| !t.destroyed);
                if !alive(victim) || !alive(player) {
                    return Value::Num(-1.0);
                }
                let bin = idx(2) as i32;
                let type_ = self.types.get(victim).copied().unwrap_or(-1);
                if bin == 24 {
                    // Item (5) throws itself; anything else needs a weapon to drop, which is not
                    // modelled — the script then takes its `else` branch and stops the power.
                    Value::Num(if type_ == 5 { 1.0 } else { -1.0 })
                } else if let Some(answer) = self
                    .thing_class_cog
                    .get(&victim)
                    .and_then(|instance| self.class_skill.get(instance))
                    .and_then(|answers| answers.iter().find(|(b, _)| *b == bin))
                    .map(|(_, value)| *value as f64)
                {
                    // The shipped answer, e.g. `actor_pm.cog`'s 10 for grip.
                    Value::Num(answer)
                } else if bin == 31 {
                    // No class script answers this bin: the engine's `SendMessageEx` returns its
                    // default, which the scripts treat as "no damage".
                    Value::Num(self.template_damage.get(&self.templates[victim].to_ascii_lowercase()).copied().unwrap_or(10.0) as f64)
                } else {
                    Value::Num(0.0)
                }
            }
            "getactorweapon" => Value::Num(
                self.actor_weapon.get(&ti(0)).copied().unwrap_or(-1) as f64,
            ),
            "setactorweapon" => {
                // `(actor, slot, template)`: `-1` clears the slot, which is the pull's disarm.
                self.actor_weapon.insert(ti(0), idx(2) as i32);
                Value::Nil
            }
            // Visual only: the wound a disarmed actor shows. Recorded as a no-op.
            "amputatejoint" => Value::Nil,
            // `PlayMode(thing, mode)`: the victim's choke animation (`25`) or the player's power
            // animation (`24`). The pose comes from the model, which no script can force yet.
            "playmode" => {
                self.play_mode.insert(ti(0), idx(1) as i32);
                Value::Nil
            }
            // `GetHealth`/`GetLevelTime`: the level clock is `time`, seconds the world has stepped.
            "gethealth" => self.call("getthinghealth", args),
            "getleveltime" => Value::Num(self.time as f64),
            "isthingvisible" => Value::Num(self.thing(idx(0)).map_or(0.0, |t| (!t.destroyed) as u8 as f64)),
            "getthingmass" => Value::Num(self.mass.get(&ti(0)).copied().unwrap_or(0.0) as f64),
            "setthingmass" => {
                self.mass.insert(ti(0), num(1) as f32);
                Value::Nil
            }
            "getthinguserdata" => Value::Num(self.user_data.get(&ti(0)).copied().unwrap_or(0.0) as f64),
            "setthinguserdata" => {
                self.user_data.insert(ti(0), num(1) as f32);
                Value::Nil
            }
            // `SetThingTimer(thing, secs)` arms the thing's **own** timer, which is a different
            // field from the life left: `sithCogFunctionThing_SetThingTimer` stores an absolute
            // deadline in `pThing->timer` and sets `SITH_TF_TIMERSET`, and `sithCog_UpdateThingTimer`
            // then sends the `timer` message to the thing's scripts. Aliasing it to `setlifeleft`
            // would have made every scripted timer a death sentence.
            "setthingtimer" => {
                let thing = ti(0);
                let secs = num(1).max(0.0) as f32;
                if secs == 0.0 {
                    self.thing_timer.remove(&thing);
                } else {
                    self.thing_timer.insert(thing, self.time + secs);
                }
                Value::Nil
            }
            "getlifeleft" => Value::Num(self.life_left.get(&ti(0)).copied().unwrap_or(0.0) as f64),
            "setlifeleft" => {
                self.life_left.insert(ti(0), num(1) as f32);
                Value::Nil
            }
            "getitemflags" => Value::Num(self.item_flags.get(&ti(0)).copied().unwrap_or(0) as f64),
            // `GetParam(n)`: the payload a timer carried. `SetTimerEx(delay, type, sender, param)`
            // stores it; grip uses it to check a thing signature is still the same thing.
            "getparam" => Value::Num(usize::try_from(idx(0)).ok().and_then(|i| self.last_params.get(i)).copied().unwrap_or(0.0)),

            "setbinwait" => {
                self.bin_wait.insert(idx(1) as i32, num(2) as f32);
                Value::Nil
            }
            // 1 = standing on a surface, 2 = against a thing face. A player on the floor is
            // `1`; the force scripts only require *either* bit (or deep water).
            "getattachflags" => Value::Num(1.0),
            "getphysicsflags" => Value::Num(self.thing(idx(0)).map(|t| t.phys_flags as f64).unwrap_or(0.0)),
            "setphysicsflags" => {
                if let Some(thing) = self.thing_mut(idx(0)) {
                    thing.phys_flags |= idx(1) as u32;
                }
                Value::Nil
            }
            "clearphysicsflags" => {
                if let Some(thing) = self.thing_mut(idx(0)) {
                    thing.phys_flags &= !(idx(1) as u32);
                }
                Value::Nil
            }

            // `jkGetChoice()`: `jkCog_GetChoice` answers `jkPlayer_GetChoice()`, which is the
            // player's inventory count in **bin 18** (`SITHBIN_CHOICE`) — not a separate flag. The
            // ending scripts branch on it (`19b_endlevel.cog` gives the light path its yellow saber),
            // and `jkPlayer_CalcAlignment` maps 1 to +100 and 2 to -100.
            "jkgetchoice" => Value::Num(self.inv.get(&18).copied().unwrap_or(0.0) as f64),
            // `jkSetSaberInfo(thing, side, tip, base_rad, tip_rad, len, wall, blood, saber)`:
            // `jkCog_SetSaberInfo` pops in reverse and calls `jkSaber_InitializeSaberInfo`, so the
            // thing is `idx(0)` and the blade's own numbers follow it.
            "jksetsaberinfo" => {
                let text = |k: usize| match args.get(k) {
                    Some(Value::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                if let Ok(thing) = usize::try_from(idx(0)) {
                    self.saber.insert(
                        thing,
                        SaberInfo {
                            side: text(1),
                            tip: text(2),
                            base_rad: num(3) as f32,
                            tip_rad: num(4) as f32,
                            length: num(5) as f32,
                            wall_sparks: text(6),
                            blood_sparks: text(7),
                            saber_sparks: text(8),
                        },
                    );
                }
                Value::Nil
            }
            // `SetThingRotVel(thing, vector)`: `sithCogFunctionThing_SetThingRotVel` stores the vector as
            // `physicsParams.angularVelocity`, and only for a physics thing.
            // The weapon mechanism half of the item scripts: `ActivateWeapon(thing, cog, rate, mode)`
            // arms a weapon cog (`sithWeapon_ActivateWeapon`), `DeactivateWeapon` clears it, and
            // `ChangeFireRate` replaces the rate `fire:` computes from `fireWait / powerBoost`.
            "activateweapon" => {
                // `ActivateWeapon(player, rate, mode)` — **three** arguments, and the cog is not one of
                // them: the engine arms the script that called it (`sithWeapon_ActivateWeapon` takes
                // the caller). Reading `idx(1)` as a cog made every weapon arm cog **0**, so the
                // `fire:` message went to an unrelated script and the player's trigger produced
                // nothing on every level (`cog_weap_bryar.cog` calls exactly this three-argument form).
                let cog = self.caller_cog;
                let rate = num(1) as f32;
                let mode = idx(2) as i32;
                if (0..2).contains(&mode) {
                    self.weapon_cog = Some((cog, rate, mode));
                    self.weapon_rate = rate;
                    // `sithInventory_ActivateBin` starts the delay when the bin is armed.
                    self.weapon_ready = self.time + rate.max(0.0);
                }
                Value::Nil
            }
            "deactivateweapon" => {
                self.weapon_cog = None;
                Value::Nil
            }
            "changefirerate" => {
                self.weapon_rate = num(1) as f32;
                if let Some((_, rate, mode)) = self.weapon_cog {
                    self.weapon_cog = Some((self.weapon_cog.map(|c| c.0).unwrap_or(-1), rate, mode));
                }
                Value::Nil
            }
            "setautoswitch" => {
                self.auto_switch = idx(0) as u32;
                Value::Nil
            }
            "getautoswitch" => Value::Num(self.auto_switch as f64),
            "setautopickup" => {
                self.auto_pickup = idx(0) as u32;
                Value::Nil
            }
            "getautopickup" => Value::Num(self.auto_pickup as f64),
            "jksetsuperflags" => {
                self.super_flags |= idx(0) as u32;
                Value::Nil
            }
            "jkclearsuperflags" => {
                self.super_flags &= !(idx(0) as u32);
                Value::Nil
            }
            "jkgetsuperflags" => Value::Num(self.super_flags as f64),
            // A `.key`'s length in frames, for a script that schedules an animation.
            "getkeylen" => {
                let key = match args.first() {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => String::new(),
                };
                Value::Num(self.key_lengths.get(&key).copied().unwrap_or(0.0) as f64)
            }
            // The first-person half: recorded, not drawn — this build has no viewmodel pass.
            "jksetweaponmesh" | "setweaponmesh" => {
                if let Some(Value::Str(mesh)) = args.get(1) {
                    self.weapon_mesh.insert(ti(0), mesh.clone());
                }
                Value::Nil
            }
            "jksetpovmodel" => {
                if let Some(Value::Str(model)) = args.get(1) {
                    self.pov_model.insert(ti(0), model.clone());
                }
                Value::Nil
            }
            "jkplaypovkey" => {
                if let Some(Value::Str(key)) = args.get(1) {
                    self.pov_key.insert(ti(0), key.clone());
                    // `jkPlayer_PlayPOVKey(thing, key, startFrame, flags)`: the third number is the
                    // start frame, the fourth the flags word `sithPuppet_PlayKey` turns into a track
                    // status. `0x4` holds the last frame, `0x20` fades it out, `0x2` ends it; a clip
                    // that sets none of them loops. Mount `0x14` and fire `0x38` are both one-shot.
                    let flags = num(3) as u32;
                    self.pov_key_loop.insert(ti(0), pov_key_loops(flags));
                }
                Value::Num(0.0)
            }
            "jkstoppovkey" => {
                self.pov_key.remove(&ti(0));
                self.pov_key_loop.remove(&ti(0));
                Value::Nil
            }
            "jksetwaggle" => {
                // `jkPlayer_SetWaggle(thing, velocities, rate)`: three amplitudes (degrees) and the
                // phase rate. The rate used to be dropped, which lost half of what the weapon cogs
                // set (`weap_bryar.cog` fires with `10.0 7.0 0.0`, rate 350).
                self.waggle.insert(ti(0), args.get(1).map(vec3).unwrap_or([0.0; 3]));
                self.waggle_rate
                    .insert(ti(0), args.get(2).map(|v| v.num() as f32).unwrap_or(0.0));
                Value::Nil
            }
            // Armed state and selection, which the item scripts set when a weapon is drawn or holstered.
            "setarmedmode" => {
                self.armed_mode.insert(ti(0), idx(1) as u32);
                Value::Nil
            }
            "getcurweaponmode" => {
                Value::Num(self.armed_mode.get(&ti(0)).copied().unwrap_or(0) as f64)
            }
            "setcurweapon" => {
                self.cur_weapon_thing = Some(ti(1));
                Value::Nil
            }
            // Puppet modes: this build plays one key per thing, so the major mode is bookkeeping.
            "setmajormode" => {
                self.major_mode.insert(ti(0), idx(1) as u32);
                Value::Nil
            }
            "getmajormode" => Value::Num(self.major_mode.get(&ti(0)).copied().unwrap_or(0) as f64),
            // The camera slot `SetCurrentCamera` chose.
            // `CycleCamera()` (`0x4c54d0`): the engine's global toggle between camera objects 0 and
            // 1 (`DAT_00549088` = {0, 1}), i.e. first person and the external/saber camera. The
            // shipped callers are the camera scripts, and they only reach it when
            // `jkGetSaberCam() == 1`, so it is how a cutscene hands the view back to the third-person
            // camera the player was already using.
            "cyclecamera" => {
                self.camera_cycle = (self.camera_cycle + 1) % 2;
                self.current_camera = self.camera_cycle;
                if std::env::var_os("BK_CAMERA_TRACE").is_some() {
                    eprintln!("camera: CycleCamera -> slot {}", self.current_camera);
                }
                Value::Nil
            }
            "getcurrentcamera" => Value::Num(self.current_camera as f64),
            // The cog a thing's class names; -1 when it names none.
            "getthingclasscog" => Value::Num(self.thing_class_cog.get(&ti(0)).copied().unwrap_or(-1) as f64),
            // A random unit vector: the engine's `randvec` picks angles, not components, so the
            // result is on the unit sphere.
            "randvec" => {
                let a = (crate::cog::pseudo_random() as f32) * std::f32::consts::TAU;
                let z = (crate::cog::pseudo_random() as f32) * 2.0 - 1.0;
                let r = (1.0 - z * z).max(0.0).sqrt();
                Value::Vec([(r * a.cos()) as f64, (r * a.sin()) as f64, z as f64])
            }
            // A thing's own thrust, from its template's `maxthrust`.
            "getthingthrust" => {
                let template = self.templates.get(ti(0)).cloned().unwrap_or_default();
                let thrust = self
                    .template_speed
                    .get(&template.to_ascii_lowercase())
                    .copied()
                    .unwrap_or(0.0);
                Value::Num(thrust as f64)
            }
            // The saber's own camera and enable flags: the saber camera is a view this build has no
            // renderer for, and `jkDisableSaber` turns the blade off.
            "jkenablesaber" => {
                self.saber_enabled = true;
                Value::Nil
            }
            "jkdisablesaber" => {
                self.saber_enabled = false;
                Value::Nil
            }
            "jkgetsabercam" => Value::Num(self.saber_cam as f64),
            "setthingrotvel" => {
                if let Some(thing) = self.thing_mut(idx(0))
                    && thing.phys_flags != u32::MAX
                {
                    let v = args.get(1).map(vec3).unwrap_or([0.0; 3]);
                    thing.rot_vel = v;
                }
                Value::Nil
            }
            // `AutoSelectWeapon(player, kind)`: `sithCogFunction_AutoSelectWeapon` answers the bin
            // `sithWeapon_AutoSelect` picked, or -1.
            "autoselectweapon" => Value::Num(self.auto_select_weapon(idx(1) as i32) as f64),
            "jkgetflags" => Value::Num(self.thing(idx(0)).map(|t| t.jk_flags as f64).unwrap_or(0.0)),
            "jksetflags" => {
                if let Some(thing) = self.thing_mut(idx(0)) {
                    thing.jk_flags |= idx(1) as u32;
                }
                Value::Nil
            }
            // `jkClearFlags` is the mask side of `jkSetFlags` (`thing->jkFlags &= ~flags`), used by the
            // dark-jedi cutscene scripts to drop the 0x20 (disarmed?) bit once they have fired.
            "jkclearflags" => {
                if let Some(thing) = self.thing_mut(idx(0)) {
                    thing.jk_flags &= !(idx(1) as u32);
                }
                Value::Nil
            }
            "jksetpersuasioninfo" => Value::Nil,
            // Sound lengths: the scripts compare a power's hold time against a clip's
            // length to decide whether to cut the sound off. Without the mixer's metadata
            // this answers one second, which keeps that comparison sane.
            "getsoundlen" => Value::Num(1.0),
            // `SetCurInvWeapon(thing, bin)` / `SelectWeapon(thing, bin)`: the engine arms a
            // weapon by inventory bin, not by template name.
            // `SetCurInvWeapon(thing, bin)` / `SelectWeapon(thing, bin)`: the engine arms a
            // weapon by inventory bin, not by template name.
            "setcurinvweapon" | "selectweapon" | "assignweapon" => {
                self.select_bin(idx(1) as i32);
                Value::Nil
            }
            "getcurinvweapon" => Value::Num(self.cur_bin as f64),
            // `sithItem_SetItemTaken` (`0x4fbea0`) sends `taken` for the local player;
            // the item's COG owns the inventory effect, not the collision handler.
            "takeitem" => {
                let item = idx(0);
                let index = usize::try_from(item).unwrap_or(usize::MAX);
                let template = self.templates.get(index).cloned().unwrap_or_default();
                if let Some(player) = self.local_player()
                    && idx(1) == player as i64
                    && self.types.get(index) == Some(&5)
                    && self.things.get(index).is_some_and(|thing| !thing.destroyed)
                {
                    self.sith_cog_item_send_message(index, player, "taken");
                }
                if let Some(thing) = self.thing_mut(item) {
                    thing.destroyed = true;
                    self.note_taken(index, &template);
                }
                Value::Nil
            }
            "capturething" => {
                // `CaptureThing(thing)` — `FUN_00502e40`: the engine stores the calling cog at
                // `thing + 0x2a0` and sets `SITH_TF_CAPTURED`. It does **not** remove the thing:
                // `00_REPLACEENEMY.cog` captures the enemy it just created, and destroying it there
                // kept the three medium-difficulty replacements out of the level.
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.flags |= TF_CAPTURED;
                }
                if let Ok(thing) = usize::try_from(idx(0))
                    && self.caller_cog >= 0
                    && self.things.get(thing).is_some()
                {
                    self.thing_capture_cog.insert(thing, self.caller_cog);
                }
                Value::Nil
            }
            "getthingtype" => Value::Num(
                self.types
                    .get(usize::try_from(idx(0)).unwrap_or(usize::MAX))
                    .copied()
                    .unwrap_or(0) as f64,
            ),
            // The engine's `SectorAdjoins(sector, 0|1)` is the same switch as
            // `SetSectorAdjoins`; the shipped scripts spell it both ways.
            "sectoradjoins" => self.call("setsectoradjoins", args),
            // Sector iteration: `FirstThingInSector` gives the first thing whose own
            // sector is that one, then `NextThingInSector` walks the rest by index.
            "firstthinginsector" => {
                let sector = idx(0) as i32;
                Value::Num(self.first_in_sector(sector) as f64)
            }
            "nextthinginsector" => {
                let sector = idx(0) as i32;
                let after = idx(1);
                Value::Num(self.next_in_sector(sector, after) as f64)
            }
            // Script text: the argument is a `.uni` key (`jkPrintUNIString("GUI_NOPE")`).
            // The `jkString*` buffer: `jkCog_jkstring` is one 128-character buffer the concat verbs
            // append to, `jkStringClear` empties, and `jkStringOutput` puts up. The engine's own cap is
            // 128 characters (`finalLen < 0x81`), so an overlong line is dropped rather than appended.
            "jkstringclear" => {
                self.string_buffer.clear();
                Value::Nil
            }
            "jkstringconcatint" => {
                self.string_buffer_push(&format!("{:.0}", num(0)));
                Value::Nil
            }
            "jkstringconcatflex" => {
                self.string_buffer_push(&format!("{:.2}", num(0)));
                Value::Nil
            }
            "jkstringconcatspace" => {
                self.string_buffer_push(" ");
                Value::Nil
            }
            "jkstringconcatunistring" => {
                // A `.uni` key by number: resolve it through the install's table when the app prints,
                // so the buffer keeps the key the script named.
                let text = match args.first() {
                    Some(Value::Str(s)) => s.clone(),
                    _ => format!("COG_{:05}", num(0) as i64),
                };
                self.string_buffer_push(&text);
                Value::Nil
            }
            "jkstringconcatstring" | "jkstringconcatasciistring" => {
                let text = match args.first() {
                    Some(Value::Str(s)) => s.clone(),
                    _ => format!("{:.0}", num(0)),
                };
                self.string_buffer_push(&text);
                Value::Nil
            }
            "jkstringconcatvector" => {
                let v = args.first().map(vec3).unwrap_or([0.0; 3]);
                self.string_buffer_push(&format!("{:.2} {:.2} {:.2}", v[0], v[1], v[2]));
                Value::Nil
            }
            "jkstringconcatplayername" => {
                self.string_buffer_push("Kyle");
                Value::Nil
            }
            // `jkStringOutput(player, type)`: the finished line goes up. This build has no HUD text
            // channel of its own, so it lands in the same `prints` list `jkPrintUNIString` writes.
            "jkstringoutput" => {
                let line = std::mem::take(&mut self.string_buffer);
                if !line.is_empty() {
                    self.prints.push(line.clone());
                    self.hud_message = Some((line, self.time + Self::HUD_MESSAGE_TIME));
                }
                Value::Nil
            }
            // `SyncScores`: multiplayer score replication. Single-player has no scores to sync, and
            // the engine's own arm does nothing when `sithNet_isMulti` is clear.
            "syncscores" => Value::Nil,
            // `jkPrintUNIString(thing, key)`: the engine puts the string in the message area, the
            // same place `jkStringOutput` sends a composed line. The first argument is the thing the
            // line belongs to — every shipped caller passes the player — and the **second** is the
            // `.uni` key, either a name or its numeric index (`jkPrintUNIString(player, bin)` in
            // `pow_shields.cog` names bin 60, "Shield Recharge"). Reading the first argument printed
            // `COG_00000` — the player's own thing index — where the pickup's name belonged.
            "jkprintunistring" => {
                let key_at = if args.len() >= 2 { 1 } else { 0 };
                let line = match args.get(key_at) {
                    Some(Value::Str(s)) => s.clone(),
                    Some(_) => format!("COG_{:05}", num(key_at) as i64),
                    None => String::new(),
                };
                if !line.is_empty() {
                    self.prints.push(line.clone());
                    self.hud_message = Some((line, self.time + Self::HUD_MESSAGE_TIME));
                }
                Value::Nil
            }
            // `CreateThing(template, positionSource)`: same position and sector as the
            // source thing, health from the template. Pushes the new index, or -1.
            "creatething" => {
                let template = match args.first() {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => return Value::Num(-1.0),
                };
                let source = idx(1);
                let Some(src) = self.thing(source) else { return Value::Num(-1.0) };
                // `sithThing_CreateThing`: the marker's orientation carries over, and the position is the
                // marker's **plus the difference between the two insert offsets**, rotated into the marker's
                // frame. Copying the marker's position alone lands a spawned prop offset by its own height.
                let (src_pos, src_sector) = (src.pos, src.sector);
                let (src_pitch, src_yaw, src_roll) = (src.pitch, src.yaw, src.roll);
                let src_insert = src.insert;
                let insert = template_insert(&template);
                let delta = [
                    insert[0] - src_insert[0],
                    insert[1] - src_insert[1],
                    insert[2] - src_insert[2],
                ];
                let moved = rotate_by_angles(delta, src_pitch, src_yaw, src_roll);
                let pos = [src_pos[0] + moved[0], src_pos[1] + moved[1], src_pos[2] + moved[2]];
                let sector = src_sector;
                Value::Num(self.spawn_at(template, pos, sector, [src_pitch, src_yaw, src_roll]) as f64)
            }
            // `CreateThingAtPosNR(template, sector, pos, angles)` — the general spawner behind the force
            // scripts' cones and spheres (`force_absorb`'s `cone_tpl`, `force_grip`'s `sphere_tpl`,
            // `item_wrenchyellow`'s splashes). It takes the position it is given, unlike `CreateThing`,
            // which offsets from a marker's insert point.
            "createthingatposnr" => {
                let template = match args.first() {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => return Value::Num(-1.0),
                };
                let sector = idx(1) as i32;
                let pos = args.get(2).map(vec3).unwrap_or([0.0; 3]);
                let angles = args.get(3).map(vec3).unwrap_or([0.0; 3]);
                Value::Num(self.spawn_at(template, pos, sector, angles) as f64)
            }
            // `CreateThingNR(template, source)`: the source thing's position and sector with no
            // orientation — `creatething` without the marker's rotation carried across.
            "createthingnr" => {
                let template = match args.first() {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => return Value::Num(-1.0),
                };
                let source = idx(1);
                let Some(src) = self.thing(source) else { return Value::Num(-1.0) };
                let (pos, sector) = (src.pos, src.sector);
                Value::Num(self.spawn_at(template, pos, sector, [0.0, 0.0, 0.0]) as f64)
            }
            // `sithInventory_CreateBackpack` (`0x4d2740`): a `+backpack` thing at the player's own pose
            // carrying the inventory, at most `0x10` bins, each kept when its amount is above
            // `_DAT_00522ea8`, which the executable spells **0.0** — so any amount at all.
            // shortcut: the engine also requires the `items.dat` record's `0x1`/`0x80` flag bits, which
            // the runtime does not keep; a bin with an amount is carried in every shipped script.
            "createbackpack" => {
                let player = idx(0);
                let Some(p) = self.thing(player) else { return Value::Num(0.0) };
                let (pos, sector) = (p.pos, p.sector);
                let thing = self.spawn_at("+backpack".into(), pos, sector, [0.0, 0.0, 0.0]);
                let items: Vec<(i32, f32)> = self
                    .inv
                    .iter()
                    .filter(|(_, amount)| **amount > 0.0)
                    .take(16)
                    .map(|(bin, amount)| (*bin, *amount))
                    .collect();
                self.backpack.insert(thing, items);
                Value::Num(thing as f64)
            }
            // `DamageThing(thing, damage)`: the health bookkeeping only. Death, the
            // killer's credit and the `killed` messages belong to combat (stage 5).
            "damagething" => {
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.health -= num(1) as f32;
                }
                Value::Nil
            }
            "getthingpos" => match self.thing(idx(0)) {
                Some(t) => Value::Vec([t.pos[0] as f64, t.pos[1] as f64, t.pos[2] as f64]),
                None => Value::Vec([0.0; 3]),
            },
            // `GetThingLVec(thing)`: the thing's look vector, the local `+Y` through its own pitch,
            // yaw and roll. `weap_bryar`/`weap_fists` kick the shooter back with
            // `ApplyForce(player, VectorScale(GetThingLVec(player), -80))`, so a missing answer is a
            // firearm with no recoil at all.
            "getthinglvec" => match self.thing(idx(0)) {
                Some(t) => {
                    let v = rotate_by_angles([0.0, 1.0, 0.0], t.pitch, t.yaw, t.roll);
                    Value::Vec([v[0] as f64, v[1] as f64, v[2] as f64])
                }
                None => Value::Vec([0.0; 3]),
            },
            "setthingpos" => {
                let p = args.get(1).map(vec3).unwrap_or([0.0; 3]);
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.pos = p;
                }
                Value::Nil
            }
            "getthingrot" => match self.thing(idx(0)) {
                Some(t) => Value::Vec([t.pitch as f64, t.yaw as f64, t.roll as f64]),
                None => Value::Vec([0.0; 3]),
            },
            "setthingrot" => {
                let r = args.get(1).map(vec3).unwrap_or([0.0; 3]);
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.pitch = r[0];
                    t.yaw = r[1];
                    t.roll = r[2];
                }
                Value::Nil
            }
            // `SetThingLook(thing, dir, blend)` — `sithPhysics_SetThingLook` (`0x4f5a80`): aim the
            // thing's forward axis along `dir`, re-orthogonalising the basis and setting the jkFlag
            // `0x100`. A zero blend snaps (`*forward = *dir`); anything else turns the forward part of
            // the way there (`forward += dir * blend * K`, normalised). The port keeps pitch/yaw/roll,
            // so the same thing is a direction-to-angles conversion, blended on the vector.
            "setthinglook" => {
                let dir = args.get(1).map(vec3).unwrap_or([0.0, 1.0, 0.0]);
                let blend = num(2) as f32;
                let Some(t) = self.thing_mut(idx(0)) else { return Value::Nil };
                let cur = rotate_by_angles([0.0, 1.0, 0.0], t.pitch, t.yaw, t.roll);
                let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
                if len > 1e-6 {
                    let dir = [dir[0] / len, dir[1] / len, dir[2] / len];
                    let mixed = if blend == 0.0 {
                        dir
                    } else {
                        // `blend * _DAT_005231a0`: the executable's scale for that partial turn is **10.0**.
                        let m = [
                            cur[0] + dir[0] * blend * 10.0,
                            cur[1] + dir[1] * blend * 10.0,
                            cur[2] + dir[2] * blend * 10.0,
                        ];
                        let l = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
                        if l < 1e-6 { dir } else { [m[0] / l, m[1] / l, m[2] / l] }
                    };
                    // The port's forward is the `l` row of `rotate_by_angles`: `[-sy*cx, cy*cx, sx]`.
                    t.pitch = mixed[2].clamp(-1.0, 1.0).asin().to_degrees();
                    t.yaw = (-mixed[0]).atan2(mixed[1]).to_degrees();
                    t.jk_flags |= 0x100;
                }
                Value::Nil
            }
            // `SetThingPulse(thing, seconds)`: start the thing's own pulse timer; `0` stops it.
            "setthingpulse" => {
                let interval = num(1) as f32;
                let now = self.time;
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.pulse = interval.max(0.0);
                    t.pulse_at = now + interval.max(0.0);
                }
                Value::Nil
            }
            "getthingsector" => Value::Num(self.thing(idx(0)).map(|t| t.sector as f64).unwrap_or(-1.0)),
            "setthingsector" => {
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.sector = idx(1) as i32;
                }
                Value::Nil
            }
            "getthingflags" => Value::Num(self.thing(idx(0)).map(|t| t.flags as f64).unwrap_or(0.0)),
            // `SetThingFlags` **ORs** (`thing->flags |= flags`) and `ClearThingFlags` masks
            // (`&= ~flags`) — `sithCogFunctionThing.c:1334`. Neither assigns: assigning would drop
            // bits the level set, and `IN_CAMFOV` is one of them, which would make `sighted` fire
            // again for a thing the player has already seen.
            "setthingflags" => {
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.flags |= idx(1) as u32;
                }
                Value::Nil
            }
            "clearthingflags" => {
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.flags &= !(idx(1) as u32);
                }
                Value::Nil
            }
            "getthinghealth" => Value::Num(self.thing(idx(0)).map(|t| t.health as f64).unwrap_or(0.0)),
            "setthinghealth" => {
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.health = num(1) as f32;
                }
                Value::Nil
            }
            "getthingtemplate" => self
                .templates
                .get(idx(0) as usize)
                .map(|t| Value::Str(t.clone()))
                .unwrap_or(Value::Nil),
            "getcurframe" => Value::Num(self.thing(idx(0)).map(|t| t.frame as f64).unwrap_or(0.0)),
            "ismoving" | "isthingmoving" => {
                Value::Num(self.thing(idx(0)).map(|t| t.is_moving()).unwrap_or(false) as u8 as f64)
            }
            // `MoveToFrame(thing, frame, speed)` — the door and lift primitive.
            "movetoframe" => {
                if let Some(t) = self.thing_mut(idx(0))
                    && t.frames.len() > 1
                {
                    let last = (t.frames.len() - 1) as f32;
                    t.target = Some(((num(1) as f32).clamp(0.0, last), num(2) as f32));
                    self.things_moved += 1;
                }
                Value::Nil
            }
            "stopthing" => {
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.target = None;
                }
                Value::Nil
            }
            // `SetSectorAdjoins(sector, 0|1)` is `sithSector_HideSectorAdjoins`/`ShowSectorAdjoins`:
            // it switches `SITH_SECTOR_ADJOINSOFF` (0x80), and that is how every shipped door
            // script closes its own doorway (`00_door.cog`: `SetSectorAdjoins(doorSector, 0)` on
            // startup, `1` when it opens). `SectorAdjoins` is the same verb under its other spelling.
            // `0x4f8de0`/`0x4f8e10` also hide/restore each owned adjoin via marker `0x20`, preserving MOVE.
            "setsectoradjoins" => {
                let shut = idx(1) == 0;
                let sector = idx(0);
                let mut changed = false;
                if let Some(sector_flags) = self.sector_flags.get_mut(sector as usize) {
                    for (&owner, &row) in self.surface_sector.iter().zip(&self.surface_adjoin) {
                        if owner != sector as i32 { continue; }
                        let Some(flags) = usize::try_from(row).ok().and_then(|r| self.adjoin_flags.get_mut(r)) else { continue };
                        let next = if shut && *flags & 1 != 0 {
                            *flags & !1 | 0x20
                        } else if !shut && *flags & 0x20 != 0 {
                            *flags & !0x20 | 1
                        } else {
                            *flags
                        };
                        changed |= next != *flags;
                        *flags = next;
                    }
                    changed |= (*sector_flags & SECTOR_ADJOINS_OFF != 0) != shut;
                    *sector_flags = if shut { *sector_flags | SECTOR_ADJOINS_OFF } else { *sector_flags & !SECTOR_ADJOINS_OFF };
                }
                if changed {
                    self.adjoins_changes += 1;
                }
                Value::Nil
            }
            "getsectoradjoins" => Value::Num(
                self.sector_flags
                    .get(idx(0) as usize)
                    .map(|f| (f & SECTOR_ADJOINS_OFF == 0) as u8 as f64)
                    .unwrap_or(0.0),
            ),
            // `SetAdjoinFlags(surface, bits)` / `ClearAdjoinFlags(surface, bits)`:
            // `pSurface->pAdjoin->flags |= / &= ~bits`, on the row that surface carries — the row
            // the sweep reads when a body crosses that way (`collide::Gates`).
            "setadjoinflags" | "clearadjoinflags" => {
                let set = verb.eq_ignore_ascii_case("setadjoinflags");
                let bits = idx(1) as u16;
                let row = self.surface_adjoin.get(idx(0) as usize).copied().unwrap_or(-1);
                if let Some(flags) = usize::try_from(row).ok().and_then(|r| self.adjoin_flags.get_mut(r)) {
                    *flags = if set { *flags | bits } else { *flags & !bits };
                }
                Value::Nil
            }
            // `SetSectorFlags(sector, bits)`: `sector->flags |= bits` and nothing else. The one
            // bit a shipped script sets is `SITH_SECTOR_UNDERWATER` (0x2), which is what makes a
            // "passthru layer" a water volume.
            "setsectorflags" => {
                if let Some(flags) = self.sector_flags.get_mut(idx(0) as usize) {
                    *flags |= idx(1) as u32;
                }
                Value::Nil
            }
            "setsectorlight" => {
                // The spelling every shipped script uses. `sectorlight` below is its alias.
                self.set_sector_light(idx(0), num(1) as f32, args.get(2).map(|v| v.num() as f32).unwrap_or(0.0));
                Value::Nil
            }
            "setsectortint" => {
                let t = args.get(1).map(vec3).unwrap_or([0.0; 3]);
                if let Some(s) = self.sector_mut(idx(0)) {
                    s.tint = t;
                }
                Value::Nil
            }
            "setcolormap" | "setsectorcolormap" => {
                if let Some(s) = self.sector_mut(idx(0)) {
                    s.colormap = idx(1) as i32;
                }
                Value::Nil
            }
            "getsectorflags" => {
                Value::Num(self.sector_flags.get(idx(0) as usize).copied().unwrap_or(0) as f64)
            }
            "playsoundthing" | "playsoundpos" | "playsoundlocal" | "playsound" => {
                let name = args.first().map(render).unwrap_or_default();
                // `PlaySoundThing(sound, thing, volume, pan, flags)` and
                // `PlaySoundPos(sound, pos, volume, …)` take the volume third;
                // `PlaySoundLocal(sound, volume, pan, flags)` takes it second.
                let lower = verb.to_ascii_lowercase();
                let vol_at = if lower.contains("local") { 1 } else { 2 };
                let volume = args.get(vol_at).map(|v| v.num() as f32).unwrap_or(1.0);
                // The position: a thing index for `PlaySoundThing`, three numbers for
                // `PlaySoundPos`. `PlaySoundLocal` has none.
                let pos = if lower.contains("pos") {
                    let xyz: Vec<f32> = args.iter().skip(1).take(3).map(|v| v.num() as f32).collect();
                    (xyz.len() == 3).then(|| [xyz[0], xyz[1], xyz[2]])
                } else if lower.contains("thing") {
                    self.things.get(idx(1) as usize).map(|t| t.pos)
                } else {
                    None
                };
                if !name.is_empty() {
                    // `PlaySoundThing(sound, thing, volume, min, max, flags)` and
                    // `PlaySoundPos(sound, pos, volume, min, max, flags)` carry the mixer flags in
                    // their last argument; `PlaySoundLocal(sound, volume, pan, flags)` in its fourth.
                    // `0x045` (the ambient installs pass it) sets the loop bit, so the request is a
                    // **channel** the mixer keeps playing, not a one-shot. Without this the
                    // `00_PlayAmbient.cog` rumble was requested every 0.5 s and dropped (no `.snd`
                    // clip), and the audio trace could not see the bar's only loop.
                    let flags_at = match lower.as_str() {
                        "playsoundlocal" => 3,
                        _ => 5,
                    };
                    let flags = if args.len() > flags_at { args[flags_at].num() as i32 } else { 0 };
                    let (min_radius, max_radius) = match lower.as_str() {
                        "playsoundlocal" => (0.0, 0.0),
                        _ => (
                            args.get(3).map(|v| v.num() as f32).unwrap_or(0.0),
                            args.get(4).map(|v| v.num() as f32).unwrap_or(0.0),
                        ),
                    };
                    // `PlaySound*` answers the **channel**, which is what `ChangeSoundVol` and
                    // `ChangeSoundPitch` take — `sithSoundMixer_PlaySound` returns one and the shipped
                    // scripts store it (`02_crowstart.cog`).
                    let channel = self.sound_volume.len();
                    self.sound_volume.push(volume);
                    self.sound_pitch.push(1.0);
                    let source = match lower.as_str() {
                        "playsoundlocal" => "cog PlaySoundLocal -> sithSoundMixer_PlaySound (0x4db4f0)",
                        "playsoundpos" => "cog PlaySoundPos -> sithSoundMixer_PlaySoundPos (0x4db880)",
                        "playsound" => "cog PlaySound -> sithSoundMixer_PlaySoundThing (0x4dba60)",
                        _ => "cog PlaySoundThing -> sithSoundMixer_PlaySoundThing (0x4dba60)",
                    };
                    if flags & 0x1 != 0 {
                        // The engine's ambient installs re-issue the same request every 0.5 s; the
                        // mixer holds one loop per name and place, so refresh instead of stacking.
                        if let Some(existing) = self
                            .sound_loops
                            .iter_mut()
                            .find(|l| l.name.eq_ignore_ascii_case(&name) && l.pos == pos)
                        {
                            existing.volume = volume;
                            existing.min_radius = min_radius;
                            existing.max_radius = max_radius;
                            existing.flags = flags;
                        } else {
                            self.sound_loops.push(SoundLoop {
                                name,
                                volume,
                                pos,
                                min_radius,
                                max_radius,
                                flags,
                            });
                        }
                    } else {
                        self.sounds.push(SoundRequest { name, volume, pos, channel, pitch: 1.0, source });
                    }
                    Value::Num(channel as f64)
                } else {
                    Value::Num(-1.0)
                }
            }
            // `ChangeSoundVol(channel, volume, fade)`: `sithCogFunctionSound_ChangeVolume` acts only
            // when the fade is positive, clamps to [0, 1.5], and ramps the channel's volume — every
            // shipped call passes a real time (`1.0`, `2.5`, `8.7`), so a zero fade really is a no-op.
            "changesoundvol" => {
                let channel = usize::try_from(idx(0)).ok();
                let volume = (num(1) as f32).clamp(0.0, 1.5);
                let fade = num(2) as f32;
                if let Some(channel) = channel
                    && channel < self.sound_volume.len()
                    && fade > 0.0
                {
                    let rate = (volume - self.sound_volume[channel]) / fade;
                    self.sound_fade.insert(channel, (volume, rate));
                }
                Value::Nil
            }
            // `ChangeSoundPitch(channel, pitch, fade)`: `sithCogFunctionSound_ChangePitch` needs both a
            // positive pitch and a positive time, then ramps the channel's playback rate.
            "changesoundpitch" => {
                let channel = usize::try_from(idx(0)).ok();
                let pitch = num(1) as f32;
                let fade = num(2) as f32;
                if let Some(channel) = channel
                    && channel < self.sound_pitch.len()
                    && pitch > 0.0
                    && fade > 0.0
                {
                    self.sound_pitch[channel] = pitch;
                }
                Value::Nil
            }
            // `SectorSound(sector, sound, volume)`: `sithSoundMixer_SetSectorAmbientSound` replaces the
            // sector's ambient sound and its volume — level 3's paddle pool swaps in `wind` at 0.3.
            "sectorsound" => {
                let sector = idx(0) as i32;
                let name = match args.get(1) {
                    Some(Value::Str(s)) => s.to_ascii_lowercase(),
                    _ => String::new(),
                };
                let volume = num(2) as f32;
                if !name.is_empty() {
                    self.sector_ambient.insert(sector, (name, volume));
                }
                Value::Nil
            }
            // `sleep`/`waitformessage`/`settimer`/`settimerex`/`setpulse` are handled by the **cog VM
            // first** (`cog::Vm::call_verb`), which suspends the label and resumes it across frames
            // (`Vm::resume`, `resume_sleeping_labels`, gated by `--sleep`). This arm is only reached by
            // a *direct* runtime call for those names — a script call never gets here — so it records
            // the wait without scheduling it, and that is why it is not the script's clock.
            "sleep" | "settimer" | "setpulse" | "settimerx" => {
                self.timers += num(0).max(0.0) as f32;
                Value::Nil
            }
            // `GetInv(thing, bin)`, `SetInv(thing, bin, value)`, `ChangeInv(thing, bin, delta)`:
            // the *thing* is the first argument (the player, in every powerup script), so the
            // bin is the second. Reading the bin from argument 0 put every pickup's count in
            // bin 0 — the inventory looked empty and the bins never moved.
            "getinv" => Value::Num(self.inv.get(&(idx(1) as i32)).copied().unwrap_or(0.0) as f64),
            // How many of a bin the player can hold. The engine's `sithInventory_GetInventoryMaximum`
            // answers the class's own ceiling; the loader hands `items.dat`'s `max` column over, so a
            // bin the file names gets its real number (shields 200, energy 50, …). A bin it does not
            // name — the engine's own counters (goals, alignment, mana) — has no ceiling here, so the
            // current amount is reported and a powerup guard declines instead of stacking without end.
            "getinvmax" => Value::Num(match self.item_max.get(&(idx(1) as i32)) {
                Some(max) => *max as f64,
                // A bin `items.dat` does not name — the engine's own counters, and every bin in a
                // check that builds a runtime without the loader's table — has no known ceiling, so
                // answer roomy: a powerup's guard then accepts the item the way retail's does. The
                // engine's own bin records do carry maxima, but they are not recovered here.
                None => 1.0e9,
            }),
            "setinvavailable" => {
                self.inv_available.insert(idx(0) as i32, idx(1) != 0);
                Value::Nil
            }
            // `AddDynamicTint(player, r, g, b)`: `sithPlayer_AddDynamicTint` **adds** to the player's
            // palette tint with the engine's clamps — a powerup flash and damage share the channels —
            // so this is not a replacement.
            // `AddTint(player, r, g, b)` is the same channel `AddDynamicTint` writes:
            // `pow_keyyellow` adds `(0, 0, 0.2)` through one name and `00_aiturncoat` the same value
            // through the other.
            "adddynamictint" | "addtint" => {
                add_dynamic_tint(&mut self.tint, [num(1) as f32, num(2) as f32, num(3) as f32]);
                Value::Nil
            }
            // `NewColorEffect(filterR, filterG, filterB, tintR, tintG, tintB, addR, addG, addB, fade)`:
            // ten arguments, answered with a handle. The engine's `stdPalEffects` table applies the
            // filter/tint/add to the palette and fades it out over `fade` seconds; this build keeps
            // the request (the draw is the screen-effect pass it does not have yet) so a script can
            // create, modify and free one and read back the same arithmetic.
            "newcoloreffect" => {
                let handle = self.next_color_effect;
                self.next_color_effect += 1;
                self.color_effects.insert(
                    handle,
                    ColorEffect {
                        filter: [idx(0) as i32, idx(1) as i32, idx(2) as i32],
                        tint: [num(3) as f32, num(4) as f32, num(5) as f32],
                        add: [idx(6) as i32, idx(7) as i32, idx(8) as i32],
                        fade: num(9) as f32,
                        started: self.time,
                    },
                );
                Value::Num(handle as f64)
            }
            // `ModifyColorEffect`: change one that exists, answering 1 or 0.
            "modifycoloreffect" => {
                let handle = idx(0) as i32;
                let Some(effect) = self.color_effects.get_mut(&handle) else {
                    return Value::Num(0.0);
                };
                effect.filter = [idx(1) as i32, idx(2) as i32, idx(3) as i32];
                effect.tint = [num(4) as f32, num(5) as f32, num(6) as f32];
                effect.add = [idx(7) as i32, idx(8) as i32, idx(9) as i32];
                effect.fade = num(10) as f32;
                effect.started = self.time;
                Value::Num(1.0)
            }
            // `FreeColorEffect`: drop one, answering 1 or 0.
            "freecoloreffect" => Value::Num(
                self.color_effects.remove(&(idx(0) as i32)).map(|_| 1.0).unwrap_or(0.0),
            ),
            // `SetThingModel(thing, model)`: `sithThing_SetThingModel` swaps the thing's mesh. The CTF
            // scripts are the only callers, swapping a player's body for their team's model. The
            // renderer resolves a *created* thing through this override; a thing the level placed keeps
            // the mesh built at load, because replacing it means despawning and rebuilding its entities.
            // `AutosaveGame()`: the level's own story beats ask for a checkpoint (7, 11, 15, 17a,
            // 17b and 20 call it from `rahnlines`/`endlevel`). The flag is what the app writes.
            "autosavegame" => {
                self.autosave = true;
                Value::Nil
            }
            "setthingmodel" => {
                if let Ok(thing) = usize::try_from(idx(0))
                    && let Some(Value::Str(model)) = args.get(1)
                {
                    self.thing_model.insert(thing, model.clone());
                }
                Value::Nil
            }
            "setinv" => {
                self.set_bin(idx(1) as i32, num(2) as f32);
                Value::Nil
            }
            // `sithInventory_PickupBackpack` (`0x4d2860`): add the backpack's bins back to the player.
            // The caller destroys the thing (`item_wrenchyellow.cog` does), so the record goes with it.
            "pickupbackpack" => {
                // The inventory is the player's own here, so the first argument only names it.
                let thing = ti(1);
                let carried = self.backpack.get(&thing).cloned().unwrap_or_default();
                for (bin, amount) in carried {
                    let now = self.inv.get(&bin).copied().unwrap_or(0.0) + amount;
                    let now = self.clamp_bin(bin, now);
                    self.inv.insert(bin, now);
                }
                self.backpack.remove(&thing);
                Value::Nil
            }
            "changeinv" => {
                // `sithInventory_ChangeInventory` (`0x4d1ef0`) adds, then clamps to the item's own
                // `min`/`max`.
                let bin = idx(1) as i32;
                let now = self.inv.get(&bin).copied().unwrap_or(0.0) + num(2) as f32;
                let now = self.clamp_bin(bin, now);
                self.inv.insert(bin, now);
                Value::Nil
            }
            // `SetGoalFlags(player, bin, flags)` ORs flags into a goal counter: the engine keeps them
            // as inventory bins starting at `SITHBIN_GOAL00` (100), and `endlevel.cog` records its
            // objectives there. `ClearGoalFlags` is the same with an inverse mask. The engine also
            // gates on the thing being the player; every shipped call already passes `player`, and a
            // silently swallowed write is the failure mode this port exists to avoid, so it is not
            // repeated here.
            "setgoalflags" => {
                self.goal_flags(idx(1) as i32, idx(2) as u32, false);
                Value::Nil
            }
            "cleargoalflags" => {
                self.goal_flags(idx(1) as i32, idx(2) as u32, true);
                Value::Nil
            }
            "clearinv" => {
                self.inv.clear();
                Value::Nil
            }
            // `VectorSet(x, y, z)`: the force scripts build their velocities with this, so it
            // must also be listed in `handles` — an arm only `call` knows is unreachable, and
            // that is exactly how a force jump came out as zero.
            "vectorset" => Value::Vec([
                args.first().map(|v| v.num()).unwrap_or(0.0),
                args.get(1).map(|v| v.num()).unwrap_or(0.0),
                args.get(2).map(|v| v.num()).unwrap_or(0.0),
            ]),
            "vectoradd" | "vectoradd2" => vector(args, |a, b| a + b),
            "vectorsub" | "vector_sub" => vector(args, |a, b| a - b),
            // `VectorNorm(v)`: the unit vector. `force_pull.cog` builds its throw with
            // `VectorScale(VectorNorm(VectorSub(player, thing)), 30)`, so without this the force is
            // zero and a pull moves nothing.
            "vectornorm" => {
                let v = args.first().map(vec3).unwrap_or([0.0; 3]);
                let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if len < 1e-6 {
                    Value::Vec([0.0, 0.0, 0.0])
                } else {
                    Value::Vec([(v[0] / len) as f64, (v[1] / len) as f64, (v[2] / len) as f64])
                }
            }
            "vectorscale" => {
                let v = args.first().map(vec3).unwrap_or([0.0; 3]);
                let s = num(1) as f32;
                Value::Vec([(v[0] * s) as f64, (v[1] * s) as f64, (v[2] * s) as f64])
            }
            "vectordot" => {
                let a = args.first().map(vec3).unwrap_or([0.0; 3]);
                let b = args.get(1).map(vec3).unwrap_or([0.0; 3]);
                Value::Num((a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) as f64)
            }
            "vectordist" => {
                let a = args.first().map(vec3).unwrap_or([0.0; 3]);
                let b = args.get(1).map(vec3).unwrap_or([0.0; 3]);
                let d = [(a[0] - b[0]) as f64, (a[1] - b[1]) as f64, (a[2] - b[2]) as f64];
                Value::Num((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt())
            }
            "vectorx" => Value::Num(args.first().map(vec3).unwrap_or([0.0; 3])[0] as f64),
            "vectory" => Value::Num(args.first().map(vec3).unwrap_or([0.0; 3])[1] as f64),
            "vectorz" => Value::Num(args.first().map(vec3).unwrap_or([0.0; 3])[2] as f64),
            "getlocalplayerthing" => Value::Num(self.player_thing() as f64),
            "getdifficulty" => Value::Num(1.0),
            "isinvactivated" => Value::Num(0.0),
            "getcurweapon" => Value::Num(0.0),
            _ => Value::Num(0.0),
        }
    }
}

/// The clamped closest points between two segments (Ericson, *Real-Time Collision Detection*
/// §5.1.9), as the projectile hit test needs them: the bolt's swept step against the body's
/// vertical axis. Degenerate segments fall back to point-segment parameters.
fn closest_segment_parameters(p0: [f32; 3], p1: [f32; 3], q0: [f32; 3], q1: [f32; 3]) -> (f32, f32) {
    let d1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    let d2 = [q1[0] - q0[0], q1[1] - q0[1], q1[2] - q0[2]];
    let r = [p0[0] - q0[0], p0[1] - q0[1], p0[2] - q0[2]];
    let a = d1.iter().map(|v| v * v).sum::<f32>();
    let e = d2.iter().map(|v| v * v).sum::<f32>();
    let f = d2.iter().zip(r.iter()).map(|(u, v)| u * v).sum::<f32>();
    let (mut s, mut t);
    if a <= 1e-12 && e <= 1e-12 {
        return (0.0, 0.0);
    }
    if a <= 1e-12 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.iter().zip(r.iter()).map(|(u, v)| u * v).sum::<f32>();
        if e <= 1e-12 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.iter().zip(d2.iter()).map(|(u, v)| u * v).sum::<f32>();
            let denom = a * e - b * b;
            s = if denom > 1e-12 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
        }
    }
    (s, t)
}
/// The per-pass facts the scheduler's handlers share with the frame's motion half: computed once
/// per actor per tick, before the instinct pass (`sithAI_Process` runs decisions before
/// `sithAI_Tick` moves the body).
pub struct AiCtx {
    pub from: [f32; 3],
    pub yaw: f32,
    pub sector: i32,
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub sighted: bool,
    pub health: f32,
    pub template: String,
}

fn vec3(v: &Value) -> [f32; 3] {
    match v {
        Value::Vec(v) => [v[0] as f32, v[1] as f32, v[2] as f32],
        Value::Str(s) => {
            let mut out = [0.0; 3];
            for (slot, part) in out.iter_mut().zip(s.split_whitespace()) {
                *slot = part.parse().unwrap_or(0.0);
            }
            out
        }
        _ => [0.0; 3],
    }
}

fn render(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        Value::Num(n) => format!("{n}"),
        Value::Vec(v) => format!("{},{},{}", v[0], v[1], v[2]),
        Value::Nil => String::new(),
    }
}

/// Component-wise vector arithmetic, for the verbs that are just `a op b`.
fn vector(args: &[Value], op: fn(f64, f64) -> f64) -> Value {
    let a = args.first().map(vec3).unwrap_or([0.0; 3]);
    let b = args.get(1).map(vec3).unwrap_or([0.0; 3]);
    Value::Vec([
        op(a[0] as f64, b[0] as f64),
        op(a[1] as f64, b[1] as f64),
        op(a[2] as f64, b[2] as f64),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sithThing_EnterSector` (`0x4cd2c0`) messages a sector only when the sector carries
    /// `SITH_SECTOR_COGLINKED` (0x04) and the thing's flags carry none of `0x80100`. Sending to every
    /// linked sector ran `06_endpart1.cog`'s goal sequence from a walk spawn that merely stood in the
    /// room, which is what ended level `06abarons` in the middle of a capture.
    #[test]
    fn sector_messages_only_reach_a_cog_linked_sector_and_a_watched_body() {
        let mut rt = door_runtime();
        rt.sector_flags = vec![0x04, 0x00];
        rt.sector_links = vec![(0, 1, 0), (1, 1, 1)];
        // The player's own flags are clear: the watched sector's link fires, the plain one does not.
        assert_eq!(rt.sector_messages(None, Some(0), Some(0)), vec![(0, "entered", 1)]);
        assert!(rt.sector_messages(None, Some(1), Some(0)).is_empty());
        // Leaving it sends the other label.
        assert_eq!(rt.sector_messages(Some(0), Some(1), Some(0)), vec![(0, "exited", 1)]);
        // A body the scripts ignore (`0x80100`) is not messaged either way, and neither is a caller
        // that names no body (the pure-data checks ask about the table itself).
        rt.things[0].flags = 0x80000;
        assert!(rt.sector_messages(None, Some(0), Some(0)).is_empty());
        assert_eq!(rt.sector_messages(None, Some(0), None), vec![(0, "entered", 1)]);
    }

    /// `sithCog_SectorSendMessageEx` (`0x4dfd90`): a default-mask (`0x401`) link hears the player
    /// (type 10) and the world, not an actor (type 2); `mask=-1` hears every type. An AI walking into
    /// `03_goals.cog`'s goal sector must not end level 3.
    #[test]
    fn a_sector_link_hears_only_the_types_in_its_mask() {
        let mut rt = door_runtime();
        rt.sector_flags = vec![0x04];
        rt.sector_links = vec![(0, 1, 0)];
        rt.types = vec![2, 10];
        assert!(rt.sector_messages(None, Some(0), Some(0)).is_empty(), "an actor is not heard");
        assert_eq!(rt.sector_messages(None, Some(0), Some(1)), vec![(0, "entered", 1)]);
        rt.link_masks.insert((SECTOR_SOURCE, 0, 1, 0), u32::MAX);
        assert_eq!(rt.sector_messages(None, Some(0), Some(0)), vec![(0, "entered", 1)]);
    }

    /// The `SendMessageEx` verb (`0x5068a0`) is `(cog, message, param0, param1, param2, param3)`:
    /// `03_goals.cog`'s `SendMessageEx(GetMasterCOG(), user0, 2, 0, 0, 0)` reaches the master's
    /// `user0:` with `GetParam(0) == 2`, the branch that ends level 3.
    #[test]
    fn send_message_ex_carries_its_third_and_fourth_arguments() {
        let mut rt = door_runtime();
        let args = [5.0, 27.0, 2.0, 7.0, 0.0, 0.0].map(Value::Num);
        rt.call("sendmessageex", &args);
        let (name, _, cog, params) = rt.messages.last().expect("a queued message").clone();
        assert_eq!((name.as_str(), cog, params), ("#27", Some(5), [2.0, 7.0]));
    }

    /// `SetMapModeFlags`/`GetMapModeFlags`: `force_pull` writes the word and reads it back.
    #[test]
    fn the_map_mode_word_round_trips() {
        let mut rt = door_runtime();
        assert_eq!(rt.call("getmapmodeflags", &[]).num(), 0.0);
        rt.call("setmapmodeflags", &[Value::Num(3.0)]);
        assert_eq!(rt.call("getmapmodeflags", &[]).num(), 3.0);
        rt.call("clearmapmodeflags", &[]);
        assert_eq!(rt.call("getmapmodeflags", &[]).num(), 0.0);
    }

    /// `CreateThingAtPosNR` spawns where it is told (the force scripts' cones and spheres), and
    /// `CreateThingNR` at the source thing with no orientation carried over.
    #[test]
    fn spawning_at_a_position_uses_the_given_pose() {
        let mut rt = door_runtime();
        rt.things[0].pos = [1.0, 2.0, 3.0];
        rt.things[0].sector = 7;
        let at = rt
            .call(
                "createthingatposnr",
                &[
                    Value::Str("+cone".into()),
                    Value::Num(5.0),
                    Value::Vec([4.0, 5.0, 6.0]),
                    Value::Vec([0.0, 90.0, 0.0]),
                ],
            )
            .num() as usize;
        assert_eq!(rt.things[at].pos, [4.0, 5.0, 6.0]);
        assert_eq!(rt.things[at].sector, 5);
        assert_eq!(rt.things[at].yaw, 90.0);
        let nr = rt
            .call("createthingnr", &[Value::Str("+splash".into()), Value::Num(0.0)])
            .num() as usize;
        assert_eq!(rt.things[nr].pos, [1.0, 2.0, 3.0]);
        assert_eq!(rt.things[nr].sector, 7);
        assert_eq!(rt.things[nr].yaw, 0.0, "CreateThingNR carried an orientation over");
    }

    /// Two halves of the door gate's actor-set mismatch. A placement the active difficulty excludes
    /// is not created — retail leaves the pool slot type 0 and a later `CreateThing` reuses it — and
    /// `CaptureThing` captures a spawned thing instead of destroying it (`00_REPLACEENEMY.cog`
    /// creates its enemy at a ghost marker, then captures it; the port removed it again).
    #[test]
    fn a_medium_excluded_placement_is_empty_and_capture_keeps_the_spawned_actor() {
        let mut level = Level::default();
        level.templates.push(crate::jkl::Template {
            name: "_actor".into(),
            base: None,
            params: vec![("type".into(), "actor".into()), ("health".into(), "50".into())],
            model3d: None,
            sprite: None,
        });
        level.templates.push(crate::jkl::Template {
            name: "enemy".into(),
            base: Some("_actor".into()),
            params: Vec::new(),
            model3d: None,
            sprite: None,
        });
        let placed = |pos: [f32; 3], flags: u32| crate::jkl::Thing {
            template: "enemy".into(),
            pos,
            pitch: 0.0,
            yaw: 0.0,
            roll: 0.0,
            sector: 0,
            flags,
            frames: Vec::new(),
            vel: [0.0; 3],
            aiclass: None,
        };
        level.things.push(placed([0.0; 3], 0));
        level.things.push(placed([2.0, 0.0, 0.0], TF_NOT_MEDIUM));
        let mut rt = Runtime::new(&level);
        assert_eq!(rt.types, vec![2, 0], "the medium-excluded placement was still an actor");
        assert!(rt.things[1].destroyed, "the medium-excluded placement was not left empty");
        let spawned = rt
            .call("CreateThing", &[Value::Str("enemy".into()), Value::Num(0.0)])
            .num() as usize;
        assert_eq!(rt.types[spawned], 2);
        rt.caller_cog = 7;
        rt.call("CaptureThing", &[Value::Num(spawned as f64)]);
        assert!(!rt.things[spawned].destroyed, "CaptureThing removed the thing it captured");
        assert_ne!(
            rt.things[spawned].flags & TF_CAPTURED,
            0,
            "CaptureThing did not set SITH_TF_CAPTURED"
        );
        assert_eq!(
            rt.thing_instances(spawned),
            vec![7],
            "the capture cog is not on the thing's message list"
        );
    }

    /// `SetThingPulse(thing, seconds)` starts the thing's own timer: every interval its script slots
    /// get a `pulse` message (`00_backpack`'s blood trickle, `00_deathfloor`'s damage cadence).
    #[test]
    fn a_thing_pulse_messages_its_own_script() {
        let mut rt = door_runtime();
        rt.thing_links.push((7, 0, 0));
        rt.call("setthingpulse", &[Value::Num(0.0), Value::Num(0.5)]);
        assert_eq!(rt.things[0].pulse, 0.5);
        rt.time += 0.6;
        rt.step(None, 1.0 / 30.0);
        let pulses = rt
            .pending_messages
            .iter()
            .filter(|(instance, label, _, _, _)| *instance == 7 && *label == "pulse")
            .count();
        assert_eq!(pulses, 1, "the thing's pulse never reached its script");
        // The timer re-arms rather than firing every frame.
        rt.pending_messages.clear();
        rt.step(None, 1.0 / 30.0);
        assert!(
            !rt.pending_messages.iter().any(|(_, label, _, _, _)| *label == "pulse"),
            "the pulse fired on the very next frame"
        );
        // Zero stops it.
        rt.call("setthingpulse", &[Value::Num(0.0), Value::Num(0.0)]);
        rt.time += 5.0;
        rt.step(None, 1.0 / 30.0);
        assert!(!rt.pending_messages.iter().any(|(_, label, _, _, _)| *label == "pulse"));
    }

    /// The runtime thing carries the **effective** flags word — a placement's `thingflags=`
    /// replaces the template's, and a placement without one inherits the template's
    /// (`sithThing_ParseThingArg`). #85: the word lived only in a placement-side read, so
    /// `SetThingFlags(player, 0x100)` never reached the pickup gate (`0x4fbe10` reads the live
    /// word, `test ah,1`).
    #[test]
    fn a_things_flags_word_is_the_effective_one_from_load() {
        let mut level = Level::default();
        level.templates.push(crate::jkl::Template {
            name: "_item".into(),
            base: None,
            params: vec![
                ("type".into(), "item".into()),
                ("thingflags".into(), "0x100".into()),
            ],
            model3d: None,
            sprite: None,
        });
        level.templates.push(crate::jkl::Template {
            name: "flagged".into(),
            base: Some("_item".into()),
            params: Vec::new(),
            model3d: None,
            sprite: None,
        });
        let at = |flags: u32| crate::jkl::Thing {
            template: "flagged".into(),
            pos: [0.0; 3],
            pitch: 0.0,
            yaw: 0.0,
            roll: 0.0,
            sector: 0,
            flags,
            frames: Vec::new(),
            vel: [0.0; 3],
            aiclass: None,
        };
        // No placement word: the template's 0x100 is the effective word.
        level.things.push(at(0));
        // A placement word replaces the template's outright.
        level.things.push(at(0x4));
        let rt = Runtime::new(&level);
        assert_eq!(rt.things[0].flags, 0x100, "the template's thingflags never reached the runtime word");
        assert_eq!(rt.things[1].flags, 0x4, "the placement word did not replace the template's");
    }

    /// `AiGetMode` is the AI's own flag word (`ai + 8`), and the flag-word split keeps it apart from
    /// the sithActor word `GetActorFlags`/`SetActorFlags` answer: `AISetMode` writes only the AI
    /// word, and `SetActorFlags` writes only the actor word (the engine's AI bit `0x8` collides with
    /// `AF_INVULNERABLE` `0x8`, so the two must not be one map).
    #[test]
    fn ai_get_mode_reads_the_script_word() {
        let mut rt = door_runtime();
        rt.call("aisetmode", &[Value::Num(1.0), Value::Num(0x800 as f64)]);
        assert_eq!(rt.call("aigetmode", &[Value::Num(1.0)]).num(), 0x800 as f64);
        assert_eq!(rt.call("getactorflags", &[Value::Num(1.0)]).num(), 0.0);
        rt.call("setactorflags", &[Value::Num(1.0), Value::Num(0x8 as f64)]);
        assert_eq!(rt.call("aigetmode", &[Value::Num(1.0)]).num(), 0x800 as f64);
        assert_eq!(rt.call("getactorflags", &[Value::Num(1.0)]).num(), 0x8 as f64);
    }

    /// `sithAI_InstinctUpdate` (`0x4e9e90`) arms a due instinct at `now + 1000` before calling it,
    /// so an instinct fires on its own armed interval, never every frame. `talk` re-arms at its own
    /// first number (10000 ms here): the roll happens on the first pass and not again inside it.
    #[test]
    fn an_instinct_fires_on_its_own_armed_interval_not_every_frame() {
        let mut rt = door_runtime();
        let mut actor = rt.things[0].clone();
        actor.pos = [0.0, 0.0, 0.0];
        let mut player = rt.things[0].clone();
        player.pos = [-9.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "barkeep".into()];
        rt.types = vec![10, 2];
        rt.template_max_health.insert("barkeep".into(), 100.0);
        rt.template_soundclass.insert("barkeep".into(), "pednarsh.snd".into());
        rt.sound_classes.insert(
            "pednarsh.snd".into(),
            std::collections::BTreeMap::from([(
                "happy".to_string(),
                vec![("happy1.wav".to_string(), 0.8)],
            )]),
        );
        rt.ai_flags.insert(1, 0x200);
        rt.ai.insert(
            1,
            crate::ai::parse("alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nTalk 10000.0, 1.0\n"),
        );
        for _ in 0..20 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert_eq!(rt.sounds.len(), 1, "the mood roll happened more than once inside the interval");
        let armed = rt.ai_inst[&1][0];
        assert!(
            armed > rt.time + 9.0,
            "the handler did not re-arm at its own 10 s interval: {armed} at {:.2}",
            rt.time
        );
        // The slot's whole-AI deadline (`ai + 0x190`) is the earliest armed clock: the second pass
        // waits for it, at most now + 5000.
        assert!(rt.ai_next_due[&1] <= rt.time + 5.0);
    }

    /// `sithAI_Process` (`0x4e9e20`) skips the whole slot on the `0x3000` family: the shipped
    /// scripts' `0x2000` sleep switch and the `0x1000` one-shot latch `sithAI_Create` seeds, which
    /// only the awareness sweep's event 2 clears.
    #[test]
    fn the_scheduler_skips_the_0x3000_family() {
        let mut rt = door_runtime();
        let mut actor = rt.things[0].clone();
        actor.pos = [0.0, 0.0, 0.0];
        let mut player = rt.things[0].clone();
        player.pos = [-9.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "barkeep".into()];
        rt.types = vec![10, 2];
        rt.template_max_health.insert("barkeep".into(), 100.0);
        rt.template_soundclass.insert("barkeep".into(), "pednarsh.snd".into());
        rt.sound_classes.insert(
            "pednarsh.snd".into(),
            std::collections::BTreeMap::from([(
                "happy".to_string(),
                vec![("happy1.wav".to_string(), 0.8)],
            )]),
        );
        rt.ai.insert(
            1,
            crate::ai::parse("alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nTalk 100.0, 1.0\n"),
        );
        rt.ai_flags.insert(1, 0x1004);
        for _ in 0..20 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(rt.sounds.is_empty(), "a latched (0x1000) actor ran an instinct");
        assert!(!rt.ai_inst.contains_key(&1), "a latched actor got scheduler slots");
        // Event 2 clears the latch and the next pass runs.
        rt.sector_adjoins = vec![Vec::new(); 8];
        rt.transmit_awareness(7, 0, [0.0, 0.0, 0.0], 3.0, None);
        assert_eq!(rt.ai_flags.get(&1).copied().unwrap_or(0) & 0x1000, 0);
        rt.ai_tick(0, 1.0 / 30.0);
        assert!(
            rt.ai_inst.contains_key(&1),
            "the woken actor never got a scheduler pass"
        );
    }

    /// The registry's mask rule (`sithAICmd_Startup` @ `0x5091b0`): an event wakes exactly the
    /// instincts whose mask carries the event bit, and the 0x100 mode-change cascade tells the
    /// actor's own cogs (`ThingSendMessageEx(thing, 0, 0x18, 256.0)` → the `aievent` message).
    #[test]
    fn the_event_masks_wake_exactly_the_registry_instincts_and_cascade_the_mode_change() {
        // Every registered word, straight off the startup table.
        for (name, accept, reject, mask) in [
            ("listen", 0u32, 0u32, 0x7u32),
            ("lookfortarget", 0x204, 0, 0),
            ("primaryfire", 2, 0, 0x100),
            ("follow", 2, 0x800, 0xe00),
            ("turretfire", 2, 0x800, 0x100),
            ("opendoors", 2, 0, 0),
            ("jump", 0, 0, 0x604),
            ("randomturn", 4, 0, 0),
            ("roam", 4, 0, 0),
            ("flee", 0x800, 0, 0xf05),
            ("sensedanger", 4, 0x800, 0x7),
            ("hitandrun", 0xc00, 0, 0),
            ("retreat", 2, 0x800, 0),
            ("circlestrafe", 2, 0x800, 0),
            ("blindfire", 2, 0xc00, 0),
            ("returnhome", 0, 0, 0x900),
            ("lobfire", 2, 0, 0x100),
            ("talk", 0xffff, 0, 0),
            ("crouch", 2, 0, 0x100),
            ("withdraw", 0x800, 0, 0xf05),
            ("dodge", 0, 0, 0x1003),
        ] {
            assert_eq!(Runtime::instinct_registry(name), (accept, reject, mask), "{name}");
        }
        // Event 0x100 reaches lobfire's armed-mode arm and queues the `aievent` cog message.
        let mut rt = door_runtime();
        let actor = rt.things[0].clone();
        rt.things = vec![actor.clone(), actor];
        rt.templates = vec!["walkplayer".into(), "ryther".into()];
        rt.types = vec![10, 2];
        rt.things[1].health = 100.0;
        rt.ai.insert(
            1,
            crate::ai::parse("alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\nLobFire 2000.0, 0.8\n"),
        );
        rt.thing_links.push((0, 0, 1));
        rt.ai_flags.insert(1, 0x200);
        let _ = rt.emit_ai_event(1, 0x100, None);
        assert_eq!(rt.armed_mode.get(&1).copied(), Some(1));
        assert!(
            rt.pending_messages
                .iter()
                .any(|(instance, label, params, ..)| *instance == 0
                    && *label == "aievent"
                    && params[0] == 256.0),
            "the 0x100 event did not tell the actor's cogs"
        );
    }

    /// `SetThingLook` aims a thing along a direction (`sithPhysics_SetThingLook`, `0x4f5a80`) and
    /// sets the jkFlag `0x100`; a zero blend snaps, a positive one turns part of the way.
    #[test]
    fn setting_a_look_direction_turns_the_thing_to_it() {
        let mut rt = door_runtime();
        rt.things[0].pitch = 0.0;
        rt.things[0].yaw = 0.0;
        rt.things[0].roll = 0.0;
        rt.call("setthinglook", &[Value::Num(0.0), Value::Vec([1.0, 0.0, 0.0]), Value::Num(0.0)]);
        let t = &rt.things[0];
        let fwd = rotate_by_angles([0.0, 1.0, 0.0], t.pitch, t.yaw, t.roll);
        for k in 0..3 {
            let want = [1.0, 0.0, 0.0][k];
            assert!((fwd[k] - want).abs() < 1e-4, "look direction {fwd:?} is not +X");
        }
        assert_ne!(rt.things[0].jk_flags & 0x100, 0, "the look flag was not set");
        // A half blend points between the old and new forward, and still lands normalised.
        rt.call("setthinglook", &[Value::Num(0.0), Value::Vec([0.0, -1.0, 0.0]), Value::Num(0.5)]);
        let t = &rt.things[0];
        let fwd = rotate_by_angles([0.0, 1.0, 0.0], t.pitch, t.yaw, t.roll);
        assert!(fwd[0] > 0.0 && fwd[1] < 0.0, "the blend did not land between +X and -Y: {fwd:?}");
    }

    /// The surge word the light and dark powerups toggle: `pow_keyyellow`/`pow_crossbow` set and clear
    /// bit `4`, `pow_power` bit `1`, and each reads it back to know whether the surge is already on.
    #[test]
    fn the_super_flag_word_round_trips() {
        let mut rt = door_runtime();
        assert_eq!(rt.call("jkgetsuperflags", &[]).num(), 0.0);
        rt.call("jksetsuperflags", &[Value::Num(4.0)]);
        assert_eq!(rt.call("jkgetsuperflags", &[]).num(), 4.0);
        rt.call("jksetsuperflags", &[Value::Num(1.0)]);
        assert_eq!(rt.call("jkgetsuperflags", &[]).num(), 5.0, "a second surge must not clear the first");
        rt.call("jkclearsuperflags", &[Value::Num(4.0)]);
        assert_eq!(rt.call("jkgetsuperflags", &[]).num(), 1.0);
        rt.call("jkclearsuperflags", &[Value::Num(1.0)]);
        assert_eq!(rt.call("jkgetsuperflags", &[]).num(), 0.0);
    }

    /// `GetThingLVec` is the thing's own look vector, and the weapon scripts turn it into recoil:
    /// `weap_bryar`/`weap_fists` run `ApplyForce(player, VectorScale(GetThingLVec(player), -80))`.
    #[test]
    fn the_look_vector_drives_the_weapons_recoil() {
        let mut rt = door_runtime();
        rt.things[0].pitch = 0.0;
        rt.things[0].yaw = 90.0;
        rt.things[0].roll = 0.0;
        let answer = rt.call("getthinglvec", &[Value::Num(0.0)]);
        let lvec = vec3(&answer);
        let want = rotate_by_angles([0.0, 1.0, 0.0], 0.0, 90.0, 0.0);
        for k in 0..3 {
            assert!(
                (lvec[k] - want[k]).abs() < 1e-5,
                "the look vector disagrees with the thing's own rotation: {lvec:?} vs {want:?}"
            );
        }
        assert!(lvec[1].abs() < 1e-5, "a right-facing player still looks along +Y");
        // The script's own expression: the kick goes backwards, against the look vector.
        let kick = vec3(&rt.call("vectorscale", &[answer, Value::Num(-80.0)]));
        rt.call("applyforce", &[Value::Num(0.0), Value::Vec([kick[0] as f64, kick[1] as f64, kick[2] as f64])]);
        let stored = rt.force.get(&0).copied().unwrap_or([0.0; 3]);
        for k in 0..3 {
            assert!((stored[k] - kick[k]).abs() < 1e-4, "the recoil did not land as an impulse");
        }
        assert!(stored[0] > 0.0, "the kick does not oppose a -X look vector");
    }

    /// `sithInventory_CreateBackpack` (`0x4d2740`) snapshots the carried bins onto a `+backpack`
    /// thing, and `sithInventory_PickupBackpack` (`0x4d2860`) adds them back — the pair
    /// `item_wrenchyellow.cog` and `levelsong.cog` drive.
    #[test]
    fn a_backpack_carries_the_inventory_and_gives_it_back() {
        let mut rt = door_runtime();
        rt.set_bin(60, 25.0);
        rt.set_bin(14, 300.0);
        rt.set_bin(71, 0.0);
        assert!(Runtime::handles("CreateBackpack") && Runtime::handles("PickupBackpack"));
        let thing = rt.call("createbackpack", &[Value::Num(0.0)]).num() as usize;
        assert_eq!(
            rt.backpack.get(&thing).map(|b| b.len()),
            Some(2),
            "the backpack did not carry the two filled bins"
        );
        rt.inv.clear();
        rt.call("pickupbackpack", &[Value::Num(0.0), Value::Num(thing as f64)]);
        assert_eq!(rt.inv.get(&60).copied(), Some(25.0));
        assert_eq!(rt.inv.get(&14).copied(), Some(300.0));
        assert!(!rt.backpack.contains_key(&thing), "the picked-up record survived");
    }

    /// `sithInventory_ChangeInventory` (`0x4d1ef0`) and `sithInventory_SetInventory` (`0x4d2000`)
    /// clamp a bin to its own `items.dat` `min`/`max` **before** the change is reported, which is why
    /// retail's mana never reads above 100 however many powerups land. A bin the table does not list
    /// is left as the script set it: the engine only stores registered types (`(flags & 1)`), and the
    /// port's own bins (mana 14, secrets 71) have no row.
    #[test]
    fn inventory_writes_clamp_to_the_items_own_ceiling() {
        let mut rt = Runtime::new(&Level::default());
        rt.item_max.insert(60, 200.0);
        rt.item_min.insert(60, 0.0);
        rt.set_bin(60, 500.0);
        assert_eq!(rt.inv.get(&60).copied(), Some(200.0));
        rt.set_bin(60, -20.0);
        assert_eq!(rt.inv.get(&60).copied(), Some(0.0));
        rt.set_bin(60, 20.0);
        let change = |rt: &mut Runtime, delta: f32| {
            rt.call("changeinv", &[Value::Num(1.0), Value::Num(60.0), Value::Num(delta as f64)])
        };
        change(&mut rt, 20.0);
        assert_eq!(rt.inv.get(&60).copied(), Some(40.0));
        change(&mut rt, 400.0);
        assert_eq!(rt.inv.get(&60).copied(), Some(200.0), "above max clamps");
        // Unlisted bins keep whatever the script wrote (the port's modelling, not the engine's table).
        rt.call("changeinv", &[Value::Num(1.0), Value::Num(14.0), Value::Num(300.0)]);
        assert_eq!(rt.inv.get(&14).copied(), Some(300.0));
    }


    /// The `lookfortarget` instinct's scheduler clock (`ai_inst` slot), the value the retired
    /// `ai_scan` map used to hold.
    fn scan_clock(rt: &Runtime, index: usize) -> f32 {
        rt.ai
            .get(&index)
            .and_then(|class| {
                class
                    .instincts
                    .iter()
                    .position(|i| i.name.eq_ignore_ascii_case("LookForTarget"))
            })
            .and_then(|slot| rt.ai_inst.get(&index).and_then(|v| v.get(slot).copied()))
            .unwrap_or(f32::NAN)
    }

    fn door_runtime() -> Runtime {
        let mut rt = Runtime::new(&Level::default());
        rt.things = vec![ThingState {
                pos: [0.0; 3],
                next_touch_ms: 0,
                insert: [0.0; 3],
                owner: None,
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0,
                head_pitch: 0.0,
                sector: 7,
                flags: 0,
                health: 100.0,
                frame: 0.0,
                target: None,
                frames: vec![[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0, 0.0, 0.0]],
                destroyed: false,
                vel: [0.0; 3],
                damage: 0.0,
                min_damage: 0.0,
                decay: 0.0,
                phys_flags: 0,
                jk_flags: 0,
                attach: None,
                floor_attach: None,
                attach_offset: [0.0; 3],
                attach_flags: 0,
                spin: None,
            rot_vel: [0.0; 3],
            paused: false,
                pulse: 0.0,
                pulse_at: 0.0,
                light: 0.0,
                light_end: 0.0,
                light_rate: 0.0,
                extra_speed: 0.0,
                ai_target: None,
                ai_wait: 0.0,
                ai_burst: 0.0,
                arrived: false,
            }];
        rt.templates = vec!["walkplayer".into()];
        rt.types = vec![10];
        rt.template_types = BTreeMap::from([("walkplayer".to_string(), 10)]);
        rt.template_health = BTreeMap::from([("walkplayer".to_string(), 100.0)]);
        rt.gravity = 4.0;
        rt.sectors = vec![SectorState { thrust: [0.0; 3], light: 0.0, light_from: 0.0, light_end: 0.0, light_rate: 0.0, tint: [0.0; 3], colormap: 0 }];
        rt.sector_flags = vec![0];
        rt.adjoin_flags = vec![0x7];
        rt.surface_adjoin = vec![0];
        rt
    }

    fn walk_body(pos: Vec3) -> PlayerBody {
        PlayerBody {
            pos,
            vy: 0.0,
            sector: 0,
            grounded: true,
            gravity: 4.0,
            water: false,
            vel: [0.0; 2],
            step_rollover: 0.0,
            last_move: 0.0,
            fall_killed: false,
            fall_fade: 1.0,
            yaw: 0.0,
            pitch: 0.0,
            eye: crate::PLAYER_EYE,
            attach: None,
            floor_anchor: None,
            water_surface: None,
            published_vel: [0.0; 3],
            published_pose: None,
        }
    }

    /// One `_actor`-word gravity body (thing 0, type 2) over a single floor (surface 1) at z 0.
    fn floored_actor_runtime(pos: [f32; 3], mass: f32) -> Runtime {
        let mut rt = door_runtime();
        rt.things[0].pos = pos;
        rt.things[0].vel = [0.0; 3];
        // `_actor`'s shipped word: gravity bit, no thruster exemption.
        rt.things[0].phys_flags = 0x4a4f;
        rt.templates = vec!["mousebot".into()];
        rt.types = vec![2];
        rt.mass.insert(0, mass);
        let plane = || crate::collide::Plane {
            normal: [0.0, 0.0, -1.0],
            point: [0.0; 3],
            surface: 1,
        };
        rt.solids = Some(std::sync::Arc::new(crate::collide::Solids {
            sectors: vec![vec![plane()]],
            hull: vec![vec![plane()]],
            boxes: vec![None],
            portals: vec![Vec::new()],
            spheres: vec![([0.0, 0.0, 0.0], 100.0)],
            things: Default::default(),
            floors: vec![crate::collide::Face {
                normal: [0.0, 0.0, -1.0],
                point: [0.0; 3],
                verts: vec![[-10.0, -10.0, 0.0], [10.0, -10.0, 0.0], [10.0, 10.0, 0.0], [-10.0, 10.0, 0.0]],
                surface: 1,
            }],
            walls: Vec::new(),
            wall_of: Default::default(),
            floor_of: [(1usize, 0usize)].into_iter().collect(),
            faces: Vec::new(),
            face_of: Default::default(),
            portal_faces: Vec::new(),
            surface_owner: Vec::new(),
            adjoin_flags: Vec::new(),
            ..Default::default()
        }));
        rt
    }

    /// `sithThing_Update` (`0x4cc1e0`) resolves its physics bodies in array order inside one
    /// frame: two actors landing on the same cog-linked floor in the same frame send the floor's
    /// `entered` (`sithThing_AttachThingToSurface`, `0x4cde80`) in thing order, whichever index
    /// either body has.
    #[test]
    fn two_physics_bodies_resolve_in_array_order_within_one_frame() {
        let mut rt = floored_actor_runtime([1.0, 0.0, 0.5], 150.0);
        let first = rt.things[0].clone();
        rt.things.push(ThingState { pos: [-1.0, 0.0, 0.5], ..first });
        rt.templates.push("mousebot".into());
        rt.types.push(2);
        rt.mass.insert(1, 150.0);
        rt.surface_flags = vec![0, SITH_SURFACE_COG_LINKED];
        rt.surface_links.push((0, 0, 1));
        // Actors (type 2) hear the floor's link.
        rt.link_masks.insert((SURFACE_SOURCE, 0, 0, 1), 1 << 2);
        let mut senders = Vec::new();
        for _ in 0..90 {
            rt.step(None, 1.0 / 30.0);
            for (_, message, _, _, identity) in std::mem::take(&mut rt.pending_messages) {
                if message == "entered" {
                    senders.push(identity.map(|identity| identity.sender));
                }
            }
            if senders.len() == 2 {
                break;
            }
        }
        assert_eq!(senders, [Some(0), Some(1)], "the two landings were not resolved in thing order");
    }

    /// `sithThing_Update` (`0x4cc1e0`): the player's body moves at its own thing index, after the
    /// things before it and before the things after it — not before or after the whole loop.
    #[test]
    fn the_player_moves_at_its_own_thing_index() {
        let mut rt = door_runtime();
        let lift = ThingState { target: Some((1.0, 1.0)), ..rt.things[0].clone() };
        rt.things = vec![lift.clone(), rt.things[0].clone(), lift];
        rt.templates = vec!["lift".into(), "walkplayer".into(), "lift".into()];
        rt.types = vec![0, 10, 0];
        let world = World::new(rt);
        let mut seen = None;
        Runtime::sith_thing_update(&world, None, 0.1, || {
            let rt = world.borrow();
            seen = Some((rt.things[0].frame, rt.things[2].frame));
        });
        let (before, after) = seen.expect("the player's slot never ran");
        assert!(before > 0.0, "the thing before the player had not moved when the player's slot ran");
        assert_eq!(after, 0.0, "the thing after the player moved before the player's slot");
        assert!(world.borrow().things[2].frame > 0.0, "the thing after the player never moved");
    }

    /// `sithPhysics_UpdateThingPhysics`'s gravity arm is not a weapons-only rule: the shipped `_actor`
    /// carries `physflags=0x4a4f` and `mass=150`, so an actor that walks off its floor falls like the
    /// `_debris` goods. The gate is the thing's mass (`+0x204`) as well as the `0x1` bit, and a body
    /// still within `GetThingHeight * 1.1` of a floor is standing (the attach `sithThing_Initialize`
    /// makes with `FindFloor(thing, 1)`), so a placed actor keeps the height the level gave it.
    #[test]
    fn an_actor_off_its_floor_falls_to_the_contact_and_stops() {
        let runtime = floored_actor_runtime;

        // Half a unit over its floor and out of the creation reach: the body falls.
        let mut falling = runtime([0.0, 0.0, 0.5], 150.0);
        falling.step(None, 1.0 / 30.0);
        assert!(falling.things[0].vel[2] < 0.0, "an actor with the gravity bit did not start to fall");
        for _ in 0..90 {
            falling.step(None, 1.0 / 30.0);
        }
        assert!(
            (falling.things[0].pos[2] - 0.07).abs() < 1e-3,
            "the actor did not settle at its GetThingHeight rest: z {}",
            falling.things[0].pos[2]
        );
        // The fall's speed at the attach survives: the engine decays it through the attached
        // drag (`ApplyDrag` 0x4f5d50, elided — see `update_attached_thing_physics`'s doc), and
        // the tail gate caps it at 2.0; the projected move carries none of it.
        assert!((-2.0..2e-3).contains(&falling.things[0].vel[2]), "the landed actor kept a downward velocity {}", falling.things[0].vel[2]);

        // The mass gate (`0x4f6270` reads `+0x204`): the same body with no mass floats.
        let mut floating = runtime([0.0, 0.0, 0.5], 0.0);
        for _ in 0..30 {
            floating.step(None, 1.0 / 30.0);
        }
        assert_eq!(floating.things[0].pos[2], 0.5, "a massless body fell anyway");

        // Inside the creation reach (`moveSize + 0.005`, × 1.1) and above the contact: standing.
        let mut standing = runtime([0.0, 0.0, 0.07], 150.0);
        for _ in 0..30 {
            standing.step(None, 1.0 / 30.0);
        }
        assert_eq!(standing.things[0].pos[2], 0.07, "a standing actor sank onto its floor");
        assert_eq!(standing.things[0].vel[2], 0.0, "a standing actor kept a vertical velocity");
    }

    /// A type-3 gravity good keeps its origin on the plane, while an actor rests one radius above.
    /// The grounding probe must follow that landing: a good wider than the actor's flat 0.05 probe
    /// used to be measured below its own floor, so `resting_floor` refused the contact and the body
    /// sank through it for ever.
    #[test]
    fn a_gravity_good_wider_than_the_actor_probe_rests_on_a_plane_floor() {
        let mut rt = door_runtime();
        rt.things[0].pos = [0.0, 0.0, 0.5];
        rt.things[0].phys_flags = 0x1;
        rt.templates = vec!["debris".into()];
        rt.types = vec![3];
        rt.template_debris.insert("debris".into());
        rt.mass.insert(0, 0.1);
        rt.template_hit.insert("debris".into(), (0.2, 0.2));
        let plane = || crate::collide::Plane {
            normal: [0.0, 0.0, -1.0],
            point: [0.0; 3],
            surface: 1,
        };
        rt.solids = Some(std::sync::Arc::new(crate::collide::Solids {
            sectors: vec![vec![plane()]],
            hull: vec![vec![plane()]],
            boxes: vec![None],
            portals: vec![Vec::new()],
            spheres: vec![([0.0, 0.0, 0.0], 100.0)],
            floors: vec![crate::collide::Face {
                normal: [0.0, 0.0, -1.0],
                point: [0.0; 3],
                verts: vec![[-10.0, -10.0, 0.0], [10.0, -10.0, 0.0], [10.0, 10.0, 0.0], [-10.0, 10.0, 0.0]],
                surface: 1,
            }],
            walls: Vec::new(),
            wall_of: Default::default(),
            floor_of: [(1usize, 0usize)].into_iter().collect(),
            faces: Vec::new(),
            face_of: Default::default(),
            portal_faces: Vec::new(),
            surface_owner: Vec::new(),
            adjoin_flags: Vec::new(),
            ..Default::default()
        }));
        for _ in 0..90 {
            rt.step(None, 1.0 / 30.0);
        }
        assert!(
            (rt.things[0].pos[2] - 0.2).abs() < 1e-3,
            "the good did not rest on the plane: z {}",
            rt.things[0].pos[2]
        );
        // This weapon-kind debris has no physflag 0x40; it stops at contact without attaching.
        assert!(rt.things[0].floor_attach.is_none());
        assert!((-2.0..2e-3).contains(&rt.things[0].vel[2]), "the landed good kept a downward velocity {}", rt.things[0].vel[2]);
    }

    /// A `_debris` chip is `type=weapon` (`0x201`) but it is not a shot: a surface push its fall
    /// brushes is not a projectile impact. The projectile rule — die on the surface in front of you
    /// — is for weapon projectiles, which the template family names (`_debris` is rubble, and every
    /// fired template is everything else). Reading the type alone destroyed a falling chip mid-air
    /// on the first plane it touched, and because a destroyed body kept its last velocity it coasted
    /// down for ever (the `--things` gate's "landed debris chip kept falling"). The good must keep
    /// falling and land.
    #[test]
    fn a_gravity_good_that_brushes_a_surface_lands_instead_of_dying() {
        let mut rt = door_runtime();
        rt.things[0].pos = [0.0, 0.0, 0.5];
        rt.things[0].phys_flags = 0x201;
        rt.templates = vec!["debris".into()];
        rt.types = vec![3];
        rt.template_debris.insert("debris".into());
        rt.mass.insert(0, 0.1);
        rt.template_hit.insert("debris".into(), (0.2, 0.2));
        let floor = crate::collide::Plane {
            normal: [0.0, 0.0, -1.0],
            point: [0.0; 3],
            surface: 1,
        };
        // A tilted plane inside the body's radius: the push is above the mover's 0.01 horizontal
        // threshold *and* has a vertical part that opposes the fall, which is the projectile arm's
        // "surface in front of me" test.
        let brace = crate::collide::Plane {
            normal: [-0.866_025_4, 0.0, -0.5],
            point: [0.0, 0.0, 0.4],
            surface: 2,
        };
        rt.solids = Some(std::sync::Arc::new(crate::collide::Solids {
            sectors: vec![vec![floor, brace]],
            hull: vec![vec![floor]],
            boxes: vec![None],
            portals: vec![Vec::new()],
            spheres: vec![([0.0, 0.0, 0.0], 100.0)],
                floors: vec![crate::collide::Face {
                    normal: [0.0, 0.0, -1.0],
                    point: [0.0; 3],
                    verts: vec![[-10.0, -10.0, 0.0], [10.0, -10.0, 0.0], [10.0, 10.0, 0.0], [-10.0, 10.0, 0.0]],
                    surface: 1,
                }],
                walls: Vec::new(),
                wall_of: Default::default(),
                floor_of: [(1usize, 0usize)].into_iter().collect(),
                faces: Vec::new(),
                face_of: Default::default(),
            portal_faces: Vec::new(),
            surface_owner: Vec::new(),
            adjoin_flags: Vec::new(),
            ..Default::default()
        }));
        rt.step(None, 1.0 / 30.0);
        assert!(!rt.things[0].destroyed, "a gravity good died on the surface it brushed");
        for _ in 0..120 {
            rt.step(None, 1.0 / 30.0);
        }
        assert!(!rt.things[0].destroyed, "the gravity good died before it landed");
        assert!(
            (rt.things[0].pos[2] - 0.2).abs() < 1e-3,
            "the good did not reach the floor: z {}",
            rt.things[0].pos[2]
        );
        assert!(rt.things[0].floor_attach.is_none());
        assert!((-2.0..2e-3).contains(&rt.things[0].vel[2]), "the landed good kept a downward velocity {}", rt.things[0].vel[2]);
    }

    /// Shipped projectiles carry the gravity bit too: `+grenade2` and `08tiebomb` both ship
    /// `physflags=0x225`, and `08_tiebomber.cog` creates the tie bomb with `CreateThingAtPos`, so
    /// neither is fired through `fire()` in the owner sense. They are still shots and must die on the
    /// surface they hit; only the `_debris` family lands.
    #[test]
    fn gravity_bit_projectiles_still_die_on_a_surface() {
        for template in ["+grenade2", "08tiebomb"] {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let plane = |normal: [f32; 3], point: [f32; 3], surface: usize| {
                crate::collide::Plane { normal, point, surface }
            };
            let planes = vec![
                plane([-1.0, 0.0, 0.0], [1.2, 0.0, 0.0], 41),
                plane([1.0, 0.0, 0.0], [-5.0, 0.0, 0.0], 41),
                plane([0.0, -1.0, 0.0], [0.0, -2.0, 0.0], 41),
                plane([0.0, 1.0, 0.0], [0.0, 2.0, 0.0], 41),
                plane([0.0, 0.0, -1.0], [0.0, 0.0, -2.0], 41),
                plane([0.0, 0.0, 1.0], [0.0, 0.0, 2.0], 41),
            ];
            rt.solids = Some(std::sync::Arc::new(crate::collide::Solids {
                sectors: vec![planes.clone()],
                hull: vec![planes],
                boxes: vec![None],
                portals: vec![Vec::new()],
                spheres: vec![([0.0, 0.0, 0.0], 100.0)],
                things: Default::default(),
                floors: Vec::new(),
                walls: Vec::new(),
                wall_of: Default::default(),
                floor_of: Default::default(),
                faces: Vec::new(),
                face_of: Default::default(),
                portal_faces: Vec::new(),
                surface_owner: vec![0, 0, 0, 0],
                adjoin_flags: Vec::new(),
                ..Default::default()
            }));
            let mut bomb = base.clone();
            bomb.pos = [0.0, 0.0, 0.0];
            bomb.sector = 0;
            bomb.vel = [6.0, 0.0, 0.0];
            bomb.damage = 12.0;
            bomb.phys_flags = 0x225;
            rt.things = vec![base, bomb];
            rt.templates = vec!["walkplayer".into(), template.into()];
            rt.types = vec![10, 3];
            for _ in 0..60 {
                rt.step(None, 1.0 / 30.0);
                if rt.things[1].destroyed {
                    break;
                }
            }
            assert!(
                rt.things[1].destroyed,
                "`{template}` (gravity bit, not debris) froze on the wall: {:?}",
                rt.things[1].pos
            );
        }
    }

    /// `damageclass=` is the word `sithActor_DamageActor` branches on, and it follows the template
    /// chain: `+bryarbolt` inherits `_weapon`'s energy (`0x2`), while `+grenade2` ships fire (`0x4`)
    /// and `08tiebomb` impact (`0x1`). The hit path passed `DAMAGE_ENERGY` for every shot, so a
    /// grenade's fire and a tie bomb's impact took the energy branch in `item_shields.cog`.
    #[test]
    fn a_projectiles_damage_kind_comes_from_its_template_damageclass() {
        let template = |name: &str, base: Option<&str>, params: &[(&str, &str)]| crate::jkl::Template {
            name: name.to_string(),
            base: base.map(str::to_string),
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            model3d: None,
            sprite: None,
        };
        let mut level = Level::default();
        level.templates = vec![
            template("_weapon", None, &[("damageclass", "0x2")]),
            template("+bryarbolt", Some("_weapon"), &[]),
            template("+grenade2", Some("_weapon"), &[("damageclass", "0x4")]),
            template("08tiebomb", Some("_weapon"), &[("damageclass", "0x1")]),
        ];
        let mut rt = Runtime::new(&level);
        rt.templates = vec!["+bryarbolt".into(), "+grenade2".into(), "08tiebomb".into()];
        assert_eq!(rt.shot_damage_kind(0), DAMAGE_ENERGY, "a bolt inherits `_weapon`'s energy");
        assert_eq!(rt.shot_damage_kind(1), DAMAGE_FIRE, "a grenade is fire");
        assert_eq!(rt.shot_damage_kind(2), DAMAGE_IMPACT, "a tie bomb is impact");
    }

    /// The projectile body is the template's `movesize` — the engine's collision radius and what
    /// `getthingmovesize` returns — not the model's bounding radius (which includes the gun) and not
    /// a guess. This pins both the value and the height rule the hit test uses.
    #[test]
    fn a_templates_hit_body_is_its_movesize() {
        let mut level = Level::default();
        let template = |name: &str, params: &[(&str, &str)]| crate::jkl::Template {
            name: name.to_string(),
            base: None,
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            model3d: Some("x.3do".to_string()),
            sprite: None,
        };
        level.templates = vec![
            template("reeyeesgun", &[("movesize", "0.0729"), ("size", "0.0729")]),
            template("_actor", &[("movesize", "0.065")]),
            template("barething", &[]),
        ];
        let rt = Runtime::new(&level);
        let (radius, height) = rt.template_hit["reeyeesgun"];
        assert!((radius - 0.0729).abs() < 1e-4, "movesize is the radius, got {radius}");
        assert!((height / radius - 2.8).abs() < 0.01, "height follows the shipped models");
        assert_eq!(rt.template_hit["_actor"].0, 0.065);
        assert_eq!(rt.template_hit["barething"].0, 0.065, "an unnamed size keeps the player's");
    }

    /// A model face can support an actor just like a level floor.
    #[test]
    fn a_solid_things_model_is_a_floor() {
        // A `collide=3` walkstruct-style slab: its model's top face is the floor (`0x5080d0` mesh
        // arm into `FindFloor`'s thing attach), thingflag `0x40` admits the `0x10`-gated search.
        let slab = crate::collide::ThingSolid {
            index: 1,
            centre: [0.0, 0.0, 1.0],
            origin: [0.0, 0.0, 1.0],
            radius: 0.5,
            z_min: -0.5,
            z_max: 1.0123,
            sector: 0,
            half: None,
            yaw: 0.0,
            mesh: Some(vec![vec![crate::collide::Face {
                normal: [0.0, 0.0, 1.0],
                point: [0.0, 0.0, 1.0123],
                verts: vec![
                    [-0.5, -0.5, 1.0123], [0.5, -0.5, 1.0123],
                    [0.5, 0.5, 1.0123], [-0.5, 0.5, 1.0123],
                ],
                surface: usize::MAX,
            }]].into()),
            thing_flags: 0x40,
            collide: 3,
            owner: None,
            kind: 7,
            item: false,
        };
        let solids = crate::collide::Solids::default();
        solids.set_things(vec![slab.clone()]);

        // The actor starts beyond creation reach, falls, and seats on the mesh face.
        let mut rt = door_runtime();
        rt.things[0].pos = [0.0, 0.0, 1.5];
        rt.things[0].sector = 0;
        rt.things[0].phys_flags = 0x4a4f;
        rt.templates = vec!["mousebot".into()];
        rt.types = vec![2];
        rt.mass.insert(0, 150.0);
        let mut support = rt.things[0].clone();
        support.pos = slab.origin;
        support.flags = 0x40;
        support.phys_flags = 0;
        rt.things.push(support);
        rt.templates.push("testslab".into());
        rt.types.push(7);
        let plane = crate::collide::Plane {
            normal: [0.0, 0.0, -1.0],
            point: [0.0; 3],
            surface: 1,
        };
        let room = crate::collide::Solids {
            sectors: vec![vec![plane]],
            hull: vec![vec![plane]],
            spheres: vec![([0.0, 0.0, 0.0], 100.0)],
            portals: vec![Vec::new()],
            ..Default::default()
        };
        room.set_things(vec![slab]);
        rt.solids = Some(std::sync::Arc::new(room));
        for _ in 0..90 {
            rt.step(None, 1.0 / 30.0);
        }
        // `GetThingHeight` (`0x4f6210`): the template carries no `height=`, so the rest offset is
        // `moveSize + 0.005` — one radius plus the engine's 0.005 shave, not the bare radius.
        assert!(
            (rt.things[0].pos[2] - (1.0123 + 0.065 + 0.005)).abs() < 1e-3,
            "the actor did not settle on the slab: z {}",
            rt.things[0].pos[2]
        );
        let attach = rt.things[0].floor_attach.expect("the actor attached to the slab");
        assert_eq!(attach.thing.map(|hit| hit.thing), Some(1));
        assert_eq!(rt.things[0].vel[2], 0.0, "the flat contact removes persistent normal velocity");
    }

    /// A projectile's own move is what the engine tests (`sithCollision_SearchForCollisions`), not
    /// one sample per frame: a 30 u/s bolt travels 0.5 units at 60 fps, twice the 0.215 contact
    /// radius, so the end sample can be outside the body while the move passed through it.
    #[test]
    fn a_fast_bolt_is_swept_between_frames() {
        let mut rt = door_runtime();
        rt.templates = vec!["walkplayer".into(), "+bryarbolt".into()];
        rt.types = vec![2, 0];
        let mut shot = rt.things[0].clone();
        // Post-move position: half a unit past the body, having started exactly on it.
        shot.pos = [0.5, 0.0, 0.0];
        shot.vel = [30.0, 0.0, 0.0];
        rt.things.push(shot);
        assert!(!rt.shot_hits(1, 0), "the end sample alone is outside the body");
        assert!(rt.shot_hits_swept(1, 0, 1.0 / 60.0), "the frame's move crosses it");
    }

    #[test]
    fn a_body_blends_toward_its_look_frame() {
        // 90 degrees off, `maxrotthrust` 180 (so k = 180 * 0.1 * dt): forward [1,0] is yaw -90,
        // look [0,1] is yaw 0, and one 1/30 s tick blends to normalize([1,0.6]) = yaw -59.04 —
        // 30.96 degrees of the 90 covered.
        let (f, done) = turn_blend([1.0, 0.0], [0.0, 1.0], 180.0, 1.0 / 30.0);
        assert!(!done);
        assert!((yaw_of_forward(f) + 59.04).abs() < 0.01, "yaw {}", yaw_of_forward(f));
        // Aligned within the engine's 0.01 right-axis band and not behind: the blend ends without
        // moving the body further.
        let (f, done) = turn_blend([1.0, 0.0], [0.9999999, 0.0], 180.0, 1.0 / 30.0);
        assert!(done);
        assert_eq!(f, [1.0, 0.0]);
        // The blend decays the remaining angle roughly exponentially: three more ticks at k = 0.6
        // bring the remaining 59 degrees to 14.8.
        let (f, _) = turn_blend(f_next(-59.04), [0.0, 1.0], 180.0, 1.0 / 30.0);
        let (f, _) = turn_blend(f, [0.0, 1.0], 180.0, 1.0 / 30.0);
        let (f, _) = turn_blend(f, [0.0, 1.0], 180.0, 1.0 / 30.0);
        assert!((yaw_of_forward(f) + 14.82).abs() < 0.05, "yaw {}", yaw_of_forward(f));
        // Dead astern (`0x4ea630`): with k = 1 the engine steps along the **right vector**, not
        // the look vector — `look * k` against an opposite forward renormalises to the same
        // forward, so the blend could never turn a target exactly behind. rvec of [-1, 0] is
        // [0, 1]: normalize([-1, 1]) = a 45-degree step.
        let (f, done) = turn_blend([-1.0, 0.0], [1.0, 0.0], 30.0, 1.0 / 3.0);
        assert!(!done);
        assert_eq!(f, [-0.70710677, 0.70710677]);
    }

    fn f_next(deg: f32) -> [f32; 2] {
        forward_of_yaw(deg)
    }

    #[test]
    fn every_verb_the_runtime_answers_is_routed_to_it() {
        // The bug this pins: `vectorset` had a `call` arm but no `handles` entry, so the
        // interpreter treated it as unknown and returned 0 — a force jump was silently
        // zero. Any verb the runtime answers must also be routed to it.
        for verb in [
            "vectorset", "vectoradd", "vectoradd2", "vectorsub", "vectorscale",
            "setthingvel", "getthingvel", "activatebin", "deactivatebin",
            "setinvactivated", "isinvactivated", "setbinwait", "getattachflags",
            "getphysicsflags", "setphysicsflags", "clearphysicsflags", "detachthing",
            "setpulse", "jkgetflags", "jksetflags", "jksetpersuasioninfo", "getsoundlen",
            "sendmessage", "takeitem", "getinvmax", "setinvavailable", "adddynamictint",
            "selectweapon", "setcurinvweapon", "getcurinvweapon", "assignweapon",
            "firstthinginview", "nextthinginview", "haslos", "thingviewdot", "applyforce",
            "setactorflags", "clearactorflags", "getactorflags", "getthingsignature", "bittest",
            "skilltarget", "getactorweapon", "setactorweapon", "amputatejoint", "playmode",
            "getlifeleft", "setlifeleft", "getitemflags", "getparam", "gethealth", "getleveltime",
            "isthingvisible", "getthingmass", "setthingmass", "getthinguserdata", "setthinguserdata",
            "setthingtimer",
        ] {
            assert!(Runtime::handles(verb), "`{verb}` is answered but not routed");
        }
    }

    #[test]
    fn door_verbs_move_a_thing_and_close_a_sector() {
        let mut rt = door_runtime();
        rt.call("SetSectorAdjoins", &[Value::Num(0.0), Value::Num(0.0)]);
        assert_eq!(rt.adjoins_changes, 1);
        assert_eq!(rt.call("GetSectorAdjoins", &[Value::Num(0.0)]), Value::Num(0.0));

        rt.call("MoveToFrame", &[Value::Num(0.0), Value::Num(1.0), Value::Num(2.0)]);
        assert_eq!(rt.things_moved, 1);
        assert_eq!(rt.call("IsMoving", &[Value::Num(0.0)]), Value::Num(1.0));

        // Two seconds of motion at 2 frames/s reaches frame 1 and the thing moves.
        for _ in 0..240 {
            rt.step(None, 1.0 / 120.0);
        }
        assert_eq!(rt.call("GetCurFrame", &[Value::Num(0.0)]), Value::Num(1.0));
        assert_eq!(rt.things[0].pos[0], 1.0);
        assert_eq!(rt.call("GetThingSector", &[Value::Num(0.0)]), Value::Num(7.0));

        rt.call("PlaySoundPos", &[Value::Str("door.wav".into()), Value::Vec([1.0, 2.0, 3.0])]);
        assert_eq!(rt.sounds.len(), 1);
        assert_eq!(rt.sounds[0].name, "door.wav");
        // A sound from a thing carries that thing's position, which is what the mixer
        // attenuates over; `PlaySoundLocal` carries none.
        assert!(rt.sounds[0].pos.is_none(), "PlaySoundLocal should have no position");
        let at = rt.call(
            "playsoundthing",
            &[crate::cog::Value::Str("shot.wav".into()), crate::cog::Value::Num(0.0)],
        );
        let _ = at;
        let request = rt.sounds.last().expect("the thing sound");
        assert_eq!(request.name, "shot.wav");
        assert_eq!(request.pos, Some(rt.things[0].pos), "the thing's position was not captured");

        // The engine's inventory verbs take the *thing* first: `GetInv(player, bin)`.
        rt.call("SetInv", &[Value::Num(0.0), Value::Num(3.0), Value::Num(2.0)]);
        rt.call("ChangeInv", &[Value::Num(0.0), Value::Num(3.0), Value::Num(1.0)]);
        assert_eq!(rt.call("GetInv", &[Value::Num(0.0), Value::Num(3.0)]), Value::Num(3.0));
    }

    #[test]
    fn adjoin_and_sector_flags_reach_the_gates_the_solver_reads() {
        let mut rt = door_runtime();
        // `surface_adjoin` names row 0, whose shipped word is the ordinary `0x7`.
        assert_eq!(rt.adjoin_flags[0], 0x7);
        // `ClearAdjoinFlags(surface, 2)`: the force field goes up, and the sweep's gate sees it.
        rt.call("ClearAdjoinFlags", &[Value::Num(0.0), Value::Num(2.0)]);
        assert_eq!(rt.adjoin_flags[0] & crate::collide::ADJOIN_MOVE, 0);
        assert!(!rt.gates().passable(0, 1, 0), "a cleared MOVE bit must close the opening");
        // ... and `SetAdjoinFlags(surface, 2)` opens it again.
        rt.call("SetAdjoinFlags", &[Value::Num(0.0), Value::Num(2.0)]);
        assert!(rt.gates().passable(0, 1, 0));
        // Bit 0x10 blocks the walker too: `00_nogo*`-style "block rear-door access".
        rt.call("SetAdjoinFlags", &[Value::Num(0.0), Value::Num(16.0)]);
        assert!(!rt.gates().passable(0, 1, 0));
        // `ClearAdjoinFlags(visFace, 1)` hides the opening's room from the cull walk.
        rt.call("ClearAdjoinFlags", &[Value::Num(0.0), Value::Num(1.0)]);
        assert!(!rt.gates().visible(0));

        // `SetSectorFlags(sector, 2)`: the sector becomes a water volume, which is the bit the
        // walker reads; `GetSectorFlags` reports the word back.
        rt.call("SetSectorFlags", &[Value::Num(0.0), Value::Num(2.0)]);
        assert_eq!(rt.sector_flags[0] & crate::collide::SECTOR_UNDERWATER, 2);
        assert_eq!(rt.call("GetSectorFlags", &[Value::Num(0.0)]), Value::Num(2.0));
        // `SetSectorAdjoins(sector, 0)` is the same switch as `0x80` in that word.
        rt.call("SetSectorAdjoins", &[Value::Num(0.0), Value::Num(0.0)]);
        assert_eq!(rt.sector_flags[0] & crate::collide::SECTOR_ADJOINS_OFF, 0x80);
        assert!(!rt.gates().passable(0, 1, 0), "a hidden sector doorway is not crossed");
    }

    #[test]
    fn a_projectiles_damage_decays_in_flight() {
        let mut rt = door_runtime();
        let bolt = "+bryarbolt".to_string();
        rt.template_vel.insert(bolt.clone(), [0.0, 4.0, 0.0]);
        rt.template_damage.insert(bolt.clone(), 30.0);
        rt.template_min_damage.insert(bolt.clone(), 10.0);
        rt.template_rate.insert(bolt.clone(), 15.0);
        rt.template_decay.insert(bolt.clone());
        let shot = rt.fire(0, &bolt).expect("projectile");
        assert_eq!(rt.things[shot].damage, 30.0);
        // `sithWeapon_Update`: damage -= rate * dt, floored at mindamage.
        rt.step(None, 1.0);
        assert_eq!(rt.things[shot].damage, 15.0);
        // The floor holds no matter how long it flies.
        rt.step(None, 10.0);
        assert_eq!(rt.things[shot].damage, 10.0);
        // A weapon without the decay flag keeps its damage.
        rt.template_decay.remove(&bolt);
        let steady = rt.fire(0, &bolt).expect("projectile");
        rt.step(None, 5.0);
        assert_eq!(rt.things[steady].damage, 30.0);
    }

    /// A shot flies along the shooter's orientation, not just its yaw: `sithWeapon_FireProjectile`
    /// transforms the direction by the shooter's own matrix, which carries pitch. Firing level
    /// whatever the aim was the bug this pins shut.
    #[test]
    fn a_shot_follows_the_shooters_aim() {
        let mut rt = door_runtime();
        let bolt = "+bryarbolt".to_string();
        rt.template_vel.insert(bolt.clone(), [0.0, 4.0, 0.0]);
        rt.template_damage.insert(bolt.clone(), 1.0);

        // Level: the engine's forward at yaw 0 is +Y, z untouched.
        let shot = rt.fire(0, &bolt).expect("projectile");
        assert!((rt.things[shot].vel[1] - 4.0).abs() < 1e-3, "vel {:?}", rt.things[shot].vel);
        assert!(rt.things[shot].vel[0].abs() < 1e-3, "yaw 0 fired off the forward axis: {:?}", rt.things[shot].vel);
        assert!(rt.things[shot].vel[2].abs() < 1e-3, "a level shot rose: {:?}", rt.things[shot].vel);

        // Aimed 30 degrees up: the speed splits the way `sin`/`cos` say it must.
        rt.things[0].pitch = 30.0;
        let shot = rt.fire(0, &bolt).expect("projectile");
        let vel = rt.things[shot].vel;
        assert!((vel[1] - 4.0 * 30f32.to_radians().cos()).abs() < 1e-3, "vel {vel:?}");
        assert!((vel[2] - 4.0 * 30f32.to_radians().sin()).abs() < 1e-3, "vel {vel:?}");

        // And down again, which is the half a player notices on a ledge.
        rt.things[0].pitch = -45.0;
        let shot = rt.fire(0, &bolt).expect("projectile");
        assert!(rt.things[shot].vel[2] < -2.5, "vel {:?}", rt.things[shot].vel);
    }

    /// The AI's fire cadence is drawn from the same engine `rand()` stream the capture records, so
    /// seeding it makes the delay the capture's next draw (issue #16). The first draw after
    /// 811474043 is 0.5316467, and `sithAICmd_PrimaryFire` jitters the 750 ms delay by
    /// `0.8 + 0.4 * rand` in this build, i.e. 0.759 s.
    #[test]
    fn the_a_i_fire_delay_is_drawn_from_the_captured_rand_stream() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut player = base.clone();
        player.pos = [0.0, 1.0, 0.0];
        let mut gunner = base.clone();
        gunner.pos = [0.0, 0.0, 0.0];
        gunner.yaw = 0.0;
        rt.things = vec![player, gunner];
        rt.templates = vec!["walkplayer".into(), "reeyeesgun".into()];
        rt.types = vec![10, 2];
        rt.template_weapon.insert("reeyeesgun".into(), "+elaser".into());
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=200, sightdist=5.0\n\n\
                 PrimaryFire 750.0, 0.8, 3.5, 0.20, 0.0, 500.0, 0.0, 0.0, 0.0, 0.0\n",
            ),
        );
        // Already engaged, so the sight roll makes no draw of its own. The first draws of the
        // tick are `sithAI_FireWeapon`'s accuracy gate (`error` 0.20, class accuracy 0.5) and its
        // jitter cone when the gate passes; the shot delay's jitter draws after them, exactly as
        // the engine's handler writes its cadence after `FireWeapon` returns.
        rt.ai_detected.insert(1);
        rt.time = 1.0;
        rt.seed_capture_phase(
            811_474_043,
            0,
            &[(1, CapturedPhase { flags: 0x232, fire_ms: 0, acquire_ms: 0, next_pass_ms: 0 })],
        );
        rt.ai_tick(0, 0.1);
        assert_eq!(rt.shots_fired, 1, "the gunner did not fire");
        assert!(
            (rt.things[1].ai_wait - 0.649_868_8).abs() < 1e-5,
            "the delay came from another stream: {}",
            rt.things[1].ai_wait
        );
        assert_ne!(crate::cog::random_state(), 811_474_043, "the shot drew nothing from the capture");
    }

    /// The capture's origin block is the door encounter's starting phase (issue #16): the AI
    /// `rand()` word it recorded, the flag word each actor held, and the deadlines the engine had
    /// already scheduled. The actor's own `ai_wait` is the shot delay counted from the origin, the
    /// scan lands on the runtime clock at the captured offset, and an actor whose shot the capture
    /// shows as unscheduled (`fire_ms` 0) keeps whatever wait it had.
    #[test]
    fn the_door_encounter_starts_from_the_captured_a_i_phase() {
        let mut rt = door_runtime();
        let mut gunner = rt.things[0].clone();
        gunner.pos = [1.4122, 9.3586, 6.3313];
        rt.things.push(gunner);
        rt.templates.push("reeyeesgun".into());
        rt.types.push(2);
        let mut watcher = rt.things[0].clone();
        watcher.pos = [4.7624, 10.1534, 6.3313];
        watcher.ai_wait = 5.0;
        rt.things.push(watcher);
        rt.templates.push("reeyeesgun".into());
        rt.types.push(2);
        // The scheduler needs the class to seed the `lookfortarget` slot from `acquire_ms`.
        rt.ai.insert(
            1,
            crate::ai::parse("alignment=-1.0, rank=0.5, fov=90, sightdist=5.0\n\nLookForTarget 500.0\n"),
        );
        rt.ai.insert(
            2,
            crate::ai::parse("alignment=-1.0, rank=0.5, fov=90, sightdist=5.0\n\nLookForTarget 500.0\n"),
        );
        rt.time = 0.4;
        rt.seed_capture_phase(
            811_474_043,
            4200,
            &[
                // The capture's gunner: already engaged, next shot 104 ms after the origin.
                (1, CapturedPhase { flags: 1586, fire_ms: 4304, acquire_ms: 4550, next_pass_ms: 4304 }),
                // A door actor only scanning: `acquire_next_ms` 4529, no shot scheduled.
                (2, CapturedPhase { flags: 4, fire_ms: 0, acquire_ms: 4529, next_pass_ms: 4529 }),
            ],
        );
        assert_eq!(crate::cog::random_state(), 811_474_043, "the capture's own rand() word");
        assert_eq!(rt.ai_flags[&1], 1586);
        assert_eq!(rt.ai_flags[&2], 4);
        assert!((rt.things[1].ai_wait - 0.104).abs() < 1e-6, "first shot {}", rt.things[1].ai_wait);
        assert!((scan_clock(&rt, 1) - 0.75).abs() < 1e-6, "scan at {}", scan_clock(&rt, 1));
        assert!((rt.ai_detect_at[&1] - 0.504).abs() < 1e-6);
        assert!((scan_clock(&rt, 2) - 0.729).abs() < 1e-6, "scan at {}", scan_clock(&rt, 2));
        assert!((rt.things[2].ai_wait - 5.0).abs() < 1e-6, "an unscheduled shot was invented");
        // The origin clock is the capture's: the offsets are its milliseconds, not a guess.
        assert!(rt.ai_inst.get(&2).is_some_and(|slots| slots.iter().all(|c| *c > rt.time)));
    }

    /// `--encounter` telemetry (issue #9): a shot is queued where it leaves the shooter, with the
    /// runtime's own clock and the shooter's id, so the port's timeline can be compared with the
    /// retail fixture. The queue is off unless the capture harness asks (no cost in live play).
    #[test]
    fn encounter_telemetry_queues_each_shot_with_its_shooter_and_clock() {
        let mut rt = door_runtime();
        let bolt = "+bryarbolt".to_string();
        rt.template_vel.insert(bolt.clone(), [0.0, 4.0, 0.0]);
        rt.template_damage.insert(bolt.clone(), 12.0);
        rt.time = 1.25;
        rt.fire(0, &bolt).expect("projectile");
        assert!(rt.encounter_events.is_empty(), "telemetry must stay off by default");

        rt.encounter_events_enabled = true;
        rt.time = 2.5;
        rt.fire(0, &bolt).expect("projectile");
        assert_eq!(rt.encounter_events.len(), 1);
        match &rt.encounter_events[0] {
            EncounterEvent::Shot { t, owner, template, pos, target } => {
                assert!((t - 2.5).abs() < 1e-5, "event time {t}");
                assert_eq!(*owner, 0, "the shooter's thing index");
                assert_eq!(template, "+bryarbolt");
                assert_eq!(*pos, rt.things[0].pos, "the shot leaves the shooter's own position");
                assert_eq!(*target, None, "a shot nothing names has no target");
            }
            other => panic!("expected a shot event, got {other:?}"),
        }
    }

    /// The chronology says hit or miss and what the shooter was aiming at: a landed shot carries the
    /// AI's target and its launch time, and a shot that dies without landing is a `Miss` with the
    /// same launch time. The map is keyed by the projectile's own thing index.
    #[test]
    fn encounter_telemetry_tells_a_landed_shot_from_a_miss_with_the_shooters_target() {
        let mut rt = door_runtime();
        let bolt = "+bryarbolt".to_string();
        rt.template_vel.insert(bolt.clone(), [0.0, 4.0, 0.0]);
        rt.template_damage.insert(bolt.clone(), 12.0);
        rt.encounter_events_enabled = true;

        rt.time = 4.0;
        let landed = rt.fire_toward(0, &bolt, 0.0, None, Some(9), None).expect("projectile");
        rt.note_shot_landed(landed);
        match rt.encounter_events.last() {
            Some(EncounterEvent::ShotLanded { t, target, .. }) => {
                assert!((t - 4.0).abs() < 1e-5, "the launch time is reported, not the impact's");
                assert_eq!(*target, Some(9), "the AI's target thing");
            }
            other => panic!("expected a landed shot, got {other:?}"),
        }

        rt.time = 5.0;
        let missed = rt.fire_toward(0, &bolt, 0.0, None, Some(9), None).expect("projectile");
        rt.note_shot_lost(missed);
        match rt.encounter_events.last() {
            Some(EncounterEvent::Miss { t, target, .. }) => {
                assert!((t - 5.0).abs() < 1e-5, "event time {t}");
                assert_eq!(*target, Some(9));
            }
            other => panic!("expected a miss, got {other:?}"),
        }
        assert!(rt.encounter_shot_time.is_empty(), "a resolved shot is not reported twice");
    }

    /// The chronology's `Miss` is driven by the runtime, not only the reporting helper: a projectile
    /// whose `timer=` expires before it lands writes the row and is destroyed
    /// (`sithThing_DestroyDyingThing`, `life_left`).
    #[test]
    fn a_bolt_that_times_out_is_reported_as_a_miss() {
        let mut rt = door_runtime();
        let bolt = "+bryarbolt".to_string();
        rt.template_vel.insert(bolt.clone(), [0.0, 4.0, 0.0]);
        rt.template_damage.insert(bolt.clone(), 12.0);
        rt.encounter_events_enabled = true;
        rt.time = 4.0;
        let shot = rt.fire_toward(0, &bolt, 0.0, None, Some(9), None).expect("projectile");
        rt.life_left.insert(shot, 0.01);
        rt.step(None, 1.0 / 60.0);
        let miss = rt.encounter_events.iter().any(|event| {
            matches!(event, EncounterEvent::Miss { owner, target, .. } if *owner == 0 && *target == Some(9))
        });
        assert!(miss, "an expired projectile must be a miss: {:?}", rt.encounter_events);
        assert!(rt.things[shot].destroyed, "the expired projectile is destroyed");
        assert!(!rt.encounter_shot_time.contains_key(&shot), "a resolved shot is not reported twice");
    }

    /// The door fixture's own player bars, hit for hit (`tools/parity/encounter/door_encounter.tsv`,
    /// 9.9 s: `player` rows at 3, 11, 18, 23, 30, 37, 43, 52, 60, 69, 78, 85 and 93 tenths). The
    /// engine's split is not shield-first: `item_shields.cog`'s `damaged` handler (bin 60) spends the
    /// whole hit on the bar but lets health take a share that grows as the bar falls, and the
    /// fixture's `100 -> 22.3` health is what that arithmetic produces for thirteen ~11.3-point
    /// energy bolts. The expected bars are the fixture's literals, not a recomputation.
    #[test]
    fn the_door_fixtures_player_bars_come_out_of_the_shields_script_per_hit() {
        let mut rt = door_runtime();
        // (shields, health) after each of the fixture's thirteen hits.
        let fixture = [
            (88.6, 100.0),
            (77.4, 100.0),
            (66.1, 100.0),
            (54.8, 100.0),
            (43.4, 99.1),
            (32.1, 95.1),
            (20.7, 88.4),
            (9.4, 79.2),
            (0.0, 67.5),
            (0.0, 56.2),
            (0.0, 45.0),
            (0.0, 33.6),
            (0.0, 22.3),
        ];
        for (index, (shields, health)) in fixture.iter().enumerate() {
            let (got_shields, got_health) = rt.damage_player(11.3, DAMAGE_ENERGY);
            assert!(
                (got_shields - shields).abs() < 0.3,
                "hit {}: shields {got_shields:.2}, fixture {shields}",
                index + 1
            );
            assert!(
                (got_health - health).abs() < 0.3,
                "hit {}: health {got_health:.2}, fixture {health}",
                index + 1
            );
        }
    }

    /// The script only acts on the kinds it names: energy (2) and fire (4) share a branch, saber
    /// (16) gets half protection, and every other kind — an explosion's impact, force, falls —
    /// reaches health untouched, bar and all.
    #[test]
    fn only_energy_fire_and_saber_hits_meet_the_shield_script() {
        let mut rt = door_runtime();
        assert_eq!(rt.damage_player(30.0, DAMAGE_IMPACT), (100.0, 70.0), "impact bypasses the bar");

        let mut rt = door_runtime();
        let (shields, health) = rt.damage_player(11.3, DAMAGE_FIRE);
        assert!((shields - 88.7).abs() < 1e-3, "fire: shields {shields}");
        assert_eq!(health, 100.0, "fire behind a full bar must not reach health");

        let mut rt = door_runtime();
        // Half of 20 is 10; the bar takes 10 and health the same 10 (`damage + halfdamage`).
        assert_eq!(rt.damage_player(20.0, DAMAGE_SABER), (90.0, 90.0), "saber half protection");
    }

    /// A landed player hit carries the bar **after** the hit and the amount the bars actually lost,
    /// which is what the retail capture writes (`amount = previous shields + health - current`).
    #[test]
    fn encounter_telemetry_queues_landed_player_hits_with_the_bar_after() {
        let mut rt = door_runtime();
        rt.encounter_events_enabled = true;
        rt.player_shields = 50.0;
        rt.time = 3.0;
        rt.damage_player(11.3, DAMAGE_ENERGY);
        rt.time = 3.5;
        rt.damage_player(11.3, DAMAGE_ENERGY);
        let hits: Vec<(f32, f32, f32, f32, f32, f32)> = rt
            .encounter_events
            .iter()
            .filter_map(|event| match event {
                EncounterEvent::Hit { t, amount, shields, health, health_lost, shields_lost } =>
                    Some((*t, *amount, *shields, *health, *health_lost, *shields_lost)),
                _ => None,
            })
            .collect();
        assert_eq!(hits.len(), 2, "one hit row per landed shot");
        assert!((hits[0].0 - 3.0).abs() < 1e-5);
        // Bar 50 -> 38.7, health takes 11.3 * (1 - 38.7/50).
        assert!((hits[0].1 - 13.8538).abs() < 1e-3, "amount {}", hits[0].1);
        assert!((hits[0].2 - 38.7).abs() < 1e-3, "shields {}", hits[0].2);
        assert!((hits[0].3 - 97.4462).abs() < 1e-3, "health {}", hits[0].3);
        // The split: the bar took the whole 11.3, health the rest of the loss.
        assert!((hits[0].4 - 2.5538).abs() < 1e-3, "health lost {}", hits[0].4);
        assert!((hits[0].5 - 11.3).abs() < 1e-3, "shields lost {}", hits[0].5);
        // Bar 38.7 -> 27.4, health takes 11.3 * (1 - 27.4/50).
        assert!((hits[1].1 - 16.4076).abs() < 1e-3, "amount {}", hits[1].1);
        assert!((hits[1].2 - 27.4).abs() < 1e-3, "shields {}", hits[1].2);
        assert!((hits[1].3 - 92.3386).abs() < 1e-3, "health {}", hits[1].3);
        assert!((hits[1].4 - 5.1076).abs() < 1e-3, "health lost {}", hits[1].4);
        assert!((hits[1].5 - 11.3).abs() < 1e-3, "shields lost {}", hits[1].5);

        // A hit the player's invulnerability refuses is not a landed hit and writes no row.
        rt.actor_flags.insert(0, AF_INVULNERABLE);
        rt.damage_player(20.0, DAMAGE_ENERGY);
        assert_eq!(rt.encounter_events.len(), 2, "an ignored hit must not be recorded");
    }

    /// `sithAI_RandomFireVector` (0x4eb920): the aim error jitters the aim **vector** by ±error/2 per
    /// axis and keeps the shot's speed — a small cone. The old version turned the yaw inside ±error
    /// right angles, which was several times wider and left elevation alone.
    #[test]
    fn an_aim_error_jitters_the_shot_inside_a_small_cone() {
        let aim = [4.0, 0.0, 0.0];
        let clean = Runtime::jitter_aim(aim, 0.0);
        assert_eq!(clean, aim, "an error of zero must not move the shot");
        for _ in 0..500 {
            let jittered = Runtime::jitter_aim(aim, 0.2);
            let speed = (jittered[0] * jittered[0]
                + jittered[1] * jittered[1]
                + jittered[2] * jittered[2])
                .sqrt();
            assert!((speed - 4.0).abs() < 1e-3, "the jitter changed the speed: {jittered:?}");
            // ±0.1 per axis is at most ~8.1 degrees off the aim line; 0.98 is 11.5.
            assert!(jittered[0] / speed > 0.98, "a 0.2 error threw the shot wide: {jittered:?}");
        }
    }

    #[test]
    fn a_fired_projectile_flies_and_its_damage_lands() {
        let mut rt = door_runtime();
        // The template tables the level would carry: a bolt that flies down +Y at 4 u/s
        // for 30 damage, and an actor with 50 health.
        let bolt = "+bryarbolt".to_string();

        rt.template_vel.insert(bolt.clone(), [0.0, 4.0, 0.0]);
        rt.template_damage.insert(bolt.clone(), 30.0);
        assert_eq!(rt.damage_of("+bryarbolt"), 30.0);

        let shot = rt.fire(0, "+bryarbolt").expect("projectile");
        assert_eq!(rt.things.len(), 2);
        // Fired along the shooter's facing: the engine's yaw 0 faces +Y (`(-sin yaw, cos yaw)`).
        assert!(rt.things[shot].vel[1] > 0.0, "vel {:?}", rt.things[shot].vel);
        rt.step(None, 0.5);
        assert!(rt.things[shot].pos[1] > 1.0, "pos {:?}", rt.things[shot].pos);

        // Damage lands, and a killing blow marks the thing destroyed.
        assert_eq!(rt.damage(0, 30.0), 70.0);
        assert_eq!(rt.damage(0, 80.0), 0.0);
        assert!(rt.things[0].destroyed);
    }

    /// `sithActor_DamageActor` flashes the player's palette red at `damage * 0.04`, `sithPlayer_Update`
    /// decays every channel at 0.4/s, and `AddDynamicTint` **adds** to the same channels with the
    /// engine's ceilings (red/green 1.0, blue 0.5). These are the numbers the door capture needed.
    #[test]
    fn damage_and_scripts_add_to_one_decaying_palette_tint() {
        let mut rt = door_runtime();
        // 20 damage adds 0.8 of red — before the bar takes any of it, as the engine's call is.
        rt.damage_player(20.0, 0);
        assert!((rt.tint[0] - 0.8).abs() < 1e-5, "tint {:?}", rt.tint);
        // A script adds its own channels: red clamps at 1.0, blue at 0.5, and it is an add.
        rt.call(
            "AddDynamicTint",
            &[Value::Num(0.0), Value::Num(0.5), Value::Num(0.0), Value::Num(1.0)],
        );
        assert!((rt.tint[0] - 1.0).abs() < 1e-5, "red not clamped: {:?}", rt.tint);
        assert!((rt.tint[2] - 0.5).abs() < 1e-5, "blue not clamped: {:?}", rt.tint);
        // Half a second of `sithPlayer_Update` takes 0.2 off the red, and it reaches zero.
        rt.step(None, 0.5);
        assert!((rt.tint[0] - 0.8).abs() < 1e-4, "tint after half a second {:?}", rt.tint);
        rt.step(None, 10.0);
        assert_eq!(rt.tint, [0.0; 3], "the tint did not decay away");
    }

    #[test]
    fn a_sprite_explosion_walks_its_cels_over_its_life() {
        let mut rt = door_runtime();
        // A spark like `+laserhit`: `typeflags=0x1` animates the sprite, `timer=0.5` bounds it.
        rt.template_timer.insert("walkplayer".to_string(), 0.5);
        rt.template_sprite_anim.insert("walkplayer".to_string());
        let Value::Num(index) = rt.call("CreateThing", &[Value::Str("walkplayer".into()), Value::Num(0.0)])
        else {
            panic!("CreateThing returned no index")
        };
        let index = index as usize;
        // Just created: the first frame, for one eighth of half a second.
        assert_eq!(rt.sprite_cel(index, 8), 0);
        rt.life_left.insert(index, 0.375);
        assert_eq!(rt.sprite_cel(index, 8), 2);
        // The last frame is still drawn at the moment the thing dies, not one past it.
        rt.life_left.insert(index, 0.0);
        assert_eq!(rt.sprite_cel(index, 8), 7);
        // One cel is no animation, and the typeflag is what arms it at all.
        assert_eq!(rt.sprite_cel(index, 1), 0);
        rt.template_sprite_anim.clear();
        assert_eq!(rt.sprite_cel(index, 8), 0);
    }

    #[test]
    fn an_animated_surface_walks_its_cels_and_wraps() {
        let mut rt = door_runtime();
        rt.surfaces.push(SurfaceState {
            center: [0.0; 3],
            cel: 0.0,
            anim: 0.0,
            slide: [0.0; 3],
            next: 0.0,
            light: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
        });
        let index = rt.surfaces.len() - 1;
        // The material's cel count comes from the `.mat`; 3 cels here.
        rt.surface_cels = vec![1; index];
        rt.surface_cels.push(3);
        let epoch = rt.surface_epoch;
        // `SurfaceAnim(surface, 15)`: a cel every 1/15 s. Half a second is 7 steps of 1/15,
        // so cel 7, and 7 % 3 = 1.
        rt.call("SurfaceAnim", &[Value::Num(index as f64), Value::Num(15.0)]);
        rt.step(None, 0.5);
        assert_eq!(rt.surfaces[index].cel, 1.0);
        assert!(rt.surface_epoch > epoch, "a cel step did not ask for a rebuild");
        // A material with one cel never advances, whatever the rate says.
        rt.surface_cels[index] = 1;
        rt.step(None, 1.0);
        assert_eq!(rt.surfaces[index].cel, 1.0);
    }

    #[test]
    fn a_sector_light_fades_and_a_surface_light_ramps_once() {
        let mut rt = door_runtime();
        // `SetSectorLight(sector, 0.5, 2)`: half a unit of extra light over two seconds.
        rt.sectors.push(SectorState {
            thrust: [0.0; 3],
            light: 0.0,
            light_from: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
            tint: [0.0; 3],
            colormap: 0,
        });
        let sector = rt.sectors.len() - 1;
        rt.call(
            "SetSectorLight",
            &[Value::Num(sector as f64), Value::Num(0.5), Value::Num(2.0)],
        );
        assert_eq!(rt.sectors[sector].light, 0.0, "a fade should not snap to its target");
        rt.step(None, 1.0);
        assert!((rt.sectors[sector].light - 0.25).abs() < 1e-5);
        rt.step(None, 1.0);
        assert!((rt.sectors[sector].light - 0.5).abs() < 1e-5);
        // Settled: the rate is one-shot, so a long step does not overshoot.
        rt.step(None, 10.0);
        assert!((rt.sectors[sector].light - 0.5).abs() < 1e-5);

        // `SurfaceLightAnim(surface, 0.2, 1.0, 0.5)`: `cog_00_forcefield.cog`'s startup ramp,
        // `(1.0 - 0.2) / (0.5 * 0.5)` = 3.2 a second, so 0.25 s lands at 1.0 and stops.
        rt.surfaces.push(SurfaceState {
            center: [0.0; 3],
            cel: 0.0,
            anim: 0.0,
            slide: [0.0; 3],
            next: 0.0,
            light: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
        });
        let surface = rt.surfaces.len() - 1;
        let answer = rt.call(
            "SurfaceLightAnim",
            &[Value::Num(surface as f64), Value::Num(0.2), Value::Num(1.0), Value::Num(0.5)],
        );
        assert_eq!(answer, Value::Num(surface as f64), "the verb answers the surface index");
        assert_eq!(rt.surfaces[surface].light, 0.2);
        rt.step(None, 0.1);
        assert!((rt.surfaces[surface].light - 0.52).abs() < 1e-4);
        rt.step(None, 0.2);
        assert!((rt.surfaces[surface].light - 1.0).abs() < 1e-5);
        rt.step(None, 1.0);
        assert!((rt.surfaces[surface].light - 1.0).abs() < 1e-5, "the ramp kept walking");
        // A backwards ramp is no ramp: the engine answers -1 and leaves the surface alone.
        let refused = rt.call(
            "SurfaceLightAnim",
            &[Value::Num(surface as f64), Value::Num(1.0), Value::Num(0.0), Value::Num(1.0)],
        );
        assert_eq!(refused, Value::Num(-1.0));
    }

    #[test]
    fn sector_visibility_restores_only_adjoins_hidden_by_the_sector() {
        let mut rt = door_runtime();
        rt.surface_sector = vec![0, 0, 1, 0];
        rt.surface_adjoin = vec![0, 1, 2, 3];
        rt.adjoin_flags = vec![0x3, 0x2, 0x3, 0x103];
        rt.call("SetSectorAdjoins", &[Value::Num(0.0), Value::Num(0.0)]);
        assert_eq!(rt.adjoin_flags, [0x22, 0x2, 0x3, 0x122]);
        assert_ne!(rt.sector_flags[0] & SECTOR_ADJOINS_OFF, 0);
        // Hiding again still updates adjoins made visible while the sector flag remained set.
        rt.adjoin_flags[0] |= 1;
        rt.call("SetSectorAdjoins", &[Value::Num(0.0), Value::Num(0.0)]);
        assert_eq!(rt.adjoin_flags[0], 0x22);
        rt.call("SetSectorAdjoins", &[Value::Num(0.0), Value::Num(1.0)]);
        assert_eq!(rt.adjoin_flags, [0x3, 0x2, 0x3, 0x103], "unmarked invisible and other-sector adjoins must retain their flags");
        assert_eq!(rt.sector_flags[0] & SECTOR_ADJOINS_OFF, 0);
    }

    #[test]
    fn the_new_world_verbs_act_on_the_world() {
        let mut rt = door_runtime();
        // A surface's cel round-trips, and its centre is whatever the level said.
        rt.surfaces.push(SurfaceState { center: [1.0, 2.0, 3.0], cel: 0.0, anim: 0.0, slide: [0.0; 3], next: 0.0, light: 0.0, light_end: 0.0, light_rate: 0.0 });
        rt.call("SetWallCel", &[Value::Num(0.0), Value::Num(4.0)]);
        assert_eq!(rt.call("GetWallCel", &[Value::Num(0.0)]), Value::Num(4.0));
        assert_eq!(rt.call("SurfaceCenter", &[Value::Num(0.0)]), Value::Vec([1.0, 2.0, 3.0]));
        // A zero rate means the engine's default 15 fps, not "stopped".
        rt.call("SurfaceAnim", &[Value::Num(0.0), Value::Num(0.0)]);
        assert_eq!(rt.call("GetSurfaceAnim", &[Value::Num(0.0)]), Value::Num(15.0));

        // `SectorAdjoins` is the same switch as `SetSectorAdjoins`.
        rt.call("SectorAdjoins", &[Value::Num(0.0), Value::Num(0.0)]);
        assert_eq!(rt.call("GetSectorAdjoins", &[Value::Num(0.0)]), Value::Num(0.0));

        // A thing's type comes from its template (`walkplayer` is `type=player`).
        assert_eq!(rt.call("GetThingType", &[Value::Num(0.0)]), Value::Num(10.0));
        // Sector iteration walks the things placed in that sector.
        assert_eq!(rt.call("FirstThingInSector", &[Value::Num(7.0)]), Value::Num(0.0));
        assert_eq!(rt.call("NextThingInSector", &[Value::Num(7.0), Value::Num(0.0)]), Value::Num(-1.0));
        // `CreateThing` spawns at its source thing, with the template's health and type.
        let created = rt.call("CreateThing", &[Value::Str("walkplayer".into()), Value::Num(0.0)]);
        assert_eq!(created, Value::Num(1.0));
        assert_eq!(rt.things.len(), 2);
        assert_eq!(rt.types[1], 10);
        assert_eq!(rt.call("GetThingHealth", &[Value::Num(1.0)]), Value::Num(100.0));

        // `DamageThing` is health bookkeeping; `DestroyThing` removes it from the world.
        rt.call("DamageThing", &[Value::Num(0.0), Value::Num(30.0)]);
        assert_eq!(rt.call("GetThingHealth", &[Value::Num(0.0)]), Value::Num(70.0));
        rt.call("DestroyThing", &[Value::Num(1.0)]);
        assert!(rt.things[1].destroyed);
        // Script text is collected, not dropped.
        rt.call("jkPrintUNIString", &[Value::Str("GUI_NOPE".into())]);
        assert_eq!(rt.prints, vec!["GUI_NOPE".to_string()]);
        // The shipped two-argument form is `jkPrintUNIString(player, key)`: the **second** argument
        // names the `.uni` key (`jkPrintUNIString(player, bin)` in `pow_shields.cog` names bin 60,
        // "Shield Recharge"). Reading the first printed the player's thing index, which is how a
        // pickup on level 1 put `COG_00000` across the top of the screen.
        rt.call("jkPrintUNIString", &[Value::Num(0.0), Value::Num(60.0)]);
        assert_eq!(rt.prints.last().map(String::as_str), Some("COG_00060"));
        assert_eq!(rt.hud_line(), Some("COG_00060"));
        rt.call("jkPrintUNIString", &[Value::Num(0.0), Value::Str("GUI_NOPE".into())]);
        assert_eq!(rt.prints.last().map(String::as_str), Some("GUI_NOPE"));
    }

    #[test]
    fn a_script_line_reaches_the_hud_and_then_expires() {
        let mut rt = door_runtime();
        assert!(rt.hud_line().is_none(), "a fresh level shows nothing");
        rt.call("jkStringClear", &[]);
        rt.call("jkStringConcatUniString", &[Value::Num(262.0)]);
        rt.call("jkStringOutput", &[Value::Num(-1.0), Value::Num(-1.0)]);
        assert_eq!(rt.hud_line(), Some("COG_00262"), "the line did not reach the HUD");
        // It is gone once its window closes, which is what makes it a message and not a label.
        rt.time += Runtime::HUD_MESSAGE_TIME + 0.1;
        assert!(rt.hud_line().is_none(), "the line outlived its window");
    }

    #[test]
    fn the_string_buffer_builds_a_line_in_pieces() {
        // `jkCog_jkstring`: the concat verbs append, clear empties it, output puts the line up.
        let mut rt = door_runtime();
        rt.call("jkStringClear", &[]);
        rt.call("jkStringConcatInt", &[Value::Num(7.0)]);
        rt.call("jkStringConcatSpace", &[]);
        rt.call("jkStringConcatAsciiString", &[Value::Str("secrets".into())]);
        assert_eq!(rt.string_buffer, "7 secrets", "the buffer did not build the line");
        rt.call("jkStringOutput", &[Value::Num(-1.0), Value::Num(-1.0)]);
        assert_eq!(
            rt.prints.last().map(String::as_str),
            Some("7 secrets"),
            "the line was not put up"
        );
        assert!(rt.string_buffer.is_empty(), "output must consume the buffer");
        // A `.uni` key keeps the key the script named, for the app to resolve.
        rt.call("jkStringConcatUniString", &[Value::Num(262.0)]);
        assert_eq!(rt.string_buffer, "COG_00262", "the uni key was not carried through");
        // The engine's own 128-character cap: an overlong line is dropped, not appended.
        rt.call("jkStringClear", &[]);
        rt.call("jkStringConcatAsciiString", &[Value::Str("x".repeat(200))]);
        assert!(rt.string_buffer.is_empty(), "the 128-character cap did not hold");
    }

    /// The tally screen's numbers, from the engine's own formulas and the constants read out of
    /// JK.EXE at `0x522054`–`0x522074` rather than guessed.
    #[test]
    fn the_tally_reads_rank_secrets_and_alignment_from_the_inventory() {
        let mut rt = door_runtime();
        rt.inv.insert(20, 3.0);
        assert_eq!(rt.jedi_rank(), 3, "the rank is the force-level bin");
        rt.inv.insert(70, 2.0);
        rt.inv.insert(71, 5.0);
        assert_eq!(rt.secrets(), (2, 5));

        // A dark-side power at level 2 costs 12.5; the secrets ratio (2 of 5) costs 40 and the
        // constant adds 20 back: 0 - 12.5 - 40 + 20.
        rt.inv.insert(30, 2.0);
        rt.inv.insert(72, 2.0);
        rt.inv.insert(73, 5.0);
        assert!((rt.alignment() - (-32.5)).abs() < 1e-3, "alignment {}", rt.alignment());

        // A light-side power at level 1 is worth +6.25 the other way.
        rt.inv.clear();
        rt.inv.insert(25, 1.0);
        rt.inv.insert(73, 0.0);
        assert!((rt.alignment() - 26.25).abs() < 1e-3, "alignment {}", rt.alignment());

        // The clamp is the engine's: never past 100 either way.
        rt.inv.insert(25, 40.0);
        assert!((rt.alignment() - 100.0).abs() < 1e-3, "alignment {}", rt.alignment());

        // `jkPlayer_CalcAlignment` answers the ending choice first (bin 18, `jkGetChoice`), and it
        // stores its answer in bin 75 - the value retail's level-2 autosave carries as 20.
        rt.inv.clear();
        rt.inv.insert(72, 0.0);
        rt.inv.insert(73, 0.0);
        assert!((rt.calc_alignment() - 20.0).abs() < 1e-3, "the neutral alignment is 20");
        assert_eq!(rt.inv.get(&75).copied(), Some(20.0), "the tally's alignment was not stored");
        rt.inv.insert(18, 1.0);
        assert_eq!(rt.calc_alignment(), 100.0, "the light choice is +100");
        assert_eq!(rt.inv.get(&75).copied(), Some(100.0), "the choice did not overwrite bin 75");
        rt.inv.insert(18, 2.0);
        assert_eq!(rt.calc_alignment(), -100.0, "the dark choice is -100");
    }

    /// A weapon template found by its own `inv=` rather than by a name hint: the engine types it
    /// `SITH_THING_WEAPON` and the template says which bin it belongs to, so a weapon the hint list
    /// never named is still reachable.
    #[test]
    fn a_weapon_is_found_by_its_own_inv() {
        let mut rt = door_runtime();
        // A bin the hint table does not cover at all, and a name it would never guess.
        rt.template_inv.insert("raildetonator".to_string(), 7);
        rt.template_types.insert("raildetonator".to_string(), 3);
        rt.template_damage.insert("raildetonator".to_string(), 40.0);
        assert_eq!(rt.weapon_for_bin(7).as_deref(), Some("raildetonator"));
        // A named bin still resolves through `inv=` first, so the hint list is only a fallback.
        rt.template_inv.insert("someblaster".to_string(), 2);
        rt.template_types.insert("someblaster".to_string(), 3);
        rt.template_damage.insert("someblaster".to_string(), 5.0);
        assert_eq!(rt.weapon_for_bin(2).as_deref(), Some("someblaster"));
    }

    /// `jkPlayer_NextWeapon`: `[` and `]` step through the weapons carried. Nothing called
    /// `select_bin` from input before this, so a picked-up blaster never reached the player's hands.
    #[test]
    fn weapon_cycling_walks_the_weapons_carried() {
        let mut rt = door_runtime();
        // Bins 2 and 3 are the ones `weapon_for_bin` knows (`bryar`, `stlaser`); 5 is carried by
        // nobody, so it must never come up.
        rt.inv.insert(2, 1.0);
        rt.inv.insert(3, 1.0);
        rt.inv.insert(5, 1.0);
        // The item cogs the app's loader would have recorded, so arming has somewhere to go.
        rt.bin_cog_index.insert(2, 7);
        rt.bin_cog_index.insert(3, 8);
        for name in ["bryarblaster", "stlaser"] {
            rt.template_types.insert(name.to_string(), 3);
            rt.template_damage.insert(name.to_string(), 20.0);
        }
        rt.select_bin(2);
        assert_eq!(rt.cycle_weapon(1), 3, "next went the wrong way");
        assert_eq!(rt.cur_bin, 3);
        // Arming is a message pair: the old bin's cog is told `deactivated`, the new one gets
        // `selected` and `activated` — and `activated:` is where the weapon cog calls
        // `ActivateWeapon`, so without these the trigger stays dead after a switch.
        let queued: Vec<&str> = rt
            .messages
            .iter()
            .map(|(message, _, _, _)| message.as_str())
            .collect();
        assert!(
            queued.contains(&"activated"),
            "nothing was told to arm the newly selected bin: {queued:?}"
        );
        assert!(
            queued.contains(&"selected"),
            "the mount animation was never asked for: {queued:?}"
        );
        // And the trigger works once the cog has armed it: this is what the weapon cog's own
        // `activated:` does with `ActivateWeapon`, so the trigger path is exercised end to end
        // without needing the app's message pump.
        rt.messages.clear();
        rt.weapon_cog = None;
        // `ActivateWeapon(player, rate, mode)` arms the **calling** cog, so a direct `call` has to
        // say who is calling — the VM sets this before every verb (`cog.rs` does it) and the
        // trigger message is addressed to that instance.
        rt.caller_cog = 5;
        assert!(!rt.try_fire_weapon(rt.player_thing()), "an unarmed trigger fired");
        rt.call(
            "ActivateWeapon",
            &[
                Value::Num(rt.player_thing() as f64),
                Value::Num(0.5),
                Value::Num(0.0),
            ],
        );
        // The rate is the weapon&#39;s real one now (`ActivateWeapon`'s second argument), and arming
        // waits that long before the first shot is accepted.
        rt.time += 1.0;
        assert!(rt.try_fire_weapon(rt.player_thing()), "the armed trigger refused to fire");
        // The point of switching: what fires next is the new gun's shot.
        assert_eq!(rt.current_weapon(), "stlaser", "the armed weapon did not change");
        assert_eq!(rt.cycle_weapon(1), 2, "cycling did not wrap");
        assert_eq!(rt.cycle_weapon(-1), 3, "previous did not wrap the other way");
        for _ in 0..4 {
            assert_ne!(rt.cycle_weapon(1), 5, "a bin with no weapon template was armed");
        }
    }

    /// A second borrow by the thread that already holds the world is a wiring mistake, and it used
    /// to be silent: `Mutex::lock` waits, so a gate that made the mistake looked like a slow gate.
    #[test]
    #[should_panic(expected = "already borrowed")]
    fn borrowing_a_held_world_fails_loudly() {
        let world = World::new(door_runtime());
        let _held = world.borrow();
        let _second = world.borrow_mut();
    }

    #[test]
    fn a_savegame_snapshot_round_trips_through_its_bytes() {
        // A save has to survive the trip to disk and back with every field intact, not just the
        // position: the whole point is that a player who loads gets the world they left.
        let mut rt = door_runtime();
        rt.time = 42.5;
        rt.call("SetInv", &[Value::Num(0.0), Value::Num(2.0), Value::Num(3.0)]);
        rt.call(
            "jkSetSaberInfo",
            &[
                Value::Num(0.0),
                Value::Str("side.mat".into()),
                Value::Str("tip.mat".into()),
                Value::Num(0.02),
                Value::Num(0.001),
                Value::Num(0.12),
                Value::Str("+wall".into()),
                Value::Str("+blood".into()),
                Value::Str("+spark".into()),
            ],
        );
        rt.things[0].pos = [1.5, -2.5, 3.5];
        rt.things[0].yaw = 33.0;
        rt.things[0].health = 17.0;
        rt.things[0].destroyed = true;
        rt.things[0].jk_flags = 3;
        let save = rt.snapshot();
        let bytes = save.to_bytes();
        let back = Snapshot::from_bytes(&bytes).expect("the snapshot did not parse");
        assert_eq!(back, save, "the snapshot changed on the way through its bytes");
        assert_eq!(back.things[0].pos, [1.5, -2.5, 3.5]);
        assert_eq!(back.things[0].health, 17.0);
        assert!(back.things[0].destroyed);
        assert_eq!(back.inv.get(&2).copied(), Some(3.0));
        assert_eq!(back.saber.as_ref().map(|b| b.side.as_str()), Some("side.mat"));
        // Restoring it puts the world back on a fresh runtime.
        let mut fresh = door_runtime();
        fresh.restore(&back);
        assert_eq!(fresh.time, 42.5);
        assert_eq!(fresh.things[0].pos, [1.5, -2.5, 3.5]);
        assert_eq!(fresh.things[0].jk_flags, 3);
        assert_eq!(fresh.inv.get(&2).copied(), Some(3.0));
    }

    #[test]
    fn colour_effects_combine_the_way_the_palette_effect_table_does() {
        let mut rt = door_runtime();
        assert!(rt.screen_effect().is_none(), "a fresh level has no screen effect");
        // Force seeing: red out of the filter, no tint, no add, a one-second fade **in**.
        let handle = rt.call(
            "NewColorEffect",
            &[
                Value::Num(0.0),
                Value::Num(1.0),
                Value::Num(1.0),
                Value::Num(0.0),
                Value::Num(0.0),
                Value::Num(0.0),
                Value::Num(0.0),
                Value::Num(0.0),
                Value::Num(0.0),
                Value::Num(1.0),
            ],
        );
        rt.time = 0.5;
        let (filter, tint, add, alpha) = rt.screen_effect().expect("an effect");
        assert_eq!(filter, [false, true, true], "the filter channels are wrong");
        assert_eq!(tint, [1.0; 3], "an all-zero tint must not tint");
        assert_eq!(add, [0.0; 3], "there is no additive term");
        assert!((alpha - 0.5).abs() < 1e-6, "the fade is not half in at half a second: {alpha}");
        rt.time = 2.0;
        assert_eq!(rt.screen_effect().map(|e| e.3), Some(1.0), "the fade did not finish");
        // Freeing it removes the effect entirely.
        rt.call("FreeColorEffect", &[handle]);
        assert!(rt.screen_effect().is_none(), "a freed effect is still applied");
    }

    #[test]
    fn the_player_keeps_its_inventory_and_saber_across_a_level_change() {
        // `jkPlayer_*` state outlives the level: the runtime is rebuilt from the incoming `.jkl`, so
        // `carry`/`adopt` are what put the inventory, the armed bin, the weapon and the blade back on
        // the new level's player.
        let mut leaving = door_runtime();
        leaving.call("SetInv", &[Value::Num(0.0), Value::Num(2.0), Value::Num(3.0)]);
        leaving.call("SetInv", &[Value::Num(0.0), Value::Num(14.0), Value::Num(50.0)]);
        leaving.call("SetInv", &[Value::Num(0.0), Value::Num(100.0), Value::Num(3.0)]);
        leaving.weapon = Some("+bryarbolt".to_string());
        leaving.cur_bin = 2;
        leaving.call(
            "jkSetSaberInfo",
            &[
                Value::Num(0.0),
                Value::Str("side.mat".into()),
                Value::Str("tip.mat".into()),
                Value::Num(0.003),
                Value::Num(0.001),
                Value::Num(0.1),
                Value::Str("+wall".into()),
                Value::Str("+blood".into()),
                Value::Str("+spark".into()),
            ],
        );
        let carry = leaving.carry();
        let mut arriving = door_runtime();
        assert!(arriving.inv.is_empty(), "the new level's inventory started dirty");
        arriving.adopt(&carry);
        assert_eq!(arriving.inv.get(&2).copied(), Some(3.0), "the inventory did not carry");
        assert_eq!(arriving.inv.get(&14).copied(), Some(50.0), "the force mana did not carry");
        assert_eq!(
            arriving.inv.get(&100).copied(),
            Some(0.0),
            "the previous level's goal counter carried"
        );
        assert_eq!(arriving.weapon.as_deref(), Some("+bryarbolt"), "the weapon did not carry");
        assert_eq!(arriving.cur_bin, 2, "the armed bin did not carry");
        let blade = arriving.saber.get(&arriving.player_thing()).cloned().unwrap_or_default();
        assert_eq!(blade.side, "side.mat", "the saber did not carry onto the new player");
        assert!((blade.length - 0.1).abs() < 1e-6, "the carried blade lost its length");
    }

    #[test]
    fn thing_getters_and_setters_round_trip() {
        let mut rt = door_runtime();
        let t = Value::Num(0.0);
        assert_eq!(rt.call("GetHealth", &[t.clone()]), rt.call("GetThingHealth", &[t.clone()]));
        rt.time = 12.5;
        assert_eq!(rt.call("GetLevelTime", &[]), Value::Num(12.5));
        rt.call("SetThingMass", &[t.clone(), Value::Num(3.0)]);
        assert_eq!(rt.call("GetThingMass", &[t.clone()]), Value::Num(3.0));
        rt.call("SetThingUserData", &[t.clone(), Value::Num(7.0)]);
        assert_eq!(rt.call("GetThingUserData", &[t.clone()]), Value::Num(7.0));
        // `SetLifeLeft` is the life field; `SetThingTimer` is the separate deadline that sends the
        // `timer` message and leaves the life alone (`sithCogFunctionThing_SetThingTimer`).
        rt.call("SetLifeLeft", &[t.clone(), Value::Num(4.0)]);
        assert_eq!(rt.call("GetLifeLeft", &[t.clone()]), Value::Num(4.0));
        rt.call("SetThingTimer", &[t.clone(), Value::Num(2.0)]);
        assert_eq!(rt.call("GetLifeLeft", &[t.clone()]), Value::Num(4.0));
        // The clock is at 12.5, so the deadline lands at 14.5 and the life field is untouched.
        assert_eq!(rt.thing_timer.get(&0).copied(), Some(14.5));
        assert_eq!(rt.call("IsThingVisible", &[t.clone()]), Value::Num(1.0));
        rt.call("DestroyThing", &[t.clone()]);
        assert_eq!(rt.call("IsThingVisible", &[t]), Value::Num(0.0));
    }

    #[test]
    fn handles_agrees_with_call() {
        // The router list is what the interpreter trusts; a verb it accepts must not
        // be a silent no-op on the runtime side.
        assert!(Runtime::handles("SetSectorAdjoins"));
        assert!(Runtime::handles("movetoframe"));
        assert!(Runtime::handles("CreateThing"));
        assert!(Runtime::handles("SetWallCel"));
        // `AISetMode` reaches the actor-flag arm now (`sithCogFunctionAI.c:237`), so it is not a stub.
        assert!(Runtime::handles("AISetMode"));
        let mut rt = door_runtime();
        assert_eq!(
            rt.call("SetSectorAdjoins", &[Value::Num(0.0), Value::Num(1.0)]),
            Value::Nil
        );
        // `AISetMoveSpeed` stores the AI's movespeed (`ai + 0x1bc`), clamped to the engine's 0..2,
        // on an AI-controlled thing only.
        rt.ai.insert(0, crate::ai::parse(""));
        rt.call("AISetMoveSpeed", &[Value::Num(0.0), Value::Num(5.0)]);
        assert_eq!(rt.ai_move_speed.get(&0).copied(), Some(2.0));
        // `SetFireWait` arms the player's weapon cooldown: a shot inside it is refused, and the wait
        // re-arms from the shot itself, which is what `nextFire = now + wait` means.
        rt.call("SetFireWait", &[Value::Num(0.0), Value::Num(0.5)]);
        assert_eq!(rt.next_fire, 0.5);
        assert!(rt.player_fire(0, "+bryarbolt").is_none());
        rt.time = 0.6;
        assert!(rt.player_fire(0, "+bryarbolt").is_some());
        assert_eq!(rt.next_fire, 1.1);
        // `SetGoalFlags` ORs into the goal bins (`SITHBIN_GOAL00` = 100), and `ClearGoalFlags` masks
        // back out, so what a script set is what another script reads.
        rt.call("SetGoalFlags", &[Value::Num(0.0), Value::Num(0.0), Value::Num(1.0)]);
        rt.call("SetGoalFlags", &[Value::Num(0.0), Value::Num(0.0), Value::Num(2.0)]);
        assert_eq!(rt.call("GetInv", &[Value::Num(0.0), Value::Num(100.0)]), Value::Num(3.0));
        rt.call("ClearGoalFlags", &[Value::Num(0.0), Value::Num(0.0), Value::Num(1.0)]);
        assert_eq!(rt.call("GetInv", &[Value::Num(0.0), Value::Num(100.0)]), Value::Num(2.0));
        // `SITH_AF_INVULNERABLE` (0x8, `sithActor_DamageActor`) swallows a hit but not a fall:
        // 0x40 is the one type the engine exempts from the flag.
        rt.call("SetActorFlags", &[Value::Num(0.0), Value::Num(8.0)]);
        assert_eq!(rt.damage(0, 30.0), 100.0, "an invulnerable thing took a hit");
        assert_eq!(
            rt.damage_typed(0, 30.0, DAMAGE_FALL),
            70.0,
            "the fall exception did not get through"
        );
        // `SetThingFlags` ORs and `ClearThingFlags` masks; assigning would wipe bits the level set —
        // `IN_CAMFOV` among them, which is what stops `sighted` firing twice for one thing.
        rt.things[0].flags = TF_INCAMFOV;
        rt.call("SetThingFlags", &[Value::Num(0.0), Value::Num(0.0)]);
        rt.call("SetThingFlags", &[Value::Num(0.0), Value::Num(f64::from(TF_CAPTURED))]);
        assert_eq!(
            rt.call("GetThingFlags", &[Value::Num(0.0)]).num() as u32,
            TF_INCAMFOV | TF_CAPTURED,
            "SetThingFlags replaced the word instead of adding to it"
        );
        rt.call("ClearThingFlags", &[Value::Num(0.0), Value::Num(f64::from(TF_CAPTURED))]);
        assert_eq!(
            rt.call("GetThingFlags", &[Value::Num(0.0)]).num() as u32,
            TF_INCAMFOV,
            "ClearThingFlags did not clear just that bit"
        );
        // The network verbs answer for a single-player build (`sithNet_isMulti`/`_isServer` are false),
        // and the mode flags are stored rather than dropped so a script re-reading one sees its own.
        assert_eq!(rt.call("IsMulti", &[]), Value::Num(0.0));
        assert_eq!(rt.call("IsServer", &[]), Value::Num(0.0));
        rt.call("SetMultiModeFlags", &[Value::Num(4.0)]);
        assert_eq!(rt.call("GetMultiModeFlags", &[]), Value::Num(4.0));
        rt.call("ClearMultiModeFlags", &[Value::Num(4.0)]);
        assert_eq!(rt.call("GetMultiModeFlags", &[]), Value::Num(0.0));
        // `SendTrigger` broadcasts `trigger` (id 40) to every cog when its source is the local player.
        rt.caller_cog = 3;
        rt.messages.clear();
        rt.call("SendTrigger", &vec![Value::Num(0.0); 6]);
        assert!(
            rt.messages
                .iter()
                .any(|(name, sender, cog, _)| name == "#40" && *sender == Some(3) && cog.is_none()),
            "the trigger broadcast is not on the queue: {:?}",
            rt.messages
        );
        // `PlayKey` swaps a puppet's animation for one of the script's own `keyframe` symbols — level
        // 7's `07_rahnlines.cog` startup calls `PlayKey(camera1, keyfile, 1, 2)`. `StopAnim` drops it
        // and `JumpToFrame` moves the start of the clip the script already asked for.
        assert_eq!(
            rt.call(
                "PlayKey",
                &[
                    Value::Num(0.0),
                    Value::Str("01VTieB.key".into()),
                    Value::Num(1.0),
                    Value::Num(20.0)
                ]
            ),
            Value::Num(0.0)
        );
        assert_eq!(
            rt.play_key.get(&0).cloned(),
            Some(("01vtieb.key".to_string(), 1, 20)),
            "the scripted key is not stored lowercased with its start frame"
        );
        rt.call("JumpToFrame", &[Value::Num(0.0), Value::Num(4.0)]);
        assert_eq!(rt.play_key.get(&0).map(|entry| entry.2), Some(4));
        rt.call("StopAnim", &[Value::Num(0.0)]);
        assert!(rt.play_key.is_empty(), "StopAnim left the scripted key in place");
        // `GetGoalFrame` reports a path mover's destination, and 0 for a thing that is not moving.
        assert_eq!(rt.call("GetGoalFrame", &[Value::Num(0.0)]), Value::Num(0.0));
        rt.things[0].target = Some((1.0, 8.0));
        assert_eq!(rt.call("GetGoalFrame", &[Value::Num(0.0)]), Value::Num(1.0));
        rt.things[0].target = None;
        // `jkSetFlags`/`jkClearFlags` OR and mask the *JK* flag word, which is separate from the engine's
        // thing flags above; the dark-jedi cutscene scripts set and clear 0x20 through it.
        rt.things[0].jk_flags = 0;
        rt.call("JkSetFlags", &[Value::Num(0.0), Value::Num(32.0)]);
        assert_eq!(rt.call("JkGetFlags", &[Value::Num(0.0)]), Value::Num(32.0));
        rt.call("JkClearFlags", &[Value::Num(0.0), Value::Num(32.0)]);
        assert_eq!(rt.call("JkGetFlags", &[Value::Num(0.0)]), Value::Num(0.0));
        // `SetMasterCog`/`GetMasterCog` are the engine's one global master index; unset is -1, and
        // the shipped scripts only clear it by resolving an index to no cog.
        assert_eq!(rt.call("GetMasterCog", &[]), Value::Num(-1.0));
        rt.call("SetMasterCog", &[Value::Num(4.0)]);
        assert_eq!(rt.call("GetMasterCog", &[]), Value::Num(4.0));
        rt.call("SetMasterCog", &[Value::Num(-1.0)]);
        assert_eq!(rt.call("GetMasterCog", &[]), Value::Num(-1.0));
        // `SendMessage(cog, msg)` addresses a **cog** and carries its own message number, and the
        // sender it records is the calling cog — which `sithCogFunction_SendMessage` passes on and the
        // receiver reads back with `GetSenderID`.
        rt.caller_cog = 5;
        rt.messages.clear();
        rt.call("SendMessage", &[Value::Num(3.0), Value::Num(27.0)]);
        assert_eq!(
            rt.messages,
            vec![("#27".to_string(), Some(5), Some(3), [0.0; 2])]
        );
    }

    /// `sithThing_CreateThing` places a spawned thing at `marker.position + markerOrient * (insertDiff)`.
    /// The rotation has to be the engine's own basis, not a guess: a +90 yaw sends model +X to world +Y
    /// (`rdMatrix_Build34`'s `rvec`), and model +Y to world -X (its `lvec`).
    #[test]
    fn create_thing_offsets_by_the_insert_difference_in_the_marker_frame() {
        // No rotation: the difference lands straight on Z.
        let moved = rotate_by_angles([0.0, 0.0, 0.02 - 0.12], 0.0, 0.0, 0.0);
        assert!((moved[2] + 0.1).abs() < 1e-5, "{moved:?}");
        // A +90 yaw is the engine's basis, so +X becomes +Y and +Y becomes -X.
        let x = rotate_by_angles([1.0, 0.0, 0.0], 0.0, 90.0, 0.0);
        assert!(x[0].abs() < 1e-5 && (x[1] - 1.0).abs() < 1e-5, "{x:?}");
        let y = rotate_by_angles([0.0, 1.0, 0.0], 0.0, 90.0, 0.0);
        assert!((y[0] + 1.0).abs() < 1e-5 && y[1].abs() < 1e-5, "{y:?}");
        // An unknown template has no model, so its offset is zero rather than a missing entry.
        assert_eq!(template_insert("no such template"), [0.0; 3]);
    }

    /// Health and the shield bar are part of the `jkPlayer_*` state that outlives a level, like
    /// the inventory. They used to be rebuilt at the new level's full values, so finishing a
    /// level healed the player to 100/100.
    #[test]
    fn health_and_shields_cross_a_level_change() {
        let mut old = door_runtime();
        old.things[0].health = 41.0;
        old.player_shields = 73.0;
        let carried = old.carry();
        assert_eq!(carried.health, 41.0);
        assert_eq!(carried.shields, 73.0);

        // A fresh level starts the player at its own full values (100 health, 100 shields here).
        let mut next = door_runtime();
        next.things[0].health = 100.0;
        next.player_shields = 100.0;
        next.adopt(&carried);
        assert_eq!(next.things[0].health, 41.0, "the carried health was not restored");
        assert_eq!(next.player_shields, 73.0, "the carried shields were not restored");
    }

    /// `hitandrun` (`0x50bb90`): fire at the target for the instinct's first argument, then break off
    /// for its second. Shipped line: `hitandrun 1000, 3000` — a second of fire, three seconds of run.
    #[test]
    fn an_actor_with_hitandrun_breaks_off_after_firing() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [1.5, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 HitAndRun 1000.0, 3000.0\n\
                 Follow 0.0, 0.5\n",
            ),
        );
        // `hitandrun`'s registry word is `0xc00` (any of `0x400`/`0x800` accepts); the firing
        // window is the `0x400` word alone — `0x800` is the break-off bit the handler clears.
        rt.ai_flags.insert(1, 0x400);
        // It is firing: the mode bit `0x400` is what `sithAICmd_PrimaryFire` sets on every
        // committed shot and what the handler (`0x50bb90`) counts on, held through the window.
        for _ in 0..40 {
            rt.time += 1.0 / 30.0;
            *rt.ai_flags.entry(1).or_insert(0) |= 0x400;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            rt.ai_withdraw.contains_key(&1),
            "an actor firing through its window did not break off"
        );

        // One shot and then silence is **not** a firing window: the engine's accumulator resets on
        // every call without the just-fired bit, and only the ticks that are firing add a second. The
        // old wall-clock model broke off after `hold` seconds regardless of whether the actor fired.
        let mut once = door_runtime();
        let base = once.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [1.5, 0.0, 0.0];
        once.things = vec![player, actor];
        once.templates = vec!["walkplayer".into(), "tuskan".into()];
        once.types = vec![10, 2];
        once.template_speed.insert("tuskan".into(), 1.0);
        once.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 HitAndRun 1000.0, 3000.0\n\
                 Follow 0.0, 0.5\n",
            ),
        );
        // One committed shot sets `0x400` for a single pass; `sithAICmd_PrimaryFire` clears it
        // again when its sight refresh returns 3 (`flags & !0x400`, `0x50a0f0`'s tail), so the
        // accumulator never reaches the hold.
        once.ai_flags.insert(1, 0x400);
        for i in 0..40 {
            once.time += 1.0 / 30.0;
            once.ai_tick(0, 1.0 / 30.0);
            if i == 0 {
                once.ai_flags.insert(1, 0); // the shot's bit clears with the next sight pass
            }
        }
        assert!(
            !once.ai_withdraw.contains_key(&1),
            "a single shot armed the break-off: firing time must accumulate, not wall time"
        );

        // A control class without the instinct never arms the mode, even after the same shots.
        let mut quiet = door_runtime();
        let base = quiet.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [1.5, 0.0, 0.0];
        quiet.things = vec![player, actor];
        quiet.templates = vec!["walkplayer".into(), "tuskan".into()];
        quiet.types = vec![10, 2];
        quiet.template_speed.insert("tuskan".into(), 1.0);
        quiet.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 Follow 0.0, 0.5\n",
            ),
        );
        quiet.last_shot.insert(1, 0.0);
        for _ in 0..40 {
            quiet.time += 1.0 / 30.0;
            quiet.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            !quiet.ai_withdraw.contains_key(&1),
            "an actor without `HitAndRun` broke off"
        );
    }

    /// `sithAICmd_HitAndRun` (`0x50bb90`) from the decompiled arithmetic: each pass with the
    /// just-fired bit `0x400` adds 1000 ms to the accumulator (`acc -= _DAT_005232f4` with the
    /// constant **−1000.0**), the break-off lands on `fltArg[0] < acc` — so a 1000 ms hold breaks
    /// on the **second** counted pass — and the pass after the break clears `0x800` (the
    /// re-engage) once the handler's `now + fltArg[1]` re-arm comes due.
    #[test]
    fn hitandrun_counts_firing_seconds_then_re_engages() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [1.5, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 HitAndRun 1000.0, 3000.0\n",
            ),
        );
        rt.ai_flags.insert(1, 0x400);

        // Pass 1 (t = 1, the scheduler's first due clock): acc = 1000, and 1000 < 1000 is false.
        rt.time = 1.0;
        *rt.ai_flags.entry(1).or_insert(0) |= 0x400;
        rt.ai_tick(0, 1.0 / 30.0);
        assert!(
            !rt.ai_withdraw.contains_key(&1),
            "the first counted pass broke off early: hold 1000 must not be under acc 1000"
        );

        // Pass 2 (t = 2): acc = 2000 > 1000 — break off, `0x800` set, withdraw until t = 2 + 3.
        rt.time = 2.0;
        *rt.ai_flags.entry(1).or_insert(0) |= 0x400;
        rt.ai_tick(0, 1.0 / 30.0);
        assert!(rt.ai_withdraw.contains_key(&1), "the second counted pass did not break off");
        assert_ne!(
            rt.ai_flags.get(&1).copied().unwrap_or(0) & 0x800,
            0,
            "the break-off word does not carry 0x800"
        );

        // The pass after the re-arm (due at t = 5): `0x800` clears — the re-engage.
        rt.time = 5.5;
        rt.ai_tick(0, 1.0 / 30.0);
        assert_eq!(
            rt.ai_flags.get(&1).copied().unwrap_or(0) & 0x800,
            0,
            "the handler did not clear its break-off bit on the re-engage pass"
        );
    }

    /// `randomturn` (`0x50b700`): an idle actor picks a random direction at its class interval and
    /// turns to look at a point its argument's distance away. Shipped: `randomturn 1000, 0.5`.
    #[test]
    fn an_idle_actor_with_randomturn_looks_around() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [-9.0, 0.0, 0.0]; // out of sight and out of mind
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nRandomTurn 1000.0, 0.5\n",
            ),
        );

        // `randomturn`'s registry word is 4 (awake).
        rt.ai_flags.insert(1, 0x4);
        for _ in 0..40 {
            rt.time += 1.0 / 30.0;
            ai_frame(&mut rt, 0, 1.0 / 30.0);
        }
        let chosen = rt.ai_randomturn.get(&1).copied().expect("no randomturn state");
        assert!(
            chosen.abs() > 1e-3,
            "the actor never picked a direction: {chosen:?}"
        );
        assert!(
            rt.things[1].yaw.abs() > 1e-3,
            "the actor did not turn: yaw {:.2}",
            rt.things[1].yaw
        );

        // A control class without the instinct keeps looking straight ahead.
        let mut quiet = door_runtime();
        let base = quiet.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [-9.0, 0.0, 0.0];
        quiet.things = vec![player, actor];
        quiet.templates = vec!["walkplayer".into(), "tuskan".into()];
        quiet.types = vec![10, 2];
        quiet.template_speed.insert("tuskan".into(), 1.0);
        quiet.ai.insert(
            1,
            crate::ai::parse("alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nJump\n"),
        );
        for _ in 0..40 {
            quiet.time += 1.0 / 30.0;
            ai_frame(&mut quiet, 0, 1.0 / 30.0);
        }
        assert_eq!(quiet.things[1].yaw, 0.0, "an actor without `randomturn` turned");
    }

    /// `roam` (`0x50b830`): an awake, not-searching actor turns its look vector by a random yaw
    /// and orders a move (`SetMoveThing(1.0)`) to a point up to `fltArg[1]` from its **home**,
    /// re-armed at `intArg[0]`. The body then walks there through its velocity.
    #[test]
    fn an_idle_actor_with_roam_orders_a_walk_around_its_home() {
        let roamer = |class: &str| {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut actor = base.clone();
            actor.pos = [0.0, 0.0, 0.0];
            let mut player = base.clone();
            player.pos = [-9.0, 0.0, 0.0]; // out of sight and out of mind
            rt.things = vec![player, actor];
            rt.templates = vec!["walkplayer".into(), "tuskan".into()];
            rt.types = vec![10, 2];
            rt.template_maxthrust.insert("tuskan".into(), 1.0);
            rt.ai.insert(1, crate::ai::parse(class));
            rt.ai_home.insert(1, [0.5, 0.0, 0.0]);
            // `roam`'s registry word is 4 (awake); its handler also refuses while searching (`0x2`).
            rt.ai_flags.insert(1, 0x4);
            rt
        };
        let mut rt = roamer("alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nRoam 5000.0, 2.0\n");
        rt.ai_tick(0, 1.0 / 30.0);
        let point = rt.ai_move.get(&1).copied().expect("roam ordered no move");
        let from_home = ((point[0] - 0.5).powi(2) + point[1].powi(2)).sqrt();
        assert!(from_home <= 2.0, "the roam point left the radius around home: {point:?}");
        assert_eq!(rt.ai_move_speed.get(&1), Some(&1.0), "roam's SetMoveThing speed is 1.0");
        assert_ne!(rt.ai_flags[&1] & 0x1, 0, "the roam order did not set the moving bit");
        for _ in 0..30 {
            ai_frame(&mut rt, 0, 1.0 / 30.0);
        }
        assert_ne!(rt.things[1].pos, [0.0, 0.0, 0.0], "the actor did not walk its roam order");

        // A control class without `Roam` orders nothing and stands.
        let mut quiet = roamer("alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nJump\n");
        for _ in 0..30 {
            ai_frame(&mut quiet, 0, 1.0 / 30.0);
        }
        assert!(quiet.ai_move.get(&1).is_none(), "an actor without `Roam` got a move order");
        assert_eq!(quiet.things[1].pos, [0.0, 0.0, 0.0], "an actor without `Roam` wandered");
    }

    /// `AISetMode`/`AIClearMode` (`0x500f50`) OR the argument into the AI's own flag word (`ai + 8`),
    /// which every instinct handler tests, so those script bits are behaviour. `0x2000` is the
    /// switch-off the shipped sleep/trap scripts use: an actor carrying it must do nothing until a
    /// script clears it.
    #[test]
    fn a_switched_off_actor_does_nothing_until_its_mode_is_cleared() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [1.0, 0.0, 0.0]; // right in front, well inside sight
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.template_weapon.insert("tuskan".into(), "blaster".into());
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 PrimaryFire 100.0, 0.0, 5.0, 0.0, 0.0, 500.0, 0.0, 0.0, 1.0, 0.0\n\
                 Follow 0.0, 0.5\n",
            ),
        );
        // `aisetmode(actor, 0x2000)`: asleep.
        rt.ai_flags.insert(1, 0x2000);
        let asleep_at = rt.things[1].pos;
        for _ in 0..30 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert_eq!(rt.shots_fired, 0, "a switched-off actor fired");
        assert_eq!(rt.things[1].pos, asleep_at, "a switched-off actor walked");

        // `aiclearmode(0x2000)`: back to normal.
        rt.ai_flags.insert(1, 0);
        for _ in 0..30 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            rt.shots_fired > 0,
            "the actor did not come back after its mode was cleared"
        );
    }

    /// `aisetmode(actor, 0x200)` is the shipped way to place an actor "engaged" (839 call sites in the
    /// 215 cogs: 209 `0x200`, 630 `0x202`). The bit is the same word `sithAI_CheckSightThing` writes,
    /// so it must latch the target — otherwise a script-started enemy stands until it rolls sight.
    #[test]
    fn a_script_engaging_an_actor_latches_the_target() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [1.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.template_weapon.insert("tuskan".into(), "blaster".into());
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 PrimaryFire 100.0, 0.0, 5.0, 0.0, 0.0, 500.0, 0.0, 0.0, 1.0, 0.0\n\
                 Follow 0.0, 0.5\n",
            ),
        );
        assert!(!rt.ai_detected.contains(&1), "the actor was engaged before the script said so");
        rt.call("aisetmode", &[Value::Num(1.0), Value::Num(0x200 as f64)]);
        assert_eq!(rt.ai_flags.get(&1).copied(), Some(0x200));
        assert!(rt.ai_detected.contains(&1), "aisetmode(0x200) did not engage the actor");
        for _ in 0..30 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            rt.shots_fired > 0,
            "a script-engaged actor did not fire at a target in sight"
        );
    }

    /// `AIFlee` (`0x501420`) sets the fleeing mode, and the class's `Flee` instinct ends it after its
    /// third argument's seconds (shipped `1.0, 5000, 2.0`): a scripted flee is a burst, not forever.
    #[test]
    fn a_scripted_flee_expires_after_the_instincts_duration() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        let mut player = base.clone();
        player.pos = [1.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\nFlee 1.0, 5000.0, 2.0\n",
            ),
        );
        rt.call("AIFlee", &[crate::cog::Value::Num(1.0), crate::cog::Value::Num(0.0)]);
        assert!(rt.ai_flee.contains_key(&1), "AIFlee did not start the flee");

        for _ in 0..30 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            rt.ai_flee.contains_key(&1),
            "the flee ended before the instinct's duration"
        );
        for _ in 0..40 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            !rt.ai_flee.contains_key(&1),
            "the scripted flee never expired"
        );
    }

    /// The awareness record is keyed by **event kind**, not by alignment: `CreateTransmittingEvent`'s
    /// third argument is a 0..2 slot (`0x4f2c30`), and every call site passes 0, 1 or 2 — the engine
    /// has no notion of a light-side shot. The port keyed the record by the shooter's alignment, so a
    /// shot could never reach an actor of the other side (and a shot by the player, who carries no
    /// `.ai` class at all, raised no event: enemies never heard gunfire).
    #[test]
    fn a_shot_raises_an_awareness_record_across_alignments_with_its_source() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut player = base.clone();
        player.pos = [-9.0, 0.0, 0.0]; // out of sight: the shot, not the body, is what the actor hears
        let mut listener = base.clone();
        listener.pos = [0.0, 0.0, 0.0]; // sector 7, alignment 1.0 (light)
        let mut shooter = base.clone();
        shooter.pos = [-1.2, 0.0, 0.0]; // the same room, behind the listener (out of its 90-degree view)
        rt.things = vec![player, listener, shooter];
        rt.templates = vec!["walkplayer".into(), "tuskan".into(), "door".into()];
        rt.types = vec![10, 2, 0];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.sector_adjoins = vec![Vec::new(); 8];
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=1.0, rank=0.5, fov=90, sightdist=2.0, heardist=4.0\n\nListen 1.0\nFollow 0.5, 0.1\n",
            ),
        );
        rt.transmit_awareness(7, 1, [-1.2, 0.0, 0.0], super::AI_AWARE_SHOT, Some(2));
        let (_, _, _, source) = rt.sector_awareness.get(&(7, 1)).copied().expect("no record");
        assert_eq!(source, Some(2), "the event did not keep its source thing");
    }

    /// `sithWeapon_WeaponFireProjectile` (`0x4d45a0`) plays the sound the caller named through
    /// `sithSoundMixer_PlaySoundThing(sound, shooter, 1.0, …)`; the AI's own shot names none, so the
    /// shooter's class `fire1` row is what a player hears (`gr.snd`: `pistol-1.wav`).
    #[test]
    fn a_shot_queues_the_shooters_fire_sound() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut gunner = base.clone();
        gunner.pos = [3.0, 4.0, 5.0];
        gunner.sector = 7;
        rt.things = vec![base, gunner];
        rt.templates = vec!["walkplayer".into(), "greedo".into()];
        rt.types = vec![10, 2];
        rt.template_soundclass.insert("greedo".into(), "gr.snd".into());
        rt.sound_classes.insert(
            "gr.snd".into(),
            std::collections::BTreeMap::from([(
                "fire1".to_string(),
                vec![("pistol-1.wav".to_string(), 1.0)],
            )]),
        );
        rt.template_vel.insert("+ebolt".into(), [0.0, 6.0, 0.0]);
        rt.template_damage.insert("+ebolt".into(), 12.0);
        assert!(rt.fire_spread(1, "+ebolt", 0.0).is_some(), "the shot spawned");
        assert_eq!(rt.sounds.len(), 1, "the shot made no sound");
        assert_eq!(rt.sounds[0].name, "pistol-1.wav");
        assert_eq!(rt.sounds[0].pos, Some([3.0, 4.0, 5.0]), "the sound left the shooter");
        assert_eq!(rt.sounds[0].volume, 1.0);
        // The script path names its own wav (`weap_bryar.cog`'s `fireSound`).
        rt.sounds.clear();
        rt.call(
            "FireProjectile",
            &[
                super::Value::Num(1.0),
                super::Value::Str("+ebolt".into()),
                super::Value::Str("laser03.wav".into()),
            ],
        );
        assert_eq!(rt.sounds.len(), 1, "the script's named shot sound was dropped");
        assert_eq!(rt.sounds[0].name, "laser03.wav");
    }

    /// `FUN_0050aa80`'s find is a **broadcast**: the branch clears the actor's awake bit, sets the
    /// engaged/searching word and raises a kind-0 event of strength 3.0 in the *player's* sector at the
    /// spotter's position, sourced to the player. The port set the awake bit instead and told nobody.
    #[test]
    fn a_spotter_broadcasts_the_find_and_leaves_the_awake_state() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0]; // sector 7, facing +x (the engine's yaw -90)
        actor.yaw = -90.0;
        let mut player = base.clone();
        player.pos = [1.0, 0.0, 0.0]; // one unit ahead: inside the 2.0 always-noticed radius
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse("alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nLookForTarget 1000\n"),
        );
        // Awake, the way `AISetMode` leaves an actor before it scans.
        rt.ai_flags.insert(1, 0x4);
        rt.sector_adjoins = vec![Vec::new(); 8];
        for _ in 0..10 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        let flags = rt.ai_flags.get(&1).copied().unwrap_or(0);
        assert_eq!(flags & 0x4, 0, "the find kept the awake bit: flags {flags:#x}");
        assert_eq!(flags & 0x200, 0x200, "the find did not set the searching bit: {flags:#x}");
        let sector = rt.things[0].sector;
        let (strength, pos, _, source) = rt
            .sector_awareness
            .get(&(sector, 0))
            .copied()
            .expect("the find raised no awareness event");
        assert_eq!((strength, source), (super::AI_AWARE_SIGHT, Some(0)));
        assert_eq!(pos, [0.0, 0.0, 0.0], "the event belongs at the spotter's position");
        assert!(
            rt.awareness_at(sector).is_some(),
            "a listener in the player's sector reads no record"
        );
    }

    /// `LobFire`'s **live** path (`0x509cd0`): its mask is `0x100` and that branch does not fire — it
    /// sets `sithPuppet_SetArmedMode(thing, (ai flags & 0x200) != 0)` and re-arms in 1000 ms. `0x200` is
    /// the awake bit `00_aisleep.cog` writes, so a wakeable ryther draws its weapon when it wakes and
    /// holsters when it sleeps. The scheduler branch (event 0) that fires the lobbed shot is
    /// covered by `lobfire_fires_the_lob_and_arms_its_lockout`.
    #[test]
    fn lobfire_follows_the_awake_bit_instead_of_lobbing_a_shot() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        let mut player = base.clone();
        player.pos = [1.5, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "ryther".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("ryther".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\nLobFire 2000.0, 0.8, 0.5, 4.0, 0.20, 0\n",
            ),
        );
        rt.ai_flags.insert(1, 0x200);
        // The handler's mask is `0x100`: the armed mode is written from the mode-change event
        // (`sithAI_EmitEvent`'s 0x100 dispatch), not from a clock. A plain tick changes nothing.
        rt.ai_tick(0, 1.0 / 30.0);
        assert_eq!(
            rt.armed_mode.get(&1),
            None,
            "a plain tick armed the weapon without a mode-change event"
        );
        let _ = rt.emit_ai_event(1, 0x100, None);
        assert_eq!(
            rt.armed_mode.get(&1).copied(),
            Some(1),
            "an awake actor with `LobFire` did not draw its weapon on the 0x100 event"
        );
        // The same branch holsters when the awake bit is clear.
        rt.ai_flags.insert(1, 0);
        let _ = rt.emit_ai_event(1, 0x100, None);
        assert_eq!(
            rt.armed_mode.get(&1).copied(),
            Some(0),
            "a sleeping actor with `LobFire` kept its weapon drawn"
        );
        assert_eq!(rt.shots_fired, 0, "the dead `sithAI_FireWeapon` branch lobbed a shot");
    }

    /// `FUN_00509cd0`'s scheduler branch, straight from the decompile: with the player in sight and
    /// inside the class's band, `sithAI_FireWeapon` runs with `fl = 2` (the lob) and both the fire
    /// lockout `ai + 0xa2` and the handler's re-arm land at `now + arg0` (2000 ms here); a player
    /// outside the band refuses the shot, sets a look frame and retries in 500 ms; a `0x202`-flagged
    /// player stops the fight with `(flags & 0xfffff9dd) | 4`.
    #[test]
    fn lobfire_fires_the_lob_and_arms_its_lockout() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut gunner = base.clone();
        gunner.pos = [0.0, 0.0, 0.0];
        gunner.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [0.0, 1.0, 0.0];
        player.flags = 0x4; // plainly visible: the detect roll is certain
        rt.things = vec![player, gunner];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_weapon.insert("tuskan".into(), "+elaser".into());
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 LobFire 2000.0, 0.8, 0.5, 4.0, 0.20, 0\n",
            ),
        );
        rt.ai_flags.insert(1, 0x2); // the registry word: searching
        rt.ai_detected.insert(1);
        rt.time = 1.0;
        rt.ai_tick(0, 1.0 / 30.0);
        assert!(rt.shots_fired >= 1, "the scheduler branch did not lob its shot");
        assert_eq!(
            rt.ai_fire_lock.get(&1).copied(),
            Some(3.0),
            "the fire lockout `ai + 0xa2` is not armed at now + arg0"
        );
        assert_eq!(
            rt.ai_inst.get(&1).and_then(|s| s.first().copied()),
            Some(3.0),
            "the handler's own re-arm is not now + arg0"
        );
        assert_ne!(
            rt.ai_flags.get(&1).copied().unwrap_or(0) & 0x400,
            0,
            "the fired bit 0x400 was not set"
        );

        // Out of the band (6 units against maxD 4): the refusal path — a look frame, `0x400`,
        // re-arm in 500 ms, no shot.
        let mut far = door_runtime();
        let base = far.things[0].clone();
        let mut gunner = base.clone();
        gunner.pos = [0.0, 0.0, 0.0];
        gunner.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [0.0, 6.0, 0.0];
        player.flags = 0x4;
        far.things = vec![player, gunner];
        far.templates = vec!["walkplayer".into(), "tuskan".into()];
        far.types = vec![10, 2];
        far.template_weapon.insert("tuskan".into(), "+elaser".into());
        far.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=8.0\n\n\
                 LobFire 2000.0, 0.8, 0.5, 4.0, 0.20, 0\n",
            ),
        );
        far.ai_flags.insert(1, 0x2);
        far.ai_detected.insert(1);
        far.time = 1.0;
        far.ai_tick(0, 1.0 / 30.0);
        assert_eq!(far.shots_fired, 0, "a refusal still lobbed a shot");
        assert_eq!(
            far.ai_inst.get(&1).and_then(|s| s.first().copied()),
            Some(1.5),
            "a refused lob does not retry in 500 ms"
        );
        assert_ne!(far.ai_flags.get(&1).copied().unwrap_or(0) & 0x400, 0);

        // A `0x202`-flagged player: the stop branch writes `(flags & 0xfffff9dd) | 4`.
        rt.things[0].flags |= 0x202;
        rt.ai_inst.get_mut(&1).unwrap()[0] = 0.0; // due now
        rt.ai_next_due.remove(&1);
        rt.ai_flags.insert(1, 0x406);
        rt.ai_tick(0, 1.0 / 30.0);
        assert_eq!(
            rt.ai_flags.get(&1).copied(),
            Some(0x4),
            "a gone player did not stop the fight with (flags & 0xfffff9dd) | 4"
        );
    }

    /// A **projectile** that reaches a level surface dies there, the way `sithThing_Update` explodes
    /// a weapon on contact: a bolt stopped with its `vel` zeroed but its damage still live froze
    /// mid-air for the rest of its `timer=` life and could never hit anything again (level 1's door
    /// fight: every AI bolt ended up parked in a doorway).
    #[test]
    fn a_projectile_dies_on_the_surface_it_hits() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        // One room, one wall at x = 1.2 (a plane the swept resolver pushes out of).
        let plane = |normal: [f32; 3], point: [f32; 3], surface: usize| {
            crate::collide::Plane { normal, point, surface }
        };
        let planes = vec![
            plane([-1.0, 0.0, 0.0], [1.2, 0.0, 0.0], 41),
            plane([1.0, 0.0, 0.0], [-5.0, 0.0, 0.0], 41),
            plane([0.0, -1.0, 0.0], [0.0, -2.0, 0.0], 41),
            plane([0.0, 1.0, 0.0], [0.0, 2.0, 0.0], 41),
            plane([0.0, 0.0, -1.0], [0.0, 0.0, -2.0], 41),
            plane([0.0, 0.0, 1.0], [0.0, 0.0, 2.0], 41),
        ];
        rt.solids = Some(std::sync::Arc::new(crate::collide::Solids {
            sectors: vec![planes.clone()],
            hull: vec![planes],
            boxes: vec![None],
            portals: vec![Vec::new()],
            spheres: vec![([0.0, 0.0, 0.0], 100.0)],
            things: Default::default(),
            floors: Vec::new(),
            walls: Vec::new(),
            wall_of: Default::default(),
            floor_of: Default::default(),
            faces: Vec::new(),
            face_of: Default::default(),
            portal_faces: Vec::new(),
            surface_owner: vec![0, 0, 0, 0],
            adjoin_flags: Vec::new(),
                ..Default::default()
        }));
        let mut bolt = base.clone();
        bolt.pos = [0.0, 0.0, 0.0];
        bolt.sector = 0;
        bolt.vel = [6.0, 0.0, 0.0];
        bolt.damage = 12.0;
        rt.things = vec![base, bolt];
        rt.templates = vec!["walkplayer".into(), "+elaser".into()];
        rt.types = vec![10, 3];
        for _ in 0..60 {
            rt.step(None, 1.0 / 30.0);
            if rt.things[1].destroyed {
                break;
            }
        }
        assert!(rt.things[1].destroyed, "the bolt froze on the wall: {:?}", rt.things[1].pos);
        assert!(rt.things[1].pos[0] < 1.2, "the bolt was left past the wall: {:?}", rt.things[1].pos);
    }

    /// class line's own 2000 ms clock; a standing actor is refused by the AI moving bit `0x1`.
    #[test]
    fn a_blocked_actor_nudges_toward_its_target_with_jump() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 180.0;
        let mut player = base.clone();
        // The target stands clear of the leap point: a body may not step into another body's
        // cylinder, so a target *on* the leap point would be refused before the jump is read.
        player.pos = [6.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\nJump 2000.0, 3.0, 3.0\n",
            ),
        );
        rt.ai_flags.insert(1, 0x1);
        rt.ai_move.insert(1, [3.0, 0.0, 0.0]);
        assert!(
            rt.emit_ai_event(1, 0x400, None),
            "a moving blocked actor did not take the jump event"
        );
        assert!(
            (rt.things[1].vel[0] - 0.1).abs() < 1e-5,
            "the nudge was {:?}",
            rt.things[1].vel
        );
        // `sithAI_Jump` (`0x4eacf0`): the move runs at speed 2.0, the puppet play mode is the jump
        // `0x1b`, the `jumpspeed` (1.5) hop is added straight up, and the 2000 ms lockout arms. The
        // handler never sets a look frame, so the body's yaw is untouched.
        assert_eq!(rt.ai_move_speed.get(&1), Some(&2.0));
        assert_eq!(rt.play_mode.get(&1), Some(&0x1b));
        assert!((rt.things[1].vel[2] - 1.5).abs() < 1e-5, "vel {:?}", rt.things[1].vel);
        assert_eq!(rt.ai_move_lock.get(&1), Some(&2.0));
        // The handler's own 2000 ms clock: an immediate second event does nothing.
        assert!(
            !rt.emit_ai_event(1, 0x400, None),
            "the jump ignored its class interval"
        );
        // A standing actor is refused by the engine's moving bit.
        rt.ai_jump.clear();
        rt.ai_flags.insert(1, 0);
        assert!(
            !rt.emit_ai_event(1, 0x400, None),
            "a standing actor took the jump event"
        );
    }

    /// The leap point is tested with the engine's walkability helper (`FUN_004eb4b0`), not with a
    /// clear line: a point three units ahead over a void is refused even though nothing blocks the
    /// segment, and the same event lands the nudge when there is ground under it.
    #[test]
    fn a_blocked_actor_does_not_leap_onto_a_void() {
        fn jump_rt(floor_to_x: f32) -> Runtime {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut actor = base.clone();
            actor.pos = [0.0, 0.0, 0.065];
            actor.sector = 0;
            actor.yaw = 180.0;
            let mut player = base.clone();
            // Away from the leap point: a body standing *on* it would count as a solid thing's top
            // (the collider's `ThingSolid` list), which is the port's stand-in for the engine's
            // walkable-thing bit.
            player.pos = [0.0, 5.0, 0.065];
            rt.things = vec![player, actor];
            rt.templates = vec!["walkplayer".into(), "tuskan".into()];
            rt.types = vec![10, 2];
            rt.template_speed.insert("tuskan".into(), 1.0);
            rt.ai.insert(
                1,
                crate::ai::parse(
                    "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\nJump 2000.0, 3.0, 3.0\n",
                ),
            );
            rt.ai_flags.insert(1, 0x1);
            rt.ai_move.insert(1, [3.0, 0.0, 0.065]);
            let floor = crate::collide::Face {
                normal: [0.0, 0.0, -1.0],
                point: [0.0, 0.0, 0.0],
                verts: vec![
                    [-2.0, -2.0, 0.0],
                    [floor_to_x, -2.0, 0.0],
                    [floor_to_x, 2.0, 0.0],
                    [-2.0, 2.0, 0.0],
                ],
                surface: 1,
            };
            let plane = |normal: [f32; 3], point: [f32; 3]| crate::collide::Plane {
                normal,
                point,
                surface: 1,
            };
            let hull = vec![
                plane([0.0, 0.0, -1.0], [0.0, 0.0, 0.0]),
                plane([0.0, 0.0, 1.0], [0.0, 0.0, 2.0]),
                plane([-1.0, 0.0, 0.0], [-2.0, 0.0, 0.0]),
                plane([1.0, 0.0, 0.0], [6.0, 0.0, 0.0]),
                plane([0.0, -1.0, 0.0], [0.0, -2.0, 0.0]),
                plane([0.0, 1.0, 0.0], [0.0, 2.0, 0.0]),
            ];
            rt.solids = Some(std::sync::Arc::new(crate::collide::Solids {
                sectors: vec![hull.clone()],
                hull: vec![hull],
                boxes: vec![None],
                portals: vec![Vec::new()],
                spheres: vec![([0.0, 0.0, 0.0], 100.0)],
                things: Default::default(),
                floors: vec![floor.clone()],
                walls: Vec::new(),
                wall_of: Default::default(),
                floor_of: [(0usize, 0usize)].into_iter().collect(),
                faces: vec![floor],
                face_of: [(1usize, 0usize)].into_iter().collect(),
                portal_faces: Vec::new(),
                surface_owner: vec![0, 0],
                adjoin_flags: Vec::new(),
                ..Default::default()
            }));
            rt
        }

        let mut void = jump_rt(1.0);
        assert!(
            !void.emit_ai_event(1, 0x400, None),
            "the actor leapt onto a void three units ahead"
        );
        assert_eq!(void.things[1].vel, [0.0; 3], "the refused leap moved the body");

        let mut ground = jump_rt(4.0);
        assert!(
            ground.emit_ai_event(1, 0x400, None),
            "the actor refused a leap with ground under the point"
        );
        assert!(
            (ground.things[1].vel[0] - 0.1).abs() < 1e-5,
            "the leap's nudge was {:?}",
            ground.things[1].vel
        );
    }

    /// `talk`'s mood table (`0x50be20`): the AI flag word's `0x200` is the gate, `0x800` and `0x400`
    /// pick the family, and the health fraction picks the word inside it. The handler's health gates are
    /// 0.5 and 0.25 (`_DAT_005232f8`, read from the retail EXE's `.rdata`), and `AISetMode` is what the
    /// shipped scripts set (`0x200` in `00_aisleep.cog`, `0x202` in `00_aishootbarrel.cog`).
    #[test]
    fn talk_picks_its_mood_from_the_ai_flags_and_health() {
        // Awake and neither mood flag: happy from half health up, searching below it.
        assert_eq!(talk_mode(0x200, 1.0), Some("happy"));
        assert_eq!(talk_mode(0x200, 0.5), Some("happy"));
        assert_eq!(talk_mode(0x200, 0.49), Some("search"));
        // `0x202` — the scripts' "enemy" bit on top — is still the plain family.
        assert_eq!(talk_mode(0x202, 0.6), Some("happy"));
        // 0x400: boast down to a quarter health, fear under it.
        assert_eq!(talk_mode(0x200 | 0x400, 0.25), Some("boast"));
        assert_eq!(talk_mode(0x200 | 0x400, 0.24), Some("fear"));
        // 0x800 wins over 0x400 and wants strictly more than half to gloat.
        assert_eq!(talk_mode(0x200 | 0x400 | 0x800, 0.51), Some("gloat"));
        assert_eq!(talk_mode(0x200 | 0x400 | 0x800, 0.5), Some("fear"));
        // Not awake: silent, whatever else is set (`AISetMode(enemy, 0x2000)` shuts an actor down).
        assert_eq!(talk_mode(0x0, 1.0), None);
        assert_eq!(talk_mode(0x1000 | 0x4, 1.0), None);
    }

    /// The handler's clock and roll are wired into `ai_tick`: `talk`'s first number is its interval in
    /// milliseconds and its second the chance (most classes ship `10000.0, 0.10`, `boc` ships
    /// `3000.0, 0.40`). A success queues the mood sound at the actor's own position, from the actor's own
    /// `.snd` class; a chance of zero never does.
    #[test]
    fn a_talking_actor_queues_its_mood_sound_on_the_instincts_clock() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [2.0, 0.0, 0.0];
        let mut player = base.clone();
        player.pos = [-9.0, 0.0, 0.0]; // out of sight, nothing remembered
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "barkeep".into()];
        rt.types = vec![10, 2];
        rt.template_max_health.insert("barkeep".into(), 100.0);
        rt.things[1].health = 100.0;
        rt.template_soundclass.insert("barkeep".into(), "pednarsh.snd".into());
        rt.sound_classes.insert(
            "pednarsh.snd".into(),
            std::collections::BTreeMap::from([(
                "happy".to_string(),
                vec![("happy1.wav".to_string(), 0.8)],
            )]),
        );
        rt.ai_flags.insert(1, 0x200);
        let class = |chance: &str| {
            crate::ai::parse(&format!(
                "alignment=-1.0, rank=0.5, fov=90, sightdist=2.0, heardist=2.0\n\n\
                 LookForTarget 500.0, 40000.0\nTalk 10000.0, {chance}\n"
            ))
        };
        rt.ai.insert(1, class("1.0"));
        rt.time = 0.0;
        rt.ai_tick(0, 1.0 / 30.0);
        assert_eq!(rt.sounds.len(), 1, "the talking actor queued no sound");
        assert_eq!(rt.sounds[0].name, "happy1.wav");
        assert_eq!(rt.sounds[0].pos, Some([2.0, 0.0, 0.0]));
        // The handler sets its own next run from the instinct's interval: nothing inside it.
        for _ in 0..30 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert_eq!(rt.sounds.len(), 1, "the actor talked again inside its interval");
        // A zero chance never queues anything, even after the interval passes.
        rt.sounds.clear();
        rt.ai.insert(1, class("0.0"));
        for _ in 0..600 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(rt.sounds.is_empty(), "a 0.0 chance still talked");
    }

    /// `sensedanger` (`0x50b9b0`, 14 classes): an **awake** actor (`AISetMode(..., 0x4)`, which the
    /// shipped scripts set on the droids and remotes) whose class enables the instinct (`sensedanger
    /// 0.10, 1.00`; most lines ship `0.10, 0.00` and never act) looks the player over once a second
    /// while it is not yet alerted (`0x800`). Seeing them clears the awake bit, sets `0x800`, plays the
    /// class's `fear` sound, raises a strength-**3.0** awareness event at its own position (so its side
    /// converges — the handler calls `sithAIAwareness_CreateTransmittingEvent` with `0x40400000`) and
    /// takes the player as its target.
    #[test]
    fn an_awake_sensedanger_actor_alerts_when_it_sees_the_player() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [1.0, 0.0, 0.0]; // sector 7, alignment -1, facing +x (engine yaw -90)
        actor.yaw = -90.0;
        let mut player = base.clone();
        player.pos = [2.0, 0.0, 0.0]; // one unit ahead: inside sightdist 2.0 and the 90-degree cone
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "mousebot".into()];
        rt.types = vec![10, 2];
        rt.template_max_health.insert("mousebot".into(), 50.0);
        rt.things[1].health = 50.0;
        rt.template_soundclass.insert("mousebot".into(), "mouse.snd".into());
        rt.sound_classes.insert(
            "mouse.snd".into(),
            std::collections::BTreeMap::from([(
                "fear".to_string(),
                vec![("fear1.wav".to_string(), 1.0)],
            )]),
        );
        rt.ai_flags.insert(1, 0x4);
        // `transmit_awareness` needs a sector-adjoin table long enough to hold the actor's sector;
        // `door_runtime`'s default level has no adjoins, so give it empty lists.
        rt.sector_adjoins = vec![Vec::new(); 8];
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=90, sightdist=2.0, heardist=2.0\n\n\
                 SenseDanger 0.10, 1.00\n",
            ),
        );
        rt.ai_tick(0, 1.0 / 30.0);
        let flags = rt.ai_flags.get(&1).copied().unwrap_or(0);
        assert_ne!(flags & 0x800, 0, "the sighted awake actor did not alert");
        assert_eq!(flags & 0x4, 0, "the handler clears the awake bit when it alerts");
        assert_eq!(rt.sounds.len(), 1, "the alert played no fear sound");
        assert_eq!(rt.sounds[0].name, "fear1.wav");
        assert_eq!(rt.sounds[0].pos, Some([1.0, 0.0, 0.0]));
        assert_eq!(
            rt.ai_last_seen.get(&1).copied(),
            Some([2.0, 0.0, 0.0]),
            "the alert did not take the player as its target"
        );
        assert_eq!(
            rt.awareness_at(7),
            Some([1.0, 0.0, 0.0]),
            "the alert raised no awareness event"
        );
        let record = rt.sector_awareness.get(&(7, 1)).expect("no kind-1 record");
        assert_eq!(
            record.0, super::AI_AWARE_ALERT,
            "the awareness event is not the handler's 3.0 strength"
        );
    }

    /// Both of `sensedanger`'s gates, from the handler: the AI's **awake** bit (scripts set it on a
    /// subset of actors with `AISetMode(..., 0x4)`) and the class's own second number (`0.00` disables
    /// the instinct in most shipped classes). Neither alone alerts, and an already-alerted actor does
    /// nothing.
    #[test]
    fn sensedanger_needs_the_awake_bit_and_an_enabled_class() {
        let sample = |flags: u32, args: &str| {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut actor = base.clone();
            actor.pos = [1.0, 0.0, 0.0];
            actor.yaw = -90.0; // facing +x, where the body stands
            let mut player = base.clone();
            player.pos = [2.0, 0.0, 0.0];
            rt.things = vec![player, actor];
            rt.templates = vec!["walkplayer".into(), "mousebot".into()];
            rt.types = vec![10, 2];
            rt.ai.insert(
                1,
                crate::ai::parse(&format!(
                    "alignment=-1.0, rank=0.5, fov=90, sightdist=2.0, heardist=2.0\n\n\
                     SenseDanger {args}\n"
                )),
            );
            rt.ai_flags.insert(1, flags);
            rt.sector_adjoins = vec![Vec::new(); 8];
            rt.ai_tick(0, 1.0 / 30.0);
            (
                rt.ai_flags.get(&1).copied().unwrap_or(0) & 0x800,
                rt.sounds.len(),
                rt.sector_awareness.len(),
            )
        };
        assert_ne!(sample(0x4, "0.10, 1.00").0, 0, "awake and enabled did not alert");
        // The shipped default (`0.00`) disables the instinct: no alert, no sound, no event.
        assert_eq!(sample(0x4, "0.10, 0.00"), (0, 0, 0), "a disabled class alerted");
        // Enabled but not awake: the scripts never told this actor to look.
        assert_eq!(sample(0, "0.10, 1.00"), (0, 0, 0), "a dormant actor alerted");
        // Already alerted: the handler's gate stops it running again.
        assert_eq!(sample(0x4 | 0x800, "0.10, 1.00"), (0x800, 0, 0), "an alerted actor re-alerted");
    }

    /// `sithPhysics_UpdateThingPhysics`'s thrust arm (`0x4f6270`): a body in a thrust sector is
    /// accelerated by the sector's vector **unless its physflags carry `0x400000`** — the flag `_powerup`
    /// ships, and the reason m5's four weapon pickups must not climb in the sectors a script gives an
    /// updraft (before the exemption they reached z 89.3 in six seconds).
    #[test]
    fn a_thrust_sector_lifts_bodies_except_the_ones_flagged_against_it() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut free = base.clone();
        free.pos = [1.0, 0.0, 0.0];
        free.sector = 0;
        let mut exempt = base.clone();
        exempt.pos = [2.0, 0.0, 0.0];
        exempt.sector = 0;
        exempt.phys_flags = 0x400000;
        rt.things = vec![rt.things[0].clone(), free, exempt];
        rt.templates = vec!["walkplayer".into(), "crate".into(), "crate".into()];
        rt.types = vec![10, 5, 5];
        rt.sectors = vec![SectorState {
            thrust: [0.0, 0.0, 5.0],
            light: 0.0,
            light_from: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
            tint: [0.0; 3],
            colormap: 0,
        }];
        rt.step(None, 1.0);
        assert!(
            rt.things[1].pos[2] > 4.0,
            "the thrust sector did not lift a free body: {}",
            rt.things[1].pos[2]
        );
        assert_eq!(
            rt.things[2].pos[2], 0.0,
            "a body with physflags 0x400000 was lifted anyway"
        );
    }

    /// `dodge`'s damage path (`0x50b360`, event 1): `sithActor_DamageActor` emits AI event 1 with the
    /// attacker, and the handler steps the actor `arg0` units **away** from it through
    /// `sithAI_SetMoveThing`, writing the engine's shared 1000 ms move lockout (`ai + 0xa3`) — its
    /// only gate is the clear moving bit. Shipped classes are small steps: grunts ship
    /// `dodge 0.1, 0.1`, bosses `0.3, 0.1`.
    #[test]
    fn an_actor_that_is_hit_dodges_away_from_its_attacker() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [1.0, 0.0, 0.0];
        actor.sector = 0;
        let mut attacker = base.clone();
        attacker.pos = [3.0, 0.0, 0.0];
        attacker.sector = 0;
        rt.things = vec![base, actor, attacker];
        rt.templates = vec!["walkplayer".into(), "grunt".into(), "grunt".into()];
        rt.types = vec![10, 2, 2];
        rt.things[1].health = 100.0;
        rt.things[2].health = 100.0;
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=90, sightdist=2.0, heardist=2.0\n\n\
                 Dodge 0.3, 0.3\n",
            ),
        );
        rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
        // Actor against actor: the engine scales it to a tenth, so the 5 is a 0.5.
        assert_eq!(rt.things[1].health, 99.5, "the damage did not apply");
        assert_eq!(
            rt.ai_move.get(&1).copied(),
            Some([0.7, 0.0, 0.0]),
            "the actor did not step 0.3 away from its attacker"
        );
        // The moving bit the first dodge's `SetMoveThing` set: another hit while it holds leaves
        // the destination alone — the handler's only gate (`0x50b360`).
        rt.things[1].pos = [1.5, 0.0, 0.0];
        rt.things[2].pos = [3.5, 0.0, 0.0];
        rt.time = 0.5;
        rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
        assert_eq!(
            rt.ai_move.get(&1).copied(),
            Some([0.7, 0.0, 0.0]),
            "a second hit inside the 1 s cooldown moved the body again"
        );
        // … and after it, the step is measured from where the body now is — but only once the
        // moving bit the first dodge's `SetMoveThing` set has cleared on arrival: `SetMoveThing`
        // refuses a new move for a moving actor outside the jump lockout, exactly as the engine
        // (`0x4eac90`) does.
        rt.time = 1.1;
        rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
        assert_eq!(
            rt.ai_move.get(&1).copied(),
            Some([0.7, 0.0, 0.0]),
            "a moving actor dodged again before arriving"
        );
        *rt.ai_flags.get_mut(&1).unwrap() &= !0x1;
        rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
        assert_eq!(
            rt.ai_move.get(&1).copied(),
            Some([1.2, 0.0, 0.0]),
            "the dodge did not re-arm once the actor had arrived"
        );
        // The engine has **no** handler cooldown: once the body has arrived (bit `0x1` clear) it
        // dodges again even inside the 1000 ms `ai + 0xa3` lockout the first dodge wrote, which
        // `SetMoveThing` applies only to a body that is already moving (`0x4eac90`).
        rt.things[1].pos = [1.5, 0.0, 0.0];
        rt.things[2].pos = [3.5, 0.0, 0.0];
        rt.time = 0.6;
        *rt.ai_flags.get_mut(&1).unwrap() &= !0x1;
        rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
        assert_eq!(
            rt.ai_move.get(&1).copied(),
            Some([1.2, 0.0, 0.0]),
            "an arrived actor inside the lockout window did not dodge"
        );
    }

    /// The handler's gates: no instinct (or a zero first number) means no dodge, and an actor whose AI
    /// flag bit `0x1` is set — the engine's "already walking" bit — keeps its destination.
    #[test]
    fn dodge_needs_the_instinct_and_a_clear_moving_flag() {
        let sample = |flags: u32, class: Option<&str>| {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut actor = base.clone();
            actor.pos = [1.0, 0.0, 0.0];
            actor.sector = 0;
            let mut attacker = base.clone();
            attacker.pos = [3.0, 0.0, 0.0];
            attacker.sector = 0;
            rt.things = vec![base, actor, attacker];
            rt.templates = vec!["walkplayer".into(), "grunt".into(), "grunt".into()];
            rt.types = vec![10, 2, 2];
            rt.things[1].health = 100.0;
            if let Some(text) = class {
                rt.ai.insert(1, crate::ai::parse(text));
            }
            rt.ai_flags.insert(1, flags);
            rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
            rt.ai_move.get(&1).copied()
        };
        let class = "alignment=-1.0, rank=0.5\n\nDodge 0.3, 0.3\n";
        assert_eq!(sample(0, Some(class)), Some([0.7, 0.0, 0.0]), "no dodge with a clear flag");
        assert_eq!(sample(0x1, Some(class)), None, "a walking actor dodged anyway");
        assert_eq!(sample(0, Some("alignment=-1.0, rank=0.5\n\nDodge 0.3, 0.0\n")), None, "a zero step dodged");
        assert_eq!(sample(0, None), None, "an actor with no instinct dodged");
    }

    /// A runtime for the shot-dodge checks: player thing 0 at the origin facing +X (engine yaw -90),
    /// one actor at (2, 0, 0) whose class has `Dodge 0.3, 0.2` (weight 0.3, step 0.2). The template
    /// tables hold what `fire` needs — the bolt's `vel=(0/7/0)` and its shipped `typeflags=0x20440d` /
    /// `thingflags=0x20000001` — and the player's muzzle is its own eye (no `fireoffset=`), so the
    /// sweep is a clean +X line.
    fn shot_dodge_runtime(flags: u32, actor_yaw: f32, bolt_typeflags: u32, sightdist: f32) -> Runtime {
        let mut rt = door_runtime();
        let mut base = rt.things[0].clone();
        base.yaw = -90.0;
        let mut actor = base.clone();
        actor.pos = [2.0, 0.0, 0.0];
        actor.yaw = actor_yaw;
        rt.things = vec![base, actor];
        rt.templates = vec!["walkplayer".into(), "grunt".into()];
        rt.types = vec![10, 2];
        rt.things[1].health = 100.0;
        rt.template_health.insert("grunt".into(), 100.0);
        rt.template_fire_offset.insert("walkplayer".into(), [0.0; 3]);
        rt.template_vel.insert("+bryarbolt".into(), [0.0, 7.0, 0.0]);
        rt.template_typeflags.insert("+bryarbolt".into(), bolt_typeflags);
        rt.template_thingflags.insert("+bryarbolt".into(), 0x20000001);
        rt.ai_flags.insert(1, flags);
        rt.ai.insert(
            1,
            crate::ai::parse(&format!(
                // Weight 0.3, step 0.2: the handler reads `entry + 0x10`, the *second* number (the
                // port's `args[0]`), which is why the two differ here.
                "alignment=-1.0, rank=0.5, fov=90, sightdist={sightdist}, heardist=2.0\n\n\
                 Dodge 0.3, 0.2\n"
            )),
        );
        rt
    }

    /// The awareness sweep raises **event 2**, the one event the `0x1000` one-shot latch lets through,
    /// and clears the latch doing so. Without it an actor dodges exactly one shot per level: the shot
    /// arms the latch, nothing ever disarms it, and every later `0x1000` is dropped by the gate.
    #[test]
    fn the_awareness_sweep_re_arms_the_one_shot_dodge_latch() {
        let mut rt = shot_dodge_runtime(0x200, 90.0, 0x20440d, 5.0);
        rt.sector_adjoins = vec![Vec::new(); 8]; // both things sit in sector 7 (`door_runtime`)
        // A latched actor drops every event but 2 — the engine's own rate limit.
        rt.ai_flags.insert(1, 0x200 | 0x1000);
        assert!(!rt.emit_ai_event(1, 0x1000, Some(0)), "the latch let a shot through");
        // Awareness in the actor's sector is the event-2 producer; it disarms the latch.
        rt.time += 0.5;
        rt.transmit_awareness(7, 0, [2.0, 0.0, 0.0], 3.0, None);
        assert_eq!(
            rt.ai_flags.get(&1).copied().unwrap_or(0) & 0x1000,
            0,
            "event 2 must clear the one-shot latch"
        );
        // And the sweep reaches a thing at most once per tick.
        rt.ai_flags.insert(1, 0x200 | 0x1000);
        rt.transmit_awareness(7, 0, [2.0, 0.0, 0.0], 3.5, None);
        assert_eq!(
            rt.ai_flags.get(&1).copied().unwrap_or(0) & 0x1000,
            0x1000,
            "a second awareness event in the same tick must not re-raise event 2"
        );
    }

    /// `dodge`'s shot-passing path (`0x50b360`, event `0x1000`): the **player's** shot with
    /// `thingflags & 0x200000` runs a five-unit pre-sweep, and the actor it would hit — if it can see
    /// the shot (the class's `sightdist`/`fov`, clear line) — steps `arg0` units along the *negated*
    /// direction-to-the-shot rotated by a random 45-90 degrees: a sideways dodge, not a retreat.
    #[test]
    fn a_player_shot_that_would_hit_an_actor_makes_it_dodge_sideways() {
        let mut rt = shot_dodge_runtime(0x200, 90.0, 0x20440d, 5.0);
        assert!(rt.fire(0, "+bryarbolt").is_some(), "the shot did not spawn");
        let shot = rt.things.last().expect("a shot thing");
        assert_eq!(shot.flags, 0x20000001, "the shot did not inherit its template's thingflags");
        let actor = [2.0, 0.0, 0.0];
        let Some(dest) = rt.ai_move.get(&1).copied() else {
            panic!("the actor did not dodge a shot that would hit it");
        };
        let (dx, dy) = (dest[0] - actor[0], dest[1] - actor[1]);
        let step = (dx * dx + dy * dy).sqrt();
        assert!((step - 0.2).abs() < 1e-3, "the dodge stepped {step}, not the class's 0.2");
        // The step is the negated rotated direction-to-the-shot, so it is 90-135 degrees off it.
        let to_shot = [-1.0, 0.0];
        let dot = (dx * to_shot[0] + dy * to_shot[1]) / step;
        assert!(dot < 0.2, "the dodge went toward the shot instead of sideways (dot {dot:.2})");
        assert!(dest[2] == 0.0, "the dodge left the actor's floor: {dest:?}");
    }

    /// The handler's gates: the AI's `0x200` bit (`AISetMode` wakes an actor with it), the class's
    /// non-zero step, the actor being able to see the shot, and the engine's two pre-sweep conditions —
    /// the **player** is the shooter and the projectile's `typeflags` carry `0x200000`.
    #[test]
    fn the_shot_dodge_needs_the_actor_awake_the_sight_line_and_the_player_shooter() {
        // The player fires the shipped `+bryarbolt` (`typeflags=0x20440d`, `thingflags=0x20000001`).
        let fires = |mut rt: Runtime| {
            assert!(rt.fire(0, "+bryarbolt").is_some(), "the shot did not spawn");
            rt.ai_move.contains_key(&1)
        };
        // Awake, facing the player, and the shipped bolt typeflags: this is the one that dodges.
        assert!(fires(shot_dodge_runtime(0x200, 90.0, 0x20440d, 5.0)));
        // Not awake: the handler's first gate.
        assert!(!fires(shot_dodge_runtime(0x0, 90.0, 0x20440d, 5.0)));
        // Facing away: `CheckSightThing`'s cone fails.
        assert!(!fires(shot_dodge_runtime(0x200, 0.0, 0x20440d, 5.0)));
        // Too far for the class's sight.
        assert!(!fires(shot_dodge_runtime(0x200, 90.0, 0x20440d, 0.5)));
        // A projectile without the engine's typeflag bit: no pre-sweep, so no event at all.
        assert!(!fires(shot_dodge_runtime(0x200, 180.0, 0x440d, 5.0)));
        // A non-player shooter: the pre-sweep is player-only in `WeaponFireProjectile`.
        let mut rt = shot_dodge_runtime(0x200, 90.0, 0x20440d, 5.0);
        rt.things[1].yaw = 90.0;
        assert!(rt.fire(1, "+bryarbolt").is_some(), "the actor's own shot did not spawn");
        assert!(!rt.ai_move.contains_key(&1), "an actor's own shot made somebody dodge");
    }

    /// The POV key flags: mount `0x14` and fire `0x38` both carry an end bit, so neither loops; the
    /// old build wrapped every clip and the mount's raise replayed into a visible flicker.
    #[test]
    fn the_shipped_pov_clips_do_not_loop_unless_their_flags_say_so() {
        assert!(!pov_key_loops(0x14), "the mount clip must hold its last frame");
        assert!(!pov_key_loops(0x38), "the fire clip must not loop");
        assert!(pov_key_loops(0x18), "the dismount flags have no end bit");
    }

    /// `forcepowers` (`0x40ff40`) reads the shipped line as three `(range, chance, cooldown)`
    /// triples and compares **one** random draw with all three chances. The classes pin it:
    /// `yun.ai` rolls 20% persuasion (0–5 units) and 60% blinding (0–4); `sariss.ai` rolls 63%
    /// persuasion only at 1–3 units and 99% blinding otherwise.
    #[test]
    fn forcepowers_picks_the_first_rolled_ability_inside_its_range() {
        let yun = [0.0, 5.0, 0.0, 4.0, 0.0, 0.0, 0.20, 0.60, 0.0, 12000.0, 15000.0, 0.0];
        assert_eq!(force_power(&yun, 2.0, 0.19), Some((1, 12000.0)));
        assert_eq!(force_power(&yun, 2.0, 0.21), Some((2, 15000.0)));
        // 4.5 units is outside both ranges and 0.61 fails both chances.
        assert_eq!(force_power(&yun, 4.5, 0.21), None);
        assert_eq!(force_power(&yun, 2.0, 0.61), None);
        assert_eq!(force_power(&yun, 5.0, 0.19), Some((1, 12000.0)));

        let sariss = [1.0, 3.0, 0.0, 5.0, 0.0, 0.0, 0.63, 0.99, 0.0, 15000.0, 1000.0, 0.0];
        assert_eq!(force_power(&sariss, 2.0, 0.5), Some((1, 15000.0)));
        assert_eq!(force_power(&sariss, 4.0, 0.5), Some((2, 1000.0)));
        assert_eq!(force_power(&sariss, 4.0, 0.995), None);
    }

    /// The handler's engine side is a `user0` message to the actor's **own** scripts with the victim
    /// and the ability, exactly `ThingSendMessageEx(thing, 0, 27, target, power, 0, 0)`. It must
    /// reach the instance the level binds to the actor, and only once per cooldown.
    #[test]
    fn forcepowers_queues_user0_on_the_actors_script() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        rt.things = vec![base, actor];
        rt.templates = vec!["walkplayer".into(), "yun".into()];
        rt.types = vec![10, 2];
        rt.template_hit.insert("walkplayer".into(), (0.065, 0.18));
        rt.template_hit.insert("yun".into(), (0.06, 0.17));
        rt.things[0].pos = [2.0, 0.0, 0.0];
        rt.things[1].yaw = 0.0;
        rt.thing_links.push((7, 0, 1));
        rt.ai.insert(
            1,
            crate::ai::Ai {
                sightdist: 10.0,
                fov: 180.0,
                instincts: vec![crate::ai::Instinct {
                    name: "ForcePowers".into(),
                    // The handler's scheduler interval is zero here so the second tick is gated
                    // only by the selected ability's own cooldown.
                    weight: 0.0,
                    args: vec![
                        0.0, 5.0, 0.0, 5.0, 0.0, 0.0, 1.0, 1.0, 1.0, 12000.0, 15000.0, 9000.0,
                    ],
                }],
                ..Default::default()
            },
        );
        rt.ai_detected.insert(1);
        rt.ai_tick(0, 1.0 / 30.0);
        let queued: Vec<_> = rt
            .pending_messages
            .iter()
            .filter(|(instance, label, _, _, _)| *instance == 7 && *label == "user0")
            .collect();
        assert_eq!(queued.len(), 1, "the force power did not reach the actor's script");
        assert_eq!(queued[0].2, [0.0, 1.0], "the message did not carry (victim, ability)");
        // The ability's own cooldown is `fltArg[10]` (12 s), so a second tick queues nothing.
        rt.ai_tick(0, 1.0 / 30.0);
        assert_eq!(
            rt.pending_messages.iter().filter(|(_, label, _, _, _)| *label == "user0").count(),
            1,
            "the force power ignored its own cooldown"
        );
    }

    /// `saberfighting` (`0x40fa40`) walks its four `(range, damage, cooldown)` entries in order and
    /// stops at the first range below the target gap. `yun.ai` (0.25/20/1600 then 0.25/30/925) and
    /// `sariss.ai` (0.225/40/950 then 0.25/30/1400) are the two shipped shapes.
    #[test]
    fn saberfighting_picks_the_class_swing_inside_its_range() {
        let yun = [0.40, 0.25, 20.0, 1600.0, 0.25, 30.0, 925.0];
        assert_eq!(saber_attack(&yun, 0.2, 0.3, 0.0), Some((20.0, 8, 1600.0)));
        assert_eq!(saber_attack(&yun, 0.2, 0.3, 0.9), Some((30.0, 0x12, 925.0)));
        // The chance gate, then the first range's ceiling.
        assert_eq!(saber_attack(&yun, 0.2, 0.5, 0.0), None);
        assert_eq!(saber_attack(&yun, 0.3, 0.3, 0.0), None);

        let sariss = [0.50, 0.225, 40.0, 950.0, 0.25, 30.0, 1400.0];
        assert_eq!(saber_attack(&sariss, 0.2, 0.4, 0.0), Some((40.0, 8, 950.0)));
        assert_eq!(saber_attack(&sariss, 0.2, 0.4, 0.6), Some((30.0, 0x12, 1400.0)));
        // 0.24 is past the first entry's 0.225, and the walk stops there.
        assert_eq!(saber_attack(&sariss, 0.24, 0.4, 0.0), None);
    }

    /// The live half: a saber-armed dark Jedi inside range damages the player with `DAMAGE_SABER`,
    /// raises the handler's `0x400` flag and cannot swing again until the entry's cooldown.
    #[test]
    fn saberfighting_hurts_the_target_and_honours_the_cooldown() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        rt.things = vec![base, actor.clone()];
        rt.templates = vec!["walkplayer".into(), "yun".into()];
        rt.types = vec![10, 2];
        rt.template_hit.insert("walkplayer".into(), (0.065, 0.18));
        rt.template_hit.insert("yun".into(), (0.06, 0.17));
        rt.things[0].pos = [0.2, 0.0, 0.0];
        actor.pos = [0.0, 0.0, 0.0];
        rt.things[1] = actor;
        rt.saber.insert(1, SaberInfo::default());
        rt.ai.insert(
            1,
            crate::ai::Ai {
                sightdist: 10.0,
                fov: 180.0,
                instincts: vec![crate::ai::Instinct {
                    name: "SaberFighting".into(),
                    weight: 0.0,
                    args: vec![1.0, 0.25, 20.0, 1600.0, 0.25, 30.0, 925.0],
                }],
                ..Default::default()
            },
        );
        rt.ai_detected.insert(1);
        let before = rt.things[0].health;
        rt.ai_tick(0, 1.0 / 30.0);
        assert!(rt.things[0].health < before, "the saber did not hit the target");
        assert_eq!(rt.ai_flags.get(&1).copied().unwrap_or(0) & 0x400, 0x400);
        let after = rt.things[0].health;
        rt.ai_tick(0, 1.0 / 30.0);
        assert_eq!(rt.things[0].health, after, "the saber ignored its cooldown");
    }

    /// `specialattack`'s event-4 branch (`0x40fd00`): the thing-collision callback passes the
    /// collided player and the handler damages it with `DAMAGE_SABER` at `fltArg[8]` (Maw 10000,
    /// Pic 2000). The world tick raises event 4 from the `(2,10)` pair's thing collision.
    #[test]
    fn specialattack_event_four_damages_the_collided_player() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        rt.things = vec![base.clone(), base];
        rt.types = vec![10, 2];
        rt.templates = vec!["walkplayer".into(), "maw".into()];
        rt.template_hit.insert("walkplayer".into(), (0.065, 0.18));
        rt.template_hit.insert("maw".into(), (0.12, 0.3));
        rt.ai.insert(
            1,
            crate::ai::Ai {
                instincts: vec![crate::ai::Instinct {
                    name: "SpecialAttack".into(),
                    weight: 3000.0,
                    // engine fltArg 1..8; fltArg 8 = 40.0 is the damage
                    args: vec![0.75, 0.5, 1.0, 3000.0, 9.0, 0.0, 0.0, 40.0],
                }],
                ..Default::default()
            },
        );
        let before = rt.things[0].health;
        assert!(rt.emit_ai_event(1, 4, Some(0)), "the contact event did not act");
        assert!(
            rt.things[0].health <= before - 40.0,
            "the event did not damage the player: {} -> {}",
            before,
            rt.things[0].health
        );
    }

    /// `sensedanger`'s damage path (`0x50b9b0`, event 1): an aware, not-yet-alerted actor that is hit
    /// plays its `surprise` sound (mode `0x57`), takes the attacker — or the attacker's parent, for a
    /// projectile — as its target, and swaps the awake bit for the alert bit.
    #[test]
    fn sensedanger_reacts_to_damage_with_a_surprise_and_an_alert() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [1.0, 0.0, 0.0];
        let mut attacker = base.clone();
        attacker.pos = [3.0, 0.0, 0.0];
        rt.things = vec![base, actor, attacker];
        rt.templates = vec!["walkplayer".into(), "grunt".into(), "grunt".into()];
        rt.types = vec![10, 2, 2];
        rt.things[1].health = 100.0;
        rt.things[2].health = 100.0;
        rt.template_soundclass.insert("grunt".into(), "grunt.snd".into());
        rt.sound_classes.insert(
            "grunt.snd".into(),
            std::collections::BTreeMap::from([(
                "surprise".to_string(),
                vec![("surprise1.wav".to_string(), 1.0)],
            )]),
        );
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=90, sightdist=5.0, heardist=2.0\n\n\
                 SenseDanger 0.10, 1.00\n",
            ),
        );
        rt.ai_flags.insert(1, 0x4);
        rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
        assert_eq!(rt.things[1].health, 99.5, "the damage did not apply");
        let flags = rt.ai_flags.get(&1).copied().unwrap_or(0);
        assert_eq!(flags & 0x4, 0, "the awake bit survived the surprise");
        assert_ne!(flags & 0x800, 0, "the alert bit was not set");
        assert_eq!(rt.sounds.len(), 1, "no surprise sound was queued");
        assert_eq!(rt.sounds[0].name, "surprise1.wav");
        assert_eq!(
            rt.ai_last_seen.get(&1).copied(),
            Some([3.0, 0.0, 0.0]),
            "the surprise did not make the attacker the target"
        );
    }

    /// `sensedanger`'s **event-2** arm (`0x50b9ff`): a hurt, unalerted actor whose sector holds a
    /// kind-1 record names that record's source as its target, sets the alert word and re-broadcasts
    /// the event at 8.0 — but only when the instinct's own weight beats the record's strength and the
    /// source is inside `heardist` through a clear line (the engine's `0x4eb090` with a `-1.0` cone).
    #[test]
    fn sensedanger_alerts_to_a_heard_source_the_event_two_arm_names() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [1.0, 0.0, 0.0];
        let mut heard = base.clone();
        heard.pos = [2.0, 0.0, 0.0];
        rt.things = vec![base, actor, heard];
        rt.templates = vec!["walkplayer".into(), "grunt".into(), "grunt".into()];
        rt.types = vec![10, 2, 2];
        rt.things[1].health = 100.0;
        rt.things[2].health = 100.0;
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=90, sightdist=5.0, heardist=2.0\n\n\
                 SenseDanger 3.00, 1.00\n",
            ),
        );
        rt.sector_adjoins = vec![Vec::new(); 8]; // the arm re-broadcasts into sector 7
        rt.ai_flags.insert(1, 0x4);
        // A whisper of an event (`strength 1.0`) against the line's 3.0 weight: the arm runs.
        rt.sector_awareness
            .insert((7, 1), (1.0, [2.0, 0.0, 0.0], rt.time, Some(2)));
        assert!(rt.emit_ai_event(1, 2, None), "the heard source did not alert the actor");
        let flags = rt.ai_flags.get(&1).copied().unwrap_or(0);
        assert_eq!(flags & 0x4, 0, "the arm did not clear the hurt word");
        assert_ne!(flags & 0x800, 0, "the arm did not set the alert bit");
        assert_eq!(
            rt.ai_last_seen.get(&1).copied(),
            Some([2.0, 0.0, 0.0]),
            "the heard thing did not become the target"
        );
        assert_eq!(
            rt.sector_awareness.get(&(7, 1)).map(|(s, _, _, _)| *s),
            Some(4.0),
            "the arm did not re-broadcast the event at 4.0"
        );
        // The engine's own order is weight > strength: a louder record silences the arm.
        let mut quiet = door_runtime();
        let base = quiet.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [1.0, 0.0, 0.0];
        let mut heard = base.clone();
        heard.pos = [2.0, 0.0, 0.0];
        quiet.things = vec![base, actor, heard];
        quiet.templates = vec!["walkplayer".into(), "grunt".into(), "grunt".into()];
        quiet.types = vec![10, 2, 2];
        quiet.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=90, sightdist=5.0, heardist=2.0\n\n\
                 SenseDanger 0.30, 1.00\n",
            ),
        );
        quiet.ai_flags.insert(1, 0x4);
        quiet
            .sector_awareness
            .insert((7, 1), (1.0, [2.0, 0.0, 0.0], quiet.time, Some(2)));
        assert!(!quiet.emit_ai_event(1, 2, None), "a 0.3 weight beat a 1.0 record");
    }

    /// `sithActor_PlayDamageSoundFx` (`0x4ece90`): the hurt row for the damage kind, at
    /// `clamp(damage / health_after * 1.5, 0, 1)`, only for a blow of 3 or more that leaves the body
    /// alive. The mode names come from the engine's own mode table (`0x38` impact, `0x39` energy,
    /// `0x3a` fire, `0x3b` force, `0x3c` saber, `0x3d` drown).
    #[test]
    fn a_hit_that_leaves_the_body_alive_plays_its_hurt_sound() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [1.0, 0.0, 0.0];
        rt.things = vec![base, actor];
        rt.templates = vec!["walkplayer".into(), "grunt".into()];
        rt.types = vec![10, 2];
        rt.things[1].health = 100.0;
        rt.template_soundclass.insert("grunt".into(), "grunt.snd".into());
        rt.sound_classes.insert(
            "grunt.snd".into(),
            std::collections::BTreeMap::from([
                ("hurtenergy".to_string(), vec![("hurtenergy1.wav".to_string(), 1.0)]),
                ("hurtimpact".to_string(), vec![("hurtimpact1.wav".to_string(), 1.0)]),
            ]),
        );

        // Under the engine's 3-point threshold: silent.
        rt.damage_typed(1, 2.0, DAMAGE_ENERGY);
        assert!(rt.sounds.is_empty(), "a 2-point hit played a hurt sound");

        // The 2-point hit still lands (health 98), so 6 energy leaves 92 and the volume is 6 / 92 * 1.5.
        rt.damage_typed(1, 6.0, DAMAGE_ENERGY);
        assert_eq!(rt.sounds.len(), 1, "the energy hit played no sound");
        assert_eq!(rt.sounds[0].name, "hurtenergy1.wav");
        let want = 6.0 / 92.0 * 1.5;
        assert!(
            (rt.sounds[0].volume - want).abs() < 1e-4,
            "volume {} is not {want}",
            rt.sounds[0].volume
        );

        // A fall is the default arm; 86 on the 92 left leaves 6, and 86 / 6 * 1.5 clamps to 1.0.
        rt.damage_typed(1, 86.0, DAMAGE_FALL);
        assert_eq!(rt.sounds.len(), 2, "the fall hit played no sound");
        assert_eq!(rt.sounds[1].name, "hurtimpact1.wav");
        assert_eq!(rt.sounds[1].volume, 1.0, "the volume was not clamped to 1.0");

        // The lethal blow returns through the kill path, never the hurt sound.
        rt.damage_typed(1, 6.0, DAMAGE_IMPACT);
        assert_eq!(rt.sounds.len(), 2, "a lethal hit played a hurt sound");
    }

    /// `sithActor_DamageActor`: actor-versus-actor damage is a **tenth**, unless the attacker's actor
    /// flags carry 0x1000000, and drowning/falls are exempt. The player is type 10, so its own hits are
    /// never scaled.
    #[test]
    fn an_actor_hitting_another_actor_deals_a_tenth() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut victim = base.clone();
        victim.pos = [1.0, 0.0, 0.0];
        let mut attacker = base.clone();
        attacker.pos = [3.0, 0.0, 0.0];
        rt.things = vec![base, victim, attacker];
        rt.templates = vec!["walkplayer".into(), "grunt".into(), "grunt".into()];
        rt.types = vec![10, 2, 2];
        rt.things[1].health = 100.0;

        rt.damage_typed_from(1, 10.0, DAMAGE_IMPACT, 2);
        assert_eq!(rt.things[1].health, 99.0, "actor-on-actor damage was not scaled to a tenth");

        // 0x1000000 on the attacker exempts the scale.
        rt.actor_flags.insert(2, AF_ACTOR_FULL_DAMAGE);
        rt.damage_typed_from(1, 10.0, DAMAGE_IMPACT, 2);
        assert_eq!(rt.things[1].health, 89.0, "the full-damage flag was ignored");

        // Falls and drowning are exempt: the same hit unscaled.
        rt.actor_flags.insert(2, 0);
        rt.damage_typed_from(1, 2.0, DAMAGE_FALL, 2);
        assert_eq!(rt.things[1].health, 87.0, "a fall from an actor was scaled");
        rt.damage_typed_from(1, 2.0, 0x20, 2);
        assert_eq!(rt.things[1].health, 85.0, "drowning from an actor was scaled");

        // The player (type 10) is not an actor here, so its damage on an actor is full.
        rt.types[2] = 10;
        rt.damage_typed_from(1, 4.0, DAMAGE_IMPACT, 2);
        assert_eq!(rt.things[1].health, 81.0, "the player's damage was scaled");
    }

    /// The engine's mode ids are the canonical `.snd` event names, recovered from the pointer array
    /// `sithSoundClass_Startup` fills its hashtable from (`0x54b40c`); `0x57` being `surprise` is the
    /// check that the table's base is right, since the AI path already used that name.
    #[test]
    fn the_damage_kinds_map_to_the_engines_sound_modes() {
        assert_eq!(Runtime::hurt_mode(DAMAGE_IMPACT), "hurtimpact");   // 0x38
        assert_eq!(Runtime::hurt_mode(DAMAGE_ENERGY), "hurtenergy");   // 0x39
        assert_eq!(Runtime::hurt_mode(0x04), "hurtfire");              // 0x3a
        assert_eq!(Runtime::hurt_mode(0x08), "hurtmagic");             // 0x3b
        assert_eq!(Runtime::hurt_mode(DAMAGE_SABER), "hurtspecial");   // 0x3c
        assert_eq!(Runtime::hurt_mode(0x20), "drowning");              // 0x3d
        assert_eq!(Runtime::hurt_mode(DAMAGE_FALL), "hurtimpact");     // the switch's default arm
        assert_eq!(Runtime::hurt_mode(0x03), "hurtimpact", "a combined word is not an arm");
    }

    /// The dispatcher calls the class's instincts **in file order** and stops at the first handler that
    /// reacts (`sithAI_EmitEvent` breaks the loop on a non-zero return). So one hit is either a
    /// surprise or a sidestep, never both — which order the class lists decides which.
    #[test]
    fn an_ai_event_stops_at_the_first_instinct_that_reacts() {
        let hit = |sense_first: bool| {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut actor = base.clone();
            actor.pos = [1.0, 0.0, 0.0];
            let mut attacker = base.clone();
            attacker.pos = [3.0, 0.0, 0.0];
            rt.things = vec![base, actor, attacker];
            rt.templates = vec!["walkplayer".into(), "grunt".into(), "grunt".into()];
            rt.types = vec![10, 2, 2];
            rt.things[1].health = 100.0;
            rt.things[2].health = 100.0;
            rt.template_soundclass.insert("grunt".into(), "grunt.snd".into());
            rt.sound_classes.insert(
                "grunt.snd".into(),
                std::collections::BTreeMap::from([(
                    "surprise".to_string(),
                    vec![("surprise1.wav".to_string(), 1.0)],
                )]),
            );
            let lines = if sense_first {
                "SenseDanger 0.10, 1.00\nDodge 0.3, 0.3\n"
            } else {
                "Dodge 0.3, 0.3\nSenseDanger 0.10, 1.00\n"
            };
            rt.ai.insert(
                1,
                crate::ai::parse(&format!(
                    "alignment=-1.0, rank=0.5, fov=90, sightdist=5.0\n\n{lines}"
                )),
            );
            rt.ai_flags.insert(1, 0x4);
            rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
            (
                rt.ai_move.get(&1).copied(),
                rt.ai_flags.get(&1).copied().unwrap_or(0),
                rt.sounds.len(),
            )
        };
        let (moved, flags, sounds) = hit(true);
        assert_eq!(moved, None, "the sense-first class dodged after sensing");
        assert_ne!(flags & 0x800, 0);
        assert_eq!(sounds, 1);
        let (moved, flags, sounds) = hit(false);
        assert!(moved.is_some(), "the dodge-first class did not dodge");
        // `dodge` returns 0 (`0x50b360`), so the dispatch does not stop at it: the same damage
        // event continues to `sensedanger`, whose non-zero return is what ends the chain.
        assert_eq!(flags & 0x4, 0, "the dodge-first class did not alert as well");
        assert_ne!(flags & 0x800, 0);
        assert_eq!(sounds, 1, "the dodge-first class skipped the surprise");
    }

    /// `sithAI_EmitEvent`'s own outer gates: a thing whose `flags` carry `0x202` has no AI, a dead body
    /// takes no events, `0x2000` switches the AI off, and the `0x1000` one-shot latch lets only event 2
    /// through (clearing itself on the way).
    #[test]
    fn ai_events_respect_the_engines_outer_gates() {
        let sample = |setup: &dyn Fn(&mut Runtime)| {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut actor = base.clone();
            actor.pos = [1.0, 0.0, 0.0];
            let mut attacker = base.clone();
            attacker.pos = [3.0, 0.0, 0.0];
            rt.things = vec![base, actor, attacker];
            rt.templates = vec!["walkplayer".into(), "grunt".into(), "grunt".into()];
            rt.types = vec![10, 2, 2];
            rt.things[1].health = 100.0;
            rt.things[2].health = 100.0;
            rt.ai.insert(
                1,
                crate::ai::parse(
                    "alignment=-1.0, rank=0.5, fov=90, sightdist=5.0\n\nDodge 0.3, 0.3\n",
                ),
            );
            rt.ai_flags.insert(1, 0x200);
            setup(&mut rt);
            rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
            (rt.ai_move.contains_key(&1), rt.ai_flags.get(&1).copied().unwrap_or(0))
        };
        assert!(sample(&|_| {}).0, "the plain case did not dodge");
        assert!(!sample(&|rt| rt.things[1].flags = 0x202).0, "a 0x202 thing reacted");
        assert!(!sample(&|rt| rt.things[1].health = 0.0).0, "a dead body reacted");
        assert!(!sample(&|rt| {
            rt.ai_flags.insert(1, 0x200 | 0x2000);
        })
        .0, "an AI switched off reacted");
        let (moved, flags) = sample(&|rt| {
            rt.ai_flags.insert(1, 0x200 | 0x1000);
        });
        assert!(!moved, "the 0x1000 latch let an event through");
        assert_ne!(flags & 0x1000, 0, "the latch cleared without event 2");
    }

    /// `sithAI_CanDetectSightThing` (`0x4ec140`) is a probability, not a yes/no: inside two units a
    /// target is always noticed, beyond it a 0.1-per-unit falloff (capped at 0.6) applies, an unawake
    /// watcher starts at 0.5, and a physics target that is still or crouched is harder to notice. The
    /// constants are the retail `.rdata` values.
    #[test]
    fn detection_chance_falls_off_with_range_and_pose() {
        let chance = |distance: f32,
                      player_vel: [f32; 3],
                      player_physflags: u32,
                      player_flags: u32,
                      actor_flags: u32,
                      actor_thing_flags: u32| {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut actor = base.clone();
            actor.pos = [0.0, 0.0, 0.0];
            actor.flags = actor_thing_flags;
            let mut player = base.clone();
            player.pos = [distance, 0.0, 0.0];
            player.vel = player_vel;
            player.phys_flags = player_physflags;
            player.flags = player_flags;
            rt.things = vec![player, actor];
            rt.templates = vec!["walkplayer".into(), "tuskan".into()];
            rt.types = vec![10, 2];
            rt.template_move.insert("walkplayer".into(), "physics".into());
            rt.ai_flags.insert(1, actor_flags);
            rt.detect_chance(1, 0)
        };
        let moving = [1.0, 0.0, 0.0];
        assert_eq!(chance(1.0, moving, 0, 0, 0x200, 0), 1.0, "a target inside two units");
        assert!((chance(8.0, moving, 0, 0, 0x200, 0) - 0.4).abs() < 1e-5, "8 units, moving");
        assert!((chance(20.0, moving, 0, 0, 0x200, 0) - 0.4).abs() < 1e-5, "the falloff saturates at 0.6");
        assert!(
            (chance(8.0, [0.0; 3], 0, 0, 0x200, 0) - 0.2).abs() < 1e-5,
            "a still target is half as noticeable"
        );
        assert!(
            (chance(8.0, moving, 0x10000, 0, 0x200, 0) - 0.3).abs() < 1e-5,
            "a crouched target is three-quarters as noticeable"
        );
        assert!(
            (chance(8.0, [0.0; 3], 0x10000, 0, 0x200, 0) - 0.15).abs() < 1e-5,
            "still and crouched together"
        );
        assert!(
            (chance(8.0, moving, 0, 0, 0x0, 0) - 0.2).abs() < 1e-5,
            "an unawake watcher starts at half"
        );
        assert!(
            (chance(8.0, moving, 0, 0x80, 0x200, 0) - 0.05).abs() < 1e-5,
            "a hidden target floors at 0.05"
        );
        assert!(
            (chance(8.0, moving, 0, 0, 0x200, 0x800) - 0.05).abs() < 1e-5,
            "a hidden watcher floors at 0.05"
        );
        assert!(
            (chance(8.0, moving, 0, 0x80, 0x200, 0x8000) - 0.4).abs() < 1e-5,
            "the watcher's 0x8000 lifts the target's 0x80"
        );
    }

    /// Round 314: `sithAI_CanDetectSightThing`'s darkness term. The target's **sector** light
    /// (`AMBIENT LIGHT`) under 0.5 scales the chance by `light + 0.2` (`_DAT_0052308c` is -0.2),
    /// unless the watcher's thingflags carry `0x2000000`.
    #[test]
    fn a_dark_sector_makes_a_target_harder_to_notice() {
        let chance = |ambient: f32, watcher_flags: u32| {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut actor = base.clone();
            actor.flags = watcher_flags;
            let mut player = base.clone();
            player.pos = [8.0, 0.0, 0.0];
            player.vel = [1.0, 0.0, 0.0];
            player.sector = 0;
            rt.things = vec![player, actor];
            rt.things[1].sector = 0;
            rt.templates = vec!["walkplayer".into(), "tuskan".into()];
            rt.types = vec![10, 2];
            rt.template_move.insert("walkplayer".into(), "physics".into());
        rt.ai_flags.insert(1, 0x200);
            rt.sector_ambient_light = vec![ambient];
            rt.detect_chance(1, 0)
        };
        // 8 units and moving is 0.4; a 0.1-light room scales it by 0.3.
        assert!((chance(0.1, 0) - 0.12).abs() < 1e-5, "the darkness term is not applied");
        assert!((chance(0.4, 0) - 0.24).abs() < 1e-5, "0.4 light should scale by 0.6");
        assert!((chance(0.5, 0) - 0.4).abs() < 1e-5, "0.5 light is not dark");
        assert!((chance(0.1, 0x2000000) - 0.4).abs() < 1e-5, "night vision ignored the darkness");
    }

    /// `angvel=(pitch/yaw/roll)` is a **template** key (level 1's `fan1` carries one; 13 rows in its
    /// TEMPLATES section do) and it seeds every instance's angular velocity, which
    /// `ThingState::step` integrates: a 90 deg/s yaw is a quarter turn a second.
    #[test]
    fn a_placed_thing_spins_at_its_templates_angular_velocity() {
        let mut level = Level::default();
        level.templates.push(crate::jkl::Template {
            name: "fan1".into(),
            base: None,
            params: vec![("angvel".into(), "(0.000000/90.000000/0.000000)".into())],
            model3d: Some("fan1.3do".into()),
            sprite: None,
        });
        level.things.push(crate::jkl::Thing {
            template: "fan1".into(),
            pos: [0.0; 3],
            pitch: 0.0,
            yaw: 0.0,
            roll: 0.0,
            sector: 0,
            flags: 0,
            frames: Vec::new(),
            vel: [0.0; 3],
            aiclass: None,
        });
        let mut rt = Runtime::new(&level);
        assert_eq!(rt.things[0].rot_vel, [0.0, 90.0, 0.0], "the template's angvel did not seed rot_vel");
        // `step`'s return says whether the thing *moved*, which a pure spin does not: it feeds the
        // collision sweep, and a prop that only turns must not run it.
        let _ = rt.things[0].step(1.0);
        assert!((rt.things[0].yaw - 90.0).abs() < 1e-4, "yaw {} after one second", rt.things[0].yaw);
    }

    #[test]
    fn a_pure_spin_is_reported_as_turned_not_moved() {
        let mut level = Level::default();
        level.templates.push(crate::jkl::Template {
            name: "fan1".into(),
            base: None,
            params: vec![("angvel".into(), "(0.000000/90.000000/0.000000)".into())],
            model3d: Some("fan1.3do".into()),
            sprite: None,
        });
        level.things.push(crate::jkl::Thing {
            template: "fan1".into(),
            pos: [0.0; 3],
            pitch: 0.0,
            yaw: 0.0,
            roll: 0.0,
            sector: 0,
            flags: 0,
            frames: Vec::new(),
            vel: [0.0; 3],
            aiclass: None,
        });
        let mut rt = Runtime::new(&level);
        let motion = rt.step(None, 1.0);
        assert_eq!(motion.turned, vec![0], "a spinning prop was not reported as turned");
        assert!(!motion.moved.contains(&0), "a pure spin was reported as a translation");
        assert!((rt.things[0].yaw - 90.0).abs() < 1e-4, "yaw {} after one second", rt.things[0].yaw);
    }

    #[test]
    fn step_player_lands_the_body_and_keeps_the_sector() {
        let floor = crate::collide::Face {
            normal: [0.0, 0.0, -1.0],
            point: [0.0; 3],
            verts: vec![
                [-1.0, -1.0, 0.0],
                [1.0, -1.0, 0.0],
                [1.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0],
            ],
            surface: 1,
        };
        let solids = crate::collide::Solids {
            // The engine search reaches the floor polygon through the sector's plane and `floor_of`.
            sectors: vec![vec![crate::collide::Plane { normal: [0.0, 0.0, -1.0], point: [0.0; 3], surface: 1 }]],
            floor_of: [(1usize, 0usize)].into_iter().collect(),
            floors: vec![floor.clone()],
            faces: vec![floor],
            surface_owner: vec![0, 0],
            ..Default::default()
        };
        let mut rt = door_runtime();
        rt.things[0].phys_flags = 0x4a4f;
        rt.template_move.insert("walkplayer".into(), "physics".into());
        rt.template_collide.insert("walkplayer".into(), 1);
        let rules = crate::Rules::from_level(&Level::default());
        let mut body = walk_body(Vec3::new(0.0, 0.0, 0.5));
        body.grounded = false;
        let rt = World::new(rt);
        for _ in 0..200 {
            Runtime::step_player(&rt,
                &mut body,
                &solids,
                PlayerInput::default(),
                &rules,
                1.0 / 30.0,
                || {},
            );
        }
        assert!(body.grounded, "the player body never landed");
        // Attached, the body is held `GetThingHeight` (moveSize + 0.005 with no model insert) above it.
        assert!((body.pos.z - 0.07).abs() < 1e-3, "z {} after landing", body.pos.z);
        assert_eq!(body.sector, 0);
    }

    #[test]
    fn step_player_keeps_velocity_when_a_frame_has_leftover_physics_time() {
        let solids = crate::collide::test_room();
        let rt = Runtime::new(&Level::default());
        let mut rules = crate::Rules::from_level(&Level::default());
        rules.drag = 0.0;
        rules.air_drag = 0.0;
        let mut body = walk_body(Vec3::ZERO);
        body.gravity = 0.0;
        body.vel = [1.0, 0.0];
        let rt = World::new(rt);

        Runtime::step_player(&rt,
            &mut body,
            &solids,
            PlayerInput::default(),
            &rules,
            0.03,
            || {},
        );

        // UpdateMove stores distance at +0x264; it does not replace +0x1d4 velocity with distance / dt.
        assert!((body.pos.x - 0.02).abs() < 1e-6, "one 50 Hz step must move the body");
        assert!((body.vel[0] - 1.0).abs() < 1e-6, "leftover frame time must not slow the body");
    }

    #[test]
    fn step_player_reaches_the_walk_speed() {
        let solids = crate::collide::test_room();
        let rt = Runtime::new(&Level::default());
        let rules = crate::Rules::from_level(&Level::default());
        let mut body = walk_body(Vec3::new(1.5, 0.0, -0.935));
        let input = PlayerInput {
            forward: 1.0,
            yaw: 90.0,
            ..PlayerInput::default()
        };
        let rt = World::new(rt);
        for _ in 0..120 {
            Runtime::step_player(&rt,
                &mut body,
                &solids,
                input,
                &rules,
                1.0 / 60.0,
                || {},
            );
        }
        let speed = (body.vel[0] * body.vel[0] + body.vel[1] * body.vel[1]).sqrt();
        assert!(
            speed > rules.walk * 0.9 && speed < rules.walk * 1.01,
            "the walk reached {speed:.3} after two seconds, not near {:.3}",
            rules.walk
        );
        assert!(body.pos.x < 1.5, "the body did not walk: {:?}", body.pos);
    }

    #[test]
    fn step_player_strafes_right() {
        let solids = crate::collide::test_room();
        let rt = Runtime::new(&Level::default());
        let rules = crate::Rules::from_level(&Level::default());
        let base = -1.0 + rules.radius;
        let mut body = walk_body(Vec3::new(-0.5, 0.0, base));
        let input = PlayerInput {
            strafe: 1.0,
            ..PlayerInput::default()
        };
        let rt = World::new(rt);
        for _ in 0..60 {
            Runtime::step_player(&rt,
                &mut body,
                &solids,
                input,
                &rules,
                1.0 / 60.0,
                || {},
            );
        }
        assert!(body.pos.x > -0.4, "D did not move the body right: {:?}", body.pos);
    }

    #[test]
    fn step_player_jumps_and_lands() {
        let solids = crate::collide::test_room();
        let rt = Runtime::new(&Level::default());
        let rules = crate::Rules::from_level(&Level::default());
        // Standing attached: `GetThingHeight` above the floor (moveSize + 0.005 with no model insert).
        let base = -1.0 + rules.radius + 0.005;
        let mut body = walk_body(Vec3::new(0.0, 0.0, base));
        let mut peak = base;
        let rt = World::new(rt);
        for frame in 0..90 {
            let input = PlayerInput {
                jump: frame == 0,
                ..PlayerInput::default()
            };
            Runtime::step_player(&rt,
                &mut body,
                &solids,
                input,
                &rules,
                1.0 / 60.0,
                || {},
            );
            peak = peak.max(body.pos.z);
        }
        assert!(peak > base + 0.1, "the jump never rose: peak {peak:.3} from {base:.3}");
        assert!(body.grounded, "the jump never landed");
        assert!(
            (body.pos.z - base).abs() < 1e-3,
            "landed at {:.3}, not {base:.3}",
            body.pos.z
        );
    }

    /// A runtime whose level holds the local player, so a step can publish onto its thing.
    fn player_runtime() -> Runtime {
        let mut level = Level::default();
        level.templates.push(crate::jkl::Template {
            name: "walkplayer".into(),
            base: None,
            params: vec![
                ("health".into(), "100".into()), ("type".into(), "player".into()),
                ("move".into(), "physics".into()), ("collide".into(), "1".into()),
                ("physflags".into(), "0x4a4f".into()),
                ("mass".into(), "150".into()),
            ],
            model3d: None,
            sprite: None,
        });
        level.things.push(crate::jkl::Thing {
            template: "walkplayer".into(),
            pos: [0.0; 3],
            pitch: 0.0,
            yaw: 0.0,
            roll: 0.0,
            sector: 0,
            flags: 0,
            frames: Vec::new(),
            vel: [0.0; 3],
            aiclass: None,
        });
        Runtime::new(&level)
    }

    fn slide_surface() -> SurfaceState {
        SurfaceState {
            center: [0.0; 3],
            cel: 0.0,
            anim: 0.0,
            slide: [1.0, 0.0, 0.0],
            next: 0.0,
            light: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
        }
    }

    /// `test_room`'s floor plane carries no surface id; `floor_surface` and the water/carry rules
    /// need one, so this names surface 0 on the same geometry.
    fn floor_room() -> crate::collide::Solids {
        let mut room = crate::collide::test_room();
        room.sectors[0][0].surface = 0;
        room
    }

    #[test]
    fn step_player_attaches_to_a_live_path_face_and_rides_both_directions() {
        use crate::collide::{Face, ThingSolid};

        let solids = crate::collide::test_room();
        let mut rt = player_runtime();
        let mut lift = rt.things[0].clone();
        lift.pos = [0.0; 3];
        lift.vel = [0.0; 3];
        lift.flags = 0x40;
        lift.frame = 0.0;
        lift.frames = vec![[0.0; 6], [0.0, 0.0, 0.5, 0.0, 0.0, 0.0]];
        lift.target = Some((1.0, 6.0));
        let parent = rt.things.len();
        rt.things.push(lift);
        rt.templates.push("testplatform".into());
        rt.types.push(7);
        rt.template_move.insert("testplatform".into(), "path".into());
        rt.template_height.insert("walkplayer".into(), 0.125);
        let pose = |z| solids.set_things(vec![ThingSolid {
            index: parent, centre: [0.0, 0.0, z], origin: [0.0, 0.0, z], radius: 0.75,
            z_min: z - 0.05, z_max: z, sector: 0, half: None, yaw: 0.0,
            mesh: Some(vec![vec![Face {
                normal: [0.0, 0.0, 1.0], point: [-0.5, -0.5, z],
                verts: vec![[-0.5, -0.5, z], [0.5, -0.5, z], [0.5, 0.5, z], [-0.5, 0.5, z]],
                surface: usize::MAX,
            }]].into()),
            thing_flags: 0x40, collide: 3, owner: None, kind: 7, item: false,
        }]);
        pose(0.0);
        let rules = crate::Rules::from_level(&Level::default());
        let mut body = walk_body(Vec3::new(0.0, 0.0, rules.radius + 0.015));
        body.grounded = false;
        body.gravity = 0.0;
        body.vy = -0.5;
        let world = World::new(rt);
        let tick = |body: &mut PlayerBody| {
            Runtime::step_player(&world, body, &solids, PlayerInput::default(), &rules, 0.02, || {});
        };
        tick(&mut body);
        let target = body.attach.and_then(|a| a.thing).expect("mesh-floor attachment");
        assert_eq!(target.thing, parent);
        assert!((world.borrow().things[0].health - 55.0).abs() < 0.1, "impact is relative to the path's +3 u/s");
        for (frame, z) in [(0.6, 0.3), (0.4, 0.2)] {
            {
                let mut rt = world.borrow_mut();
                rt.things[parent].target = Some((frame, 6.0));
                assert!(rt.things[parent].step(0.2));
                assert_eq!(rt.things[parent].pos[2], z);
            }
            pose(z);
            for _ in 0..5 { tick(&mut body); }
            assert_eq!(body.attach.and_then(|a| a.thing), Some(target));
            assert!((body.pos.z - z - 0.125).abs() < 0.015, "body z {} on lift z {z}", body.pos.z);
        }
        assert!((world.borrow().things[0].health - 55.0).abs() < 0.1, "riding must not repeat landing damage");
    }

    #[test]
    fn step_player_carries_a_body_on_a_scrolling_surface() {
        let solids = floor_room();
        let mut rt = Runtime::new(&Level::default());
        rt.surfaces.push(slide_surface());
        rt.surface_flags = vec![0x800];
        let rules = crate::Rules::from_level(&Level::default());
        let base = -1.0 + rules.radius;
        let mut body = walk_body(Vec3::new(0.0, 0.0, base));
        // A conveyor acts on its attached rider, not on an airborne sphere over its plane.
        body.attach = Some(crate::collide::Attach {
            normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, -1.0], surface: Some(0), thing: None, friction: 1.0,
        });
        let rt = World::new(rt);
        for _ in 0..30 {
            Runtime::step_player(&rt,
                &mut body,
                &solids,
                PlayerInput::default(),
                &rules,
                1.0 / 60.0,
                || {},
            );
        }
        assert!(
            body.pos.x > 0.2,
            "the conveyor did not carry the body: x {:.3}",
            body.pos.x
        );
    }

    #[test]
    fn step_player_lifts_a_body_in_a_thrusting_sector() {
        let solids = crate::collide::test_room();
        let mut rt = Runtime::new(&Level::default());
        rt.sectors.push(SectorState {
            thrust: [0.0, 0.0, 20.0],
            light: 0.0,
            light_from: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
            tint: [0.0; 3],
            colormap: 0,
        });
        let rules = crate::Rules::from_level(&Level::default());
        let base = -1.0 + rules.radius;
        let mut body = walk_body(Vec3::new(0.0, 0.0, base));
        let rt = World::new(rt);
        for _ in 0..30 {
            Runtime::step_player(&rt,
                &mut body,
                &solids,
                PlayerInput::default(),
                &rules,
                1.0 / 60.0,
                || {},
            );
        }
        assert!(
            body.pos.z > base + 0.1,
            "the updraft did not lift the body: z {:.3} from {base:.3}",
            body.pos.z
        );
    }

    #[test]
    fn step_player_halves_the_walk_in_water() {
        let solids = floor_room();
        let rules = crate::Rules::from_level(&Level::default());
        let input = PlayerInput {
            forward: 1.0,
            yaw: 90.0,
            ..PlayerInput::default()
        };
        let base = -1.0 + rules.radius;
        let speed = |rt: Runtime, body: &mut PlayerBody| {
            let rt = World::new(rt);
            for _ in 0..120 {
                Runtime::step_player(&rt,
                    body,
                    &solids,
                    input,
                    &rules,
                    1.0 / 60.0,
                    || {},
                );
            }
            (body.vel[0] * body.vel[0] + body.vel[1] * body.vel[1]).sqrt()
        };
        let dry_rt = Runtime::new(&Level::default());
        let mut dry = walk_body(Vec3::new(1.5, 0.0, base));
        let dry_speed = speed(dry_rt, &mut dry);
        let mut wet_rt = Runtime::new(&Level::default());
        // `SITH_SURFACE_WATER` 0x20000 on the test room's floor (surface 0).
        wet_rt.surface_flags = vec![0x2_0000];
        let mut wet = walk_body(Vec3::new(1.5, 0.0, base));
        let wet_speed = speed(wet_rt, &mut wet);
        assert!(wet.water, "the body on a water surface is not wet");
        assert!(
            (wet_speed - dry_speed * 0.5).abs() < dry_speed * 0.1,
            "water walked at {wet_speed:.3} against {dry_speed:.3} dry"
        );
    }

    #[test]
    fn step_player_publishes_its_final_pose() {
        let solids = crate::collide::test_room();
        let rt = player_runtime();
        let rules = crate::Rules::from_level(&Level::default());
        let base = -1.0 + rules.radius;
        let mut body = walk_body(Vec3::new(0.0, 0.0, base));
        body.eye = 0.5;
        body.yaw = std::f32::consts::FRAC_PI_2;
        body.pitch = 0.25;
        let input = PlayerInput {
            forward: 1.0,
            yaw: body.yaw,
            ..PlayerInput::default()
        };
        let rt = World::new(rt);
        for _ in 0..10 {
            Runtime::step_player(&rt,
                &mut body,
                &solids,
                input,
                &rules,
                1.0 / 60.0,
                || {},
            );
        }
        let rt = rt.borrow();
        let player = rt.local_player().expect("the level's walkplayer");
        let thing = &rt.things[player];
        assert!((thing.pos[0] - body.pos.x).abs() < 1e-4, "x {:?} vs {:?}", thing.pos, body.pos);
        assert!((thing.pos[1] - body.pos.y).abs() < 1e-4, "y {:?} vs {:?}", thing.pos, body.pos);
        assert!(
            (thing.pos[2] - (body.pos.z + body.eye)).abs() < 1e-4,
            "the thing is not the eye point: {:?} vs {:?}",
            thing.pos,
            body.pos
        );
        assert!((thing.yaw - 90.0).abs() < 1e-3, "yaw {}", thing.yaw);
        assert!((thing.pitch - body.pitch.to_degrees()).abs() < 1e-3, "pitch {}", thing.pitch);
        assert_eq!(thing.sector, body.sector as i32);
    }

    #[test]
    fn a_hard_floor_contact_charges_only_when_surface_bit_0x80_is_clear() {
        let solids = floor_room();
        let rules = crate::Rules::from_level(&Level::default());
        for (flags, health) in [(0, 55.0), (0x80, 100.0)] {
            let mut rt = player_runtime();
            rt.surface_flags = vec![flags];
            let rt = World::new(rt);
            let base = -1.0 + rules.radius;
            let mut body = walk_body(Vec3::new(0.0, 0.0, base + 0.05));
            body.grounded = false;
            body.gravity = 0.0;
            // The actual collision takes the pre-restitution speed: (3.5 - 2.5)^2 * 45.
            body.vy = -3.5;
            Runtime::step_player(&rt, &mut body, &solids, PlayerInput::default(), &rules, 0.02, || {});
            assert!(body.vy >= 0.0, "the sweep hit the floor");
            let rt = rt.borrow();
            let player = rt.local_player().unwrap();
            assert_eq!(rt.things[player].health, health, "surface flags {flags:#x}");
        }
    }

    #[test]
    fn a_thing_contact_exchanges_force_before_charging_the_mover() {
        let solids = floor_room();
        let mut rules = crate::Rules::from_level(&Level::default());
        rules.air_drag = 0.0;
        let mut rt = player_runtime();
        let x = rules.radius + 0.1 + 0.04;
        let mut target = rt.things[0].clone();
        target.pos = [x, 0.0, 0.0];
        target.flags = 0x80; // Unlike a surface, a thing never suppresses the charge.
        rt.things.push(target);
        rt.templates.push("walkplayer".into());
        rt.types.push(2);
        rt.mass.insert(1, 150.0);
        solids.set_things(vec![crate::collide::ThingSolid {
            index: 1, centre: [x, 0.0, 0.0], origin: [x, 0.0, 0.0], radius: 0.1,
            z_min: -0.1, z_max: 0.1, sector: 0, half: None, yaw: 0.0, mesh: None,
            thing_flags: 0x80, collide: 1, kind: 2, owner: None, item: false,
        }]);
        let rt = World::new(rt);
        let mut body = walk_body(Vec3::ZERO);
        body.grounded = false;
        body.gravity = 0.0;
        body.vel = [4.0, 0.0];
        Runtime::step_player(&rt, &mut body, &solids, PlayerInput::default(), &rules, 0.02, || {});
        let rt = rt.borrow();
        // Equal masses, no bounce flags: (4 * 0.5 * 0.5) = 1 u/s transferred.
        assert_eq!(rt.things[1].vel, [1.0, 0.0, 0.0]);
        assert_ne!(rt.things[1].phys_flags & 0x8000, 0);
        assert_ne!(rt.things[0].phys_flags & 0x8000, 0);
        assert!(body.vel[0].abs() < 0.001, "{:?}", body.vel);
        // The fresh contact charges for 3 u/s, not the original 4 or the rest position.
        assert_eq!(rt.things[0].health, 88.75);
    }

    #[test]
    fn a_moving_throwable_crate_pushes_another_crate_during_the_tick() {
        for mover in [0, 1] {
            let target = 1 - mover;
            let solids = floor_room();
            let mut rt = player_runtime();
            let mut crate_body = rt.things[0].clone();
            crate_body.phys_flags = 0x404041;
            rt.things = vec![crate_body; 2];
            rt.things[mover].vel[0] = 4.0;
            rt.things[target].pos[0] = 0.24;
            rt.templates = vec!["_throwable".into(); 2];
            rt.types = vec![4; 2];
            rt.template_move.insert("_throwable".into(), "physics".into());
            rt.template_collide.insert("_throwable".into(), 1);
            rt.template_hit.insert("_throwable".into(), (0.1, 0.1));
            rt.mass = [(0, 25.0), (1, 25.0)].into();
            rt.gravity = 0.0;
            solids.set_things(rt.things.iter().enumerate().map(|(index, t)| crate::collide::ThingSolid {
                index, centre: t.pos, origin: t.pos, radius: 0.1, z_min: -0.1, z_max: 0.1, sector: 0,
                half: None, yaw: 0.0, mesh: None, thing_flags: 0, collide: 1, kind: 4, owner: None, item: false,
            }).collect());

            rt.step(Some(&solids), 0.02);

            assert_eq!(rt.things[target].vel, [1.0, 0.0, 0.0], "equal masses transfer one quarter of the approach");
            assert!(rt.things[mover].vel[0].abs() < 0.001, "the mover also runs CollideHurt");
            if target < mover {
                rt.step(Some(&solids), 0.02);
            }
            assert!((rt.things[target].pos[0] - 0.26).abs() < 1e-6, "the other crate's next move uses that impulse");
        }
    }

    #[test]
    fn integrate_walk_keeps_the_fixed_fifty_hz_substeps() {
        let mut vel = [0.0f32; 2];
        let mut rollover = 0.0;
        let mut steps = 0;
        for _ in 0..60 {
            steps += integrate_walk(&mut vel, [1.0, 0.0], 3.2, 1.0 / 60.0, &mut rollover);
        }
        assert_eq!(steps, 50, "one second at 60 fps was not fifty 0.02 s sub-steps");
        let mut vel = [0.0f32; 2];
        let mut rollover = 0.0;
        assert_eq!(
            integrate_walk(&mut vel, [1.0, 0.0], 3.2, 0.1, &mut rollover),
            5,
            "a 0.1 s frame was not five 0.02 s sub-steps"
        );
        // The engine's exponential approach settles at the target: ten seconds at 60 fps.
        let mut vel = [0.0f32; 2];
        let mut rollover = 0.0;
        for _ in 0..600 {
            integrate_walk(&mut vel, [1.2, 0.0], 3.2, 1.0 / 60.0, &mut rollover);
        }
        let settled = vel[0];
        for _ in 0..60 {
            integrate_walk(&mut vel, [1.2, 0.0], 3.2, 1.0 / 60.0, &mut rollover);
        }
        assert!(
            (vel[0] - settled).abs() < 1e-3,
            "the walk was still moving at {settled:.3} -> {:.3}",
            vel[0]
        );
        // The measured retail band is 1.07-1.23 u/s; the engine's decay lands inside it.
        assert!(
            (1.07..=1.23).contains(&settled),
            "the walk settled at {settled:.3}, outside the retail band"
        );
    }

    #[test]
    fn a_frame_path_reports_its_arrival() {
        let mut rt = door_runtime();
        // The local player's path move is the walker's; use a door so `step` owns the pose.
        rt.templates = vec!["door1".into()];
        rt.thing_links = vec![(5, 0, 0)];
        rt.things[0].target = Some((1.0, 10.0));
        let motion = rt.step(None, 0.2);
        assert_eq!(motion.arrived, vec![0], "the arriving thing was not reported");
        assert!(motion.moved.contains(&0), "the path move was not reported");
        assert!(
            rt.pending_messages
                .iter()
                .any(|(instance, label, _, _, _)| *instance == 5 && *label == "arrived"),
            "the arrival was not queued for the linked cog: {:?}",
            rt.pending_messages
        );
        assert_eq!(rt.things[0].frame, 1.0);
        assert_eq!(rt.things[0].target, None);
        let next = rt.step(None, 0.2);
        assert!(next.arrived.is_empty(), "the arrival was reported twice");
        assert_eq!(
            rt.pending_messages
                .iter()
                .filter(|(_, label, _, _, _)| *label == "arrived")
                .count(),
            1,
            "the arrival was queued twice"
        );
    }

    /// A lit saber is the block's own gate: with the blade on, the range falloff, the darkness term
    /// and the pose terms are skipped, so a still, crouched body 20 units away in a dark room is as
    /// noticeable as one at arm's length.
    #[test]
    fn a_lit_saber_makes_the_body_obvious() {
        let chance = |saber: bool| {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut actor = base.clone();
            actor.pos = [0.0, 0.0, 0.0];
            let mut player = base.clone();
            player.pos = [20.0, 0.0, 0.0];
            player.vel = [0.0; 3];
            player.phys_flags = 0x10000;
            player.sector = 0;
            rt.things = vec![player, actor];
            rt.things[1].sector = 0;
            rt.templates = vec!["walkplayer".into(), "tuskan".into()];
            rt.types = vec![10, 2];
            rt.template_move.insert("walkplayer".into(), "physics".into());
            rt.ai_flags.insert(1, 0x200);
            rt.sector_ambient_light = vec![0.1];
            if saber {
                rt.saber.insert(
                    0,
                    SaberInfo {
                        side: String::new(),
                        tip: String::new(),
                        base_rad: 0.0,
                        tip_rad: 0.0,
                        length: 1.0,
                        wall_sparks: String::new(),
                        blood_sparks: String::new(),
                        saber_sparks: String::new(),
                    },
                );
            }
            rt.detect_chance(1, 0)
        };
        // Dark, still and crouched at 20 units: 0.4 * 0.3 * 0.75 * 0.5 = 0.045, floored at 0.05.
        assert!((chance(false) - 0.05).abs() < 1e-5, "the dark/pose terms are not applied");
        assert!((chance(true) - 1.0).abs() < 1e-5, "a lit saber did not make the body obvious");
    }

    /// The roll is on **acquisition**: an actor 8 units away with the player standing still has a 0.2
    /// chance a check, so it does not snap to the player instantly — but it does acquire, and after
    /// that the port keeps the deterministic line test (the engine re-rolls on refresh; ledger).
    #[test]
    fn a_ranged_actor_acquires_the_player_with_a_roll() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        let mut player = base.clone();
        player.pos = [8.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_move.insert("walkplayer".into(), "physics".into());
        rt.template_weapon.insert("tuskan".into(), "blaster".into());
        rt.template_vel.insert("blaster".into(), [0.0, 7.0, 0.0]);
        rt.ai_flags.insert(1, 0x200);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=20.0\n\n\
                 PrimaryFire 100.0, 0.0, 5.0, 0.0, 0.0, 500.0, 0.0, 0.0, 1.0, 0.0\n\
                 Follow 0.0, 0.5\n",
            ),
        );
        assert!((rt.detect_chance(1, 0) - 0.2).abs() < 1e-5, "the setup is not a 0.2 roll");
        assert!(!rt.ai_detected.contains(&1), "the actor detected the player before ticking");
        let mut acquired = false;
        for _ in 0..200 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
            if rt.ai_detected.contains(&1) {
                acquired = true;
                break;
            }
        }
        assert!(acquired, "the actor never acquired the player at 8 units");
        // Once registered the port keeps the target: the actor fires at point-blank range without
        // needing another roll.
        rt.things[0].pos = [1.0, 0.0, 0.0];
        for _ in 0..60 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(rt.shots_fired > 0, "a registered actor at point-blank range never fired");
    }

    /// The sight roll runs once per **instinct pass**, not once per frame: `sithAI_InstinctUpdate`
    /// (`0x4e9e90`) sets each instinct's own clock to `now + 1000` before calling it. Rolling every
    /// frame latched a 50 % chance in one or two frames, which is why the port's tuskens fired inside
    /// the first second on level 3 while retail's series recorded no damage at all.
    #[test]
    fn sight_detection_rolls_once_a_second_not_once_a_frame() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        rt.things = vec![base, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_move.insert("walkplayer".into(), "physics".into());
        rt.ai_flags.insert(1, 0x200);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=20.0\n\n\
                 PrimaryFire 100.0, 0.0, 5.0, 0.0, 0.0, 500.0, 0.0, 0.0, 1.0, 0.0\n\
                 Follow 0.0, 0.5\n",
            ),
        );
        rt.things[0].pos = [8.0, 0.0, 0.0];
        assert!((rt.detect_chance(1, 0) - 0.2).abs() < 1e-5, "the setup is not a 0.2 roll");
        rt.time += 1.0 / 30.0;
        rt.ai_tick(0, 1.0 / 30.0);
        if rt.ai_detected.contains(&1) {
            // The first attempt is immediate — the engine's instincts start at `next = 0` — and it may
            // succeed. Then there is nothing left to schedule.
            assert!(rt.ai_detect_at.contains_key(&1));
            return;
        }
        let first = rt.ai_detect_at.get(&1).copied().expect("no attempt was scheduled");
        assert!((first - (rt.time + 1.0)).abs() < 1e-4, "the first attempt was not armed 1 s out");
        // Half a second of frames must not roll again; the next attempt is a whole second away.
        for _ in 0..15 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert_eq!(rt.ai_detect_at.get(&1).copied(), Some(first), "the roll ran again mid-second");
        assert!(!rt.ai_detected.contains(&1), "a failed roll still latched the target");
    }

    /// `turretfire` (`0x50a0f0`): a turret's class (`turdefault.ai`) carries **no `PrimaryFire`**, so
    /// the port used to leave every turret silent; and the handler never calls `SetMovePos`, so a
    /// turret must not walk either.
    #[test]
    fn a_turret_fires_and_does_not_walk() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [2.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "turret".into()];
        rt.types = vec![10, 2];
        // Even at a walking speed a turret must not move.
        rt.template_speed.insert("turret".into(), 1.0);
        rt.template_weapon.insert("turret".into(), "blaster".into());
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=360, sightdist=5.0, heardist=2.0\n\n\
                 TurretFire 750.0, 75.0, 160.0, 160.0, 5.0, 0.20, 500.0, 10000.0, 0.0\n",
            ),
        );
        // Acquisition is a separate roll (`sithAI_CanDetectSightThing`) with the engine's one-per-second
        // cadence; this test is about the TurretFire path, so start it already engaged.
        rt.ai_detected.insert(1);
        rt.sector_adjoins = vec![Vec::new(); 8];
        let emplaced = rt.things[1].pos;
        for _ in 0..30 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(rt.shots_fired > 0, "the turret never fired");
        assert_eq!(rt.things[1].pos, emplaced, "the turret walked");
        // Both awareness kinds a weapon raises: the shot (1) and the weapon's own firing state (2),
        // which `listen` scans first.
        for kind in [1u8, 2] {
            assert!(
                rt.sector_awareness.contains_key(&(7, kind)),
                "firing raised no kind-{kind} awareness record"
            );
        }
    }

    /// `turretfire`'s **damaged** arm (`FUN_0050a0f0`): below `arg5` (0.20) of full health the handler
    /// sweeps its head randomly inside ±arg2/±arg3, fires on its own `arg6` (500 ms) cadence, and
    /// arms a self-destruct at `now + arg7` (10 s) that calls `sithActor_KillActor` on the turret
    /// itself. The port's turret was accurate at any health and never broke down.
    #[test]
    fn a_damaged_turret_sweeps_and_kills_itself() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        actor.health = 10.0; // one tenth of the 100 below: past the 0.20 threshold
        let mut player = base.clone();
        player.pos = [2.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "turret".into()];
        rt.types = vec![10, 2];
        rt.template_health.insert("turret".into(), 100.0);
        rt.template_max_health.insert("turret".into(), 100.0);
        rt.template_weapon.insert("turret".into(), "blaster".into());
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=360, sightdist=5.0, heardist=2.0\n\n\
                 TurretFire 750.0, 75.0, 160.0, 160.0, 5.0, 0.20, 500.0, 10000.0, 0.0\n",
            ),
        );
        rt.ai_detected.insert(1);
        rt.sector_adjoins = vec![Vec::new(); 8];
        // Three seconds: the turret fires on the damaged cadence and is still alive.
        for _ in 0..90 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(rt.shots_fired > 0, "the damaged turret never fired");
        assert!(!rt.things[1].destroyed, "the turret died before its self-destruct delay");
        // Past the ten-second deadline the handler kills its own body.
        for _ in 0..330 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            rt.things[1].destroyed,
            "the damaged turret outlived its `arg7` self-destruct delay"
        );

        // A healthy turret never arms it.
        let mut healthy = door_runtime();
        let base = healthy.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.health = 100.0;
        let mut player = base.clone();
        player.pos = [2.0, 0.0, 0.0];
        healthy.things = vec![player, actor];
        healthy.templates = vec!["walkplayer".into(), "turret".into()];
        healthy.types = vec![10, 2];
        healthy.template_health.insert("turret".into(), 100.0);
        healthy.template_max_health.insert("turret".into(), 100.0);
        healthy.template_weapon.insert("turret".into(), "blaster".into());
        healthy.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=360, sightdist=5.0, heardist=2.0\n\n\
                 TurretFire 750.0, 75.0, 160.0, 160.0, 5.0, 0.20, 500.0, 10000.0, 0.0\n",
            ),
        );
        healthy.ai_detected.insert(1);
        for _ in 0..420 {
            healthy.time += 1.0 / 30.0;
            healthy.ai_tick(0, 1.0 / 30.0);
        }
        assert!(!healthy.things[1].destroyed, "a healthy turret destroyed itself");
    }

    /// `withdraw` reads `ai[0x70]`, and `sensedanger` (`0x50b9b0`) is what writes the player into it
    /// when a surprise succeeds (`param_1[0x70] = player`, alongside `flags & ~0x4 | 0x800`). So a class
    /// that ships **`Withdraw` without `Retreat`** — six of the thirty in the install — still arms the
    /// withdraw mode from a surprise. (The handler's own move, `0x50b150`, is not ported.)
    #[test]
    fn a_surprised_actor_with_only_withdraw_arms_the_withdraw_mode() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        let mut player = base.clone();
        player.pos = [1.0, 0.0, 0.0]; // inside the two-unit always-noticed radius
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 SenseDanger 0.0, 1.0\n\
                 Withdraw 5000.0, 1.0\n",
            ),
        );
        // Awake (`AISetMode(..., 0x4)`) and not yet alerted: what `sensedanger` acts on.
        rt.ai_flags.insert(1, 0x4);
        rt.ai_tick(0, 1.0 / 30.0);
        let flags = rt.ai_flags.get(&1).copied().unwrap_or(0);
        assert_eq!(flags & 0x800, 0x800, "the surprise did not set the alert bit: {flags:#x}");
        assert!(
            rt.ai_withdraw.contains_key(&1),
            "`sensedanger` did not store the player as the withdraw target"
        );
    }

    /// `retreat` (`0x50bc60`) arms a withdraw when health falls under `arg0 × max` and its `arg1`
    /// chance succeeds; the mode lasts while its target exists. Shipped lines: `retreat 0.50, 0.50,
    /// 2000, 1.0` and `withdraw 5000, 1.0`.
    #[test]
    fn a_hurt_actor_with_retreat_arms_the_withdraw_mode() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.health = 10.0; // 10% of its 100 max
        let mut player = base.clone();
        player.pos = [2.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.template_max_health.insert("tuskan".into(), 100.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 Retreat 0.50, 1.0, 2000.0, 1.0\n\
                 Withdraw 5000.0, 1.0\n",
            ),
        );

        // `retreat`'s registry word is 2 (searching), reject 0x800.
        rt.ai_flags.insert(1, 0x2);
        rt.ai_tick(0, 1.0 / 30.0);
        assert!(
            rt.ai_withdraw.contains_key(&1),
            "a hurt actor with `Retreat` did not arm the withdraw mode"
        );

        // The instinct's 5000 ms is how often the handler runs, **not** a lifetime: `FUN_0050b150`
        // clears the mode on `ai[0x70] == 0`, so the withdrawal lasts while the target exists. A run of
        // frames past the interval must keep it armed, and losing the target must clear it.
        rt.time += 6.0;
        rt.ai_tick(0, 1.0 / 30.0);
        assert!(
            rt.ai_withdraw.contains_key(&1),
            "the withdraw mode expired on the instinct's interval, which is only its call rate"
        );
        // The target is gone; the actor is still alive and ticking.
        rt.things[0].destroyed = true;
        rt.ai_tick(0, 1.0 / 30.0);
        assert!(
            !rt.ai_withdraw.contains_key(&1),
            "the withdraw mode survived the loss of its target"
        );

        // A healthy actor with the same class never arms the mode — and with the body out of its
        // 2-unit sight it has nothing else to walk to, so it stays put.
        let mut healthy = door_runtime();
        let base = healthy.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.health = 100.0;
        let mut player = base.clone();
        player.pos = [10.0, 0.0, 0.0];
        healthy.things = vec![player, actor];
        healthy.templates = vec!["walkplayer".into(), "tuskan".into()];
        healthy.types = vec![10, 2];
        healthy.template_speed.insert("tuskan".into(), 1.0);
        healthy.template_max_health.insert("tuskan".into(), 100.0);
        healthy.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=2.0\n\n\
                 Retreat 0.50, 1.0, 2000.0, 1.0\n\
                 Withdraw 5000.0, 1.0\n",
            ),
        );
        for _ in 0..30 {
            healthy.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            !healthy.ai_withdraw.contains_key(&1),
            "a healthy actor armed the withdraw mode"
        );
        assert_eq!(healthy.things[1].pos[0], 0.0, "a healthy actor walked away");
    }

    /// A script's `AIClearMode(actor, 0x606)` switches the actor's instincts **off**: no sight,
    /// no acquisition, no shot, even with a clear line inside `sightdist` - exactly level 1's intro
    /// Ry, whose `01_initialry.cog` starts with that call before it walks the body into the bar.
    /// `AISetMode(actor, 0x200)` (the bit `00_aisleep.cog` wakes with) turns them back on.
    #[test]
    fn a_script_that_clears_the_ai_modes_stops_acquiring_and_the_wake_mode_restores_it() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        let mut player = base.clone();
        player.pos = [8.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_move.insert("walkplayer".into(), "physics".into());
        rt.ai_flags.insert(1, 0x1004);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=20.0\n\n\
                 PrimaryFire 100.0, 0.0, 5.0, 0.0, 0.0, 500.0, 0.0, 0.0, 1.0, 0.0\n\
                 Follow 0.0, 0.5\n",
            ),
        );
        rt.ai_home.insert(1, [0.0, 0.0, 0.0]);
        rt.call("AIClearMode", &[Value::Num(1.0), Value::Num(0x606 as f64)]);
        let mut acquired = false;
        for _ in 0..200 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
            acquired |= rt.ai_detected.contains(&1);
        }
        assert!(!acquired, "a script-disabled actor acquired the player");
        assert_eq!(rt.shots_fired, 0, "a script-disabled actor fired");
        rt.call("AISetMode", &[Value::Num(1.0), Value::Num(0x200 as f64)]);
        for _ in 0..200 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
            if rt.ai_detected.contains(&1) {
                acquired = true;
                break;
            }
        }
        assert!(acquired, "the wake mode did not re-enable the AI");
    }

    /// `returnhome` (`0x50bd70`) event `0x100`: an actor that just woke (awake bit set, clear in
    /// the old word) orders `SetMoveThing(home, 1.0)` and looks at home; the body walks there.
    #[test]
    fn an_actor_with_returnhome_walks_back_to_its_anchor() {
        let homer = |class: &str| {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut actor = base.clone();
            actor.pos = [2.0, 0.0, 0.0];
            let mut player = base.clone();
            // Far and behind: the actor has nothing to chase and nothing to remember.
            player.pos = [-9.0, 0.0, 0.0];
            rt.things = vec![player, actor];
            rt.templates = vec!["walkplayer".into(), "tuskan".into()];
            rt.types = vec![10, 2];
            rt.template_maxthrust.insert("tuskan".into(), 1.0);
            rt.ai.insert(1, crate::ai::parse(class));
            rt.ai_home.insert(1, [0.0, 0.0, 0.0]);
            rt
        };
        let mut rt = homer("alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nReturnHome\n");
        // Without the awake bit nothing wakes and nothing walks.
        ai_frame(&mut rt, 0, 1.0 / 30.0);
        assert_eq!(rt.things[1].pos[0], 2.0, "an actor that never woke walked home anyway");
        rt.call("AISetMode", &[Value::Num(1.0), Value::Num(0x4 as f64)]);
        assert_eq!(rt.ai_move.get(&1), Some(&[0.0, 0.0, 0.0]), "waking did not order the walk home");
        for _ in 0..30 {
            ai_frame(&mut rt, 0, 1.0 / 30.0);
        }
        let walked = rt.things[1].pos[0];
        assert!(walked < 1.9, "the awake actor did not walk home: x {walked:.2}");

        // A control class without `ReturnHome` stays where it was put.
        let mut quiet = homer("alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nJump\n");
        quiet.call("AISetMode", &[Value::Num(1.0), Value::Num(0x4 as f64)]);
        for _ in 0..30 {
            ai_frame(&mut quiet, 0, 1.0 / 30.0);
        }
        assert_eq!(quiet.things[1].pos[0], 2.0, "an actor without `ReturnHome` walked away");
    }

    /// The AI's facing is the engine's `(-sin yaw, cos yaw)`, not a math angle from +X. An actor
    /// already aimed at the player therefore fires on its first tick with its yaw unchanged, and the
    /// bolt leaves along +Y — which is exactly what retail's own door gunner does (placed at yaw 0,
    /// firing down +Y, turning the two degrees onto the body instead of a quarter turn).
    #[test]
    fn an_actor_already_aimed_at_the_player_fires_on_the_first_tick() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0; // engine forward is +Y
        let mut player = base.clone();
        player.pos = [0.0, 1.5, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "reeyeesgun".into()];
        rt.types = vec![10, 2];
        rt.template_move.insert("walkplayer".into(), "physics".into());
        rt.template_weapon.insert("reeyeesgun".into(), "blaster".into());
        rt.template_vel.insert("blaster".into(), [0.0, 8.0, 0.0]);
        rt.template_damage.insert("blaster".into(), 10.0);
        rt.ai_flags.insert(1, 0x200);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 PrimaryFire 750.0, 0.8, 4.0, 0.0, 0.0, 500.0, 0.0, 0.0, 1.0, 0.0\n",
            ),
        );
        let fired = rt.ai_tick(0, 1.0 / 60.0);
        assert_eq!(fired, 1, "an actor already facing the player did not fire");
        assert!(
            rt.things[1].yaw.abs() < 0.5,
            "the actor turned {} degrees onto a target it was already facing",
            rt.things[1].yaw
        );
        let shot = rt.things.last().expect("a projectile");
        assert!(shot.vel[1] > 0.0, "the shot left off the +Y axis: {:?}", shot.vel);
        assert!(shot.vel[0].abs() < 1e-3, "the shot left off the +Y axis: {:?}", shot.vel);
    }

    /// `blindfire` (`0x509b30`) fires at the **remembered** position when the body is out of sight:
    /// the engine tests that spot against the class's cos limit and a fixed 10-unit reach, rolls the
    /// instinct's chance and fires with its spread. The shipped line is
    /// `750, 0.80, 0, 0.8, 0.0, 0.30` — delay, chance, -, cos limit, minimum distance, spread.
    #[test]
    fn an_actor_that_lost_sight_fires_at_the_remembered_position() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        // Behind the actor and out of its 2-unit sight: it cannot see the body any more.
        player.pos = [-5.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_weapon.insert("tuskan".into(), "blaster".into());
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, maxstep=0.3, sightdist=2.0, heardist=2.0, accuracy=0.15\n\n\
                 PrimaryFire 1500.0, 0.8, 4.0, 0.20, 0.0, 500.0, 0.00, 0.20, 0.00, 0.00\n\
                 BlindFire 750.0, 1.0, 0, 0.8, 0.0, 0.30\n",
            ),
        );
        // It remembers the body three units ahead — out of sight, inside the blind reach.
        rt.ai_last_seen.insert(1, [3.0, 0.0, 0.0]);

        let fired = rt.ai_tick(0, 0.1);
        assert_eq!(fired, 1, "the actor did not blind-fire at the remembered position");
        assert!(rt.shots_fired >= 1, "no projectile was spawned for the blind shot");

        // A failed roll is the engine's **1 s** retry, not a re-roll at the shot cadence: the handler
        // sets its next call to `now + 1000` when the chance misses.
        let mut miss = door_runtime();
        let base = miss.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [-5.0, 0.0, 0.0];
        miss.things = vec![player, actor];
        miss.templates = vec!["walkplayer".into(), "tuskan".into()];
        miss.types = vec![10, 2];
        miss.template_weapon.insert("tuskan".into(), "blaster".into());
        miss.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, maxstep=0.3, sightdist=2.0, heardist=2.0, accuracy=0.15\n\n\
                 BlindFire 750.0, 0.0, 0, 0.8, 0.0, 0.30\n",
            ),
        );
        miss.ai_last_seen.insert(1, [3.0, 0.0, 0.0]);
        assert_eq!(miss.ai_tick(0, 0.1), 0, "a zero-chance blind fire still fired");
        assert!(
            (miss.things[1].ai_wait - 1.0).abs() < 1e-4,
            "a failed blind-fire roll did not schedule the engine's 1 s retry: {}",
            miss.things[1].ai_wait
        );

        // A control class without `BlindFire` stays silent with the same memory.
        let mut quiet = door_runtime();
        let base = quiet.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0;
        let mut player = base.clone();
        player.pos = [-5.0, 0.0, 0.0];
        quiet.things = vec![player, actor];
        quiet.templates = vec!["walkplayer".into(), "tuskan".into()];
        quiet.types = vec![10, 2];
        quiet.template_weapon.insert("tuskan".into(), "blaster".into());
        quiet.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, maxstep=0.3, sightdist=2.0, heardist=2.0, accuracy=0.15\n\n\
                 PrimaryFire 1500.0, 0.8, 4.0, 0.20, 0.0, 500.0, 0.00, 0.20, 0.00, 0.00\n",
            ),
        );
        quiet.ai_last_seen.insert(1, [3.0, 0.0, 0.0]);
        assert_eq!(quiet.ai_tick(0, 0.1), 0, "an actor without `BlindFire` fired");
    }

    /// `sithPlayerActions_Activate` (`0x4fc0a0`): the use sweep takes a linked thing unless it is a
    /// **weapon (type 3) or an item (type 5)** — those are fired or picked up, not used — and its
    /// reach is `moveSize + 0.1` (the `-0.1` at `0x523200` subtracted, not a second 0.1 sphere), **0.16** units
    /// for the player. The old port used 3.0, which is why its cantina switch fired from 0.8 units
    /// where retail's own use action does nothing.
    #[test]
    fn the_use_action_skips_weapons_and_stops_at_the_engine_reach() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut bolt = base.clone();
        bolt.pos = [-0.1, 0.0, 0.0]; // nearer, but a weapon: fired, not used
        bolt.flags |= TF_CAPTURED;
        let mut door = base.clone();
        door.pos = [-0.1, 0.0, 0.0]; // inside the 0.16 sweep
        door.flags |= TF_CAPTURED; // `sithCog_LinkCog` (`0x4df610`) sets it on every linked thing
        rt.things = vec![base.clone(), bolt, door];
        rt.templates = vec!["walkplayer".into(), "+bryarbolt".into(), "door".into()];
        rt.types = vec![10, 3, 0];
        rt.thing_links.push((0, 1, 1));
        rt.thing_links.push((0, 2, 2));
        let used = rt.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 7);
        assert_eq!(
            used,
            Some(UseHit::Thing(2)),
            "the use action took a weapon instead of the door: {used:?}"
        );
        assert!(
            rt.pending_messages
                .iter()
                .any(|(_, label, _, sender, _)| *label == "activate" && *sender == Some(2)),
            "the door's script was not messaged: {:?}",
            rt.pending_messages
        );

        // The same door one step further out is beyond the sweep and is not used.
        let mut far = door_runtime();
        let mut door = far.things[0].clone();
        door.pos = [-0.5, 0.0, 0.0];
        door.flags |= TF_CAPTURED;
        far.things = vec![far.things[0].clone(), door];
        far.templates = vec!["walkplayer".into(), "door".into()];
        far.types = vec![10, 0];
        far.thing_links.push((0, 2, 1));
        assert_eq!(
            far.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 7),
            None,
            "a door half a unit away was used past the engine's 0.16 sweep"
        );

        // A thing without a cog link is not a candidate: `sithCog_LinkCog` never set 0x400 on it.
        let mut unlinked = door_runtime();
        let mut door = unlinked.things[0].clone();
        door.pos = [-0.1, 0.0, 0.0];
        unlinked.things = vec![unlinked.things[0].clone(), door];
        unlinked.templates = vec!["walkplayer".into(), "door".into()];
        unlinked.types = vec![10, 0];
        unlinked.thing_links.push((0, 2, 1));
        assert_eq!(
            unlinked.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 7),
            None,
            "an unlinked thing answered the use action"
        );
    }

    /// The two poses level 1's exit route depends on, at their measured positions.
    ///
    /// The door piece (`thing 4`, `2x3door`) is at `(1.200002, 9.650073, 6.35)`. The route warps to
    /// eye `(1.28, 9.62, 6.357114)`, sector 60, yaw 105 (west) — a **port stop pose derived from the
    /// level geometry**, not a retail measurement: committed `walkplayer` placement 174's eye is
    /// `(1.400084, 9.700065, 6.357114)`, 0.206 from the leaf and outside the reach, and no walktrace
    /// of the exit door exists. From the derived pose the eye is ~0.086 from the door origin, inside
    /// the `moveSize + 0.1 = 0.165` reach (`sithPlayerActions_Activate`, `0x4fc0a0`), and the facing
    /// dot is 0.81. The earlier `walk_door0` stop pose `(1.30, 9.14)` facing north is 0.52 away: the
    /// pose, not the reach, is why that walk never opened the door.
    ///
    /// The one-way door (`thing 47`, `00_1waydoor.cog`) is used from the tunnel (sector 407, what the
    /// script's `inside` symbol names); its own origin is 0.087-0.116 from the body resting against
    /// the shut leaf in the door's stub (sector 223, the route's next sector), inside the same reach.
    ///
    /// Both bodies stand in a **different** sector from their target, so each fixture carries the
    /// doorway plane the 0.165 sweep crosses: the sector scope here is the engine's search
    /// (`sithCollision_SearchForCollisions`), not an equality test.
    #[test]
    fn the_level_1_door_poses_are_inside_the_use_sweep() {
        // One doorway plane standing in for the level's own: the sweep from the body's room reaches
        // `room` only across it, which is the engine's rule.
        let with_door = |pos: [f32; 3],
                         room: i32,
                         body: usize,
                         normal: [f32; 3],
                         point: [f32; 3]| {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut door = base.clone();
            door.pos = pos;
            door.sector = room;
            door.flags |= TF_CAPTURED;
            rt.things = vec![base, door];
            rt.templates = vec!["walkplayer".into(), "2x3door".into()];
            rt.types = vec![10, 0];
            rt.thing_links.push((0, 0, 1));
            let mut solids = crate::collide::Solids::default();
            solids.portals = vec![Vec::new(); 408];
            solids.portals[body].push(crate::collide::Portal {
                adjoin: room as usize,
                link: 0,
                verts: Vec::new(),
                normal,
                point,
            });
            rt.solids = Some(std::sync::Arc::new(solids));
            rt
        };

        // Door 0 from the exit warp: eye (1.28, 9.62, 6.357114) in sector 60, the leaf in 74, and the
        // doorway plane half-way along yaw 105's `(-sin, cos)`.
        let yaw = 105f32.to_radians();
        let look = [-yaw.sin(), yaw.cos(), 0.0];
        let plane = [
            1.28 + look[0] * 0.05,
            9.62 + look[1] * 0.05,
            6.357114 + look[2] * 0.05,
        ];
        let mut door0 = with_door([1.200002, 9.650073, 6.35], 74, 60, look, plane);
        let used = door0.sith_player_actions_activate(
            0,
            [1.28, 9.62, 6.357114],
            look,
            0.165,
            60,
        );
        assert_eq!(
            used,
            Some(UseHit::Thing(1)),
            "the door pose did not reach the linked door: {used:?}"
        );
        assert!(
            door0
                .pending_messages
                .iter()
                .any(|(_, label, _, sender, _)| *label == "activate" && *sender == Some(0)),
            "the door's script was not messaged: {:?}",
            door0.pending_messages
        );

        // The `walk_door0` stop pose (north-facing, 0.52 away) is refused by the same runtime.
        let mut stop = with_door([1.200002, 9.650073, 6.35], 74, 60, look, plane);
        assert_eq!(
            stop.sith_player_actions_activate(
                0,
                [1.30, 9.14, 6.357114],
                [0.0, 1.0, 0.0],
                0.165,
                60
            ),
            None,
            "the north-facing stop pose used the door it cannot reach or face"
        );

        // The one-way door from inside the tunnel: the body in 407, the shut leaf in the stub (223).
        let mut tunnel = with_door(
            [-4.05, 13.95, 4.375],
            223,
            407,
            [0.0, 1.0, 0.0],
            [-4.05, 13.915, 4.357114],
        );
        let used = tunnel.sith_player_actions_activate(
            0,
            [-4.05, 13.865, 4.357114],
            [0.0, 1.0, 0.0],
            0.165,
            407,
        );
        assert_eq!(
            used,
            Some(UseHit::Thing(1)),
            "the tunnel pose did not reach the one-way door: {used:?}"
        );

        // Both halves rank together, because the engine pops one stack: a plate nearer the eye than
        // the leaf is the one that answers, and the leaf is not messaged.
        let mut both = with_door([1.200002, 9.650073, 6.35], 74, 60, look, plane);
        both.surfaces = vec![SurfaceState {
            center: [1.20, 9.70, 6.357],
            cel: 0.0,
            anim: 0.0,
            slide: [0.0; 3],
            next: 0.0,
            light: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
        }];
        both.surface_sector = vec![74];
        both.surface_links = vec![(0, 2, 0)];
        let used = both.sith_player_actions_activate(
            0,
            [1.28, 9.70, 6.357],
            [-1.0, 0.0, 0.0],
            0.165,
            60,
        );
        assert_eq!(used, Some(UseHit::Surface(0)), "the nearer plate did not win: {used:?}");
        assert!(
            both.pending_messages.iter().all(|(_, _, _, sender, _)| *sender == Some(2)),
            "the leaf was messaged as well as the plate: {:?}",
            both.pending_messages
        );
    }

    /// The surface half of the same sweep: a linked surface is used when its centre is inside the
    /// engine's 0.16-unit reach and not when it is outside — the switch case the port over-reached
    /// from 0.8 units before round 271.
    #[test]
    fn the_surface_use_stops_at_the_engine_reach() {
        let surface = |center: [f32; 3]| SurfaceState {
            center,
            cel: 0.0,
            anim: 0.0,
            slide: [0.0; 3],
            next: 0.0,
            light: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
        };
        let mut near = door_runtime();
        near.surfaces = vec![surface([-0.1, 0.0, 0.0]), surface([-0.5, 0.0, 0.0])];
        near.surface_sector = vec![0, 0];
        near.surface_links = vec![(0, 2, 0), (0, 3, 1)];
        assert_eq!(
            near.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 0),
            Some(UseHit::Surface(0)),
            "the plate inside the sweep was not used"
        );
        assert!(
            near.pending_messages
                .iter()
                .any(|(instance, label, _, sender, _)| *instance == 0 && *label == "activate" && *sender == Some(2)),
            "the plate's script was not messaged: {:?}",
            near.pending_messages
        );

        let mut far = door_runtime();
        far.surfaces = vec![surface([-0.5, 0.0, 0.0])];
        far.surface_sector = vec![0];
        far.surface_links = vec![(0, 3, 0)];
        assert_eq!(
            far.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 0),
            None,
            "a plate half a unit away was used past the engine's 0.16 sweep"
        );
    }

    /// The engine searches **the body's sector** for a switch (`sithPlayerActions_Activate`,
    /// `0x4fc0a0`): the cantina plate belongs to sector 7's surface slice while the player stands in 260,
    /// which is why retail's `doorstatus` stays 0 at every measured pose (rounds 302-303). A linked plate
    /// in another room must not be searched.
    #[test]
    fn the_use_scan_ignores_a_plate_an_adjoining_room_owns() {
        let mut rt = door_runtime();
        rt.surfaces = vec![SurfaceState {
            center: [-0.1, 0.0, 0.0],
            cel: 0.0,
            anim: 0.0,
            slide: [0.0; 3],
            next: 0.0,
            light: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
        }];
        rt.surface_links = vec![(0, 2, 0)];
        rt.surface_sector = vec![7];
        assert_eq!(
            rt.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 260),
            None,
            "a plate in sector 7 was used from sector 260"
        );
        assert_eq!(
            rt.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 7),
            Some(UseHit::Surface(0)),
            "the plate's own sector did not reach it"
        );
    }

    /// The same wait the engine keeps: `sithPlayerActions_Activate` (`0x4fc0a0`) refuses a second
    /// press inside `DAT_008554bc + 0xfa` (250 ms), and skips the wait while no master cog exists
    /// (`DAT_00832628 == 0`). The stamp is written before the sweep.
    #[test]
    fn the_use_action_waits_250_ms_once_a_level_has_a_master_cog() {
        let with_plate = || {
            let mut rt = door_runtime();
            rt.surfaces = vec![SurfaceState {
                center: [-0.1, 0.0, 0.0],
                cel: 0.0,
                anim: 0.0,
                slide: [0.0; 3],
                next: 0.0,
                light: 0.0,
                light_end: 0.0,
                light_rate: 0.0,
            }];
            rt.surface_links = vec![(0, 2, 0)];
            rt.surface_sector = vec![7];
            rt.master_cog = Some(0);
            rt
        };
        let mut rt = with_plate();
        assert!(rt.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 7).is_some());
        assert!(
            rt.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 7).is_none(),
            "the sweep ran again at the same clock"
        );
        rt.time = 0.249;
        assert!(
            rt.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 7).is_none(),
            "a press 249 ms after the last was allowed"
        );
        rt.time = 0.25;
        assert!(
            rt.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 7).is_some(),
            "a press at exactly 250 ms was refused"
        );

        // With no master cog the engine's `if` is short-circuited and no wait applies.
        let mut no_master = with_plate();
        no_master.master_cog = None;
        no_master.time = 0.0;
        assert!(no_master.sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 7).is_some());
        assert!(
            no_master
                .sith_player_actions_activate(0, [0.0; 3], [-1.0, 0.0, 0.0], 0.16, 7)
                .is_some(),
            "a level without a master cog waited"
        );
    }

    /// `sithCog_SendMessageEx` (`0x4dedc0`) carries an identity, and the senders differ only in it:
    /// a thing message names type 3 and the thing, a sector message type 5, a surface message type 6,
    /// and a world sender is index -1 with type 0. `GetSenderId()` answers the **link id** — the
    /// plate's `linkid=2` is what `00_doorswitch.cog`'s `activated:` handler compares against.
    #[test]
    fn a_message_carries_the_engine_source_and_sender() {
        assert_eq!(
            SourceSender::new(THING_SOURCE, 204, None),
            SourceSender { source_type: 3, source: 204, sender_type: 0, sender: -1 },
            "a world-sent thing message"
        );
        assert_eq!(
            SourceSender::new(THING_SOURCE, 204, Some(0)),
            SourceSender { source_type: 3, source: 204, sender_type: 3, sender: 0 },
            "the player's use action sends from its own thing"
        );
        assert_eq!(SourceSender::new(SECTOR_SOURCE, 31, None).source_type, 5);
        assert_eq!(SourceSender::new(SURFACE_SOURCE, 12, None).source_type, 6);

        let mut rt = door_runtime();
        rt.surfaces = vec![SurfaceState {
            center: [-0.1, 0.0, 0.0],
            cel: 0.0,
            anim: 0.0,
            slide: [0.0; 3],
            next: 0.0,
            light: 0.0,
            light_end: 0.0,
            light_rate: 0.0,
        }];
        rt.surface_links = vec![(0, 2, 0)];
        assert_eq!(rt.sith_cog_surface_send_message(0, None, "sighted"), 1);
        let (_, _, _, link, identity) = rt.pending_messages.pop().expect("a queued message");
        assert_eq!(link, Some(2), "the plate's link id is what GetSenderId answers");
        assert_eq!(
            identity,
            Some(SourceSender::new(SURFACE_SOURCE, 0, None)),
            "the world's message must carry the engine's (-1, 0) sender"
        );
        // `sighted` reaches the thing's links with the world as the sender (0), which is what
        // `sithRender_RenderThing` (`0x4c8070`) passes; a thing with no link hears nothing.
        rt.things.push({
            let mut t = rt.things[0].clone();
            t.flags |= TF_CAPTURED;
            t
        });
        rt.thing_links = vec![(0, 0, 1)];
        assert_eq!(rt.sith_cog_thing_send_message(1, None, "sighted"), 1);
        let (_, _, _, link, identity) = rt.pending_messages.pop().expect("a queued message");
        assert_eq!(link, Some(0));
        assert_eq!(identity, Some(SourceSender::new(THING_SOURCE, 1, None)));
        assert_eq!(rt.sith_cog_sector_send_message(5, None, "entered"), 0);
    }

    /// A surface crossing carries its own link id as the sender, which is what tells an
    /// `onAdjoin` (`linkid=1`) from an `offAdjoin` (`linkid=0`): level 1's soundtrack script fades
    /// the bar's music up or down on that number, so a message without it always faded down.
    #[test]
    fn a_surface_crossing_carries_the_link_id_as_the_sender() {
        let mut rt = door_runtime();
        rt.surface_links = vec![(0, 1, 206), (1, 0, 213), (2, 2, 206)];
        let player = Some(rt.player_thing());
        assert_eq!(rt.sith_cog_surface_send_message(206, player, "crossed"), 2, "both links to surface 206 were messaged");
        let senders: Vec<(usize, Option<i64>)> = rt
            .pending_messages
            .iter()
            .map(|(instance, _, _, sender, _)| (*instance, *sender))
            .collect();
        assert_eq!(senders, vec![(0, Some(1)), (2, Some(2))], "a crossing lost its link id");
        assert_eq!(rt.sith_cog_surface_send_message(213, player, "crossed"), 1, "the offAdjoin link was not messaged");
        assert_eq!(rt.pending_messages[2].3, Some(0), "link 0 must arrive as sender 0");
    }

    /// `sithThing_AttachThingToSurface` (`0x4cde80:50-52`): the attach sends the surface's linked
    /// cog `entered` (id 6) when the attach surface **changed** — the engine's early return keeps an
    /// attach to the *same* surface silent, so a move from surface to surface, or the first attach
    /// (creation included), is what fires, not "was unattached, now attached".
    #[test]
    fn an_attach_that_changes_surface_sends_entered() {
        let mut rt = door_runtime();
        rt.surface_flags = vec![0x4, 0x4, 0x4];
        rt.surface_flags[1] |= 2;
        rt.surface_flags[2] |= 2;
        rt.surface_links = vec![(0, 0, 1), (0, 1, 2)];
        let player = Some(rt.player_thing());
        // None -> surface 1: the creation/first attach, engine sends.
        rt.sith_thing_attach_thing_to_surface(None, Some(1), 0, player);
        assert_eq!(rt.pending_messages.len(), 1, "the first attach to a linked surface did not send");
        assert_eq!(rt.pending_messages[0].1, "entered");
        // surface 1 -> surface 1: the engine's early return, no message.
        rt.sith_thing_attach_thing_to_surface(Some(1), Some(1), 0, player);
        assert_eq!(rt.pending_messages.len(), 1, "a re-attach to the same surface sent again");
        // surface 1 -> surface 2: a move between surfaces sends too.
        rt.sith_thing_attach_thing_to_surface(Some(1), Some(2), 0, player);
        assert_eq!(rt.pending_messages.len(), 2, "a move to a different linked surface did not send");
        // A surface without the cog-linked bit never sends.
        rt.sith_thing_attach_thing_to_surface(Some(2), Some(0), 0, player);
        assert_eq!(rt.pending_messages.len(), 2, "an unlinked surface sent entered");
    }

    /// The same function's flag gate: a thing whose flags carry `0x80100` does not message the
    /// surface's cog on attach; either sub-bit alone suppresses (`0x4cde80:50` masks the whole word).
    #[test]
    fn an_attach_does_not_send_when_the_thing_flags_carry_0x80100() {
        let mut rt = door_runtime();
        rt.surface_flags = vec![0x6];
        rt.surface_links = vec![(0, 0, 0)];
        rt.sith_thing_attach_thing_to_surface(None, Some(0), 0x80100, None);
        assert!(rt.pending_messages.is_empty(), "a thing flagged 0x80100 sent entered");
        rt.sith_thing_attach_thing_to_surface(None, Some(0), 0x80000, None);
        rt.sith_thing_attach_thing_to_surface(None, Some(0), 0x100, None);
        assert!(rt.pending_messages.is_empty(), "either 0x80100 bit alone suppresses the send");
        rt.sith_thing_attach_thing_to_surface(None, Some(0), 0x4, None);
        assert_eq!(rt.pending_messages.len(), 1, "flags clear of 0x80100 must still send");
    }

    /// `sithCog_LinkCog` (`0x4df610`): at link time a surface symbol ORs
    /// `SITH_SURFACE_COG_LINKED` (2) into the surface's surfflags — the shipped data usually
    /// carries the bit already; the OR is the rule. A `nolink` symbol sets nothing.
    #[test]
    fn link_cog_sets_the_surface_cog_linked_bit() {
        let mut rt = door_runtime();
        rt.surface_flags = vec![0x4, 0x4];
        let script = crate::cog::parse("symbols\nsurface a\nsurface b nolink\nend\ncode\nstartup:\n    return;\nend\n").unwrap();
        let scripts = std::collections::HashMap::from([("t.cog".to_string(), script)]);
        rt.wire_links(
            &[crate::jkl::CogInstance { script: "t.cog".into(), values: vec!["0".into(), "1".into()] }],
            &scripts,
        );
        assert_eq!(rt.surface_flags[0], 0x6, "the linked surface did not gain the cog-linked bit");
        assert_eq!(rt.surface_flags[1], 0x4, "a nolink surface gained the bit");
        assert_eq!(rt.surface_links, vec![(0, 0, 0)], "the nolink symbol was recorded as a link");
    }

    /// One app frame for an AI test: the scheduler (`sithAI_Process`, `ai_tick`), then the world
    /// tick whose per-thing slot runs `sithAI_Tick` and the body's motion.
    fn ai_frame(rt: &mut Runtime, player: usize, dt: f32) -> usize {
        let fired = rt.ai_tick(player, dt);
        rt.step(None, dt);
        fired
    }

    /// An AI actor (thing 1) with `_humanactor`'s numbers — `maxthrust 0.8`, `surfdrag 3`,
    /// `movesize 0.065`, mass 150 — standing on `test_room`'s floor (z = -1) at `x`, seated by a
    /// few world ticks; thing 0 is the player in the room's corner.
    fn ai_walker_runtime(x: f32, class: &str) -> (Runtime, crate::collide::Solids) {
        let solids = crate::collide::test_room();
        let mut rt = door_runtime();
        let mut player = rt.things[0].clone();
        player.pos = [-1.8, 1.8, -0.93];
        player.sector = 0;
        let mut actor = player.clone();
        actor.pos = [x, 0.0, -0.93];
        actor.phys_flags = 0x4a4f;
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "_humanactor".into()];
        rt.types = vec![10, 2];
        rt.mass.insert(1, 150.0);
        rt.template_maxthrust.insert("_humanactor".into(), 0.8);
        rt.template_drag.insert("_humanactor".into(), (3.0, 0.0));
        rt.template_hit.insert("_humanactor".into(), (0.065, 0.18));
        rt.ai.insert(1, crate::ai::parse(class));
        for _ in 0..10 {
            rt.step(Some(&solids), 1.0 / 30.0);
        }
        assert_ne!(rt.things[1].attach_flags, 0, "the walker did not seat on the floor");
        (rt, solids)
    }

    /// `sithAI_idk_msgarrived_target` (`0x4ea890`) plus the attached drag arm (`0x4f7430:180-208`):
    /// each frame adds `movespeed (1.5) × maxthrust (0.8) × dt` along the move direction, then drag
    /// takes `surfdrag × dt` of the velocity, so the body settles where `v = (v + a·dt)(1 − 3·dt)`:
    /// `a (1 − 3 dt) / 3` = 0.36 u/s at 30 Hz.
    #[test]
    fn an_attached_humanactor_settles_at_the_move_arm_terminal_speed() {
        let (mut rt, solids) = ai_walker_runtime(-1.5, "alignment=-1.0, rank=0.5\n");
        let goal = [1.9, 0.0, -0.93];
        rt.call("AISetMovePos", &[Value::Num(1.0), Value::Vec(goal.map(f64::from))]);
        assert_eq!(rt.ai_move.get(&1), Some(&goal));
        let start = rt.things[1].pos;
        for _ in 0..90 {
            rt.step(Some(&solids), 1.0 / 30.0);
        }
        let vel = rt.things[1].vel;
        let speed = (vel[0] * vel[0] + vel[1] * vel[1]).sqrt();
        let terminal = 1.5 * 0.8 * (1.0 - 3.0 / 30.0) / 3.0;
        assert!((speed - terminal).abs() < 1e-3, "settled at {speed:.4}, not {terminal:.4}: {vel:?}");
        assert!(vel[2].abs() < 1e-4, "a walker's push has no z: {vel:?}");
        assert!(rt.things[1].pos[0] > start[0] + 0.5, "the body did not travel: {:?}", rt.things[1].pos);
    }

    /// Arrival (`0x4ea890:103-112`): the distance to the move position at or under `moveSize`
    /// zeroes the velocity, clears the moving bit, sends `arrived` (cog message `0xb`) and emits
    /// AI event `0x800` — which `returnhome`'s arrival arm answers by looking along its placed facing.
    #[test]
    fn an_aisetmovepos_actor_arrives_within_movesize() {
        let (mut rt, solids) = ai_walker_runtime(-1.0, "alignment=-1.0, rank=0.5\n\nReturnHome\n");
        rt.thing_links.push((9, 0, 1));
        *rt.ai_flags.entry(1).or_insert(0) |= 0x4; // awake: `returnhome`'s 0x800 arm reads it
        rt.ai_home_look.insert(1, [0.0, 1.0, 0.0]);
        let goal = [-0.5, 0.0, -0.93];
        rt.call("AISetMovePos", &[Value::Num(1.0), Value::Vec(goal.map(f64::from))]);
        rt.pending_messages.clear();
        let mut arrived_at = None;
        for frame in 0..120 {
            rt.step(Some(&solids), 1.0 / 30.0);
            if rt.ai_flags[&1] & 0x1 == 0 {
                arrived_at = Some(frame);
                break;
            }
        }
        assert!(arrived_at.is_some(), "the actor never arrived: {:?}", rt.things[1].pos);
        let p = rt.things[1].pos;
        let left = ((goal[0] - p[0]).powi(2) + (goal[1] - p[1]).powi(2)).sqrt();
        // The test is taken before the frame's move, so the body ends at most one frame past it.
        assert!(left <= 0.065 + 0.4 / 30.0, "stopped {left:.3} from the point");
        assert_eq!(rt.things[1].vel, [0.0; 3], "arrival did not zero the velocity");
        assert!(
            rt.pending_messages.iter().any(|(instance, msg, ..)| *instance == 9 && *msg == "arrived"),
            "arrival did not send `arrived` to the thing's cog"
        );
        assert_eq!(rt.ai_look_dir.get(&1), Some(&[0.0, 1.0]), "event 0x800 did not reach `returnhome`");
    }

    /// `sithAI_CanWalk` (`0x4eb4b0`): `FindSectorInRadius` to the step's point, then a
    /// `moveSize × 0.25` sphere swept down `GetThingHeight + maxstep` (0.07 + 0.5). A floor inside
    /// the reach answers non-zero; a deeper drop is a ledge (0).
    #[test]
    fn the_ledge_reach_is_the_body_height_plus_maxstep() {
        fn two_levels(drop: f32) -> Runtime {
            let mut rt = door_runtime();
            let base = rt.things[0].clone();
            let mut actor = base.clone();
            actor.pos = [0.2, 0.0, 0.065];
            actor.sector = 0;
            rt.things = vec![base, actor];
            rt.templates = vec!["walkplayer".into(), "tuskan".into()];
            rt.types = vec![10, 2];
            let face = |surface: usize, z: f32, x0: f32, x1: f32| crate::collide::Face {
                normal: [0.0, 0.0, -1.0],
                point: [0.0, 0.0, z],
                verts: vec![[x0, -2.0, z], [x1, -2.0, z], [x1, 2.0, z], [x0, 2.0, z]],
                surface,
            };
            let plane = |normal: [f32; 3], point: [f32; 3], surface: usize| crate::collide::Plane { normal, point, surface };
            let upper = face(1, 0.0, -1.0, 0.5);
            let lower = face(3, -drop, 0.5, 3.0);
            let planes = vec![
                plane([0.0, 0.0, -1.0], [0.0, 0.0, 0.0], 1),
                plane([0.0, 0.0, -1.0], [0.5, 0.0, -drop], 3),
            ];
            rt.solids = Some(std::sync::Arc::new(crate::collide::Solids {
                sectors: vec![planes.clone()],
                hull: vec![planes],
                boxes: vec![None],
                portals: vec![Vec::new()],
                spheres: vec![([0.0, 0.0, 0.0], 100.0)],
                floors: vec![upper.clone(), lower.clone()],
                floor_of: [(1usize, 0usize), (3usize, 1usize)].into_iter().collect(),
                faces: vec![upper, lower],
                face_of: [(1usize, 0usize), (3usize, 1usize)].into_iter().collect(),
                surface_owner: vec![0, 0, 0, 0],
                ..Default::default()
            }));
            rt
        }
        let past_edge = [0.6, 0.0, 0.065];
        let step = two_levels(0.4);
        assert_eq!(step.sith_ai_can_walk(1, [0.3, 0.0, 0.065], step.solids.as_deref()), 2, "the floor under the body");
        assert_eq!(step.sith_ai_can_walk(1, past_edge, step.solids.as_deref()), 2, "a 0.4 drop is a step");
        let ledge = two_levels(1.0);
        assert_eq!(ledge.sith_ai_can_walk(1, past_edge, ledge.solids.as_deref()), 0, "a 1.0 drop is a ledge");
    }

    /// The move arm's refusal (`0x4ea890:62-70`): an attached walker whose next point
    /// (`pos + vel × dt`) `CanWalk` refuses has its velocity zeroed and emits `0x400`; the order and
    /// the moving bit stay. The same push over a shallow step keeps its velocity.
    #[test]
    fn an_actor_at_a_ledge_refuses_the_step() {
        let (mut rt, solids) = ai_walker_runtime(1.7, "alignment=-1.0, rank=0.5\n");
        let mut ledge = crate::collide::test_room();
        ledge.floors[0].verts = vec![[-2.0, -2.0, -1.0], [1.75, -2.0, -1.0], [1.75, 2.0, -1.0], [-2.0, 2.0, -1.0]];
        rt.call("AISetMovePos", &[Value::Num(1.0), Value::Vec([1.95, 0.0, -0.93])]);
        rt.things[1].vel = [1.0, 0.0, 0.0];
        rt.sith_ai_msgarrived_target(1, 0.1, Some(&ledge));
        assert_eq!(rt.things[1].vel, [0.0; 3], "the ledge did not stop the push");
        assert_ne!(rt.ai_flags[&1] & 0x1, 0, "the refusal dropped the order");

        rt.things[1].vel = [1.0, 0.0, 0.0];
        rt.sith_ai_msgarrived_target(1, 0.1, Some(&solids));
        assert!(rt.things[1].vel[0] > 1.0, "a walkable step was refused: {:?}", rt.things[1].vel);
    }

    /// A walker driven into a standing player is stopped by the `(2,10)` pair's contact response
    /// (`sithCollision_ThingCollisionHandler`, `0x4e8c50`), and the actor wrapper (`0x4ed210`)
    /// raises AI event 4 once for the frame — `specialattack` answers it with its damage.
    #[test]
    fn a_walker_is_stopped_by_a_standing_player_and_raises_event_4_once() {
        let (mut rt, solids) = ai_walker_runtime(
            -1.0,
            "alignment=-1.0, rank=0.5\n\nSpecialAttack 1000.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 5.0\n",
        );
        rt.things[0].pos = [-0.4, 0.0, -0.93];
        solids.set_things(vec![crate::collide::ThingSolid {
            index: 0, centre: [-0.4, 0.0, -0.93], origin: [-0.4, 0.0, -0.93], radius: 0.065,
            z_min: -1.0, z_max: -0.8, sector: 0, half: None, yaw: 0.0, mesh: None,
            thing_flags: 0, collide: 1, kind: 10, owner: None, item: false,
        }]);
        rt.call("AISetMovePos", &[Value::Num(1.0), Value::Vec([1.5, 0.0, -0.93])]);
        let health = rt.things[0].health;
        let mut hit = false;
        for _ in 0..240 {
            rt.step(Some(&solids), 1.0 / 30.0);
            if rt.things[0].health != health {
                hit = true;
                break;
            }
        }
        assert!(hit, "the walker never touched the player: {:?}", rt.things[1].pos);
        assert_eq!(rt.things[0].health, health - 5.0, "event 4 did not fire exactly once");
        assert!(rt.things[1].pos[0] < -0.4 - 0.065, "the walker passed into the player: {:?}", rt.things[1].pos);
    }

    /// The move arm keeps the push's z for a flyer (physflag `0x2000`): only a walker outside water
    /// has it replaced by `_DAT_00523050` (0.0).
    #[test]
    fn a_flyers_move_keeps_its_z() {
        let mut rt = door_runtime();
        let mut actor = rt.things[0].clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.sector = 0;
        rt.things = vec![rt.things[0].clone(), actor];
        rt.templates = vec!["walkplayer".into(), "probe".into()];
        rt.types = vec![10, 2];
        rt.template_maxthrust.insert("probe".into(), 1.0);
        rt.ai.insert(1, crate::ai::parse("alignment=-1.0, rank=0.5\n"));
        rt.call("AISetMovePos", &[Value::Num(1.0), Value::Vec([0.0, 3.0, 4.0])]);
        rt.things[1].phys_flags = 0x2000;
        rt.sith_ai_msgarrived_target(1, 0.1, None);
        let vel = rt.things[1].vel;
        assert!((vel[2] - 1.5 * 0.8 * 0.1).abs() < 1e-5, "the flyer lost its climb: {vel:?}");
        rt.things[1].vel = [0.0; 3];
        rt.things[1].phys_flags = 0;
        rt.sith_ai_msgarrived_target(1, 0.1, None);
        assert_eq!(rt.things[1].vel[2], 0.0, "a walker kept a z push: {:?}", rt.things[1].vel);
    }

    /// `opendoors` (handler 0x50abc0) runs the **player's activate action** on the actor's own body:
    /// the engine calls `sithPlayerActions_Activate` and messages a captured thing in front with
    /// `activated` (id 1). The port reuses the use scans, so an actor facing a closed door queues its
    /// `activate` — which is what stops it from being trapped behind the door.
    #[test]
    fn an_actor_with_opendoors_activates_the_door_in_front() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        // Player at the origin, the actor at the origin facing -x (the port's forward is
        // `(-sin, cos)`), and a door thing **0.12** units along -x (the cantina plate's measured
        // in-reach pose), linked to cog 0 as link 3. `sithCog_LinkCog` (`0x4df610`) also sets the
        // thing's captured bit. The reach is the actor's `movesize + 0.1`,
        // the engine's own activate rule, so a door a body-length away is *not* in reach.
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 90.0;
        let mut door = base.clone();
        door.pos = [-0.12, 0.0, 0.0];
        door.flags |= TF_CAPTURED;
        rt.things = vec![base, actor, door];
        rt.templates = vec!["walkplayer".into(), "tuskan".into(), "door".into()];
        rt.types = vec![10, 2, 0];
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, maxstep=0.3, sightdist=5.0, heardist=2.0, accuracy=0.15\n\n\
                 OpenDoors\n",
            ),
        );
        rt.thing_links.push((0, 3, 2));
        // `00_door.cog` declares its doors `mask=0x405` (world, actor, player): the default `0x401`
        // would not hear an actor's activate.
        rt.link_masks.insert((THING_SOURCE, 0, 3, 2), 0x405);
        // `FUN_0050abc0` runs the activate only while the AI flag word's bit `0x1` (moved) is set.
        rt.ai_flags.insert(1, 0x3); // opendoors: accept 2, the handler itself needs the moving bit 1

        rt.ai_tick(0, 0.1);
        assert!(
            rt.pending_messages.iter().any(|(instance, label, _, sender, _)| {
                *instance == 0 && *label == "activate" && *sender == Some(3)
            }),
            "the actor did not activate the door in front: {:?}",
            rt.pending_messages
        );

        // The handler writes `now + 1000 ms` into its own state (`0x50abc0`), so a moving actor does
        // not message the same door again inside that window.
        rt.pending_messages.clear();
        rt.ai_flags.insert(1, 0x3);
        rt.time = 0.5;
        rt.ai_tick(0, 0.1);
        assert!(
            rt.pending_messages.is_empty(),
            "the handler ran again inside its 1000 ms re-arm: {:?}",
            rt.pending_messages
        );
        rt.ai_flags.insert(1, 0x3);
        rt.time = 1.01;
        rt.ai_tick(0, 0.1);
        assert!(
            rt.pending_messages.iter().any(|(instance, label, _, sender, _)| {
                *instance == 0 && *label == "activate" && *sender == Some(3)
            }),
            "the handler did not re-arm after its 1000 ms: {:?}",
            rt.pending_messages
        );

        // The same door at 0.93 — the pose retail's use action refuses — is out of reach.
        let mut far = door_runtime();
        let base = far.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 90.0;
        let mut door = base.clone();
        door.pos = [-0.93, 0.0, 0.0];
        door.flags |= TF_CAPTURED;
        far.things = vec![base, actor, door];
        far.templates = vec!["walkplayer".into(), "tuskan".into(), "door".into()];
        far.types = vec![10, 2, 0];
        far.ai.insert(
            1,
            crate::ai::parse("alignment=-1.0, rank=0.5, fov=180\n\nOpenDoors\n"),
        );
        far.thing_links.push((0, 3, 2));
        far.ai_flags.insert(1, 0x3);
        far.ai_tick(0, 0.1);
        assert!(
            far.pending_messages.is_empty(),
            "an actor activated a door 0.93 away, past its own reach: {:?}",
            far.pending_messages
        );

        // Without the instinct the same body does nothing to the door.
        let mut quiet = door_runtime();
        let base = quiet.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 90.0;
        let mut door = base.clone();
        door.pos = [-1.5, 0.0, 0.0];
        door.flags |= TF_CAPTURED;
        quiet.things = vec![base, actor, door];
        quiet.templates = vec!["walkplayer".into(), "tuskan".into(), "door".into()];
        quiet.types = vec![10, 2, 0];
        quiet.ai.insert(1, crate::ai::parse("alignment=-1.0, rank=0.5, fov=180\n\nLookForTarget 500.0\n"));
        quiet.thing_links.push((0, 3, 2));
        quiet.ai_flags.insert(1, 0x3);
        quiet.ai_tick(0, 0.1);
        assert!(
            quiet.pending_messages.is_empty(),
            "an actor without `opendoors` activated something: {:?}",
            quiet.pending_messages
        );
    }

    /// `LookForTarget` scans the class's **view cone**, not just its range. The engine's recovered
    /// handler (round 210: `sithAICmd_Startup` registers 0x50aa80) scans through
    /// `sithAI_FirstThingInView`, which sets the class's FOV limits, then runs
    /// `sithAI_CheckSightThing`. The port's stand-in used `range <= sightdist` alone, so an actor
    /// acquired the body **behind it** — which is how the level-3 engagement divergence could happen.
    #[test]
    fn look_for_target_scans_the_class_view_cone() {
        let mut rt = door_runtime();
        // The body at the origin, an actor two units along +x. The engine's yaw -90 faces +x, which
        // here is away from the body.
        let mut actor = rt.things[0].clone();
        actor.pos = [2.0, 0.0, 0.0];
        actor.yaw = -90.0;
        let player = rt.things[0].clone();
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=90, maxstep=0.3, sightdist=5.0, heardist=2.0, accuracy=0.15\n\n\
                 LookForTarget 500.0, 30000.0\n",
            ),
        );

        // The scan (branch A) runs only while the awake bit is set and the searching bit is clear:
        // `FUN_0050aa80` routes a `0x200` word to branch B, the memory-decay wake
        // (`last_seen + fltArg[1] < now`), which never fires here.
        rt.ai_flags.insert(1, 0x200);
        assert_eq!(rt.ai_flags.get(&1).copied(), Some(0x200));
        // Facing away: outside the 90-degree cone, so the scan must not find the body even though the
        // range (2.0) is well inside `sightdist` (5.0).
        rt.ai_tick(0, 0.1);
        assert!(
            !rt.ai_last_seen.contains_key(&1),
            "the scan acquired a body outside the class's view cone"
        );

        // Turn the actor onto it: same range, now inside the cone, so the find lands. The target's
        // `thingflags & 0x4` is the engine's "plainly visible" bit, which makes the detection roll
        // certain and keeps this a test of the cone, not of the random draw.
        rt.things[0].flags = 0x4;
        rt.things[1].yaw = 90.0;
        rt.ai_flags.insert(1, 0x4); // branch A: awake, not searching
        rt.ai_next_due.remove(&1);
        if let Some(slots) = rt.ai_inst.get_mut(&1) {
            slots.iter_mut().for_each(|clock| *clock = 0.0);
        }
        rt.ai_last_seen.clear();
        rt.ai_tick(0, 0.1);
        assert!(
            rt.ai_last_seen.contains_key(&1),
            "the scan did not acquire a body inside the class's view cone"
        );
    }

    /// `sithAI_sub_4EAD60` builds the sight eye from the thing's model offset above its origin
    /// (`rdMatrix_TransformVector34` over `thing + 0x17c`), and `sithAI_CheckSightThing` casts from
    /// that eye. The door encounter's `greedo` 236 stands behind the balcony rail: the origin-to-origin
    /// line is clear while the eye's line crosses the rail, and retail's actor never acquires the body.
    #[test]
    fn the_scan_casts_from_the_models_eye_point() {
        let mut rt = door_runtime();
        let mut solids = crate::collide::test_room();
        // A rail between the two at 0.52..0.58 above the floor: the thing-origin ray (z 0.5) passes
        // under it, the eye ray (z 0.575 -> 0.5) crosses it, which is the balcony the door gunner
        // stands behind.
        solids.walls.push(crate::collide::Face {
            normal: [0.0, -1.0, 0.0],
            point: [0.0, 1.0, 0.0],
            verts: vec![
                [-1.0, 1.0, 0.52],
                [1.0, 1.0, 0.52],
                [1.0, 1.0, 0.58],
                [-1.0, 1.0, 0.58],
            ],
            surface: 9,
        });
        rt.solids = Some(std::sync::Arc::new(solids));
        let mut actor = rt.things[0].clone();
        actor.pos = [0.0, 0.0, 0.5];
        actor.yaw = 0.0;
        let mut player = rt.things[0].clone();
        player.pos = [0.0, 2.0, 0.5];
        player.flags = 0x4;
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "greedo".into()];
        rt.types = vec![10, 2];
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=200, maxstep=0.6, sightdist=5.0, heardist=2.0, accuracy=0.25\n\n\
                 LookForTarget 500.0, 50000.0\n",
            ),
        );
        // Awake (the scan's own gate) and `0x200`, which holds `sithAI_CanDetectSightThing`'s chance
        // at 1.0 — this grades the sight ray, not the random draw.
        rt.ai_flags.insert(1, 0x204);
        assert!(
            !rt.solids.as_ref().unwrap().segment_blocked([0.0, 0.0, 0.5], [0.0, 2.0, 0.5]),
            "the fixture must leave the thing-origin line clear"
        );
        rt.ai_tick(0, 0.1);
        assert!(
            !rt.ai_last_seen.contains_key(&1),
            "the scan saw through the rail: it cast from the thing origin, not the model's eye"
        );
    }

    /// The scan's range gate is `sithAI_CheckSightThing`'s **twice** radius-subtracted distance
    /// (`param_5 < dist − r` on a `dist` that already removed `r`): a body stays visible out to
    /// `sightdist + 2·radius` of centre. `Ai::sees` refuses this target (5.05 > 5.0); the scan
    /// accepts it because the eye→centre length 5.0506 − 2 × 0.065 = 4.92 <= 5.0.
    #[test]
    fn the_scan_accepts_a_body_edge_inside_sightdist() {
        let mut rt = door_runtime();
        let mut actor = rt.things[0].clone();
        actor.pos = [0.0, 0.0, 0.5];
        actor.yaw = 0.0;
        let mut player = rt.things[0].clone();
        player.pos = [0.0, 5.05, 0.5];
        player.flags = 0x4;
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "greedo".into()];
        rt.types = vec![10, 2];
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=200, maxstep=0.6, sightdist=5.0, heardist=2.0, accuracy=0.25\n\n\
                 LookForTarget 500.0, 50000.0\n",
            ),
        );
        let ai = rt.ai[&1].clone();
        // The plain range test the scan used to apply refuses this body...
        assert!(!ai.sees([0.0, 0.0, 0.575], 0.0, [0.0, 5.05, 0.5]), "sanity: centre is out of range");
        rt.ai_flags.insert(1, 0x4); // branch A: awake, searching bit clear (0x200 routes to branch B)
        rt.ai_last_seen.clear();
        rt.ai_tick(0, 0.1);
        assert!(
            rt.ai_last_seen.contains_key(&1),
            "the scan must accept a body whose edge is inside sightdist"
        );
    }

    /// `sithAI_CheckSightThing` (`0x4eb090`) returns the engine's own codes: 0 visible, 1 out of
    /// range (after the radius leaves the distance **twice**), 2 outside the class's cone, 3 no
    /// line. Worked from the decompiled arithmetic: eye at the origin, player radius 0.065,
    /// sightdist 5, fov 180 (`fovcos = 0`: the whole front hemisphere).
    #[test]
    fn check_sight_thing_returns_the_engine_codes() {
        let mut rt = door_runtime();
        let mut actor = rt.things[0].clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 0.0; // looks along +Y
        let mut player = rt.things[0].clone();
        player.pos = [0.0, 5.10, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "greedo".into()];
        let fovcos = (180.0f32 * 0.5).to_radians().cos(); // 0
        // 5.10 − 2 × 0.065 = 4.97 <= 5: visible where a single subtraction (5.035) would refuse.
        assert_eq!(rt.check_sight_thing(1, [0.0, 0.0, 0.0], 0, fovcos, 5.0), 0);
        // 5.20 − 0.13 = 5.07 > 5: out of range.
        rt.things[0].pos = [0.0, 5.20, 0.0];
        assert_eq!(rt.check_sight_thing(1, [0.0, 0.0, 0.0], 0, fovcos, 5.0), 1);
        // Behind the facing at any range: code 2 (fovcos 0 rejects the rear hemisphere).
        rt.things[0].pos = [0.0, -3.0, 0.0];
        assert_eq!(rt.check_sight_thing(1, [0.0, 0.0, 0.0], 0, fovcos, 5.0), 2);
        // A wall between the two: code 3.
        rt.things[0].pos = [0.0, 3.0, 0.0];
        let mut solids = crate::collide::test_room();
        solids.walls.push(crate::collide::Face {
            normal: [0.0, -1.0, 0.0],
            point: [0.0, 1.0, 0.0],
            verts: vec![[-1.0, 1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]],
            surface: 9,
        });
        rt.solids = Some(std::sync::Arc::new(solids));
        assert_eq!(rt.check_sight_thing(1, [0.0, 0.0, 0.0], 0, fovcos, 5.0), 3);
    }

    /// `sithAI_FirstThingInView`/`sithAI_GetThingsInView` (`0x4eb790`/`0x4ebe80`): the frustum
    /// collects the sector's own things, drops the one behind the frame, and recurses through an
    /// adjoin that faces the look direction — gated by the adjoin's `dist` depth, which the
    /// engine accumulates on every hop.
    #[test]
    fn first_thing_in_view_walks_the_adjoins_by_depth() {
        let mut rt = door_runtime();
        let quad = || vec![[-1.0, 0.0, -1.0], [1.0, 0.0, -1.0], [1.0, 0.0, 1.0], [-1.0, 0.0, 1.0]];
        rt.solids = Some(std::sync::Arc::new(crate::collide::Solids {
            portals: vec![
                vec![crate::collide::Portal {
                    adjoin: 1, link: 0, verts: quad(), normal: [0.0, 1.0, 0.0], point: [0.0, 0.0, 0.0],
                }],
                vec![
                    crate::collide::Portal {
                        adjoin: 0, link: 1, verts: quad(), normal: [0.0, -1.0, 0.0], point: [0.0, 0.0, 0.0],
                    },
                    crate::collide::Portal {
                        adjoin: 2, link: 2, verts: quad(), normal: [0.0, 1.0, 0.0], point: [0.0, 8.0, 0.0],
                    },
                ],
                vec![crate::collide::Portal {
                    adjoin: 1, link: 3, verts: quad(), normal: [0.0, -1.0, 0.0], point: [0.0, 8.0, 0.0],
                }],
            ],
            ..crate::collide::test_room()
        }));
        let mut thing = rt.things[0].clone();
        thing.pos = [0.0, 1.0, 0.0];
        thing.sector = 0;
        let ahead = thing.clone();
        let mut behind = thing.clone();
        behind.pos = [0.0, -1.0, 0.0];
        let mut next_room = thing.clone();
        next_room.pos = [0.0, 8.0, 0.0];
        next_room.sector = 1;
        let mut far_room = thing.clone();
        far_room.pos = [0.0, 16.0, 0.0];
        far_room.sector = 2;
        rt.things = vec![ahead, behind, next_room, far_room];
        rt.types = vec![2, 2, 2, 2];
        // corridor links: 0 (sector 0 -> 1), 2 (1 -> 2); depth 3 per hop. The engine enters a
        // child when the **parent's** depth is still inside `max_dist` (`param_3 <= max` before
        // the loop), so the child runs one hop deeper than the limit itself.
        rt.adjoin_depth = vec![3.0, 0.0, 3.0, 0.0];
        // yaw 0 looks along +Y, straight down the corridor. max_dist 2.5: root depth 0 passes,
        // sector 1 (depth 3) refuses its own adjoin, so the far room never runs.
        let found = rt.first_thing_in_view(0, [0.0, 0.0, 0.0], 0.0, 90.0, 90.0, 8, 1 << 2, 2.5);
        assert_eq!(found, vec![0, 2], "the thing behind the frame is dropped; one hop in");
        // max_dist 3 lets sector 1's own adjoin pass, which reaches the far room.
        let found = rt.first_thing_in_view(0, [0.0, 0.0, 0.0], 0.0, 90.0, 90.0, 8, 1 << 2, 3.0);
        assert_eq!(found, vec![0, 2, 3]);
        // A type outside the mask never lands.
        rt.types = vec![2, 2, 3, 2];
        let found = rt.first_thing_in_view(0, [0.0, 0.0, 0.0], 0.0, 90.0, 90.0, 8, 1 << 2, 3.0);
        assert_eq!(found, vec![0, 3]);
    }

    /// A captured deadline of 0 is **due now**: `sithAI_Process` calls the instinct update when
    /// `instance + 0x190 <= now`, and `sithAI_InstinctUpdate` always arms its slots before writing
    /// the next pass, so a fresh instance's 0 is a pass for the next tick, not "never".
    #[test]
    fn a_captured_zero_deadline_is_due_now_and_a_set_one_is_offset_from_the_origin() {
        let mut rt = door_runtime();
        rt.ai.insert(
            1,
            crate::ai::parse("alignment=-1.0, rank=0.5, fov=90, sightdist=5.0\n\nLookForTarget 500.0\n"),
        );
        rt.time = 4.2;
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.seed_capture_phase(
            1,
            4200,
            &[(
                1,
                CapturedPhase { flags: 4, fire_ms: 0, acquire_ms: 0, next_pass_ms: 0 },
            )],
        );
        assert_eq!(scan_clock(&rt, 1), 4.2, "a captured 0 scan must be due now");
        assert_eq!(rt.ai_detect_at[&1], 4.2, "a captured 0 pass must be due now");
        // A captured deadline is relative to the capture's own origin.
        rt.seed_capture_phase(
            1,
            4200,
            &[(
                1,
                CapturedPhase { flags: 4, fire_ms: 0, acquire_ms: 4304, next_pass_ms: 4529 },
            )],
        );
        assert!((scan_clock(&rt, 1) - 4.304).abs() < 1e-4, "the scan waited {}", scan_clock(&rt, 1));
        assert!((rt.ai_detect_at[&1] - 4.529).abs() < 1e-4, "the pass waited {:?}", rt.ai_detect_at[&1]);
    }
    /// The #93 fire-chain fixture: an engaged gunner one unit from the player, on the door
    /// runtime, with the weapon's speed pinned so every direction test can read `vel` directly.
    fn fire_chain_runtime(class: &str) -> Runtime {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut player = base.clone();
        player.pos = [0.0, 2.0, 0.0];
        let mut gunner = base.clone();
        gunner.pos = [0.0, 0.0, 0.0];
        gunner.yaw = 0.0;
        rt.things = vec![player, gunner];
        rt.templates = vec!["walkplayer".into(), "reeyeesgun".into()];
        rt.types = vec![10, 2];
        rt.template_weapon.insert("reeyeesgun".into(), "+elaser".into());
        rt.template_vel.insert("+elaser".into(), [0.0, 4.0, 0.0]);
        rt.ai.insert(1, crate::ai::parse(class));
        rt.ai_detected.insert(1);
        rt.time = 1.0;
        rt
    }

    /// `sithAI_FireWeapon` (`0x4eba10`): the facing gate is `|rvec·dir| <= 1 - minDot` with **no
    /// behind test**, so a target straight behind the gunner still fires and one broadside does
    /// not. The port's old `forward·dir >= minDot` refused the shot behind.
    #[test]
    fn the_fire_gate_is_the_engines_sideways_cone() {
        let class = "alignment=-1.0, rank=0.5, fov=360, sightdist=9.0, accuracy=1.0\n\n\
                     PrimaryFire 750.0, 0.8, 9.0, 0.0, 0.0, 500.0, 0.0, 0.0, 0.0, 0.0\n";
        let mut behind = fire_chain_runtime(class);
        // A zero turn rate pins the body's own facing, so the gate is read on the pre-turn
        // orient exactly as the engine's scheduler does mid-blend.
        behind.template_rotthrust.insert("reeyeesgun".into(), 0.0);
        rt_things_set_pos(&mut behind, 0, [0.0, -2.0, 0.0]);
        behind.ai_tick(0, 0.1);
        assert_eq!(behind.shots_fired, 1, "a target straight behind must still fire");
        let mut broadside = fire_chain_runtime(class);
        broadside.template_rotthrust.insert("reeyeesgun".into(), 0.0);
        rt_things_set_pos(&mut broadside, 0, [2.0, 0.0, 0.0]);
        broadside.ai_tick(0, 0.1);
        assert_eq!(broadside.shots_fired, 0, "a broadside target is outside the cone");
    }

    /// The range gate is the engine's `minD <= ai[0x7c] <= maxD`: the radius-subtracted **3D
    /// eye** distance (`CheckSightThing` `0x4eb090` writes `max(len - r, 0)`), not the port's old
    /// flat 2D body distance.
    #[test]
    fn the_fire_range_is_the_eye_distance_less_the_targets_radius() {
        let class = "alignment=-1.0, rank=0.5, fov=360, sightdist=9.0, accuracy=1.0\n\n\
                     PrimaryFire 750.0, 0.0, 1.4, 0.0, 0.0, 500.0, 0.0, 0.0, 0.0, 0.0\n";
        let mut rt = fire_chain_runtime(class);
        // 3D distance 1.5 minus the player's 0.065 radius is 1.435: outside a 1.4 `maxDist`,
        // while the old flat 2D distance (1.2) was inside it.
        rt_things_set_pos(&mut rt, 0, [0.0, 1.2, 0.9]);
        rt.ai_tick(0, 0.1);
        assert_eq!(rt.shots_fired, 0, "the 3D eye distance was not the gate");

        let mut rt = fire_chain_runtime(class);
        rt_things_set_pos(&mut rt, 0, [0.0, 1.2, 0.9]);
        // The same pose against a 1.44 reach: 1.435 is inside.
        let class = "alignment=-1.0, rank=0.5, fov=360, sightdist=9.0, accuracy=1.0\n\n\
                     PrimaryFire 750.0, 0.0, 1.44, 0.0, 0.0, 500.0, 0.0, 0.0, 0.0, 0.0\n";
        rt.ai.insert(1, crate::ai::parse(class));
        rt.ai_tick(0, 0.1);
        assert_eq!(rt.shots_fired, 1, "a radius-subtracted 1.435 was refused inside a 1.44 reach");
    }

    /// The accuracy gate: the engine jitters only when `class.accuracy < rand()/32768`
    /// (`0x4eba10`), so a class with `accuracy=1.0` fires an exact line however large its
    /// `error` argument is. The port always jittered.
    #[test]
    fn a_perfectly_accurate_actor_shoots_a_clean_line() {
        let mut rt = fire_chain_runtime(
            "alignment=-1.0, rank=0.5, fov=360, sightdist=9.0, accuracy=1.0\n\n\
             PrimaryFire 750.0, 0.0, 9.0, 0.20, 0.0, 500.0, 0.0, 0.0, 0.0, 0.0\n",
        );
        crate::cog::set_random_seed(811_474_043);
        rt.ai_tick(0, 0.1);
        assert_eq!(rt.shots_fired, 1);
        let shot = rt.things.len() - 1;
        let vel = rt.things[shot].vel;
        assert!((vel[0] - 0.0).abs() < 1e-4 && (vel[1] - 4.0).abs() < 1e-4 && (vel[2] - 0.0).abs() < 1e-4,
            "accuracy 1.0 still jittered: {vel:?}");
    }

    /// The lead shot (`p7 & 1`): with the target an actor and moving, the engine aims along
    /// `norm(target.vel + dir * weaponspeed)` when that stays within `dot > 0.5` of the line.
    #[test]
    fn a_leading_actor_aims_ahead_of_its_target() {
        let mut rt = fire_chain_runtime(
            "alignment=-1.0, rank=0.5, fov=360, sightdist=9.0, accuracy=1.0\n\n\
             PrimaryFire 750.0, 0.0, 9.0, 0.0, 0.0, 500.0, 1.0, 0.0, 0.0, 0.0\n",
        );
        // The target is an AI actor moving at 2 u/s along +X: its physics velocity is what the
        // lead reads.
        rt.ai.insert(0, crate::ai::parse(""));
        rt.things[0].vel = [2.0, 0.0, 0.0];
        let outcome = rt.sith_ai_fire_weapon(1, 0, [0.0, 2.0, 0.0], 0.0, 9.0, 0.0, 0.0, 0, 1, Some(1.0));
        assert!(matches!(outcome, FireOutcome::Shot(_)), "refused: {outcome:?}");
        assert_eq!(rt.shots_fired, 1);
        let shot = rt.things.len() - 1;
        let vel = rt.things[shot].vel;
        let len = (2.0f32 * 2.0 + 4.0 * 4.0).sqrt();
        assert!(
            (vel[0] - 2.0 * 4.0 / len).abs() < 1e-4 && (vel[1] - 4.0 * 4.0 / len).abs() < 1e-4,
            "the shot did not lead the target's velocity: {vel:?}"
        );
    }

    /// The lob (`p7 & 2`): the engine scales the direction by the weapon speed, lifts z by
    /// `(dist/speed) * 0.5 * gravity`, renormalises, and passes the speed scale `len/speed` to
    /// `WeaponFire` — so the bolt leaves with exactly the arced velocity vector.
    #[test]
    fn a_lob_shot_arcs_with_gravity() {
        let mut rt = fire_chain_runtime(
            "alignment=-1.0, rank=0.5, fov=360, sightdist=9.0, accuracy=1.0\n\n\
             LobFire 2500.0, 0.8, 0.5, 4.0, 0.20, 0\n",
        );
        rt.gravity = 4.0;
        match rt.sith_ai_fire_weapon(1, 0, [0.0, 3.0, 0.0], 0.5, 4.0, 0.8, 0.0, 0, 2, Some(1.0)) {
            FireOutcome::Shot(shot) => {
                // dist = 3 - 0.065 = 2.935; lift = (2.935/4) * 0.5 * 4 = 1.4675.
                let lift = (2.935_f32 / 4.0) * 0.5 * 4.0;
                let vel = rt.things[shot].vel;
                assert!((vel[0] - 0.0).abs() < 1e-4, "{vel:?}");
                assert!((vel[1] - 4.0).abs() < 1e-4, "{vel:?}");
                assert!((vel[2] - lift).abs() < 1e-4, "the lob did not lift z by {lift}: {vel:?}");
            }
            other => panic!("the lob shot was refused: {other:?}"),
        }
    }

    /// The fire point: the engine's `WeaponFire` receives `ai + 0x82`, which `sub_4EAD60` built
    /// as `pos + orient * thing[0x17c]` — the template's `fireoffset=` through the whole oriented
    /// basis (forward `lvec`, right `rvec`, up `uvec`), pitch included.
    #[test]
    fn the_fire_point_is_the_oriented_fireoffset() {
        let mut rt = fire_chain_runtime(
            "alignment=-1.0, rank=0.5, fov=360, sightdist=9.0, accuracy=1.0\n\n\
             PrimaryFire 750.0, 0.0, 9.0, 0.0, 0.0, 500.0, 0.0, 0.0, 0.0, 0.0\n",
        );
        rt.template_fire_offset.insert("reeyeesgun".into(), [0.0, 2.0, 0.5]);
        rt.things[1].pitch = 30.0;
        match rt.sith_ai_fire_weapon(1, 0, [0.0, 3.0, 1.0], 0.0, 9.0, 0.0, 0.0, 0, 0, Some(1.0)) {
            FireOutcome::Shot(shot) => {
                // The full basis at yaw 0: `lvec = (0, cp, sp)`, `rvec = (1, 0, 0)` and
                // `uvec = (0, -sp, cp)` — the up vector's y tilts back with the pitch, which is
                // what `rdMatrix_TransformVector34` multiplies the offset's z by.
                let (s, c) = 30f32.to_radians().sin_cos();
                let want = [0.0, 2.0 * c - 0.5 * s, 2.0 * s + 0.5 * c];
                let pos = rt.things[shot].pos;
                for i in 0..3 {
                    assert!((pos[i] - want[i]).abs() < 1e-4, "fire point {pos:?} != {want:?}");
                }
            }
            other => panic!("the shot was refused: {other:?}"),
        }
    }

    /// A thing flagged `0x20000` (a puppet actor) does not fire inline: the engine stores the
    /// shot (`fl|8`, error, mode) and plays the fire track, and the bolt leaves when the track
    /// reaches its fire marker. The port has no marker clock, so the deferred shot leaves on the
    /// actor's next fire pass — no shipped level-1/2 template carries the flag.
    #[test]
    fn a_puppet_flagged_actor_defers_its_shot() {
        let mut rt = fire_chain_runtime(
            "alignment=-1.0, rank=0.5, fov=360, sightdist=9.0, accuracy=1.0\n\n\
             PrimaryFire 750.0, 0.0, 9.0, 0.0, 0.0, 500.0, 0.0, 0.0, 0.0, 0.0\n",
        );
        // The engine tests the **actor word** (`thing + 0x130`, `puVar1[0x4c] & 0x20000`), not
        // the thing's own flags.
        rt.actor_flags
            .insert(1, rt.actor_flags.get(&1).copied().unwrap_or(0) | 0x20000);
        rt.ai_tick(0, 0.1);
        assert_eq!(rt.shots_fired, 0, "the puppet actor fired inline");
        assert!(rt.ai_deferred_fire.contains_key(&1), "the shot was not stored");
        assert_eq!(rt.play_mode.get(&1), Some(&(8)), "the fire track was not played");
        // The marker stand-in: the stored shot leaves on the next pass with its gates skipped.
        rt.things[1].ai_wait = 0.0;
        rt.ai_tick(0, 0.1);
        assert_eq!(rt.shots_fired, 1, "the deferred shot never left");
        // The puppet flag is still set, so the actor's own cadence defers its next shot again —
        // the engine's marker loop does the same while `0x20000` stays on the thing.
    }

    /// `sithAICmd_PrimaryFire` (`0x509e40`) sets AI mode bit `0x400` when `FireWeapon` returns
    /// fired — the bit `HitAndRun` (`0x50bb90`) counts on, which the port used to set only in
    /// saberfighting.
    #[test]
    fn firing_sets_the_hit_and_run_mode_bit() {
        let mut rt = fire_chain_runtime(
            "alignment=-1.0, rank=0.5, fov=360, sightdist=9.0, accuracy=1.0\n\n\
             PrimaryFire 750.0, 0.0, 9.0, 0.0, 0.0, 500.0, 0.0, 0.0, 0.0, 0.0\n",
        );
        rt.ai_tick(0, 0.1);
        assert_eq!(rt.shots_fired, 1);
        assert!(rt.ai_flags.get(&1).is_some_and(|f| f & 0x400 != 0), "no 0x400 after firing");
    }

    /// `mode 1` fires the second weapon (`thing[0x55]`, the template's `weapon2=`) and plays the
    /// `0x48` sound row; the primaryfire handler rolls it against `fltArg[7]` (the Ree-Yees
    /// punches with `weapon2=+ry_punch` at chance 1.00).
    #[test]
    fn a_fire2_roll_switches_to_the_second_weapon() {
        let mut rt = fire_chain_runtime(
            "alignment=-1.0, rank=0.5, fov=360, sightdist=9.0, accuracy=1.0\n\n\
             PrimaryFire 1500.0, 0.6, 9.0, 0.00, 0.0, 200.0, 0.0, 1.00, 0.00, 0.00\n",
        );
        rt.template_vel.insert("+ry_punch".into(), [0.0, 2.0, 0.0]);
        rt.template_weapon2.insert("reeyeesgun".into(), "+ry_punch".into());
        rt.ai_tick(0, 0.1);
        assert_eq!(rt.shots_fired, 1);
        let shot = rt.things.len() - 1;
        let vel = rt.things[shot].vel;
        assert!((vel[1] - 2.0).abs() < 1e-4, "the first weapon fired, not weapon2: {vel:?}");
    }

    /// #93's encounter regression, root-caused: the hit test swept in **plan only**, so a
    /// descending (lob) bolt with no plan motion took the `len2 == 0` default — the segment's
    /// far endpoint — and skipped the body's z-band entirely. The engine sweeps the whole step
    /// (`sithCollision_MoveThing`): a bolt crossing the band mid-step hits.
    #[test]
    fn a_falling_bolt_crossing_the_body_band_mid_step_hits() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        // A standing body 0.18 tall at the origin, and a bolt descending through x = 0.05
        // (inside the radius sum 0.115): this tick it steps from z +0.3 to z -0.3 — straight
        // through the band.
        let mut bolt = base.clone();
        bolt.pos = [0.05, 0.0, -0.3];
        bolt.vel = [0.0, 0.0, -6.0];
        bolt.damage = 10.0;
        rt.things = vec![base, bolt];
        rt.templates = vec!["walkplayer".into(), "+bolt".into()];
        rt.types = vec![10, 0];
        rt.template_hit.insert("+bolt".into(), (0.05, 0.1));
        let hits = rt.impacts(1, 0.1);
        assert!(!hits.is_empty(), "the falling bolt tunneled through the body's z-band");
    }
    fn rt_things_set_pos(rt: &mut Runtime, index: usize, pos: [f32; 3]) {
        rt.things[index].pos = pos;
    }
}
