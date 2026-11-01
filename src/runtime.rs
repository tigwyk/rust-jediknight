//! The world state COG scripts act on.
//!
//! The interpreter (`cog.rs`) runs every shipped script, but a script only becomes
//! game logic when its verbs reach real state. This module holds that state — thing
//! positions, per-thing flags, sector adjoin flags, inventory — and answers the verbs
//! that read or write it. Everything starts from the level file and is then mutated
//! by scripts, exactly as the engine does.

use std::collections::BTreeMap;

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

#[derive(Clone)]
pub struct ThingState {
    /// The thing's model INSERT OFFSET, for CreateThing's marker-relative positioning.
    pub insert: [f32; 3],
    pub pos: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
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
    /// `GetPhysicsFlags`/`SetPhysicsFlags` (`physflags=`), and the `jkSetFlags` bits the
    /// force scripts set (`0x20` while a power is mid-flash).
    pub phys_flags: u32,
    pub jk_flags: u32,
    /// `AttachThingToThing(child, parent)`: the parent this thing rides, and where it sits in that
    /// parent's own frame — `sithThing_AttachThingToThing` stores exactly that offset (in
    /// `field_4C`), so a moving parent carries the child.
    pub attach: Option<usize>,
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
pub const DAMAGE_SABER: u32 = 0x10;
pub const DAMAGE_FALL: u32 = 0x40;
/// `SITH_AF_INVULNERABLE`: `actor->flags` bit 3. `sithActor_DamageActor` checks it before anything
/// else, so `SetActorFlags(player, 0x8)` is how a script makes someone untouchable.
pub const AF_INVULNERABLE: u32 = 0x8;
/// `SITH_SECTOR_COGLINKED`: a cog is placed on this sector, so it can hear `sighted`/`entered`.
pub const SECTOR_COGLINKED: u32 = 0x4;
/// `SITH_SECTOR_SEEN`: latched the first time the renderer draws the sector.
pub const SECTOR_SEEN: u32 = 0x4000;
/// `SITH_TF_CAPTURED`: the thing has a cog placed on it (`sithRender_RenderThing` is what checks it).
pub const TF_CAPTURED: u32 = 0x400;
/// `SITH_TF_INCAMFOV`: latched the first time the renderer draws the thing.
pub const TF_INCAMFOV: u32 = 0x100000;
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

/// Per solid template, the **top and footprint** of the model it draws: `(top_offset, half_x, half_y)`
/// in model space, which is world space here. `sithCollision` treats a thing's own model as solid for
/// every body, so a catwalk slab or a crate top is a floor; the port's surface sweep only knows the
/// level's geometry. Filled by the loader (which has the models), read by `Runtime::thing_floor`.
static TEMPLATE_FLOORS: std::sync::OnceLock<BTreeMap<String, (f32, f32, f32)>> =
    std::sync::OnceLock::new();

/// Register the solid templates' model tops. Called once by the loader; a later call is ignored.
pub fn set_template_floors(map: BTreeMap<String, (f32, f32, f32)>) {
    let _ = TEMPLATE_FLOORS.set(map);
}

fn template_floor(template: &str) -> Option<(f32, f32, f32)> {
    TEMPLATE_FLOORS
        .get()?
        .get(&template.to_ascii_lowercase())
        .copied()
}

/// The radius a placed thing's **collision volume** has, from the engine's own template rule.
///
/// `misc\things.dat`'s class rows carry `collide=`: `_structure` is `collide=3` and `_decor` is
/// `collide=0`, and `_walkstruct` inherits the first through its `_structure` base. Every placed
/// template carries its own `size=` (the radius), so a thing is solid exactly when its chain's
/// `collide` is non-zero. That is why `column01` (`_structure`, `size=0.204987`) stops a body in
/// retail while `tuskensoused` (`_ghostdecor`) does not — and why the port, which has no
/// model collision at all, walks through both.
///
/// Returns `None` for a thing the engine lets a body pass through.
pub fn solid_radius(level: &crate::jkl::Level, template: &str) -> Option<f32> {
    let collide = level.template_param(template, "collide")?.trim().parse::<i32>().ok()?;
    if collide == 0 {
        return None;
    }
    // The radius is the template's own `size=`; `movesize=` is the same for everything placed.
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
    pub slide: [f32; 2],
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
/// Turn `before` toward `wanted` by at most `rate * dt` degrees, taking the short way round.
///
/// The engine turns a thing with `maxrotthrust=`, an angular rate; snapping the yaw instead made
/// every AI face the player the moment it noticed them, so engagements started and finished faster
/// than retail's.
fn turn_toward(before: f32, wanted: f32, rate: f32, dt: f32) -> f32 {
    let mut delta = (wanted - before + 180.0).rem_euclid(360.0) - 180.0;
    let limit = rate.max(0.0) * dt.max(0.0);
    if delta.abs() > limit {
        delta = limit * delta.signum();
    }
    before + delta
}

/// `sithAIAwareness_ProcessEvent`'s floor (`0x52315c`): a propagated event stops crossing adjoins
/// once its strength falls below this.
pub const AI_AWARE_FLOOR: f32 = 0.00277;
/// The strength a fired shot's event carries: `sithWeapon_WeaponFire` (`0x4d44f0`) creates it with
/// `0x40800000` = 4.0. `sithActor_KillActor` uses 5.0 and `sithWeapon_DestroyWeapon` 2.0; the port
/// raises the firing one, which is what a fight is mostly made of.
pub const AI_AWARE_SHOT: f32 = 4.0;
/// How long a record stays fresh. The engine's own fade is not recovered; the flat value keeps the
/// old stand-in's window.
pub const AI_AWARE_SECONDS: f32 = 8.0;
/// The strength `sensedanger` gives the awareness event it raises when an awake actor spots the player:
/// the handler (`0x50b9b0`) passes `0x40400000` = 3.0, so the same side converges on the fight.
pub const AI_AWARE_ALERT: f32 = 3.0;
/// The `listen` handler's two distance/bearing constants, read from the retail `JK.EXE`:
/// `_DAT_005232c8` = **1.0** unit and `_DAT_005232a0` = **0.5** (cos 60°). A sound heard through
/// geometry is only heard when it is nearer than one unit **and** inside a 60° half-cone in front;
/// a sound in clear line of sight is heard at any bearing inside `heardist`.
pub const AI_LISTEN_NEAR: f32 = 1.0;
pub const AI_LISTEN_FRONT_DOT: f32 = 0.5;
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

/// Whether an actor at `from` facing `forward` **hears** the sound at `pos` — the `listen` handler's
/// own test (`0x50a6f0`). Inside `heardist` always; a sound the path finds `blocked` (geometry
/// between the two) only when it is nearer than [`AI_LISTEN_NEAR`] and the forward dot is above
/// [`AI_LISTEN_FRONT_DOT`], i.e. within 60° of straight ahead.
fn hearable(from: [f32; 3], forward: [f32; 2], pos: [f32; 3], heardist: f32, blocked: bool) -> bool {
    let (dx, dy, dz) = (pos[0] - from[0], pos[1] - from[1], pos[2] - from[2]);
    let reach = (dx * dx + dy * dy + dz * dz).sqrt();
    if reach > heardist {
        return false;
    }
    if !blocked {
        return true;
    }
    reach < AI_LISTEN_NEAR
        && reach > 1e-4
        && (forward[0] * dx + forward[1] * dy) / reach > AI_LISTEN_FRONT_DOT
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
    /// A template's `physflags=`: a created thing starts with it (the engine copies the template word),
    /// and the generic mover reads bit `0x1` (gravity) and its thruster exemption `0x400000`.
    pub template_phys_flags: BTreeMap<String, u32>,
    /// World gravity, from the level's own `World Gravity` line: what pulls a physics thing down.
    pub gravity: f32,
    /// Every shot the runtime spawned, so a check can ask "did the trigger work" without sampling
    /// live things: a bolt can be born and destroyed between two frames at 10 fps.
    pub shots_fired: usize,
    /// A template's `health=`/`maxhealth=`, so a created thing starts with real health.
    pub template_health: BTreeMap<String, f32>,
    /// A template's `vel=` — a projectile's launch speed — and its `damage=`.
    pub template_vel: BTreeMap<String, [f32; 3]>,
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
    /// A template's `thingflags=`, the word `SITH_TF_*` bits live in: a shot starts with it, and the
    /// player-fire pre-sweep tests its bit `0x200000` before raising `dodge`'s shot event.
    pub template_thingflags: BTreeMap<String, u32>,
    /// A template's full `typeflags=` word (the `SITH_WF_*`/`SITHEXPLOSION_FLAG_*` bits). `+bryarbolt`
    /// ships `0x20440d`, whose bit `0x200000` is what `sithWeapon_WeaponFireProjectile`'s player
    /// pre-sweep tests before it raises `dodge`'s shot event.
    pub template_typeflags: BTreeMap<String, u32>,
    /// A template's `soundclass=`, keyed by template name and reduced to the `.snd` class's lowercase
    /// basename: the class `talk` picks its mood sounds from (`sithSoundClass_PlayModeRandom`).
    pub template_soundclass: BTreeMap<String, String>,
    /// Every shipped `.snd` class's rows, by the class's lowercase basename and then by the engine's
    /// sound-mode **event name** (`create`, `happy`, `fear`, …), each a list of `(wav, maxVolume)`.
    /// The runtime has no file access, so `setup` hands this over after the loader parses them.
    pub sound_classes: BTreeMap<String, BTreeMap<String, Vec<(String, f32)>>>,
    /// `timer=`: how long a created thing lives, in seconds, before `sithThing_Update` destroys it.
    /// An impact effect's whole lifetime is this field.
    pub template_timer: BTreeMap<String, f32>,
    /// `SetThingTimer`'s deadline per thing, on the world clock: the engine's `pThing->timer` plus
    /// `SITH_TF_TIMERSET`. When it passes, the thing's own scripts get the `timer` message.
    pub thing_timer: BTreeMap<usize, f32>,
    /// The player's shield bar (`SITHBIN_SHIELDS`, 0..=200) and whether health has reached 0.
    pub player_shields: f32,
    pub player_dead: bool,
    /// `(cog instance, link id, surface)` for every `surface` symbol a level's scripts declare:
    /// `sithCog_LinkCogToSurface` in tabular form. The player's use action messages these.
    pub surface_links: Vec<(usize, i32, usize)>,
    /// Portals: `(sector a, sector b, centre)` for every surface two sectors share. Actors steer
    /// through these when the straight line to their target is walled off, which is the cheap half
    /// of the engine's pathing: it routes through doorways without a graph search.
    pub portals: Vec<(usize, usize, [f32; 3])>,
    /// `(cog instance, link id, thing)` for every `thing` symbol a level's scripts declare:
    /// `sithCog_LinkCogToThing` in tabular form.
    pub thing_links: Vec<(usize, i32, usize)>,
    /// Messages a script has earned but that have not been delivered yet: the runtime holds no VM,
    /// so it queues `(cog instance, label, params, sender)` and a system drains it. The params are what
    /// `GetParam(0)`/`GetParam(1)` answer inside the handler — `damaged` carries the amount and the
    /// damage type, which is how `force_absorb.cog` decides how much of it to take.
    pub pending_messages: Vec<(usize, &'static str, [f64; 2], Option<i64>)>,
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
    /// `AISetMoveSpeed`: a multiplier on the actor's walking speed, clamped 0..2 by the engine
    /// (`sithCogFunctionAI.c:16`). Absent means 1.
    pub actor_speed: std::collections::BTreeMap<usize, f32>,
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
    pub scroll_bucket: i32,
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
    /// `(cog instance, link id, sector)` for every `sector` symbol a level's scripts declare:
    /// `sithCog_LinkCogToSector` in tabular form, so a sector's `entered`/`exited` can find its cogs.
    pub sector_links: Vec<(usize, i32, usize)>,


    /// The actors that walked on the last `ai_tick`, so the caller can put them back on the floor
    /// and out of walls: `ai_tick` has no collider (the runtime is level-agnostic), and this is the
    /// hand-off to the system that does.
    pub walked: Vec<usize>,
    /// The velocity each of those actors walked at, in world units per second: `(x, y)`. This is
    /// the port's `physicsParams.vel` for an actor — `sithPuppet_UpdateThingMove` transforms it
    /// into the thing's own frame to choose `stand`/`walk`/`run`/`walkback`/`strafe*`, and nothing
    /// else knows how fast a body is walking: the AI writes positions, and a collision push can
    /// move an actor further in one frame than a stride would.
    pub walk_vel: std::collections::BTreeMap<usize, [f32; 2]>,
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
    /// `SetWeaponMesh`/`jkSetPOVModel`/`jkSetWaggle`/`jkPlayPOVKey`: the first-person weapon's own
    /// data. The amplitudes are **degrees** and the rate is the degrees the sway phase advances per
    /// unit of movement (`jkPlayer_SetWaggle(thing, velocities, rate)`); `jkPlayer_DrawPov` uses both.
    pub weapon_mesh: BTreeMap<usize, String>,
    pub pov_model: BTreeMap<usize, String>,
    pub pov_key: BTreeMap<usize, String>,
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
    /// `SetCurWeapon`'s weapon thing, when a script named one.
    pub cur_weapon_thing: Option<usize>,
    /// `GetThingClassCog`: the cog a thing's class names.
    pub thing_class_cog: BTreeMap<usize, i32>,
    /// Things the engine has woken. `sithRender` latches `SITH_SECTOR_SEEN` the first time a sector is
    /// drawn, and a thing's class cog is sent `created` when its thing is woken — **not** at level
    /// load. Retail's own saves show it: level 1's load-time autosave carries no population bin
    /// (73 = 0), while level 2's carries 10.5 after the level was played. Every placed thing used to
    /// wake at load here, so level 1's population read 12.5 before the player had moved.
    pub awakened: std::collections::BTreeSet<usize>,
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
    pub messages_sent: usize,
    pub sectors: Vec<SectorState>,
    /// Each sector's `SITH_SECTOR_*` word, as the level ships it and as `SetSectorFlags` and
    /// `SetSectorAdjoins` change it. `0x2` underwater, `0x80` adjoins hidden — the two the
    /// shipped scripts switch, and the two collision reads (`collide::Gates`).
    pub sector_flags: Vec<u32>,
    /// Each `World adjoins` row's `SITH_SURFACE_ADJOIN_*` word: `SetAdjoinFlags`/
    /// `ClearAdjoinFlags` switch these, which is how a force field opens and closes.
    pub adjoin_flags: Vec<u16>,
    /// Surface → the adjoin row it carries (`-1` for a wall), so a script's `surface` symbol
    /// reaches its `pAdjoin` without the runtime holding the whole level.
    pub surface_adjoin: Vec<i32>,
    /// Each face's `geometryMode` as the loader sets it: the `.jkl` `geo` column, forced to
    /// `GEOMETRY_NONE` for a face with no material (`sithSurface.c:225`–`237`). A script's
    /// `SetFaceGeoMode` overrides it in `face_geomode`; `GetFaceGeoMode` reads both.
    pub surface_geomode: Vec<i32>,
    /// Per-surface state, indexed the way `.jkl` surfaces are: cels, animation, slides.
    pub surfaces: Vec<SurfaceState>,
    /// Sounds a script asked for, in order: the wav, its volume, and where it came from.
    ///
    /// `PlaySoundThing` names a thing and `PlaySoundPos` a position; a request with no place
    /// is `PlaySoundLocal`, which the mixer does not attenuate by distance.
    pub sounds: Vec<SoundRequest>,
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
    /// `AISetMovePos` / `AISetMoveThing` / `AISetLookPos`: the scripted destination, the thing to
    /// follow instead of a fixed point, and the spot a standing actor turns to face. The engine keeps
    /// the same three on the actor block (`movePos`, `pMoveThing`, look frame) — see `ai_tick`.
    pub ai_move: BTreeMap<usize, [f32; 3]>,
    pub ai_chase: BTreeMap<usize, usize>,
    pub ai_look: BTreeMap<usize, [f32; 3]>,
    /// Where an actor last had the player in view (`sithAI`'s last known position). Losing sight
    /// means "walk there", and arriving clears it.
    pub ai_last_seen: BTreeMap<usize, [f32; 3]>,
    /// Actors that have **registered** the player: `sithAI_CanDetectSightThing` (`0x4ec140`) is a
    /// probability roll and the engine re-rolls it while refreshing a target; the port rolls it on
    /// acquisition and then keeps the deterministic sight test (ledger).
    pub ai_detected: std::collections::BTreeSet<usize>,
    /// When each actor may run its `LookForTarget` scan next. The instinct's first shipped argument
    /// is the interval (500 ms in every class), and a scan that finds a target hands it to the normal
    /// chase/aim logic rather than snapping the body round.
    pub ai_scan: BTreeMap<usize, f32>,
    /// The one roll `listen` makes per heard sound: which event an actor has answered, and whether
    /// the class's reaction chance passed. `sithAI_EmitEvent` invokes the handler once per event, so
    /// a failed roll must not be retried while that same sound is still in the sector's ring.
    pub ai_listen: BTreeMap<usize, (f32, [f32; 3], bool)>,
    /// When each actor may run its `Talk` instinct next. The handler sets its own next run from the
    /// instinct's first number (`talk 10000.0, 0.10` in most shipped classes), so it keeps no global
    /// interval.
    pub ai_talk: BTreeMap<usize, f32>,
    /// When each actor's `lobfire` may re-arm the puppet's armed mode. Its handler (`0x509cd0`) only
    /// has the event-`0x100` branch in its mask, and that branch sets
    /// `sithPuppet_SetArmedMode(thing, (ai flags & 0x200) != 0)` and re-arms in 1000 ms.
    pub ai_lobfire: BTreeMap<usize, f32>,
    /// When each not-yet-alerted actor may check for danger again (`sensedanger` retries once a second
    /// while the player is out of sight).
    pub ai_sense: BTreeMap<usize, f32>,
    /// When each actor may dodge again. `dodge`'s damage path (`0x50b360` event 1) sets its own
    /// cooldown to `now + 1000` ms (`ai + 0xa3`), and the shot-passing path has none of its own.
    pub ai_dodge: BTreeMap<usize, f32>,
    /// When each actor's `jump` may nudge again: the handler (`0x50abf0`, event `0x400`) sets its own
    /// next check to `now + the class line's first number` (`jump 2000.0, 3.0, 3.0`).
    pub ai_jump: BTreeMap<usize, f32>,
    /// When each actor may try the `opendoors` action next. Its handler (`0x50abc0`) calls
    /// `sithPlayerActions_Activate` — the **player's** use action — on the actor's own body, and that
    /// function throttles itself to 250 ms. An actor with the instinct opens the door it is walking
    /// into instead of being trapped behind it.
    pub ai_opendoors: BTreeMap<usize, f32>,
    /// Each actor's home anchor: where it was placed. `returnhome`'s handler (`0x50bd70`) walks the
    /// actor back to `ai + 0x93` (`SetMoveThing`) and then turns it there (`SetLookFrame`), so the
    /// anchor is the thing's spawn position. Set when the instincts are attached to the live runtime.
    pub ai_home: BTreeMap<usize, [f32; 3]>,
    /// When each scripted `AIFlee` ends. The verb (`0x501420`) only sets the fleeing mode; the class's
    /// `Flee` instinct is what walks, and its third argument is the duration in seconds (1–2 s in the
    /// shipped classes), after which the handler clears the mode. The port records the same deadline.
    pub ai_flee_until: BTreeMap<usize, f32>,
    /// Actors in the `retreat`/`withdraw` mode: the thing they are backing away from, and when the
    /// mode expires. `retreat` (`0x50bc60`) sets it when health falls under `arg0 × max` and its
    /// `arg1` chance succeeds; `withdraw` (`0x50b150`) walks the actor away from that thing. The port
    /// reuses the scripted flee walk and re-arms on the withdraw instinct's own interval (5000 ms in
    /// the shipped classes).
    pub ai_withdraw: BTreeMap<usize, (usize, f32)>,
    /// The signed side (`, -1` or `+1`) each actor circles a target on, for `circlestrafe`
    /// (`0x509890`). The handler picks it at random the first time and flips it when the step is
    /// blocked; the port persists it the same way so the orbit does not jitter every frame.
    pub ai_strafe: BTreeMap<usize, f32>,
    /// When each actor started its current `hitandrun` engagement — its first shot after it came back.
    /// The handler (`0x50bb90`) watches the engine's just-fired flag and breaks off after the
    /// instinct's first argument (1000–10000 ms shipped) for its second (1500–3000 ms); the port
    /// records the first firing and arms the same withdraw mode `retreat` uses.
    pub ai_hitrun: BTreeMap<usize, f32>,
    /// Each idle actor's next `randomturn` due time and the yaw it chose, for the `randomturn`
    /// handler (`0x50b700`): every class interval (1000–2000 ms shipped) the actor picks a random
    /// direction and turns to look at a point its own argument's distance away (`SetLookFrame`).
    pub ai_randomturn: BTreeMap<usize, (f32, f32)>,
    /// Each actor's current `roam` point and when it may pick a new one. The handler (`0x50b830`)
    /// wanders around the **home anchor** (`ai + 0x93`): a random direction, a random distance inside
    /// the instinct's first argument, every class interval, then `SetLookFrame` + `SetMovePos` there.
    pub ai_roam: BTreeMap<usize, (f32, [f32; 3])>,
    /// `sithAIAwareness`: a position other actors of the same side were told about, and when. A
    /// shot is the event; the engine keeps per-sector records that propagate through adjoins and
    /// fade. This build has neither the per-adjoin attenuation nor the recovered reader, so an
    /// alert lands on buyers within a radius and expires (see `AWARE_RADIUS`/`AWARE_SECONDS`).
    /// `sithAIAwareness`'s records: per sector and side, the **ring of recent sounds** — newest
    /// first, at most three entries (the engine's listener scans three), each `(strength, position,
    /// time)`. `ProcessEvent` keeps a 25-dword block per sector indexed by side; these are the fields
    /// the AI actually reads plus the fade time.
    pub sector_awareness: BTreeMap<(i32, i32), Vec<(f32, [f32; 3], f32)>>,
    /// The adjoin graph the record propagates through: `(neighbour sector, cost)` per sector. The
    /// cost is the adjoin's own `dist` float — the only per-portal number a level carries, and the
    /// field `004f2c30_sithAIAwareness_ProcessEvent.c` reads through the adjoin link.
    pub sector_adjoins: Vec<Vec<(i32, f32)>>,
    /// Per actor, the doorway centres it is walking through when its straight line is walled off,
    /// from `Collision::route`. A waypoint is popped on arrival, and a scripted destination starts
    /// again with an empty route.
    pub ai_route: BTreeMap<usize, Vec<[f32; 3]>>,
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
    /// How close an actor walks to the player before it stops: this build's standoff, since
    /// the engine's is in the instinct args.
    pub ai_standoff: f32,
    /// An actor template's walking speed, `maxthrust / surfdrag` — the same conversion `Rules`
    /// uses for the player.
    pub template_speed: BTreeMap<String, f32>,
    /// An actor template's turn rate, `maxrotthrust=` in degrees a second (`_actor` 180, a Ree-Yees
    /// gunner 70). The body turns at this rate; it does not snap.
    pub template_turn: BTreeMap<String, f32>,
    /// Redbook music: `(start track, end track, loop-back track)` as `PlaySong` set it.
    pub music: Option<(i32, i32, i32)>,
    /// `jkEndLevel(flag)`: the level is finished; the campaign advances when the app notices.
    /// The engine registers the verb as `jkendlevel` (`jkCog.c`), and a level's final script
    /// is what calls it.
    pub level_complete: bool,
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
    /// What the scripts actually changed, for the run report.
    pub adjoins_changes: usize,
    pub things_moved: usize,
}

impl Runtime {
    pub fn new(level: &Level) -> Self {
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
        // `sithAIAwareness` propagates through the **adjoin graph** (`ProcessEvent` recurses over a
        // sector's adjoin list, subtracting each adjoin's attenuation). An adjoin is shared by the
        // two surfaces of one opening, so pairing those surfaces gives the sector-to-sector links;
        // the cost is the adjoin row's `dist` float, which is the only per-portal number shipped.
        let mut sector_adjoins: Vec<Vec<(i32, f32)>> = vec![Vec::new(); level.sectors.len()];
        {
            // A surface's sector: the level groups surfaces per room (`SECTOR` rows carry
            // `(first, count)` over the surface list).
            let mut surface_sector: Vec<i32> = vec![-1; level.surfaces.len()];
            for (sector, room) in level.sectors.iter().enumerate() {
                let (first, count) = room.surfaces;
                for index in first..(first + count).min(level.surfaces.len()) {
                    surface_sector[index] = sector as i32;
                }
            }
            // Each surface owns one adjoin row, and the row's `mirror` names the row of the surface on
            // the other side of the same opening (`World adjoins 0: 0x7 50 2.00` — flags, mirror,
            // dist). Pairing those rows gives the sector-to-sector links.
            let mut surface_of_adjoin: Vec<Option<usize>> = vec![None; level.adjoins.len()];
            for (index, surface) in level.surfaces.iter().enumerate() {
                if let Some(slot) = usize::try_from(surface.adjoin).ok().filter(|slot| *slot < level.adjoins.len()) {
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
                sector_adjoins[a as usize].push((b, cost));
                sector_adjoins[b as usize].push((a, cost));
            }
        }
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
        // aimed at the player. The height is the model height the templates imply.
        let template_hit: BTreeMap<String, (f32, f32)> = level
            .templates
            .iter()
            .map(|t| {
                let name = t.name.to_ascii_lowercase();
                let radius = level
                    .template_f32(&t.name, "movesize")
                    .or_else(|| level.template_f32(&t.name, "size"))
                    .unwrap_or(0.065)
                    .max(0.02);
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
        Self {
            things: level
                .things
                .iter()
                .map(|t| ThingState {
                    pos: t.pos,
                    insert: template_insert(&t.template.to_ascii_lowercase()),
                    yaw: t.yaw,
                    pitch: t.pitch,
                    roll: t.roll,
                    sector: t.sector,
                    flags: t.flags,
                    health: template_health
                        .get(&t.template.to_ascii_lowercase())
                        .copied()
                        .unwrap_or(1.0),
                    frame: 0.0,
                    target: None,
                    frames: t.frames.clone(),
                    destroyed: false,
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
                    owner: None,
                    attach_offset: [0.0; 3],
                    attach_flags: 0,
                    spin: None,
            rot_vel: [0.0; 3],
            paused: false,
                    light: 0.0,
                    light_end: 0.0,
                    light_rate: 0.0,
                })
                .collect(),
            templates: level.things.iter().map(|t| t.template.clone()).collect(),
            types: level
                .things
                .iter()
                .map(|t| thing_type(level.template_param(&t.template, "type")))
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
            gravity: level.gravity,
            shots_fired: 0,
            prints: Vec::new(),
            hud_message: None,
            string_buffer: String::new(),
            loaded_sounds: Vec::new(),
            weapon_cog: None,
            weapon_rate: 0.0,
            weapon_ready: 0.0,
            auto_switch: 1,
            auto_pickup: 1,
            weapon_mesh: BTreeMap::new(),
            pov_model: BTreeMap::new(),
            pov_key: BTreeMap::new(),
            waggle: BTreeMap::new(),
            waggle_rate: BTreeMap::new(),
            key_lengths: BTreeMap::new(),
            last_shot: BTreeMap::new(),
            last_hit: BTreeMap::new(),
            armed_mode: BTreeMap::new(),
            major_mode: BTreeMap::new(),
            cur_weapon_thing: None,
            thing_class_cog: BTreeMap::new(),
            awakened: std::collections::BTreeSet::new(),
            saber_enabled: true,
            saber_cam: 0,
            item_cog: BTreeMap::new(),
            item_max: BTreeMap::new(),
            bin_cog_index: BTreeMap::new(),
            pending_cogs: Vec::new(),
            spawned_cogs: std::collections::BTreeSet::new(),
            messages: Vec::new(),
            caller_cog: -1,
            messages_sent: 0,
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
            template_explode,
            template_fire_offset,
            template_timer,
            template_thingflags,
            template_typeflags,
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
            thing_timer: BTreeMap::new(),
            player_shields: 100.0,
            player_dead: false,
            ir_mode: false,
            map_mode_flags: 0,
            walked: Vec::new(),
            walk_vel: BTreeMap::new(),
            sector_links: Vec::new(),
            surface_links: Vec::new(),
            mana_regen: MANA_REGEN,
            mana_max: MANA_MAX,
            actor_flags: Default::default(),
            actor_speed: Default::default(),
            item_flags: Default::default(),
            life_left: Default::default(),
            life_total: Default::default(),
            mass: Default::default(),
            user_data: Default::default(),
            force: Default::default(),
            actor_weapon: Default::default(),
            play_mode: Default::default(),
            signature: Vec::new(),
            view_list: Vec::new(),
            view_at: 0,
            last_params: Vec::new(),
            portals: Vec::new(),
            thing_links: Vec::new(),
            pending_messages: Vec::new(),
            solids: None,
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
            adjoin_flags: level.adjoins.iter().map(|a| a.flags).collect(),
            surface_adjoin: level.surfaces.iter().map(|s| s.adjoin).collect(),
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
                    SurfaceState { center, cel: 0.0, anim: 0.0, slide: [0.0; 2], next: 0.0, light: 0.0, light_end: 0.0, light_rate: 0.0 }
                })
                .collect(),
            sounds: Vec::new(),
            timers: 0.0,
            inv: BTreeMap::new(),
            inv_available: BTreeMap::new(),
            inv_activated: BTreeMap::new(),
            bin_active: BTreeMap::new(),
            bin_due: Vec::new(),
            bin_wait: BTreeMap::new(),
            time: 0.0,
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
            script_camera: false,
            shake_pos: [0.0; 3],
            shake_angle: [0.0; 3],
            shake_pos_rate: 0.0,
            shake_angle_rate: 0.0,
            geomode_epoch: 0,
            surface_epoch: 0,
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
            scroll_bucket: 0,
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
            ai_standoff: 0.25,
            ai_move: BTreeMap::new(),
            ai_chase: BTreeMap::new(),
            ai_look: BTreeMap::new(),
            ai_route: BTreeMap::new(),
            ai_last_seen: BTreeMap::new(),
            ai_detected: std::collections::BTreeSet::new(),
            ai_scan: BTreeMap::new(),
            ai_listen: BTreeMap::new(),
            ai_talk: BTreeMap::new(),
            ai_lobfire: BTreeMap::new(),
            ai_sense: BTreeMap::new(),
            ai_dodge: BTreeMap::new(),
            ai_jump: BTreeMap::new(),
            ai_opendoors: BTreeMap::new(),
            ai_home: BTreeMap::new(),
            ai_flee_until: BTreeMap::new(),
            ai_withdraw: BTreeMap::new(),
            ai_strafe: BTreeMap::new(),
            ai_hitrun: BTreeMap::new(),
            ai_randomturn: BTreeMap::new(),
            ai_roam: BTreeMap::new(),
            sector_awareness: BTreeMap::new(),
            sector_adjoins: sector_adjoins.clone(),
            ai_fire_target: BTreeMap::new(),
            ai_flee: BTreeMap::new(),
            ai_classes: BTreeMap::new(),
            level_complete: false,
            autosave: false,
            music: None,
            music_volume: 1.0,
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
            // `maxrotthrust` is an angular *thrust*, not a rate: the engine's `ApplyDrag` decays
            // rotation the same way it decays velocity, so the steady turn is `maxrotthrust / drag`
            // — a Ree-Yees gunner's 70 over the same `surfdrag` 3 is 23 degrees a second, which is
            // what spaces retail's engagements out (its door series is still neutral at 8 s).
            template_turn: level
                .templates
                .iter()
                .map(|t| {
                    let thrust = level.template_f32(&t.name, "maxrotthrust").unwrap_or(180.0);
                    let drag = level
                        .template_f32(&t.name, "surfdrag")
                        .filter(|d| *d > 0.0)
                        .unwrap_or(3.0);
                    (t.name.to_ascii_lowercase(), thrust / drag)
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
            adjoins_changes: 0,
            things_moved: 0,
        }
    }

    pub fn handle(level: &Level) -> World {
        World::new(Self::new(level))
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

    /// A scrolling surface (`SlideWall`) needs its texture re-laid, and the mesh is rebuilt whole, so
    /// the epoch moves at a fixed **15 Hz** rather than every frame: the offset itself is computed
    /// from the clock (`slide * time`), so the only cost of the slower epoch is that the mesh shows an
    /// offset up to 66 ms old. `ponytail:` the real fix is a per-vertex scroll channel in the shader.
    fn bump_scroll_epoch(&mut self) {
        if !self.surfaces.iter().any(|s| s.slide != [0.0; 2])
            && self.sky_scroll.iter().all(|rate| *rate == [0.0; 2])
        {
            return;
        }
        let bucket = (self.time * 15.0) as i32;
        if bucket != self.scroll_bucket {
            self.scroll_bucket = bucket;
            self.surface_epoch += 1;
        }
    }

    /// Create a thing from a template at a pose: the one constructor `CreateThing` (marker-relative)
    /// and `CreateThingAtPos` (absolute) share, so the two cannot drift apart.
    fn spawn_at(&mut self, template: String, pos: [f32; 3], sector: i32, angles: [f32; 3]) -> usize {
        let insert = template_insert(&template);
        let health = self.template_health.get(&template).copied().unwrap_or(1.0);
        let kind = self.template_types.get(&template).copied().unwrap_or(0);
        self.things.push(ThingState {
            pos,
            insert,
            owner: None,
            pitch: angles[0],
            yaw: angles[1],
            roll: angles[2],
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
            attach_offset: [0.0; 3],
            attach_flags: 0,
            spin: None,
            rot_vel: [0.0; 3],
            paused: false,
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

    /// Advance every moving thing. Returns the indices that moved, so a renderer can
    /// rewrite just those transforms instead of the whole level's.
    pub fn step(&mut self, dt: f32) -> Vec<usize> {
        let mut moved = Vec::new();
        self.time += dt;
        // `PlayKey` on a **thing** (a script camera's `MAcamera.key`) moves its pose: the key's single
        // node carries the dolly, relative to the pose the key started from. Level 14's opening key walks
        // its `droidcam` from (39.47, 11.38, 35.15) to about (39.65, -6.6, 28.4) — onto Kyle — where the
        // retail shot looks down at him; without it the view stays at the placed pose. Actors' `PlayKey`
        // names a puppet track, whose nodes pose the *model*, so only a key whose single node is a camera
        // box is a thing track. `ponytail:` a mesh-name test, not the engine's root-track rule in
        // `sithPuppet_UpdateThingMove`; the upgrade path is to read that rule and drop the name test.
        if !self.play_key.is_empty() {
            let entries: Vec<(usize, String, i32)> = self
                .play_key
                .iter()
                .map(|(thing, (name, _flags, start))| (*thing, name.clone(), *start))
                .collect();
            for (thing, name, start) in entries {
                let Some(key) = self.keys.get(&name) else { continue };
                if key.nodes.len() != 1 {
                    continue;
                }
                let node = &key.nodes[0];
                if !node.mesh.to_ascii_lowercase().contains("camera") {
                    continue;
                }
                let Some(base) = self.key_base.get(&thing).copied() else { continue };
                let Some(pose) = node.at(start as f32 + self.time * key.fps) else { continue };
                let Some(t) = self.things.get_mut(thing) else { continue };
                let (sin, cos) = base.1.to_radians().sin_cos();
                t.pos = [
                    base.0[0] + pose.pos[0] * cos - pose.pos[1] * sin,
                    base.0[1] + pose.pos[0] * sin + pose.pos[1] * cos,
                    base.0[2] + pose.pos[2],
                ];
                t.yaw = base.1 + pose.rot[1];
                t.pitch = base.2 + pose.rot[0];
                t.roll = base.3 + pose.rot[2];
                moved.push(thing);
            }
        }
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
        self.decay_damage(dt);
        self.decay_dynamic_tint(dt);
        self.advance_surface_cels();
        self.advance_material_cels();
        self.bump_scroll_epoch();
        self.advance_lights(dt);
        self.decay_shake(dt);
        self.advance_thing_lights(dt);
        self.advance_sound_fades(dt);
        // `timer=`/`SetLifeLeft` expiry: `sithThing_Update` counts `msecLifeLeft` down and calls
        // `sithThing_DestroyDyingThing` when it reaches zero. Without this an impact spark lives
        // forever, and a corpse never goes away.
        let expired: Vec<usize> = self
            .life_left
            .iter_mut()
            .filter_map(|(index, left)| {
                *left -= dt;
                (*left <= 0.0).then_some(*index)
            })
            .collect();
        for index in expired {
            self.life_left.remove(&index);
            self.life_total.remove(&index);
            if let Some(thing) = self.things.get_mut(index) {
                thing.destroyed = true;
            }
        }
        // `SetThingTimer`'s deadline passed: `sithCog_UpdateThingTimer` sends `timer` (4) to the
        // thing's own scripts, which is how `03_thinglight.cog` rotates its colours.
        let due: Vec<usize> = self
            .thing_timer
            .iter()
            .filter(|(_, at)| **at <= self.time)
            .map(|(index, _)| *index)
            .collect();
        for index in due {
            self.thing_timer.remove(&index);
            for instance in self.thing_instances(index) {
                self.pending_messages.push((instance, "timer", [0.0; 2], None));
            }
        }
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
        }        // A frame path that reaches its target sends `arrived` to the cogs linked to that thing.
        // `ThingState::step` sets the flag; clearing it here keeps each arrival one message.
        let arrived: Vec<usize> = self
            .things
            .iter()
            .enumerate()
            .filter(|(_, t)| t.arrived)
            .map(|(i, _)| i)
            .collect();
        for index in arrived {
            if let Some(thing) = self.things.get_mut(index) {
                thing.arrived = false;
            }
            for instance in self.thing_instances(index) {
                self.pending_messages.push((instance, "arrived", [0.0; 2], None));
            }
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
                moved.push(index);
            }
        }
        let mut blocked: Vec<usize> = Vec::new();
        // `sithPhysics.c`: a body in a `SITH_SECTOR_USETHRUST` sector gets `thrust * dt` added to its
        // velocity each step, exactly like gravity. Level 21's core tube lifts what stands in it.
        let thrust: Vec<[f32; 3]> = self.sectors.iter().map(|s| s.thrust).collect();
        // Gravity now runs for the shipped gravity goods (`physflags & 0x1` `type=weapon` bodies — the
        // `_debris` family), with the vertical stop below; actors are left to `ai_tick`'s walker, whose
        // floor integration owns their vertical axis (round 246, roadmap ledger). `ponytail:` the stop
        // uses the sector's floor planes at the body's origin, not the engine's swept
        // `SearchForSurfaceCollisions` and `GetThingHeight` resting rule; the shipped shrapnel's insert
        // offset is zero, so the two agree there, and a non-zero-insert model would rest that much low.
        // The player is **not** a generic mover: `walk_player` integrates the body and consumes a
        // scripted velocity (`force_jump.cog`'s `SetThingVel`) into `player.vy`. Letting both loops run
        // meant the generic one moved the *thing*, hit the level, zeroed the velocity and rewound the
        // position, so the jump's impulse was spent before the walker ever saw it.
        let player_at = self
            .templates
            .iter()
            .position(|t| t.eq_ignore_ascii_case("walkplayer"));
        for (i, t) in self.things.iter_mut().enumerate() {
            if Some(i) == player_at {
                // Path moves still run — a script can walk the player along frames — but the body's
                // velocity is the walker's to integrate, and `t.vel` is left for it to consume.
                let _ = t.step(dt);
                continue;
            }
            // `sithPhysics_UpdateThingPhysics`'s gravity arm (`0x4f6270`): a physics body whose
            // physflags carry `0x1` falls at the level's gravity (`0x40000` halves it), unless it is
            // attached to something that carries it. Actors are excluded because `ai_tick`'s walker
            // owns their vertical integration (roadmap ledger). The shipped gravity goods are the
            // `_debris` family (`type=weapon physflags=0x201`) — spawned chips that used to fly
            // straight through the floor and out of the level.
            let gravity_body = t.phys_flags & 0x1 != 0
                && self.types.get(i).copied() == Some(3)
                && t.attach.is_none()
                && !t.destroyed;
            if gravity_body {
                let g = if t.phys_flags & 0x40000 != 0 {
                    self.gravity * 0.5
                } else {
                    self.gravity
                };
                t.vel[2] -= g * dt;
            }
            if let Some(drive) = usize::try_from(t.sector).ok().and_then(|s| thrust.get(s)).copied()
                && drive != [0.0; 3]
                // `sithPhysics_UpdateThingPhysics`'s thrust arm (`0x4f6270`) skips a body whose
                // physflags carry `0x400000` — the flag `_powerup` ships (`physflags=0x400000`), which
                // is why m5's four weapon pickups stay put in the sectors a script gives an updraft
                // instead of climbing to z 89.3. The player is excluded below: `walk_player` owns it.
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
                let sector = t.sector.max(0) as usize;
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
                if hit && (resolved[0] - t.pos[0]).abs() + (resolved[1] - t.pos[1]).abs() > 0.01 {
                    t.pos[0] = resolved[0];
                    t.pos[1] = resolved[1];
                    t.vel = [0.0; 3];
                    t.target = None;
                    blocked.push(i);
                } else if escaped || solids.segment_blocked(previous, t.pos) {
                    // Put it back where it last made sense and stop it: the segment test is what
                    // catches a mover that stepped clean over a wall between two frames.
                    t.pos = previous;
                    t.vel = [0.0; 3];
                    t.target = None;
                    blocked.push(i);
                }
            }
            if path_moved || flying {
                moved.push(i);
            }
            // The vertical stop the x/y resolver above cannot make. `sithCollision_MoveThing` resolves
            // the fall against the sector's surfaces; the port's generic path only corrected x and y,
            // which is why gravity was off (a settling body sank out of the world). A face above where
            // the step started is a lip the body passed, not a floor it landed on, so it is ignored.
            if gravity_body
                && let Some(solids) = &self.solids
            {
                let mut floor: Option<f32> = None;
                for sector in solids.sectors_around(t.pos) {
                    if let Some((z, _)) =
                        solids.floor_surface(sector, [t.pos[0], t.pos[1], t.pos[2] + 0.6])
                        && z <= previous[2] + 0.05
                    {
                        floor = Some(floor.map_or(z, |near: f32| near.max(z)));
                    }
                }
                if let Some(floor) = floor
                    && t.pos[2] <= floor
                {
                    t.pos[2] = floor;
                    if t.vel[2] < 0.0 {
                        t.vel[2] = 0.0;
                    }
                    if let Some(sector) = solids.sector_at(t.pos, 0.1) {
                        t.sector = i32::try_from(sector).unwrap_or(t.sector);
                    }
                }
            }
        }
        for index in blocked {
            for instance in self.thing_instances(index) {
                self.pending_messages.push((instance, "blocked", [0.0; 2], None));
            }
        }
        // `RotatePivot`: a thing turning about the pivot its frame names, a whole turn of `seconds`.
        // The pose is a function of progress from the pose it started at, so a long frame cannot
        // drift the way an accumulator would.
        for index in 0..self.things.len() {
            let Some(spin) = self.things[index].spin.clone() else { continue };
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
            if !moved.contains(&index) {
                moved.push(index);
            }
        }
        // Attached things follow their parent: `sithThing_AttachThingToThing` stores the offset in the
        // parent's own frame (`rdMatrix_TransformVectorOrtho34` into `field_4C`), and the parent's
        // motion carries the child from there. A crate riding a lift and a catwalk hanging off a ship
        // are both this, and so is the sphere `force_absorb.cog` hangs on the player.
        self.update_attachments(&mut moved);
        self.things_moved += moved.len();
        moved
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
                | "setthingrot"
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

    /// `sithAIAwareness_ProcessEvent` (`0x004f2c30`): record the event in its own sector for its
    /// side, then walk the adjoin graph, subtracting each adjoin's attenuation and stopping under the
    /// engine's floor. Each sector keeps a **ring of the three most recent events** with their own
    /// positions — that is what `sithAICmd_Listen` scans (newest first), so three are kept here too.
    pub fn transmit_awareness(&mut self, sector: i32, side: i32, pos: [f32; 3], strength: f32) {
        let mut stack = vec![(sector, strength)];
        let mut visited = 0usize;
        while let Some((at, strength)) = stack.pop() {
            visited += 1;
            if visited > 4096 {
                break; // ponytail: a breadth guard; the strength floor ends real propagation first.
            }
            if at < 0 || at as usize >= self.sector_adjoins.len() {
                continue;
            }
            let ring = self.sector_awareness.entry((at, side)).or_default();
            // A stronger event already in the ring carries the propagation; a new one only adds its
            // own position for the listener to scan.
            if ring.iter().any(|r| r.0 >= strength) {
                continue;
            }
            ring.insert(0, (strength, pos, self.time));
            ring.truncate(3);
            for (next, cost) in &self.sector_adjoins[at as usize] {
                let next_strength = strength - cost;
                if next_strength > AI_AWARE_FLOOR {
                    stack.push((*next, next_strength));
                }
            }
        }
        // Fade the records that are past the window, the way the engine's own timer does.
        self.sector_awareness.retain(|_, ring| {
            ring.retain(|(_, _, at)| self.time - *at < AI_AWARE_SECONDS);
            !ring.is_empty()
        });
    }

    /// What an actor standing in `sector` on `side` would read as the last known enemy position: the
    /// freshest event in the sector's ring that is still inside the window.
    pub fn awareness_at(&self, sector: i32, side: i32) -> Option<[f32; 3]> {
        self.sector_awareness
            .get(&(sector, side))?
            .iter()
            .find(|(_, _, at)| self.time - *at < AI_AWARE_SECONDS)
            .map(|(_, pos, _)| *pos)
    }

    /// `listen` (`0x50a6f0`, the `sithAICmd_Listen` handler): the newest sound in `sector`'s ring
    /// that a body at `from`, facing `forward`, can actually **hear**, with the sound's event time.
    ///
    /// The handler's own conditions, and the constants it reads from the retail EXE
    /// ([`AI_LISTEN_NEAR`], [`AI_LISTEN_FRONT_DOT`]): the sound must be inside the class's
    /// `heardist`, and either in clear line of sight — then heard at any bearing — or, when the path
    /// is blocked, nearer than one unit **and** inside a 60° half-cone in front (dot > 0.5). A slot
    /// that fails is skipped and the next, older event in the ring is tried.
    pub fn heard_event(
        &self,
        sector: i32,
        side: i32,
        from: [f32; 3],
        forward: [f32; 2],
        heardist: f32,
    ) -> Option<([f32; 3], f32)> {
        self.sector_awareness
            .get(&(sector, side))?
            .iter()
            .filter(|(_, _, at)| self.time - *at < AI_AWARE_SECONDS)
            .find(|(_, pos, _)| {
                let blocked = self
                    .solids
                    .as_ref()
                    .is_some_and(|solids| solids.segment_blocked(from, *pos));
                hearable(from, forward, *pos, heardist, blocked)
            })
            .map(|(_, pos, at)| (*pos, *at))
    }

    /// One random row of an actor's own `.snd` class for a sound-mode event — what
    /// `sithSoundClass_PlayModeRandom` (`0x4e6b70`) picks: `class->+0x20 + mode*4` is the mode's list
    /// and a random entry is played. The class comes from the template's `soundclass=`.
    fn chatter_wav(&self, index: usize, mode: &str) -> Option<String> {
        let template = self.templates.get(index)?.to_ascii_lowercase();
        let class = self.template_soundclass.get(&template)?;
        let rows = self.sound_classes.get(class)?.get(mode)?;
        let pick = (crate::cog::pseudo_random() * rows.len() as f64) as usize;
        rows.get(pick.min(rows.len().saturating_sub(1)))
            .map(|(wav, _)| wav.clone())
    }

    /// Queue the random row of the actor's own class for a sound mode at `at` — what
    /// `sithSoundClass_PlayModeRandom` plays, through the port's sound-request pipeline. Shared by
    /// `talk` (its moods) and `sensedanger` (fear).
    fn queue_mode_sound(&mut self, index: usize, mode: &str, at: [f32; 3]) {
        let Some(wav) = self.chatter_wav(index, mode) else { return };
        // `PlayModeEntry` plays at full volume; `play_requested_sounds` owns the fade and the rear pan.
        let channel = self.sound_volume.len();
        self.sound_volume.push(1.0);
        self.sound_pitch.push(1.0);
        self.sounds.push(SoundRequest {
            name: wav,
            volume: 1.0,
            pos: Some(at),
            channel,
            pitch: 1.0,
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
        if self.actor_flags.get(&actor).copied().unwrap_or(0) & 0x1 != 0 {
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
        if step == 0.0 || self.time < self.ai_dodge.get(&actor).copied().unwrap_or(0.0) {
            return false;
        }
        let (Some(from), Some(to)) = (
            self.things.get(actor).map(|t| t.pos),
            self.things.get(attacker).map(|t| t.pos),
        ) else {
            return false;
        };
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        let flat = (dx * dx + dy * dy).sqrt();
        if flat < 1e-4 {
            return false;
        }
        let point = [from[0] - dx / flat * step, from[1] - dy / flat * step, from[2]];
        self.ai_move.insert(actor, point);
        self.ai_dodge.insert(actor, self.time + 1.0);
        if std::env::var_os("BK_AI_TRACE").is_some() {
            eprintln!("ai: {actor} dodges {step:.2} away from thing {attacker} to {point:?}");
        }
        true
    }

    /// `sensedanger`'s damage path (`0x50b9b0`, event 1): an **awake** actor (`0x4`) that has not
    /// yet been alerted (`0x800` clear) plays its `surprise` sound (mode `0x57`, seventh of the mood
    /// range 75–87), takes `sithThing_GetThingParent(attacker)` as its target — the shooter, for a
    /// projectile — and swaps the awake bit for the alert bit (`flags & !0x4 | 0x800`). Unlike the
    /// scheduler path it raises **no** awareness event: that is the on-sight branch's.
    fn sensedanger_from_damage(&mut self, actor: usize, attacker: usize) -> bool {
        let flags = self.actor_flags.get(&actor).copied().unwrap_or(0);
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
        self.actor_flags.insert(actor, (flags & !0x4) | 0x800);
        // `ai[0x70]` is the target thing; the port's chase reads the target's last-seen position.
        self.ai_last_seen.insert(actor, target_pos);
        self.queue_mode_sound(actor, "surprise", at);
        if std::env::var_os("BK_AI_TRACE").is_some() {
            eprintln!("ai: {actor} is surprised by thing {attacker} (target {target})");
        }
        true
    }

    /// The event mask a registered instinct enables (the registry's fifth argument): only the two
    /// instincts this build gives event handlers to are listed — `dodge` `0x1003` (damage, event 2,
    /// the shot) and `sensedanger` `0x7` (damage, event 2, event 4). Everything else is
    /// scheduler-only, which is why the mask table does not need the other 23 rows.
    fn instinct_event_mask(name: &str) -> u32 {
        match name.to_ascii_lowercase().as_str() {
            "dodge" => 0x1003,
            "sensedanger" => 0x7,
            // The registry word the `aistatus` dump reads (`jump` row: `mask 0x604`). Only `0x400` is
            // ever emitted — the engine's steering helper (`sithAI_idk_msgarrived_target`) raises it when
            // the way ahead of a moving actor is not walkable — so the handler's `4`/`0x200` arms are dead
            // content, like `dodge`'s event 2.
            "jump" => 0x604,
            _ => 0,
        }
    }

    /// `sithAI_EmitEvent` (`0x4e9f80`): raise one AI **event** on one actor. The engine's outer
    /// gates come first — the thing is not `0x202`-flagged (`SITH_TF` says no AI), it is alive, the AI
    /// is not switched off (`0x2000`), and the `0x1000` one-shot latch lets only event 2 through and
    /// clears itself doing so — then every instinct whose class line enables the event's mask is
    /// called **in file order**, stopping at the first handler that returns non-zero.
    ///
    /// Every emit site in the executable is accounted for: `sithActor_DamageActor` raises 1 (this
    /// build calls it from `damage_typed_from`), `sithWeapon_WeaponFireProjectile` raises `0x1000`
    /// (called from `fire_with_spread`), `sithAI_InstinctUpdate`/the two AI-flag helpers raise the
    /// `0x100` cascade, and the track-target helper raises `0x400`/`0x800`. **Nothing emits event 2 or
    /// event 4**, so those branches of `dodge` and `sensedanger` are dead in retail content too — the
    /// dispatcher's mask table lists them because the handlers do, not because content reaches them.
    pub fn emit_ai_event(&mut self, actor: usize, event: u32, source: Option<usize>) -> bool {
        let Some(thing) = self.things.get(actor) else {
            return false;
        };
        if thing.flags & 0x202 != 0 || thing.health <= 0.0 {
            return false;
        }
        let flags = self.actor_flags.get(&actor).copied().unwrap_or(0);
        if flags & 0x2000 != 0 {
            return false;
        }
        if flags & 0x1000 != 0 {
            if event != 2 {
                return false;
            }
            self.actor_flags.insert(actor, flags & !0x1000);
        }
        let Some(class) = self.ai.get(&actor).cloned() else {
            return false;
        };
        for instinct in &class.instincts {
            if Self::instinct_event_mask(&instinct.name) & event == 0 {
                continue;
            }
            let acted = match (instinct.name.to_ascii_lowercase().as_str(), event) {
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
                ("jump", 0x400) => self.jump_from_obstacle(actor, instinct.weight),
                _ => false,
            };
            if acted {
                return true;
            }
        }
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
        if self.actor_flags.get(&actor).copied().unwrap_or(0) & 0x200 == 0 {
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
        self.ai_move.insert(actor, point);
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
    /// body not recovered, on the ledger); the port refuses when the level blocks that line, which is the
    /// same question a step would ask.
    fn jump_from_obstacle(&mut self, actor: usize, weight: f32) -> bool {
        let Some(t) = self.things.get(actor) else {
            return false;
        };
        if t.attach.is_some() || self.actor_flags.get(&actor).copied().unwrap_or(0) & 0x1 == 0 {
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
        let (from, vel, goal) = (t.pos, t.vel, t.ai_target);
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
        if self
            .solids
            .as_ref()
            .is_some_and(|solids| solids.segment_blocked(from, point))
        {
            return false;
        }
        self.ai_jump
            .insert(actor, self.time + (weight / 1000.0).max(0.05));
        if let Some(t) = self.things.get_mut(actor) {
            // `_DAT_005232e4` = 0.1: the engine's velocities are level units a second (`thing + 0x234`
            // is `vel * dt`), so this is a real but small unstick toward the target.
            t.vel[0] += dir[0] * 0.1;
            t.vel[1] += dir[1] * 0.1;
            t.yaw = dir[1].atan2(dir[0]).to_degrees();
        }
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
    /// Not modelled: the engine's darkness term (`*(float *)(sector + 4) < 0.5` scales the chance by
    /// the sector's light level, and this build keeps no sector light), and its second falloff gate
    /// reads a byte at `thing + 0x2a8` whose meaning is unrecovered. Both are on the ledger.
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
        if distance >= DETECT_NEAR && seen.flags & 0x4 == 0 {
            if self.actor_flags.get(&actor).copied().unwrap_or(0) & 0x200 == 0 {
                chance = 0.5;
            }
            let falloff =
                ((distance - DETECT_NEAR) * DETECT_FALLOFF).clamp(0.0, DETECT_MAX_FALLOFF);
            chance *= 1.0 - falloff;
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
        if seen.flags & 0x80 != 0 && watcher.flags & 0x8000 == 0 {
            chance *= DETECT_UNSEEN;
        }
        if watcher.flags & 0x800 != 0 {
            chance *= DETECT_UNSEEN;
        }
        chance.clamp(DETECT_UNSEEN, 1.0)
    }

    /// One roll of [`Self::detect_chance`] — `sithAI_CanDetectSightThing`'s `rand/32768` test.
    pub fn can_detect(&self, actor: usize, target: usize) -> bool {
        crate::cog::pseudo_random() < self.detect_chance(actor, target) as f64
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
        let mut walked = 0;
        self.walked.clear();
        self.walk_vel.clear();
        let mut shots: Vec<(usize, String, f32)> = Vec::new();
        // Every actor with an instinct class, **plus** any actor a script gave a destination to.
        // `AISetMovePos`/`AISetMoveFrame` are gated on `controlType == SITH_CT_AI` in the engine, which
        // an actor has whether or not its template names an `aiclass=` — level 1's 8t88 has no class but
        // its goal script walks him to frame 1.
        let mut actors: Vec<usize> = self.ai.keys().copied().collect();
        for index in self.ai_move.keys().chain(self.ai_chase.keys()) {
            if !actors.contains(index) {
                actors.push(*index);
            }
        }
        for index in actors {
            let ai = self.ai.get(&index).cloned();
            let ai_template = self.templates.get(index).cloned().unwrap_or_default();
            // `AISetMode`/`AIClearMode` (`sithCogFunctionAI_AISetMode`, recovered at `0x500f50`) OR the
            // argument into the AI's **own** flag word (`ai + 8`), which is the word every instinct
            // handler tests — so those flags are behaviour, not decoration. `0x2000` is the switch-off
            // the shipped sleep/trap scripts use (`00_aisleep.cog` "aisetmode(enemy, 0x2000)",
            // `06_hidethings1.cog`, `00_cameratricks.cog` "shuts off dark jedi"), and with it set the
            // engine's instincts do nothing at all; the actor stays standing until a script clears it
            // (`aiclearmode(turret, 0x2000)` in `06_turrettrap.cog`).
            if self.actor_flags.get(&index).copied().unwrap_or(0) & 0x2000 != 0 {
                if let Some(thing) = self.things.get_mut(index) {
                    thing.ai_target = None;
                }
                continue;
            }
            // `opendoors` (handler 0x50abc0): the engine runs the **player's activate action**
            // (`sithPlayerActions_Activate`, 0x4fc0a0) on the actor's own body, throttled to 250 ms,
            // and that messages the captured thing or cog-linked surface in front with `activated`
            // (id 1). The port reuses the same two scans the use key drives, so an actor with the
            // instinct opens the door it is walking into instead of standing behind it. The engine
            // sweeps a small sphere at the body's leading edge; the use scans are a look-cone
            // approximation of that, and the reach is the port's use reach.
            if ai
                .as_ref()
                .is_some_and(|class| class.instinct("OpenDoors").is_some())
                && let Some(body) = self.things.get(index)
            {
                let (pos, yaw) = (body.pos, body.yaw);
                let due = self.ai_opendoors.get(&index).copied().unwrap_or(0.0);
                if self.time >= due {
                    self.ai_opendoors.insert(index, self.time + 0.25);
                    let facing = [-yaw.to_radians().sin(), yaw.to_radians().cos()];
                    self.activate_near(pos, facing, 1.5);
                    self.use_thing(pos, facing, 1.5);
                }
            }
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
            // `talk` (`0x50be20`, the second-most-used instinct after `listen`: 198 classes): idle
            // chatter on the instinct's own clock. Its first number is the interval in milliseconds
            // (`talk 10000.0, 0.10` in most shipped classes, `3000.0, 0.40` in `boc`) and the second
            // the chance; a success picks a mood from the AI flag word and the health fraction
            // (`talk_mode`) and plays a random row of the actor's own `.snd` class for it. The
            // handler's own upstream gate (the AI's one-shot bit) is not modelled.
            if let Some(talk) = ai.as_ref().and_then(|class| class.instinct("Talk")).cloned() {
                let due = self.ai_talk.get(&index).copied().unwrap_or(self.time);
                if self.time >= due {
                    self.ai_talk
                        .insert(index, self.time + (talk.weight / 1000.0).max(0.05));
                    let chance = talk.args.first().copied().unwrap_or(0.0) as f64;
                    let max_health = self
                        .template_max_health
                        .get(&ai_template)
                        .copied()
                        .unwrap_or(health)
                        .max(1.0);
                    if crate::cog::pseudo_random() < chance
                        && let Some(mode) = talk_mode(
                            self.actor_flags.get(&index).copied().unwrap_or(0),
                            health / max_health,
                        )
                    {
                        self.queue_mode_sound(index, mode, from);
                    }
                }
            }
            // `LobFire` (`0x509cd0`) — the name promises a lobbed shot, but its handler's mask is
            // `0x100` and that branch does not fire: it calls
            // `sithPuppet_SetArmedMode(thing, (ai flags & 0x200) != 0)` and re-arms in 1000 ms. `0x200`
            // is the awake bit `00_aisleep.cog` writes (`aisetmode(enemy, 0x200)`), so a wakeable ryther
            // draws its weapon when it wakes and holsters when it sleeps. The `param_4 == 0` branch —
            // the one that calls `sithAI_FireWeapon` with a lobbed chance — is **dead content**: the
            // engine's emit inventory raises only 1, `0x100`, `0x400`, `0x800` and `0x1000`, so no
            // event-0 reaches it (the same finding as `dodge`'s event 2).
            if ai
                .as_ref()
                .is_some_and(|class| class.instinct("LobFire").is_some())
            {
                let due = self.ai_lobfire.get(&index).copied().unwrap_or(self.time);
                if self.time >= due {
                    self.ai_lobfire.insert(index, self.time + 1.0);
                    let flags = self.actor_flags.get(&index).copied().unwrap_or(0);
                    self.armed_mode.insert(index, u32::from(flags & 0x200 != 0));
                }
            }
            // `maxrotthrust=` is the angular rate the body can turn at (50-85 degrees a second for
            // level 1's gunners). Snapping the yaw let every actor face the player the instant it
            // noticed them, which is what made the port's fights start and finish faster than
            // retail's; the turn is accumulated here and clamped once, below.
            let turn_rate = self
                .template_turn
                .get(&ai_template)
                .copied()
                .unwrap_or(180.0);
            let mut want_yaw = yaw;
            // A scripted destination wins over the chase: `AISetMovePos`/`AISetMoveThing` are how
            // `00_AIPatrol.cog` and the sidekick scripts walk a puppet somewhere. `AISetMoveThing`
            // keeps a *thing* rather than a point, and the engine re-reads its position every tick
            // (`sithAI.c:1018`) — that is what makes a follower follow.
            let scripted = self
                .ai_chase
                .get(&index)
                .and_then(|t| self.things.get(*t))
                .map(|t| t.pos)
                .or_else(|| self.ai_move.get(&index).copied());
            let look = self.ai_look.get(&index).copied();
            // `AISetLookPos` is a standing turn: it decides facing whenever the legs are not carrying
            // the actor somewhere, which is how `00_AIPatrol.cog` reverses at the end of each leg.
            let look_yaw = look.map(|l| (l[1] - from[1]).atan2(l[0] - from[0]).to_degrees());
            // No instinct class means no sight and no instincts: a class-less actor still walks a
            // destination a script gave it, but it never spots the player on its own.
            // Sight needs a **clear line**. `Ai::sees` is distance and cone only, so without
            // this an actor sees — and shoots — straight through walls. The engine casts its own
            // sight ray (`sithAI.c`); one segment test is the same rule here.
            let sighted = ai.as_ref().is_some_and(|ai| ai.sees(from, yaw, target))
                && self.solids.as_ref().is_none_or(|s| !s.segment_blocked(from, target));
            // `sithAI_CanDetectSightThing` (`0x4ec140`): a clear line is not the same as noticing. The
            // engine rolls its detection chance whenever it refreshes a target; the port rolls it when
            // the actor first registers the player and keeps the deterministic line test afterwards, so
            // an actor cannot lose a target it has already engaged (ledger). A target inside two units
            // (or one with `thingflags & 0x4`) passes at 1.0, so this only delays ranged acquisition.
            let mut sighted = sighted;
            if sighted && !self.ai_detected.contains(&index) {
                if self.can_detect(index, player) {
                    self.ai_detected.insert(index);
                } else {
                    sighted = false;
                }
            }
            if sighted && std::env::var_os("BK_AI_TRACE").is_some() {
                let (sx, sy) = (target[0] - from[0], target[1] - from[1]);
                let range = (sx * sx + sy * sy).sqrt().max(1e-4);
                let (sin, cos) = yaw.to_radians().sin_cos();
                eprintln!(
                    "ai: sees {index} range {range:.2} dot {:.2} ready {ready} yaw {yaw:.1} rate {turn_rate:.1} from {from:?} target {target:?}",
                    (cos * sx + sin * sy) / range
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
            // Where the actor last had the player in view: `sithAI` keeps the last seen position
            // and walks to it once the target disappears behind something. Without that an actor
            // forgets an enemy the instant they round a corner.
            if sighted {
                self.ai_last_seen.insert(index, target);
            }
            // `sensedanger` (`0x50b9b0`, 14 classes): an **awake** actor (`AISetMode(..., 0x4)` — what
            // the scripts set for the droids and remotes) whose class enables the instinct (its second
            // number is non-zero; most shipped lines are `sensedanger 0.10, 0.00` and never act) looks
            // the player over once a second while it is not yet alerted. Seeing them clears the awake
            // bit, sets the alert bit (`0x800`, also what `talk` reads for its `fear`/`gloat` moods),
            // plays the class's `fear` sound, raises a strength-3.0 awareness event at its own position
            // so the same side converges, and makes the player its target. The engine's own helper
            // (`sithAI_sub_4EAD60`) refreshes the target through `CheckSightThing`; the port's `sighted`
            // is that same range/cone/clear-line test.
            let sense_flags = self.actor_flags.get(&index).copied().unwrap_or(0);
            if sense_flags & 0x4 != 0
                && sense_flags & 0x800 == 0
                && !self.player_dead
                && let Some(sense) = ai.as_ref().and_then(|c| c.instinct("SenseDanger")).cloned()
                && sense.args.first().copied().unwrap_or(0.0) != 0.0
            {
                let due = self.ai_sense.get(&index).copied().unwrap_or(0.0);
                if self.time >= due {
                    if sighted {
                        // `flags & ~0x4 | 0x800`, exactly the handler's two writes.
                        self.actor_flags.insert(index, (sense_flags & !0x4) | 0x800);
                        self.ai_last_seen.insert(index, target);
                        self.queue_mode_sound(index, "fear", from);
                        let side = ai.as_ref().map(|c| c.alignment as i32).unwrap_or(0);
                        let sector = self.things.get(index).map(|t| t.sector).unwrap_or(-1);
                        self.transmit_awareness(sector, side, from, AI_AWARE_ALERT);
                        if std::env::var_os("BK_AI_TRACE").is_some() {
                            eprintln!(
                                "ai: {index} senses the danger and alerts sector {sector} side {side} from {from:?}"
                            );
                        }
                    } else {
                        self.ai_sense.insert(index, self.time + 1.0);
                    }
                }
            }
            // `roam` (`0x50b830`): an actor with nothing else to do wanders around its **home**
            // anchor — a random direction, a random distance inside the instinct's first argument, at
            // the class interval — then `SetLookFrame` + `SetMovePos` there. The port feeds that point
            // through the same remembered-position slot, so the wandering walk is the one already
            // tuned; it is the last fallback, after a remembered enemy, an awareness record and home.
            let roam_point = if let Some(roam) = ai.as_ref().and_then(|class| class.instinct("Roam")).cloned()
                && let Some(home) = self.ai_home.get(&index).copied()
            {
                let (mut due, mut point) = self.ai_roam.get(&index).copied().unwrap_or((0.0, home));
                if self.time >= due {
                    due = self.time + (roam.weight / 1000.0).max(0.5);
                    let radius = roam.args.first().copied().unwrap_or(2.0).max(0.0) as f32;
                    let angle = crate::cog::pseudo_random() as f32 * std::f32::consts::TAU;
                    let reach = radius * (0.25 + 0.75 * crate::cog::pseudo_random() as f32);
                    point = [
                        home[0] + angle.cos() * reach,
                        home[1] + angle.sin() * reach,
                        home[2],
                    ];
                }
                self.ai_roam.insert(index, (due, point));
                let here = self.things.get(index).map(|t| t.pos).unwrap_or(home);
                let d2 = (point[0] - here[0]).powi(2) + (point[1] - here[1]).powi(2);
                (d2 > 0.25).then_some(point)
            } else {
                None
            };
            // `sithAIAwareness`'s record is a *position*, which is used exactly like a last-seen
            // one: the actor turns and walks there, and the fight reaches it without it having seen
            // anything itself. The record lives per **sector and side**, so an actor reads the one
            // its own room and alignment own — that is the reader the stand-in could not express.
            let remembered = self.ai_last_seen.get(&index).copied().or_else(|| {
                let sector = self.things.get(index).map(|t| t.sector).unwrap_or(-1);
                let side = ai.as_ref().map(|ai| ai.alignment as i32).unwrap_or(0);
                // `listen` (`0x50a6f0`): the ring is scanned **newest first**, and the handler's own
                // conditions decide what is heard (`heard_event`: `heardist`, line of sight, and — for
                // a sound through geometry — nearer than one unit and in front). It then rolls the
                // class's reaction chance **once per sound** (`listen`'s first number: 0.50 in most
                // shipped classes, 1.00 in some, 0.00 for a sentry, which never walks at one), and
                // `SetMoveThing` only on success. A failed roll ignores that sound for good, so the
                // decision is remembered here instead of re-rolled every tick.
                let here = self.things.get(index)?.pos;
                let yaw = self.things.get(index)?.yaw.to_radians();
                let heardist = ai
                    .as_ref()
                    .map(|class| class.heardist)
                    .unwrap_or(f32::INFINITY);
                let (heard, at) =
                    self.heard_event(sector, side, here, [yaw.cos(), yaw.sin()], heardist)?;
                if let Some((last_at, last_pos, answered)) = self.ai_listen.get(&index).copied()
                    && last_at == at
                    && last_pos == heard
                {
                    return answered.then_some(heard);
                }
                let chance = ai
                    .as_ref()
                    .and_then(|class| class.instinct("Listen"))
                    .map(|listen| listen.weight)
                    .unwrap_or(1.0);
                let answered = crate::cog::pseudo_random() < f64::from(chance);
                self.ai_listen.insert(index, (at, heard, answered));
                if answered && std::env::var_os("BK_AI_TRACE").is_some() {
                    eprintln!("ai: {index} hears an event in sector {sector} side {side}");
                }
                answered.then_some(heard)
            }).or_else(|| {
                // `returnhome` (handler `0x50bd70`): an awake actor with nothing else to do walks back
                // to its home anchor (`SetMoveThing` to `ai + 0x93`, then `SetLookFrame` at it). The
                // anchor is the spawn position, recorded when the instincts were attached. Feeding it
                // through the remembered-position slot reuses the walk-and-turn that is already tuned,
                // and the half-unit dead zone stops an actor shuffling on its own spot.
                if !ai.as_ref().is_some_and(|class| class.instinct("ReturnHome").is_some()) {
                    return None;
                }
                let home = self.ai_home.get(&index).copied()?;
                let here = self.things.get(index)?.pos;
                let (dx, dy) = (home[0] - here[0], home[1] - here[1]);
                ((dx * dx + dy * dy).sqrt() > 0.5).then_some(home)
            }).or_else(|| roam_point);
            // `AIFlee(actor, from)`: the actor runs away from that thing and does not shoot, which
            // is a whole mode in the engine (`sithAI_Flee`). It is a script's order, so it does not
            // need sight or a memory — read before the "nothing to do" return below, or a fleeing
            // actor that has lost the player just stands there.
            // `retreat` (`0x50bc60`) and `withdraw` (`0x50b150`): a hurt actor backs away from what
            // hurt it. retreat sets the mode when its health falls under `arg0 × max` (0.50 in every
            // shipped class) and its `arg1` chance (0.25-0.50) succeeds, remembering the current
            // target; withdraw walks the actor away from that target. The port reuses the scripted
            // flee walk for the step and clears the mode on the withdraw instinct's own interval
            // (5000 ms shipped) — the engine instead ends it when the target comes back into sight.
            if let Some((from, until)) = self.ai_withdraw.get(&index).copied()
                && (self.time >= until || self.things.get(from).is_none())
            {
                self.ai_withdraw.remove(&index);
            }
            if !self.ai_withdraw.contains_key(&index)
                && let Some(retreat) = ai.as_ref().and_then(|c| c.instinct("Retreat")).cloned()
                && let Some(withdraw) = ai.as_ref().and_then(|c| c.instinct("Withdraw"))
            {
                let max_health = self
                    .template_max_health
                    .get(&ai_template)
                    .copied()
                    .unwrap_or(0.0);
                let health = self.things.get(index).map(|t| t.health).unwrap_or(0.0);
                let fraction = retreat.weight;
                let chance = retreat.args.first().copied().unwrap_or(0.0) as f64;
                if max_health > 0.0
                    && health < fraction * max_health
                    && crate::cog::pseudo_random() < chance
                {
                    let ms = withdraw.weight.max(1.0);
                    self.ai_withdraw.insert(index, (player, self.time + ms / 1000.0));
                }
            }
            // `hitandrun` (`0x50bb90`): fire at the target for the instinct's first argument, then break
            // off for its second and come back. The engine watches its just-fired flag; the port watches
            // `last_shot` and arms the same withdraw mode `retreat` uses, so the run-away and the return
            // are the walk that is already there. Shipped: `1000, 3000` (and `10000, 1500`).
            if !self.ai_withdraw.contains_key(&index)
                && let Some(run) = ai.as_ref().and_then(|class| class.instinct("HitAndRun")).cloned()
            {
                let hold = run.weight.max(1.0) / 1000.0;
                let break_off = run.args.first().copied().unwrap_or(3000.0).max(1.0) as f32 / 1000.0;
                if !sighted {
                    self.ai_hitrun.remove(&index);
                } else {
                    let fired = self
                        .last_shot
                        .get(&index)
                        .is_some_and(|at| self.time - at < 0.5);
                    if fired && !self.ai_hitrun.contains_key(&index) {
                        self.ai_hitrun.insert(index, self.time);
                    }
                    if let Some(start) = self.ai_hitrun.get(&index).copied()
                        && self.time - start >= hold
                    {
                        self.ai_withdraw.insert(index, (player, self.time + break_off));
                        self.ai_hitrun.remove(&index);
                    }
                }
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
                    self.ai_withdraw
                        .get(&index)
                        .and_then(|(from, _)| self.things.get(*from).map(|t| t.pos))
                });
            if scripted.is_none()
                && !sighted
                && aim.is_none()
                && remembered.is_none()
                && fleeing.is_none()
            {
                // `LookForTarget`: an idle actor with nothing in sight periodically scans for one.
                // The command is recovered now (`sithAICmd_Startup` registers handler 0x50aa80, round
                // 210): it paces by the class's interval in milliseconds, then scans through
                // `sithAI_FirstThingInView` — which sets the class's horizontal **and** vertical FOV
                // limits before `sithAI_GetThingsInView` walks the cone — and finishes with
                // `sithAI_CheckSightThing`'s range/facing/collision test. The class's own `sees` is that
                // cone plus `sightdist`; scanning on range alone let an actor notice a target **behind
                // it**, which is what the level-3 engagement divergence measured. A find goes into the
                // last-seen slot, so the body turns and walks at the usual `maxrotthrust / surfdrag`
                // clamp instead of snapping. Vertical limits are still approximated by the one `fov`.
                if std::env::var_os("BK_NO_SCAN").is_none()
                    && let Some(look) = ai.as_ref().and_then(|ai| ai.instinct("LookForTarget")).cloned()
                {
                    let interval = (look.weight / 1000.0).max(0.05);
                    let due = self.ai_scan.get(&index).copied().unwrap_or(0.0);
                    if self.time >= due {
                        self.ai_scan.insert(index, self.time + interval);
                        if let Some(thing) = self.things.get(index).map(|t| (t.pos, t.sector, t.yaw)) {
                            let (dx, dy, dz) = (
                                target[0] - thing.0[0],
                                target[1] - thing.0[1],
                                target[2] - thing.0[2],
                            );
                            let range = (dx * dx + dy * dy + dz * dz).sqrt();
                            let in_cone = ai.as_ref().is_some_and(|ai| ai.sees(thing.0, thing.2, target));
                            let clear = self.solids.as_ref().is_none_or(|s| {
                                !s.segment_blocked(thing.0, target)
                            });
                            if in_cone && clear {
                                self.ai_last_seen.insert(index, target);
                                if std::env::var_os("BK_AI_TRACE").is_some() {
                                    eprintln!("ai: {index} `{ai_template}` LookForTarget finds the body at {range:.2}");
                                }
                                continue;
                            }
                        }
                    }
                }
                // Out of sight and nothing remembered: the actor stands where it is, turning to a
                // scripted look target — or, with `randomturn` (`0x50b700`), to a direction of its own
                // choosing: every class interval (1000-2000 ms shipped) the handler picks a random
                // direction and `SetLookFrame`s a point its argument's distance away (0.5 units in
                // every shipped class). The port keeps the chosen yaw and turns at the body's own
                // rate, which is what makes an idle guard sweep its view instead of snapping.
                let mut idle_yaw = look_yaw;
                if let Some(turn) = ai
                    .as_ref()
                    .and_then(|class| class.instinct("RandomTurn"))
                    .cloned()
                {
                    let (mut due, mut target) =
                        self.ai_randomturn.get(&index).copied().unwrap_or((0.0, yaw));
                    if self.time >= due {
                        due = self.time + (turn.weight / 1000.0).max(0.05);
                        target = yaw + crate::cog::pseudo_random() as f32 * 360.0;
                    }
                    self.ai_randomturn.insert(index, (due, target));
                    idle_yaw = Some(target);
                }
                if let Some(thing) = self.things.get_mut(index) {
                    thing.ai_target = None;
                    if let Some(yaw) = idle_yaw {
                        thing.yaw = turn_toward(thing.yaw, yaw, turn_rate, dt);
                    }
                }
                continue;
            }
            if let Some(threat) = fleeing {
                let away = [from[0] - threat[0], from[1] - threat[1]];
                let len = (away[0] * away[0] + away[1] * away[1]).sqrt().max(0.001);
                let goal = [
                    from[0] + away[0] / len * 10.0,
                    from[1] + away[1] / len * 10.0,
                    from[2],
                ];
                let speed = self.template_speed.get(&ai_template).copied().unwrap_or(0.0)
                    * self.actor_speed.get(&index).copied().unwrap_or(1.0);
                if speed > 0.0 {
                    let step = (speed * dt).min(len);
                    let next = [
                        from[0] + away[0] / len * step,
                        from[1] + away[1] / len * step,
                        from[2],
                    ];
                    if let Some(thing) = self.things.get_mut(index) {
                        thing.pos = next;
                        thing.ai_target = Some(goal);
                        thing.yaw =
                            turn_toward(thing.yaw, away[1].atan2(away[0]).to_degrees(), turn_rate, dt);
                    }
                    walked += 1;
                    self.walked.push(index);
                    let dt = dt.max(1e-4);
                    self.walk_vel
                        .insert(index, [(next[0] - from[0]) / dt, (next[1] - from[1]) / dt]);
                }
                continue;
            }
            // `AISetFireTarget`: what this actor shoots at, when a script names it. Sight is still
            // needed, but of *that* thing — `00_AITurnCoat.cog` turns an actor on a victim it can see.
            // A script's destination wins; otherwise the chase is the player while in sight and the
            // last place they were seen once they are not.
            let chase = if sighted { target } else { remembered.unwrap_or(target) };
            let mut want = scripted.unwrap_or(chase);
            let mut pace = 1.0f32;
            let mut strafing = false;
            // `circlestrafe` (`0x509890`): while it has a target **within the instinct's own engage
            // distance** (0.6-1.0 units in the shipped lines — this is the close-range, lightsaber
            // behaviour), the actor steps around the target by the instinct's angle (30-45 degrees)
            // at half speed, and keeps facing it. The side is picked at random once and kept in a
            // signed slot, exactly as the handler persists it. The engine replans every 300-500 ms;
            // the port re-evaluates each frame, so the orbit is smoother than retail's — a deviation
            // to watch in a windowed leg.
            if scripted.is_none()
                && sighted
                && let Some(strafe) = ai.as_ref().and_then(|class| class.instinct("CircleStrafe"))
            {
                let (tx, ty) = (target[0] - from[0], target[1] - from[1]);
                let dist = (tx * tx + ty * ty).sqrt();
                let angle = strafe.args.first().copied().unwrap_or(30.0) as f32;
                let engage = strafe.args.get(1).copied().unwrap_or(1.0) as f32;
                if dist > 0.2 && dist <= engage.max(0.2) {
                    let side = *self.ai_strafe.entry(index).or_insert_with(|| {
                        if crate::cog::pseudo_random() < 0.5 { -1.0 } else { 1.0 }
                    });
                    let (sin, cos) = (angle * side).to_radians().sin_cos();
                    // The target -> actor vector, rotated around the target, keeps the radius and
                    // moves the body along the ring.
                    let (vx, vy) = (-tx, -ty);
                    want = [
                        target[0] + vx * cos - vy * sin,
                        target[1] + vx * sin + vy * cos,
                        target[2],
                    ];
                    pace = 0.5;
                    strafing = true;
                }
            }
            // A route in hand wins over the destination: each waypoint is a doorway centre the search
            // found, and reaching one pops it. When the route is empty the destination is the goal
            // again, which is what makes the last leg (doorway -> target) a straight walk.
            let route_goal = self.ai_route.get(&index).and_then(|r| r.first().copied());
            let goal = route_goal.unwrap_or(want);
            let dx = goal[0] - from[0];
            let dy = goal[1] - from[1];
            let reach = (dx * dx + dy * dy).sqrt();
            // `sithAI.c:814` steps the actor by `maxThrust * dt * moveSpeed`, so a scripted
            // `AISetMoveSpeed` scales the template's walking speed rather than replacing it.
            let speed = self.template_speed.get(&ai_template).copied().unwrap_or(0.0)
                * self.actor_speed.get(&index).copied().unwrap_or(1.0)
                * pace;
            // A scripted destination is walked all the way onto; the chase stops at its standoff, and
            // so does each doorway on a route. The standoff is the class's own `Follow` instinct: its
            // second argument is the distance the chase keeps from the target (`rydefault` 0.75,
            // `grdefault` 2.0). The port used a single 0.25 for everything, so the room's friends
            // crowded onto the player and absorbed every bolt aimed at it; the engine's gunners stand
            // off and shoot from there.
            let standoff = if scripted.is_some() || route_goal.is_some() || strafing {
                0.0
            } else {
                ai.as_ref()
                    .and_then(|a| a.standoff())
                    .unwrap_or(self.ai_standoff)
            };

            // The line to the *destination* is what decides whether a route is needed: testing the
            // next step instead was a bug, because a step is a centimetre and an actor in the middle
            // of a room never saw the wall it was about to press into.
            let blocked = route_goal.is_none()
                && self.solids.as_ref().is_some_and(|s| s.segment_blocked(from, want));
            if blocked
                && let Some(route) = self.sector_route(index, from, want)
                && !route.is_empty()
            {
                self.ai_route.insert(index, route);
            }
            // `sithAI_idk_msgarrived_target`: a moving actor whose way ahead is not walkable clears its
            // steering and emits `0x400`, the only event `jump` can receive. The port routes first, so
            // "cannot walk ahead" is "blocked and no route answered".
            if blocked && self.ai_route.get(&index).is_none() {
                let _ = self.emit_ai_event(index, 0x400, None);
            }
            let route_goal = self.ai_route.get(&index).and_then(|r| r.first().copied());
            let goal = route_goal.unwrap_or(want);
            let dx = goal[0] - from[0];
            let dy = goal[1] - from[1];
            let reach = (dx * dx + dy * dy).sqrt();
            // Arrival is measured against the **destination**, not the doorway the actor is aiming
            // at: a waypoint is a place on the way, and stopping at one dropped a script's order.
            let destination_reach =
                ((want[0] - from[0]).powi(2) + (want[1] - from[1]).powi(2)).sqrt();

            // A class with `TurretFire` never moves its body: its handler (`0x50a0f0`) only turns the
            // articulated head and fires the weapon, so the port leaves a turret where it was placed
            // instead of walking it at the player.
            let turret = ai.as_ref().is_some_and(|class| class.instinct("TurretFire").is_some());
            if reach > standoff && speed > 0.0 && !turret {
                let step = (speed * dt).min(reach - standoff);
                let next = [
                    from[0] + dx / reach * step,
                    from[1] + dy / reach * step,
                    from[2],
                ];
                // `sithActor_UpdateAimJoints`: an actor that has a target keeps its **aim** on it
                // while its legs carry it somewhere else. Facing the walk heading instead made a
                // scripted move (a patrol leg) turn the body away from the player it could see, lose
                // the view cone, and never fire — which is why level 1's door room stayed quiet.
                let face = if sighted { target } else { goal };
                let facing = (face[0] - from[0], face[1] - from[1]);
                if let Some(thing) = self.things.get_mut(index) {
                    thing.pos = next;
                    thing.ai_target = Some(goal);
                }
                if facing.0.abs() > 1e-4 || facing.1.abs() > 1e-4 {
                    want_yaw = facing.1.atan2(facing.0).to_degrees();
                }
                walked += 1;
                self.walked.push(index);
                // A waypoint is a doorway centre the route search found: reaching it pops the list so
                // the next frame aims at the one after, and an empty list hands the goal back to the
                // destination. Without this an actor walked to the first doorway and stalled there,
                // because `first()` never changed.
                if route_goal.is_some() && (reach - step) <= 0.05 {
                    let empty = match self.ai_route.get_mut(&index) {
                        Some(route) => {
                            if !route.is_empty() {
                                route.remove(0);
                            }
                            route.is_empty()
                        }
                        None => false,
                    };
                    if empty {
                        self.ai_route.remove(&index);
                    }
                }
                // The velocity this body is walking at, which is what the renderer needs to pick
                // its puppet state — a position delta cannot tell a stride from a collision push.
                let dt = dt.max(1e-4);
                self.walk_vel.insert(
                    index,
                    [(next[0] - from[0]) / dt, (next[1] - from[1]) / dt],
                );
            } else if let Some(thing) = self.things.get_mut(index) {
                thing.ai_target = None;
                // The same aim rule while standing: a target in sight wins over a scripted look.
                if sighted {
                    want_yaw = (target[1] - from[1]).atan2(target[0] - from[0]).to_degrees();
                } else if let Some(yaw) = look_yaw {
                    want_yaw = yaw;
                } else if reach > 0.001 {
                    want_yaw = dy.atan2(dx).to_degrees();
                }
            }
            // Standing on the remembered spot and finding nobody there forgets the target — the
            // engine's last-known-position chase giving up.
            if !sighted
                && scripted.is_none()
                && remembered.is_some()
                && destination_reach <= self.ai_standoff
            {
                self.ai_last_seen.remove(&index);
                self.ai_route.remove(&index);
            }
            // Arriving clears the scripted destination, the way the engine drops
            // `SITHAI_MODE_MOVING` when the puppet reaches `movePos`, so it returns to its instincts.
            if scripted.is_some() && destination_reach <= self.ai_standoff {
                self.ai_move.remove(&index);
                self.ai_chase.remove(&index);
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
            // The one clamp for this frame's desired facing: `maxrotthrust * dt`. It has to happen
            // for every active actor, not just the ones about to fire — an actor that is still
            // turning spends most frames behind its `ai_wait` and would never rotate otherwise.
            if let Some(thing) = self.things.get_mut(index) {
                thing.yaw = turn_toward(yaw, want_yaw, turn_rate, dt);
            }
            let mut shooting_at = aim.unwrap_or(target);
            let can_see = if aim.is_some() { true } else { sighted };
            // `BK_AI_TRACE=2`: name the gate that stops an in-sight actor from firing. The engine's
            // `sithAICmd_PrimaryFire` has exactly three besides readiness — the instinct itself, its
            // range, and its `minDot` facing test — so one line per refusal is enough to tell an AI
            // that is asleep from one that is merely still turning.
            let fire_trace = std::env::var("BK_AI_TRACE").is_ok_and(|v| v == "2");
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
                // Every shipped line is `750, 0.80, 0, 0.8, 0.0, 0.30`: delay, chance, unused, cos
                // limit, minimum distance, spread. The reach is the engine's fixed 10 units.
                (arg(0), arg(2), 10.0, arg(4), arg(3))
            } else if turret_shot {
                // `turretfire`'s shipped line is `750, 75, 160, 160, 5.0, 0.20, 500, 10000, 0`: the first
                // number is the delay, and the rest are the aim-head sweep and cooldown. The port has no
                // articulated head, so the shot uses the class's own `sightdist` as the reach and no
                // facing gate (the turret's fov is 360), and is modelled as accurate.
                (arg(0), 0.0, ai.as_ref().map(|a| a.sightdist).unwrap_or(5.0), 0.0, 0.0)
            } else {
                (arg(0), arg(1), arg(2), arg(3), arg(4))
            };
            let (burst, burst_ms) = if blind || turret_shot {
                (1.0, 0.0)
            } else {
                (arg(8), arg(9))
            };
            // The engine rolls the chance on its own tick; the port rolls it when the shot slot is up.
            if blind && arg(1) > 0.0 && crate::cog::pseudo_random() > arg(1) as f64 {
                continue;
            }
            let (sx, sy) = (shooting_at[0] - from[0], shooting_at[1] - from[1]);
            let range = (sx * sx + sy * sy).sqrt();
            if range > max_dist.max(0.001) || range < min_dist {
                if fire_trace {
                    eprintln!(
                        "ai: {index} range {range:.2} outside {instinct} [{min_dist:.2}, {max_dist:.2}]"
                    );
                }
                continue;
            }
            // A script that names a fire target is telling the actor to shoot *that*, so it turns to
            // it first — `00_AITurnCoat.cog` turns an actor on the victim behind it.
            if aim.is_some()
                && let Some(thing) = self.things.get_mut(index)
            {
                thing.yaw = sy.atan2(sx).to_degrees();
            }
            let yaw = self.things.get(index).map(|t| t.yaw).unwrap_or(yaw);
            // `minDot`: the actor fires only once it is facing the target, which is what makes it
            // turn before shooting rather than snap.
            let facing = yaw.to_radians().sin_cos();
            let dot = if range > 1e-4 {
                (facing.1 * sx + facing.0 * sy) / range
            } else {
                1.0
            };
            if dot < min_dot {
                if fire_trace {
                    eprintln!("ai: {index} facing dot {dot:.2} < {instinct} minDot {min_dot:.2}");
                }
                continue;
            }
            let Some(weapon) = self
                .templates
                .get(index)
                .and_then(|name| self.template_weapon.get(name).cloned())
            else {
                continue;
            };
            shots.push((index, weapon, error));
            if let Some(thing) = self.things.get_mut(index) {
                thing.yaw = sy.atan2(sx).to_degrees();
                // `sithActor_UpdateAimJoints`: the aim points at the target in **three** dimensions.
                // Without the elevation an actor shoots level at a player standing on a ledge and the
                // bolt passes underneath — the bug the player had until round 109.
                let dz = shooting_at[2] - from[2];
                let flat = (sx * sx + sy * sy).sqrt().max(1e-4);
                thing.pitch = dz.atan2(flat).to_degrees().clamp(-60.0, 60.0);
                // `sithAICmd_PrimaryFire`: a burst of `fltArg[8]` shots `intArg[9]` apart, then the
                // shot delay jittered by ±20% (`(1.2 - 0.2 * frand) * fltArg[0]`).
                if thing.ai_burst > 0.0 {
                    thing.ai_burst -= 1.0;
                    thing.ai_wait = burst_ms / 1000.0;
                } else {
                    thing.ai_burst = (burst - 1.0).max(0.0);
                    let jitter = 1.2 - 0.2 * crate::cog::pseudo_random() as f32;
                    thing.ai_wait = delay_ms / 1000.0 * jitter;
                }
            }
        }
        let mut shooters: Vec<usize> = Vec::new();
        for (index, weapon, error) in shots {
            if self.fire_spread(index as i64, &weapon, error).is_some() {
                // The renderer plays a puppet's `fire` track off this: the engine's
                // `sithPuppet_UpdateThingMove` picks the action track while the actor is shooting.
                self.last_shot.insert(index, self.time);
                shooters.push(index);
                fired += 1;
            }
        }
        // `sithAIAwareness_CreateTransmittingEvent` (`0x4f2a90`) + `ProcessEvent` (`0x4f2c30`):
        // firing raises an event at the shooter with strength 4.0, and the engine's propagation
        // carries it through the adjoin graph for the shooter's own side. The record is the
        // shooter's position, which is what makes reinforcements converge on gunfire.
        for shooter in shooters {
            let Some(side) = self.ai.get(&shooter).map(|a| a.alignment as i32) else { continue };
            let Some(thing) = self.things.get(shooter) else { continue };
            let (sector, pos) = (thing.sector, thing.pos);
            self.transmit_awareness(sector, side, pos, AI_AWARE_SHOT);
        }
        let _ = walked;
        fired
    }

    /// `sithWeapon_Update`'s decay, one step's worth: damage falls by `rate * dt`, floored
    /// at `mindamage`. A weapon whose typeflags lack `SITH_WF_DAMAGE_DECAY` (0x4000) keeps
    /// its damage.
    fn decay_damage(&mut self, dt: f32) {
        for thing in &mut self.things {
            if thing.decay > 0.0 && thing.damage > thing.min_damage {
                thing.damage = (thing.damage - thing.decay * dt).max(thing.min_damage);
            }
        }
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
        self.fire_with_spread(shooter, template, error)
    }

    /// Fire `template` from the shooter: spawn it just in front of the shooter's eye and give it the
    /// template's own `vel=`, rotated into the shooter's facing.
    ///
    /// The engine's projectile is a `SITH_THING_WEAPON` created by `CreateThing` from the weapon
    /// template, launched along the barrel; the speed in the shipped templates is `vel=(0/4/0)` for
    /// `+bryarbolt`, i.e. 4 units per second down the model's +Y.
    pub fn fire(&mut self, shooter: i64, template: &str) -> Option<usize> {
        self.fire_with_spread(shooter, template, 0.0)
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

    fn fire_with_spread(
        &mut self,
        shooter: i64,
        template: &str,
        error: f32,
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
        let flat = [speed[1] * cos - speed[0] * sin, speed[1] * sin + speed[0] * cos];
        // Pitch tilts that horizontal aim up or down and the template's own z rides along, so a shot
        // aimed 30 degrees up leaves with `speed * sin(30)` in z. `rotate_by_angles` is the engine's
        // `rdMatrix_Build34`, whose yaw starts at +Y; this world's camera yaw starts at +X, so the
        // rotation is written out here rather than reused.
        let (sp, cp) = pitch.to_radians().sin_cos();
        let mut forward = [
            flat[0] * cp,
            flat[1] * cp,
            (flat[0] * flat[0] + flat[1] * flat[1]).sqrt() * sp + speed[2] * cp,
        ];
        if error > 0.0 {
            forward = Self::jitter_aim(forward, error);
        }
        let health = self.template_health.get(&template).copied().unwrap_or(1.0);
        let kind = self.template_types.get(&template).copied().unwrap_or(3);
        // The shot starts at full damage and loses it in flight when the weapon decays.
        let damage = self.template_damage.get(&template).copied().unwrap_or(0.0);
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
        let muzzle = {
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
                // `sithWeapon_FireProjectile` does `rdMatrix_TransformVector34Acc(offset,
                // &senderOrient)` — the offset is in the shooter's own frame, whose model +Y is
                // forward and +X is **right**. With yaw 0 facing +X and up +Z, that right is
                // (0, -1), which is `(ahead[1], -ahead[0])` for a general heading.
                Some([x, y, z]) => [
                    pos[0] + ahead[0] * y + ahead[1] * x,
                    pos[1] + ahead[1] * y - ahead[0] * x,
                    pos[2] + z,
                ],
                _ => [
                    pos[0] + ahead[0] * 1.0,
                    pos[1] + ahead[1] * 1.0,
                    pos[2] + 0.1,
                ],
            }
        };
        self.things.push(ThingState {
            pos: muzzle,
            insert: [0.0; 3],
            owner: usize::try_from(shooter).ok(),
            yaw,
            pitch: 0.0,
            roll: 0.0,
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
            attach_offset: [0.0; 3],
            attach_flags: 0,
            spin: None,
            rot_vel: [0.0; 3],
            paused: false,
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
        Some(index)
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
    /// The engine keeps this in `weaponData[bin]` (a template path per bin); that table is
    /// not in the level, so the name is matched against the level's own weapon templates
    /// instead. A `ponytail:` heuristic — good enough to arm the bryar and the stormtrooper
    /// rifle on level 1, replaced by the real table when it is recovered.
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
    /// takes the highest; this build instantiates no item cogs, so the score is the weapon's own
    /// `damage=` — the one engine-side number that ranks the shipped weapons the way the cogs'
    /// hand-tuned constants do. `kind` (0..2 in the engine) is accepted and ignored: the cogs use it
    /// to answer differently per slot, and there is nothing to ask.
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


    /// The top of the highest **solid thing** under `pos` whose model the body's feet are on or just
    /// above, or `None` when the body is over open level geometry.
    ///
    /// `sithCollision` treats a thing's own model as solid for every body, so a catwalk slab, a crate
    /// or a lift platform is a floor. The surface sweep only knows the level's own faces, so without
    /// this a body on a `_walkstruct` falls through it — level 1's catwalk pose is the standing case.
    /// `ponytail:` the footprint is the model's axis-aligned half extents, so a thing rotated off the
    /// axes is covered a little generously; the engine uses the drawn surfaces.
    pub fn thing_floor(&self, pos: [f32; 3], body_radius: f32) -> Option<f32> {
        let feet = pos[2] - body_radius;
        let mut best: Option<f32> = None;
        for (index, thing) in self.things.iter().enumerate() {
            if thing.destroyed {
                continue;
            }
            let Some(template) = self.templates.get(index) else { continue };
            let Some((top_offset, half_x, half_y)) = template_floor(template) else { continue };
            if (pos[0] - thing.pos[0]).abs() > half_x + body_radius
                || (pos[1] - thing.pos[1]).abs() > half_y + body_radius
            {
                continue;
            }
            let top = thing.pos[2] + top_offset;
            // A surface under the feet: the caller decides whether the body is close enough to rest on
            // it. Anything above the feet is something the body is beside, not standing on.
            if top <= feet + 0.1 && best.is_none_or(|at| top > at) {
                best = Some(top);
            }
        }
        best
    }

    /// Every (shot, target, carried damage) impact this frame, from `first_created` onward.
    ///
    /// The scan lives here rather than in the Bevy system so the combat path can be checked
    /// without running the app: `--hit` drives it directly. Placed things are targets too — the
    /// old version skipped everything below `first_created`, which made every actor the level
    /// placed immune and left only run-time `CreateThing` actors killable.
    pub fn impacts(&self, first_created: usize) -> Vec<(usize, usize, f32)> {
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

                if self.shot_hits(index, other) {
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

    /// The player's use action: `activate` to the cogs linked to the surface nearest the eye.
    ///
    /// The engine's rule is a looked-at surface within the player's reach (§16.10); the *reach* and
    /// the facing cone here are calibrated rather than recovered, because the use action lives in
    /// `jkPlayer.c`, which this source tree does not carry. What is data-driven is the target: only
    /// surfaces a script actually linked are candidates.
    pub fn activate_near(&mut self, eye: [f32; 3], facing: [f32; 2], reach: f32) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (_, _, surface) in &self.surface_links {
            let Some(centre) = self.surfaces.get(*surface).map(|s| s.center) else { continue };
            let (dx, dy) = (centre[0] - eye[0], centre[1] - eye[1]);
            let flat = (dx * dx + dy * dy).sqrt();
            // The use is a **look**, so the reach and ranking are 3D: level 1's death floor sits 17
            // units below the cantina switch and won the switch's use on flat distance alone (round 200).
            let dz = centre[2] - eye[2];
            let dist = (flat * flat + dz * dz).sqrt();
            if std::env::var_os("BK_USE_TRACE").is_some() {
                println!("use-candidate: surface {} centre {:?} dist {:.2} dz {:.2} dot {:.2}", surface, centre, dist, dz, if flat > 0.01 { (dx / flat) * facing[0] + (dy / flat) * facing[1] } else { 1.0 });
            }
            if dist > reach {
                continue;
            }
            // Behind the player is not in reach: the engine's use is a look, not a touch.
            if flat > 0.01 && (dx / flat) * facing[0] + (dy / flat) * facing[1] < 0.3 {
                continue;
            }
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((*surface, dist));
            }
        }
        let (surface, _) = best?;
        for (instance, link, linked) in &self.surface_links {
            if *linked == surface {
                // `GetSenderId()` is how a switch tells its link ids apart, so the link id is the sender.
                self.pending_messages.push((*instance, "activate", [0.0; 2], Some(*link as i64)));
            }
        }
        Some(surface)
    }


    /// Which cog instances a level's own `cogs` section links to its things, surfaces and sectors.
    ///
    /// A level's `cogs` rows carry one value per non-local symbol, and the script's `symbols` section
    /// says what kind each one is: `surface 12`, `thing 5`, `sector 3` are indices into the level. The
    /// engine calls this `sithCog_LinkCogToSurface` / `_ToThing` / `_ToSector`; the port had the three
    /// tables declared, initialised empty and read, with nothing ever writing to them, so no message
    /// from the world reached a script. Fills them once, in table order.
    ///
    /// `ponytail:` the link id and mask are carried but not filtered on, which is the same simplification
    /// `sector_messages` documents — every link to that target hears the message.
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
                let link = symbol.linkid.unwrap_or(0) as i32;
                match symbol.kind {
                    crate::cog::SymKind::Thing => self.thing_links.push((index, link, target)),
                    crate::cog::SymKind::Surface => self.surface_links.push((index, link, target)),
                    crate::cog::SymKind::Sector => self.sector_links.push((index, link, target)),
                    _ => {}
                }
            }
        }
    }

    /// The player's use action on a *thing*: the nearest thing a script links, in front of the eye.
    ///
    /// The engine's use is a look, not a touch (`sithPlayer`'s `use`), so the same cone the surface path
    /// uses applies. It answers with the thing it used, if any.
    pub fn use_thing(&mut self, eye: [f32; 3], facing: [f32; 2], reach: f32) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (_, _, target) in &self.thing_links {
            let Some(thing) = self.things.get(*target) else { continue };
            let (dx, dy) = (thing.pos[0] - eye[0], thing.pos[1] - eye[1]);
            let flat = (dx * dx + dy * dy).sqrt();
            // The use is a **look**, so the reach and ranking are 3D: level 1's death floor sits 17
            // units below the cantina switch and won the switch's use on flat distance alone (round 200).
            let dz = thing.pos[2] - eye[2];
            let dist = (flat * flat + dz * dz).sqrt();
            if dist > reach {
                continue;
            }
            if flat > 0.01 && (dx / flat) * facing[0] + (dy / flat) * facing[1] < 0.3 {
                continue;
            }
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((*target, dist));
            }
        }
        let (target, _) = best?;
        for (instance, link, linked) in &self.thing_links {
            if *linked == target {
                // `activate`, not `used`: no shipped cog declares a `used` label (0 of 540), while 82 of them -- every door, switch and item -- answer `activate`. It is also what the surface half of this action already sends.
                // `GetSenderId()` is how a switch tells its link ids apart, so the link id is the sender.
                self.pending_messages.push((*instance, "activate", [0.0; 2], Some(*link as i64)));
            }
        }
        Some(target)
    }

    /// The portal that best shortcuts the way from `from` to `target`, by the sum of the two legs.
    ///
    /// `ponytail:` greedy, not a search: it picks the doorway that looks closest on both legs and
    /// does not look past it. An actor in a room with two doors may pick the wrong one; the engine's
    /// `sithAI` runs a real path over sector adjoins. This is the "walks through the door instead of
    /// into the wall" step, which is what the level geometry actually needs.
    /// The doorway centres from `from`'s room to the room `want` stands in, for an actor whose straight
    /// line is walled off. The sector of a point comes from the collision build's own hull tests,
    /// which is the same answer a body standing there gets.
    fn sector_route(&self, index: usize, from: [f32; 3], want: [f32; 3]) -> Option<Vec<[f32; 3]>> {
        let solids = self.solids.as_deref()?;
        // A point can sit in more than one hull (doorway space belongs to both rooms), so every
        // candidate is tried rather than trusting the first.
        let mut starts: Vec<usize> = Vec::new();
        if let Some(claimed) = self
            .things
            .get(index)
            .map(|t| t.sector)
            .and_then(|s| usize::try_from(s).ok())
            .filter(|s| *s < solids.spheres.len())
        {
            starts.push(claimed);
        }
        starts.extend(solids.sectors_around(from));
        let goals = solids.sectors_around(want);
        for start in starts {
            for goal in &goals {
                if let Some(route) = solids.route(start, *goal, self.gates())
                    && !route.is_empty()
                {
                    return Some(route);
                }
            }
        }
        None
    }

    pub fn best_portal(&self, from: [f32; 3], target: [f32; 3]) -> Option<[f32; 3]> {
        let mut best: Option<([f32; 3], f32)> = None;
        for (_, _, centre) in &self.portals {
            let leg = |a: [f32; 3], b: [f32; 3]| {
                ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
            };
            let cost = leg(from, *centre) + leg(*centre, target);
            if best.is_none_or(|(_, c)| cost < c) {
                best = Some((*centre, cost));
            }
        }
        best.map(|(centre, _)| centre)
    }

    /// Wake every thing standing in `sector` and answer the class instances that should be sent
    /// `created`. A thing wakes once: the second call for the same sector answers nothing, which is
    /// what makes the wake idempotent across frames.
    pub fn awaken_sector(&mut self, sector: i32) -> Vec<usize> {
        let things: Vec<usize> = self
            .things
            .iter()
            .enumerate()
            .filter(|(_, thing)| thing.sector == sector && !thing.destroyed)
            .map(|(index, _)| index)
            .collect();
        let mut woke = Vec::new();
        for thing in things {
            if !self.awakened.insert(thing) {
                continue;
            }
            if let Some(instance) = self.thing_class_cog.get(&thing).copied() {
                woke.push(instance as usize);
            }
        }
        woke
    }
    /// Which instances are linked to `thing`, so a damage or death can message them.
    pub fn thing_instances(&self, thing: usize) -> Vec<usize> {
        self.thing_links
            .iter()
            .filter(|(_, _, target)| *target == thing)
            .map(|(instance, _, _)| *instance)
            .collect()
    }

    /// Which cog instances a move from one sector to another messages.
    ///
    /// `sithThing_EnterSector` sends `entered` to the sector's links and `sithThing_LeaveSector`
    /// sends `exited` (§16.10). The link id and mask are carried but not filtered on, which is a
    /// documented simplification: every link to that sector hears the message.
    pub fn sector_messages(&self, old: Option<usize>, new: Option<usize>) -> Vec<(usize, &'static str, i32)> {
        let mut out = Vec::new();
        for (instance, link, sector) in &self.sector_links {
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

    /// The first time the renderer draws a **thing** (`sithRender_RenderThing`, `sithRender.c:2995`):
    /// `SITH_TF_INCAMFOV` (0x100000) is latched, and a `SITH_TF_CAPTURED` (0x400) thing — one that has
    /// a cog placed on it — sends `sighted` (id 9) to those cogs. Level 1's goal is exactly this:
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
        let mut sent = 0;
        for (instance, _link, target) in &self.thing_links {
            if *target == thing {
                self.pending_messages.push((*instance, "sighted", [0.0; 2], None));
                sent += 1;
            }
        }
        sent
    }

    /// The first time the renderer draws a sector: `sithRender` latches `SITH_SECTOR_SEEN` (0x4000)
    /// and, when the sector is `SITH_SECTOR_COGLINKED` (0x4), sends `sighted` (id 9) to the cogs placed
    /// on it. That is how level 1's goal fires — `01_goal0.cog` sits on the sector that holds 8t88, so
    /// *seeing* the area completes "Find 8t88 before he escapes Nar Shaddaa".
    pub fn sector_sighted(&mut self, sector: usize) -> usize {
        let Some(flags) = self.sector_flags.get_mut(sector) else { return 0 };
        if *flags & SECTOR_SEEN != 0 {
            return 0;
        }
        *flags |= SECTOR_SEEN;
        if *flags & SECTOR_COGLINKED == 0 {
            return 0;
        }
        let mut sent = 0;
        for (instance, _link, target) in &self.sector_links {
            if *target == sector {
                self.pending_messages.push((*instance, "sighted", [0.0; 2], None));
                sent += 1;
            }
        }
        sent
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
            insert: [0.0; 3],
            owner: None,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
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
            attach_offset: [0.0; 3],
            attach_flags: 0,
            spin: None,
            rot_vel: [0.0; 3],
            paused: false,
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

    /// Damage the **player**, the way `sithPlayer` does: shields absorb it first, and the rest
    /// comes off health. Returns the pair after the hit.
    ///
    /// A generic `DamageThing` on the walkplayer bypasses the shield bar, which is why the HUD's
    /// second number never moved: `--hit` showed the player taking 9 straight off health while the
    /// shields read 100. This is the engine's order — `sithPlayer_Update` spends shields before
    /// health and only dies when health reaches 0.
    pub fn damage_player(&mut self, amount: f32, kind: u32) -> (f32, f32) {
        let player = self.player_thing();
        // The player goes through the same guard as any actor (`sithActor_DamageActor` takes the
        // player branch too), so a scripted invulnerability covers bolts as well as everything else.
        if self.invulnerable(player, kind) {
            return (
                self.player_shields,
                self.things.get(player).map(|t| t.health).unwrap_or(0.0),
            );
        }
        // `sithActor_DamageActor` tints the local player's palette on damage that lands, before the
        // shields are spent: its call is `sithPlayer_AddDynamicTint(damage * 0.04, 0, 0)`.
        add_dynamic_tint(&mut self.tint, [amount * DAMAGE_TINT, 0.0, 0.0]);
        let mut left = amount;
        if self.player_shields > 0.0 {
            let absorbed = left.min(self.player_shields);
            self.player_shields -= absorbed;
            left -= absorbed;
        }
        if let Some(thing) = self.things.get_mut(player) {
            thing.health = (thing.health - left).max(0.0);
            if thing.health <= 0.0 {
                thing.destroyed = true;
                self.player_dead = true;
            }
        }
        (self.player_shields, self.things.get(player).map(|t| t.health).unwrap_or(0.0))
    }

    /// Put the player back on its feet at the level's own spawn: full health and shields, the
    /// death flag cleared. The engine reloads or respawns at the last checkpoint; this build has
    /// no checkpoint list yet, so it is the level's `walkplayer` position.
    pub fn respawn_player(&mut self, spawn: [f32; 3]) -> (f32, f32) {
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
            thing.pos = spawn;
            thing.vel = [0.0; 3];
        }
        self.player_shields = 100.0;
        self.player_dead = false;
        (self.player_shields, health)
    }

    /// Whether a shot at `shot` is inside a target's body.
    ///
    /// A thing's position is its **feet**, so testing the distance to it misses a bolt fired at
    /// eye height: the player's eye is about 1.4 units up and a placed actor's origin is on the
    /// floor, so a 0.5-unit point test never landed on anything that was placed in the level.
    /// The body comes from the template's `size=x/y/z` (half-extents) or `radius=`, so the test is
    /// "inside the cylinder the level's own template describes".
    pub fn shot_hits(&self, shot: usize, target: usize) -> bool {
        let (Some(a), Some(b)) = (self.things.get(shot), self.things.get(target)) else {
            return false;
        };
        // The target's body, in the engine's own terms: its `movesize` radius and the model height
        // that implies. A missing template keeps the old cylinder so a script-spawned thing is still
        // hittable.
        let name = self.templates.get(target).cloned().unwrap_or_default().to_ascii_lowercase();
        let (radius, height) = self.template_hit.get(&name).copied().unwrap_or((0.065, 0.18));
        let (dx, dy) = (a.pos[0] - b.pos[0], a.pos[1] - b.pos[1]);
        let flat = (dx * dx + dy * dy).sqrt();
        // A bolt has its own small radius; a hit is when the two touch.
        if flat > radius + 0.15 {
            return false;
        }
        let low = b.pos[2];
        let high = b.pos[2] + height;
        a.pos[2] >= low - 0.15 && a.pos[2] <= high + 0.15
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
        // `SITH_MESSAGE_DAMAGED` (10) and `SITH_MESSAGE_KILLED` (12) go to the cogs linked to the
        // thing that was hit; the level's scripts wait on both (level 1: `damaged` x4, `killed` x5).
        // The params are the engine's: the amount and the damage-type word, which is what a handler's
        // `GetParam(0)`/`GetParam(1)` read (`force_absorb.cog`, `force_protection.cog`).
        let params = [amount as f64, kind as f64];
        for instance in self.thing_instances(index) {
            self.pending_messages.push((instance, "damaged", params, Some(source)));
            if died {
                self.pending_messages.push((instance, "killed", params, Some(source)));
            }
        }
        // What a handler's answer has to amend: the engine applies the amount the `damaged` handler
        // returns, not the one it was handed.
        self.damage_applied.insert(index, amount);
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

    /// `actor->flags & SITH_AF_INVULNERABLE` with the engine's fall exception.
    fn invulnerable(&self, index: usize, kind: u32) -> bool {
        kind != DAMAGE_FALL
            && self.actor_flags.get(&index).copied().unwrap_or(0) & AF_INVULNERABLE != 0
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
        self.templates.iter().position(|t| t.eq_ignore_ascii_case("walkplayer")).unwrap_or(0)
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
    /// `ponytail:` this is the mechanism, not the input. Retail reaches it through the inventory
    /// screen, which the port does not draw yet, so nothing calls this from a key — `--use-item`
    /// drives it.
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
    pub fn set_bin(&mut self, bin: i32, amount: f32) {
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
                    s.slide = [-vec[0] * scale, -vec[1] * scale];
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
            // `jkEndLevel(flag)`: a level's final script calls this. The flag asks for the
            // end-of-level screen; this build only records that the level is done, so a
            // caller can resolve the next one through the episode graph.
            "jkendlevel" | "endlevel" => {
                self.level_complete = idx(0) != 0 || num(0) != 0.0;
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
                let shot = self.fire_with_spread(index as i64, &weapon, error as f32);
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
            // `SendMessageEx(cog, message, senderType, senderRef, p0, p1)`: the same bus with the
            // message's two parameters. The shipped callers pass zeros (`03_goals.cog` and friends
            // telling the master cog which objective finished), but the parameters are what the
            // receiver's `GetParam(0)`/`GetParam(1)` read, so they are carried.
            "sendmessageex" => {
                let cog = (idx(0) >= 0).then(|| idx(0) as i32);
                let name = match args.get(1) {
                    Some(Value::Str(s)) => s.clone(),
                    Some(value) => format!("#{}", value.num() as i64),
                    None => "message 0".to_string(),
                };
                let sender = (self.caller_cog >= 0).then_some(self.caller_cog as usize);
                let params = [num(4), num(5)];
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
            // `sithCogFunctionAI_AISetMode` is `actor->flags |= mode` and `AIClearMode` is
            // `actor->flags &= ~mode` (`sithCogFunctionAI.c:237`), which is exactly the actor-flag arm below.
            "setactorflags" | "aisetmode" => {
                *self.actor_flags.entry(ti(0)).or_insert(0) |= idx(1) as u32;
                Value::Nil
            }
            "clearactorflags" | "aiclearmode" => {
                *self.actor_flags.entry(ti(0)).or_insert(0) &= !(idx(1) as u32);
                Value::Nil
            }
            "getactorflags" => Value::Num(self.actor_flags.get(&ti(0)).copied().unwrap_or(0) as f64),
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
            // `sithCogFunctionAI_AISetMoveSpeed` clamps to 0..2 and stores it on the actor; the AI
            // step multiplies by it. The clamp is the engine's, so it is applied on the way in.
            "aisetmovespeed" => {
                self.actor_speed.insert(ti(0), (num(1) as f32).clamp(0.0, 2.0));
                Value::Nil
            }
            // `AISetMovePos(actor, pos)` stores the destination; `sithAI_SetMoveThing` copies it into
            // `movePos` and raises `SITHAI_MODE_MOVING`, which `ai_tick` now walks on. The engine
            // gates both on the thing having an actor; a script calling it on a non-actor gets
            // nothing, and so does this port, because `ai_tick` only iterates things with an `.ai`.
            "aisetmovepos" | "aisetlookpos" => {
                let pos = match args.get(1) {
                    Some(Value::Vec(xyz)) => [xyz[0] as f32, xyz[1] as f32, xyz[2] as f32],
                    _ => [0.0; 3],
                };
                let table = if verb.eq_ignore_ascii_case("aisetmovepos") {
                    &mut self.ai_move
                } else {
                    &mut self.ai_look
                };
                table.insert(ti(0), pos);
                Value::Nil
            }
            // `AISetMoveFrame`/`AISetLookFrame` (`sithCogFunctionAI.c:73`) index the actor's own frame
            // list — the `numframes=`/`frame=(x/y/z:…)` parameters the level places on the thing, which
            // is the same list `MoveToFrame` walks. Level 1's 8t88 has two frames and its goal script
            // sends him to frame 1. The engine refuses a frame past `loadedFrames`, and this does too.
            "aisetmoveframe" | "aisetlookframe" => {
                let (thing, frame) = (ti(0), idx(1));
                if let Some(at) = self.frame_of(thing, frame) {
                    if verb.eq_ignore_ascii_case("aisetmoveframe") {
                        self.ai_move.insert(thing, at);
                    } else {
                        self.ai_look.insert(thing, at);
                    }
                }
                Value::Nil
            }
            // `AISetMoveThing(actor, thing)`: a *live* destination. The engine re-reads the thing's
            // position each tick, so the actor keeps following a moving target.
            "aisetmovething" => {
                let (actor, destination) = (ti(0), ti(1));
                if destination != usize::MAX {
                    self.ai_chase.insert(actor, destination);
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
            "skilltarget" => {
                let victim = ti(0);
                let bin = idx(2) as i32;
                let type_ = self.types.get(victim).copied().unwrap_or(-1);
                if bin == 24 {
                    // Item (5) throws itself; anything else needs a weapon to drop, which is not
                    // modelled — the script then takes its `else` branch and stops the power.
                    Value::Num(if type_ == 5 { 1.0 } else { -1.0 })
                } else if bin == 31 {
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
                }
                Value::Num(0.0)
            }
            "jkstoppovkey" => {
                self.pov_key.remove(&ti(0));
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
            // `TakeItem(item, player)`: the item leaves the world. The inventory is the
            // script's job — `sithItem_SetItemTaken` runs the item's own `taken` label,
            // which calls `ChangeInv` — so this only removes the thing. A script-less item
            // is handled by `Runtime::take_item` on the direct path.
            "takeitem" => {
                let index = usize::try_from(idx(0)).unwrap_or(usize::MAX);
                let template = self.templates.get(index).cloned().unwrap_or_default();
                if let Some(thing) = self.thing_mut(idx(0)) {
                    thing.destroyed = true;
                }
                if index != usize::MAX {
                    self.note_taken(index, &template);
                }
                Value::Nil
            }
            "capturething" => {
                if let Some(t) = self.thing_mut(idx(0)) {
                    t.destroyed = true;
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
            "setsectoradjoins" => {
                let shut = idx(1) == 0;
                let mut changed = false;
                if let Some(flags) = self.sector_flags.get_mut(idx(0) as usize) {
                    changed = (*flags & SECTOR_ADJOINS_OFF != 0) != shut;
                    *flags = if shut {
                        *flags | SECTOR_ADJOINS_OFF
                    } else {
                        *flags & !SECTOR_ADJOINS_OFF
                    };
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
                    // `PlaySound*` answers the **channel**, which is what `ChangeSoundVol` and
                    // `ChangeSoundPitch` take — `sithSoundMixer_PlaySound` returns one and the shipped
                    // scripts store it (`02_crowstart.cog`).
                    let channel = self.sound_volume.len();
                    self.sound_volume.push(volume);
                    self.sound_pitch.push(1.0);
                    self.sounds.push(SoundRequest { name, volume, pos, channel, pitch: 1.0 });
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
            // `Sleep` parks the script until a clock fires; the world records the wait
            // but there is no scheduler to resume it, so the script runs straight on.
            // ponytail: no timer queue — a level that only works across resumed sleeps
            // will run its later statements early.
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
            "adddynamictint" => {
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
            "changeinv" => {
                *self.inv.entry(idx(1) as i32).or_default() += num(2) as f32;
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

    fn door_runtime() -> Runtime {
        let mut rt = Runtime::new(&Level::default());
        rt.things = vec![ThingState {
                pos: [0.0; 3],
                insert: [0.0; 3],
                owner: None,
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0,
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
                attach_offset: [0.0; 3],
                attach_flags: 0,
                spin: None,
            rot_vel: [0.0; 3],
            paused: false,
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

    /// `maxrotthrust / surfdrag` degrees a second, the short way round: a body that is 60 degrees
    /// off turns 10 degrees in a 0.6 s frame at a 16 deg/s rate rather than snapping onto its aim.
    /// A solid thing's own model is a surface the engine collides with: the loader registers its top
    /// and footprint, and the body's floor probe finds it. Level 1's catwalk is the standing case.
    #[test]
    fn a_solid_things_model_is_a_floor() {
        set_template_floors(BTreeMap::from([(
            "testcatwalk".to_string(),
            (0.0123, 0.5, 0.5),
        )]));
        let mut rt = door_runtime();
        rt.templates = vec!["walkplayer".into(), "testcatwalk".into()];
        rt.types = vec![10, 0];
        let mut slab = rt.things[0].clone();
        slab.pos = [0.0, 0.0, 1.0];
        rt.things.push(slab);
        // The model top sits `0.0123` above the thing's position; a body centre 0.065 over it is
        // standing on the slab.
        assert_eq!(rt.thing_floor([0.0, 0.0, 1.2], 0.065), Some(1.0123));
        // Off the footprint is open air, and a surface above the feet is something the body is beside.
        assert_eq!(rt.thing_floor([2.0, 0.0, 1.2], 0.065), None);
        assert_eq!(rt.thing_floor([0.0, 0.0, 0.8], 0.065), None);
    }

    #[test]
    fn a_body_turns_at_its_own_rate() {
        assert!((turn_toward(0.0, 60.0, 16.0, 0.6) - 9.6).abs() < 1e-4);
        assert!((turn_toward(0.0, 5.0, 16.0, 0.6) - 5.0).abs() < 1e-4, "a small turn is reached");
        // The short way: 350 -> 10 is +20, not -340.
        assert!((turn_toward(350.0, 10.0, 100.0, 0.1) - 360.0).abs() < 1e-4);
        assert!((turn_toward(10.0, 350.0, 100.0, 0.1)).abs() < 1e-4);
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
            rt.step(1.0 / 120.0);
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
        rt.step(1.0);
        assert_eq!(rt.things[shot].damage, 15.0);
        // The floor holds no matter how long it flies.
        rt.step(10.0);
        assert_eq!(rt.things[shot].damage, 10.0);
        // A weapon without the decay flag keeps its damage.
        rt.template_decay.remove(&bolt);
        let steady = rt.fire(0, &bolt).expect("projectile");
        rt.step(5.0);
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

        // Level: the template's own heading, z untouched.
        let shot = rt.fire(0, &bolt).expect("projectile");
        assert!((rt.things[shot].vel[0] - 4.0).abs() < 1e-3, "vel {:?}", rt.things[shot].vel);
        assert!(rt.things[shot].vel[2].abs() < 1e-3, "a level shot rose: {:?}", rt.things[shot].vel);

        // Aimed 30 degrees up: the speed splits the way `sin`/`cos` say it must.
        rt.things[0].pitch = 30.0;
        let shot = rt.fire(0, &bolt).expect("projectile");
        let vel = rt.things[shot].vel;
        assert!((vel[0] - 4.0 * 30f32.to_radians().cos()).abs() < 1e-3, "vel {vel:?}");
        assert!((vel[2] - 4.0 * 30f32.to_radians().sin()).abs() < 1e-3, "vel {vel:?}");

        // And down again, which is the half a player notices on a ledge.
        rt.things[0].pitch = -45.0;
        let shot = rt.fire(0, &bolt).expect("projectile");
        assert!(rt.things[shot].vel[2] < -2.5, "vel {:?}", rt.things[shot].vel);
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
        // Fired along the shooter's facing: yaw 0 faces +X in this world.
        assert!(rt.things[shot].vel[0] > 0.0, "vel {:?}", rt.things[shot].vel);
        rt.step(0.5);
        assert!(rt.things[shot].pos[0] > 1.0, "pos {:?}", rt.things[shot].pos);

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
        // 20 damage adds 0.8 of red — before the shields are spent, as the engine's call is.
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
        rt.step(0.5);
        assert!((rt.tint[0] - 0.8).abs() < 1e-4, "tint after half a second {:?}", rt.tint);
        rt.step(10.0);
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
            slide: [0.0; 2],
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
        rt.step(0.5);
        assert_eq!(rt.surfaces[index].cel, 1.0);
        assert!(rt.surface_epoch > epoch, "a cel step did not ask for a rebuild");
        // A material with one cel never advances, whatever the rate says.
        rt.surface_cels[index] = 1;
        rt.step(1.0);
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
        rt.step(1.0);
        assert!((rt.sectors[sector].light - 0.25).abs() < 1e-5);
        rt.step(1.0);
        assert!((rt.sectors[sector].light - 0.5).abs() < 1e-5);
        // Settled: the rate is one-shot, so a long step does not overshoot.
        rt.step(10.0);
        assert!((rt.sectors[sector].light - 0.5).abs() < 1e-5);

        // `SurfaceLightAnim(surface, 0.2, 1.0, 0.5)`: `cog_00_forcefield.cog`'s startup ramp,
        // `(1.0 - 0.2) / (0.5 * 0.5)` = 3.2 a second, so 0.25 s lands at 1.0 and stops.
        rt.surfaces.push(SurfaceState {
            center: [0.0; 3],
            cel: 0.0,
            anim: 0.0,
            slide: [0.0; 2],
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
        rt.step(0.1);
        assert!((rt.surfaces[surface].light - 0.52).abs() < 1e-4);
        rt.step(0.2);
        assert!((rt.surfaces[surface].light - 1.0).abs() < 1e-5);
        rt.step(1.0);
        assert!((rt.surfaces[surface].light - 1.0).abs() < 1e-5, "the ramp kept walking");
        // A backwards ramp is no ramp: the engine answers -1 and leaves the surface alone.
        let refused = rt.call(
            "SurfaceLightAnim",
            &[Value::Num(surface as f64), Value::Num(1.0), Value::Num(0.0), Value::Num(1.0)],
        );
        assert_eq!(refused, Value::Num(-1.0));
    }

    #[test]
    fn the_new_world_verbs_act_on_the_world() {
        let mut rt = door_runtime();
        // A surface's cel round-trips, and its centre is whatever the level said.
        rt.surfaces.push(SurfaceState { center: [1.0, 2.0, 3.0], cel: 0.0, anim: 0.0, slide: [0.0; 2], next: 0.0, light: 0.0, light_end: 0.0, light_rate: 0.0 });
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
        // `AISetMoveSpeed` stores a speed multiplier for the AI step, clamped to the engine's 0..2.
        rt.call("AISetMoveSpeed", &[Value::Num(0.0), Value::Num(5.0)]);
        assert_eq!(rt.actor_speed.get(&0).copied(), Some(2.0));
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
        // A shot has just left the barrel: this is what `PrimaryFire` sets and what the handler reads.
        rt.last_shot.insert(1, 0.0);

        for _ in 0..40 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            rt.ai_withdraw.contains_key(&1),
            "an actor with `HitAndRun` did not break off after its firing window"
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

        for _ in 0..40 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        let chosen = rt.ai_randomturn.get(&1).copied().expect("no randomturn state");
        assert!(
            chosen.1.abs() > 1e-3,
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
            quiet.ai_tick(0, 1.0 / 30.0);
        }
        assert_eq!(quiet.things[1].yaw, 0.0, "an actor without `randomturn` turned");
    }

    /// `roam` (`0x50b830`): an actor with nothing to chase wanders around its **home** anchor — a
    /// random direction, a random distance inside the instinct's first argument, every class interval.
    #[test]
    fn an_idle_actor_with_roam_wanders_around_its_home() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        let mut player = base.clone();
        player.pos = [-9.0, 0.0, 0.0]; // out of sight and out of mind
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nRoam 5000.0, 2.0\n",
            ),
        );
        rt.ai_home.insert(1, [0.0, 0.0, 0.0]);

        for _ in 0..60 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        let p = rt.things[1].pos;
        let moved = (p[0] * p[0] + p[1] * p[1]).sqrt();
        assert!(moved > 0.2, "the actor did not wander: {p:?}");
        assert!(moved < 2.5, "the actor left its roam radius: {p:?}");

        // A control class without `Roam` stands at home.
        let mut quiet = door_runtime();
        let base = quiet.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
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
        quiet.ai_home.insert(1, [0.0, 0.0, 0.0]);
        for _ in 0..60 {
            quiet.time += 1.0 / 30.0;
            quiet.ai_tick(0, 1.0 / 30.0);
        }
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
        rt.actor_flags.insert(1, 0x2000);
        let asleep_at = rt.things[1].pos;
        for _ in 0..30 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert_eq!(rt.shots_fired, 0, "a switched-off actor fired");
        assert_eq!(rt.things[1].pos, asleep_at, "a switched-off actor walked");

        // `aiclearmode(0x2000)`: back to normal.
        rt.actor_flags.insert(1, 0);
        for _ in 0..30 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            rt.shots_fired > 0,
            "the actor did not come back after its mode was cleared"
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

    /// The sector's sound **ring** matters when two shots land in one room: `listen` scans it newest
    /// first and takes the first event inside `heardist`, so a nearer but older sound still wakes a
    /// short-ranged guard while a fresh far one does not.
    #[test]
    fn a_short_ranged_guard_hears_the_nearer_of_two_sounds_in_the_ring() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        let mut player = base.clone();
        player.pos = [-9.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=90, sightdist=2.0, heardist=2.0\n\nListen 1.0\n",
            ),
        );
        // Newest first: a fresh shot 5 units along +x (out of earshot), an older one 1.5 units along
        // +y (inside it). The guard must walk toward the one it can hear.
        rt.sector_awareness.insert(
            (7, -1),
            vec![(4.0, [5.0, 0.0, 0.0], 0.0), (3.0, [0.0, 1.5, 0.0], 0.0)],
        );
        for _ in 0..40 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        let p = rt.things[1].pos;
        assert!(
            p[1] > 0.2 && p[0].abs() < 0.1,
            "the guard walked toward the wrong sound: {p:?}"
        );
    }

    /// `listen` (`0x50a6f0`) reacts to a sound only within the class's **heardist**: the port had the
    /// awareness record and the walk, but not the gate, so every actor in a room reacted to every shot
    /// in it however deaf its class was.
    #[test]
    fn the_awareness_record_is_heard_only_inside_the_classs_heardist() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0]; // sector 7, alignment -1
        let mut player = base.clone();
        player.pos = [-9.0, 0.0, 0.0]; // out of sight, nothing remembered
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=90, sightdist=2.0, heardist=2.0\n\nListen 1.0\n",
            ),
        );
        // A shot five units away: outside the two-unit hearing, so the guard must not react.
        rt.sector_awareness
            .insert((7, -1), vec![(4.0, [5.0, 0.0, 0.0], 0.0)]);
        for _ in 0..30 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert_eq!(
            rt.things[1].pos,
            [0.0, 0.0, 0.0],
            "the actor reacted to a sound beyond its heardist"
        );
        // A shot inside it: the same actor walks toward it.
        rt.sector_awareness
            .insert((7, -1), vec![(4.0, [1.5, 0.0, 0.0], rt.time)]);
        for _ in 0..30 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            rt.things[1].pos[0] > 0.2,
            "the actor ignored a sound inside its heardist: {:?}",
            rt.things[1].pos
        );
    }

    /// `listen`'s first number is the **reaction chance**, not a period: the handler rolls
    /// `rand() / 32768 < arg0` once per sound event and only calls `SetMoveThing` when it passes.
    /// Shipped classes carry 0.50 (most of them), 1.00 (`boc`, `gorc`) or 0.00 (`bosentry`, which
    /// therefore never walks at a sound), so the roll has to be here and not a re-roll per tick.
    #[test]
    fn a_listen_chance_of_zero_ignores_a_sound_in_clear_hearing() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0]; // sector 7, alignment -1
        let mut player = base.clone();
        player.pos = [-9.0, 0.0, 0.0]; // out of sight, nothing remembered
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "tuskan".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("tuskan".into(), 1.0);
        let class = |chance: &str| {
            crate::ai::parse(&format!(
                "alignment=-1.0, rank=0.5, fov=90, sightdist=2.0, heardist=2.0\n\nListen {chance}\n"
            ))
        };
        rt.ai.insert(1, class("0.0"));
        // Inside the two-unit hearing and in clear line — a sound a `Listen 1.0` class walks at — but
        // this class's chance is zero, so it must only hear it.
        rt.sector_awareness
            .insert((7, -1), vec![(4.0, [1.5, 0.0, 0.0], 0.0)]);
        for _ in 0..40 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert_eq!(
            rt.things[1].pos,
            [0.0, 0.0, 0.0],
            "a Listen 0.0 class walked at a sound it should only hear"
        );
        // The same chance 1.0 reacts to a *new* event; the same event would not be re-rolled.
        rt.ai.insert(1, class("1.0"));
        rt.sector_awareness
            .insert((7, -1), vec![(4.0, [1.5, 0.0, 0.0], rt.time)]);
        for _ in 0..40 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            rt.things[1].pos[0] > 0.2,
            "the guard ignored a sound its class always reacts to: {:?}",
            rt.things[1].pos
        );
    }

    /// The `listen` handler's audibility gates, in its own order and with the constants it reads
    /// from the retail EXE (`_DAT_005232c8` = 1.0 unit, `_DAT_005232a0` = 0.5 = cos 60°): inside
    /// `heardist` a clear sound is heard at any bearing; a sound through geometry only when it is
    /// nearer than one unit and within 60° of straight ahead.
    #[test]
    fn a_sound_through_geometry_is_heard_only_when_close_and_in_front() {
        let from = [0.0, 0.0, 0.0];
        let ahead = [1.0, 0.0]; // yaw 0 faces +X
        let behind = [-1.0, 0.0];
        // Clear line of sight: bearing does not matter, the class's hearing distance does.
        assert!(hearable(from, behind, [2.0, 0.0, 0.0], 3.0, false));
        assert!(!hearable(from, ahead, [4.0, 0.0, 0.0], 3.0, false), "beyond heardist");
        // Blocked: one unit is the cut-off and the half-cone is 60 degrees.
        assert!(hearable(from, ahead, [0.5, 0.0, 0.0], 3.0, true), "close and in front");
        assert!(!hearable(from, ahead, [1.5, 0.0, 0.0], 3.0, true), "blocked and 1.5 away");
        assert!(!hearable(from, behind, [0.5, 0.0, 0.0], 3.0, true), "blocked behind the actor");
        let bearing = |degrees: f32, reach: f32| {
            [degrees.to_radians().cos() * reach, degrees.to_radians().sin() * reach, 0.0]
        };
        assert!(hearable(from, ahead, bearing(45.0, 0.5), 3.0, true), "inside the 60-degree cone");
        assert!(!hearable(from, ahead, bearing(75.0, 0.5), 3.0, true), "outside the 60-degree cone");
    }

    /// `LobFire`'s **live** path (`0x509cd0`): its mask is `0x100` and that branch does not fire — it
    /// sets `sithPuppet_SetArmedMode(thing, (ai flags & 0x200) != 0)` and re-arms in 1000 ms. `0x200` is
    /// the awake bit `00_aisleep.cog` writes, so a wakeable ryther draws its weapon when it wakes and
    /// holsters when it sleeps. The `param_4 == 0` branch, the one that calls `sithAI_FireWeapon` with a
    /// lobbed chance, is dead content: the emit inventory raises only 1, `0x100`, `0x400`, `0x800` and
    /// `0x1000`, so nothing ever delivers event 0 to it (the same finding as `dodge`'s event 2).
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
        rt.actor_flags.insert(1, 0x200);
        rt.ai_tick(0, 1.0 / 30.0);
        assert_eq!(
            rt.armed_mode.get(&1).copied(),
            Some(1),
            "an awake actor with `LobFire` did not draw its weapon"
        );
        // The handler's own 1000 ms interval: the bit clearing does not holster until it expires.
        rt.actor_flags.insert(1, 0);
        for _ in 0..20 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert_eq!(
            rt.armed_mode.get(&1).copied(),
            Some(1),
            "the armed mode changed before the 1000 ms interval"
        );
        for _ in 0..20 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert_eq!(
            rt.armed_mode.get(&1).copied(),
            Some(0),
            "a sleeping actor with `LobFire` kept its weapon drawn"
        );
        assert_eq!(rt.shots_fired, 0, "the dead `sithAI_FireWeapon` branch lobbed a shot");
    }

    /// `jump` (`0x50abf0`): the only arm content can reach is event `0x400`, raised by the steering
    /// helper when a moving actor's way ahead is not walkable. A moving, unattached actor gets
    /// `dir * 0.1` added to its velocity toward the target (`_DAT_005232e4`) and turns to it, on the
    /// class line's own 2000 ms clock; a standing actor is refused by the AI moving bit `0x1`.
    #[test]
    fn a_blocked_actor_nudges_toward_its_target_with_jump() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 180.0;
        let mut player = base.clone();
        player.pos = [3.0, 0.0, 0.0];
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
        rt.actor_flags.insert(1, 0x1);
        rt.things[1].ai_target = Some([3.0, 0.0, 0.0]);
        assert!(
            rt.emit_ai_event(1, 0x400, None),
            "a moving blocked actor did not take the jump event"
        );
        assert!(
            (rt.things[1].vel[0] - 0.1).abs() < 1e-5,
            "the nudge was {:?}",
            rt.things[1].vel
        );
        assert!(
            rt.things[1].yaw.abs() < 1e-3,
            "the actor did not face its target: yaw {}",
            rt.things[1].yaw
        );
        // The handler's own 2000 ms clock: an immediate second event does nothing.
        assert!(
            !rt.emit_ai_event(1, 0x400, None),
            "the jump ignored its class interval"
        );
        // A standing actor is refused by the engine's moving bit.
        rt.ai_jump.clear();
        rt.actor_flags.insert(1, 0);
        assert!(
            !rt.emit_ai_event(1, 0x400, None),
            "a standing actor took the jump event"
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
        rt.actor_flags.insert(1, 0x200);
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
        actor.pos = [1.0, 0.0, 0.0]; // sector 7, alignment -1
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
        rt.actor_flags.insert(1, 0x4);
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
        let flags = rt.actor_flags.get(&1).copied().unwrap_or(0);
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
            rt.awareness_at(7, -1),
            Some([1.0, 0.0, 0.0]),
            "the alert raised no awareness event"
        );
        let ring = rt.sector_awareness.get(&(7, -1)).expect("no ring");
        assert_eq!(
            ring[0].0, super::AI_AWARE_ALERT,
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
            rt.actor_flags.insert(1, flags);
            rt.sector_adjoins = vec![Vec::new(); 8];
            rt.ai_tick(0, 1.0 / 30.0);
            (
                rt.actor_flags.get(&1).copied().unwrap_or(0) & 0x800,
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
        rt.step(1.0);
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
    /// `sithAI_SetMoveThing`, with its own 1000 ms cooldown. Shipped classes are small steps: grunts
    /// ship `dodge 0.1, 0.1`, bosses `0.3, 0.1`.
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
        assert_eq!(rt.things[1].health, 95.0, "the damage did not apply");
        assert_eq!(
            rt.ai_move.get(&1).copied(),
            Some([0.7, 0.0, 0.0]),
            "the actor did not step 0.3 away from its attacker"
        );
        // Its own cooldown: another hit inside a second leaves the destination alone …
        rt.things[1].pos = [1.5, 0.0, 0.0];
        rt.things[2].pos = [3.5, 0.0, 0.0];
        rt.time = 0.5;
        rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
        assert_eq!(
            rt.ai_move.get(&1).copied(),
            Some([0.7, 0.0, 0.0]),
            "a second hit inside the 1 s cooldown moved the body again"
        );
        // … and after it, the step is measured from where the body now is.
        rt.time = 1.1;
        rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
        assert_eq!(
            rt.ai_move.get(&1).copied(),
            Some([1.2, 0.0, 0.0]),
            "the dodge did not re-arm after its cooldown"
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
            rt.actor_flags.insert(1, flags);
            rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
            rt.ai_move.get(&1).copied()
        };
        let class = "alignment=-1.0, rank=0.5\n\nDodge 0.3, 0.3\n";
        assert_eq!(sample(0, Some(class)), Some([0.7, 0.0, 0.0]), "no dodge with a clear flag");
        assert_eq!(sample(0x1, Some(class)), None, "a walking actor dodged anyway");
        assert_eq!(sample(0, Some("alignment=-1.0, rank=0.5\n\nDodge 0.3, 0.0\n")), None, "a zero step dodged");
        assert_eq!(sample(0, None), None, "an actor with no instinct dodged");
    }

    /// A runtime for the shot-dodge checks: player thing 0 at the origin facing +X, one actor at
    /// (2, 0, 0) whose class has `Dodge 0.3, 0.2` (weight 0.3, step 0.2). The template tables hold what
    /// `fire` needs — the bolt's `vel=(0/7/0)` and its shipped `typeflags=0x20440d` /
    /// `thingflags=0x20000001` — and the player's muzzle is its own eye (no `fireoffset=`), so the
    /// sweep is a clean +X line.
    fn shot_dodge_runtime(flags: u32, actor_yaw: f32, bolt_typeflags: u32, sightdist: f32) -> Runtime {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
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
        rt.actor_flags.insert(1, flags);
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

    /// `dodge`'s shot-passing path (`0x50b360`, event `0x1000`): the **player's** shot with
    /// `thingflags & 0x200000` runs a five-unit pre-sweep, and the actor it would hit — if it can see
    /// the shot (the class's `sightdist`/`fov`, clear line) — steps `arg0` units along the *negated*
    /// direction-to-the-shot rotated by a random 45-90 degrees: a sideways dodge, not a retreat.
    #[test]
    fn a_player_shot_that_would_hit_an_actor_makes_it_dodge_sideways() {
        let mut rt = shot_dodge_runtime(0x200, 180.0, 0x20440d, 5.0);
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
        assert!(fires(shot_dodge_runtime(0x200, 180.0, 0x20440d, 5.0)));
        // Not awake: the handler's first gate.
        assert!(!fires(shot_dodge_runtime(0x0, 180.0, 0x20440d, 5.0)));
        // Facing away: `CheckSightThing`'s cone fails.
        assert!(!fires(shot_dodge_runtime(0x200, 0.0, 0x20440d, 5.0)));
        // Too far for the class's sight.
        assert!(!fires(shot_dodge_runtime(0x200, 180.0, 0x20440d, 0.5)));
        // A projectile without the engine's typeflag bit: no pre-sweep, so no event at all.
        assert!(!fires(shot_dodge_runtime(0x200, 180.0, 0x440d, 5.0)));
        // A non-player shooter: the pre-sweep is player-only in `WeaponFireProjectile`.
        let mut rt = shot_dodge_runtime(0x200, 180.0, 0x20440d, 5.0);
        rt.things[1].yaw = 180.0;
        assert!(rt.fire(1, "+bryarbolt").is_some(), "the actor's own shot did not spawn");
        assert!(!rt.ai_move.contains_key(&1), "an actor's own shot made somebody dodge");
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
        rt.actor_flags.insert(1, 0x4);
        rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
        assert_eq!(rt.things[1].health, 95.0, "the damage did not apply");
        let flags = rt.actor_flags.get(&1).copied().unwrap_or(0);
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
            rt.actor_flags.insert(1, 0x4);
            rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
            (
                rt.ai_move.get(&1).copied(),
                rt.actor_flags.get(&1).copied().unwrap_or(0),
                rt.sounds.len(),
            )
        };
        let (moved, flags, sounds) = hit(true);
        assert_eq!(moved, None, "the sense-first class dodged after sensing");
        assert_ne!(flags & 0x800, 0);
        assert_eq!(sounds, 1);
        let (moved, flags, sounds) = hit(false);
        assert!(moved.is_some(), "the dodge-first class did not dodge");
        assert_eq!(flags & 0x4, 0x4, "the dodge-first class alerted after dodging");
        assert_eq!(sounds, 0, "the dodge-first class played the surprise anyway");
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
            rt.actor_flags.insert(1, 0x200);
            setup(&mut rt);
            rt.damage_typed_from(1, 5.0, DAMAGE_IMPACT, 2);
            (rt.ai_move.contains_key(&1), rt.actor_flags.get(&1).copied().unwrap_or(0))
        };
        assert!(sample(&|_| {}).0, "the plain case did not dodge");
        assert!(!sample(&|rt| rt.things[1].flags = 0x202).0, "a 0x202 thing reacted");
        assert!(!sample(&|rt| rt.things[1].health = 0.0).0, "a dead body reacted");
        assert!(!sample(&|rt| {
            rt.actor_flags.insert(1, 0x200 | 0x2000);
        })
        .0, "an AI switched off reacted");
        let (moved, flags) = sample(&|rt| {
            rt.actor_flags.insert(1, 0x200 | 0x1000);
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
            rt.actor_flags.insert(1, actor_flags);
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
        rt.actor_flags.insert(1, 0x200);
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
        let emplaced = rt.things[1].pos;
        for _ in 0..30 {
            rt.time += 1.0 / 30.0;
            rt.ai_tick(0, 1.0 / 30.0);
        }
        assert!(rt.shots_fired > 0, "the turret never fired");
        assert_eq!(rt.things[1].pos, emplaced, "the turret walked");
    }

    /// `circlestrafe` (`0x509890`): within the instinct's engage distance (0.6–1.0 in the shipped
    /// lines) the actor steps around the target by its angle (30–45 degrees) at half speed, keeping
    /// the radius. Shipped line: `circlestrafe 5000, 30, 0.6, 500, 0`.
    #[test]
    fn a_close_actor_with_circlestrafe_circles_the_target() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.5, 0.0, 0.0];
        actor.yaw = 180.0; // facing the body at the origin, so it is in sight
        let mut player = base.clone();
        player.pos = [0.0, 0.0, 0.0];
        rt.things = vec![player, actor];
        rt.templates = vec!["walkplayer".into(), "yun".into()];
        rt.types = vec![10, 2];
        rt.template_speed.insert("yun".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 CircleStrafe 5000.0, 30.0, 0.6, 500.0, 0.0\n\
                 Follow 0.0, 0.25\n",
            ),
        );

        for _ in 0..30 {
            rt.ai_tick(0, 1.0 / 30.0);
        }
        let p = rt.things[1].pos;
        let radius = (p[0] * p[0] + p[1] * p[1]).sqrt();
        assert!(p[1].abs() > 0.05, "the actor did not step sideways: {p:?}");
        assert!(
            (0.2..0.9).contains(&radius),
            "the actor left the ring: radius {radius:.2}"
        );

        // A control class with the same Follow but no CircleStrafe walks straight in and stops at
        // its standoff, so it never gains a sideways offset.
        let mut straight = door_runtime();
        let base = straight.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.5, 0.0, 0.0];
        actor.yaw = 180.0;
        let mut player = base.clone();
        player.pos = [0.0, 0.0, 0.0];
        straight.things = vec![player, actor];
        straight.templates = vec!["walkplayer".into(), "yun".into()];
        straight.types = vec![10, 2];
        straight.template_speed.insert("yun".into(), 1.0);
        straight.ai.insert(
            1,
            crate::ai::parse(
                "alignment=-1.0, rank=0.5, fov=180, sightdist=5.0\n\n\
                 Follow 0.0, 0.25\n",
            ),
        );
        for _ in 0..30 {
            straight.ai_tick(0, 1.0 / 30.0);
        }
        assert!(
            straight.things[1].pos[1].abs() < 1e-4,
            "an actor without `CircleStrafe` stepped sideways: {:?}",
            straight.things[1].pos
        );
    }

    /// `retreat` (`0x50bc60`) arms a withdraw when health falls under `arg0 × max` and its `arg1`
    /// chance succeeds, and `withdraw` (`0x50b150`) walks the actor away from the thing that hurt it.
    /// Shipped lines: `retreat 0.50, 0.50, 2000, 1.0` and `withdraw 5000, 1.0`.
    #[test]
    fn a_hurt_actor_with_retreat_backs_off() {
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

        rt.ai_tick(0, 1.0 / 30.0);
        assert!(
            rt.ai_withdraw.contains_key(&1),
            "a hurt actor with `Retreat` did not arm the withdraw mode"
        );
        for _ in 0..30 {
            rt.ai_tick(0, 1.0 / 30.0);
        }
        let backed = rt.things[1].pos[0];
        assert!(backed < -0.3, "the hurt actor did not back off: x {backed:.2}");

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

    /// `returnhome` (handler `0x50bd70`) walks an otherwise idle actor back to its home anchor — the
    /// engine's `SetMoveThing` to `ai + 0x93`, then `SetLookFrame` at it. The port feeds the anchor
    /// through the remembered-position slot, so the walk and turn are the ones already tuned.
    #[test]
    fn an_actor_with_returnhome_walks_back_to_its_anchor() {
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
        rt.template_speed.insert("tuskan".into(), 1.0);
        rt.ai.insert(
            1,
            crate::ai::parse("alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nReturnHome\n"),
        );
        rt.ai_home.insert(1, [0.0, 0.0, 0.0]);

        for _ in 0..30 {
            rt.ai_tick(0, 1.0 / 30.0);
        }
        let walked = rt.things[1].pos[0];
        assert!(walked < 1.7, "the actor did not walk home: x {walked:.2}");

        // A control class without `ReturnHome` stays where it was put.
        let mut quiet = door_runtime();
        let base = quiet.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [2.0, 0.0, 0.0];
        let mut player = base.clone();
        player.pos = [-9.0, 0.0, 0.0];
        quiet.things = vec![player, actor];
        quiet.templates = vec!["walkplayer".into(), "tuskan".into()];
        quiet.types = vec![10, 2];
        quiet.template_speed.insert("tuskan".into(), 1.0);
        quiet.ai.insert(1, crate::ai::parse("alignment=-1.0, rank=0.5, fov=90, sightdist=2.0\n\nJump\n"));
        quiet.ai_home.insert(1, [0.0, 0.0, 0.0]);
        for _ in 0..30 {
            quiet.ai_tick(0, 1.0 / 30.0);
        }
        assert_eq!(quiet.things[1].pos[0], 2.0, "an actor without `ReturnHome` walked away");
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

    /// `opendoors` (handler 0x50abc0) runs the **player's activate action** on the actor's own body:
    /// the engine calls `sithPlayerActions_Activate` and messages a captured thing in front with
    /// `activated` (id 1). The port reuses the use scans, so an actor facing a closed door queues its
    /// `activate` — which is what stops it from being trapped behind the door.
    #[test]
    fn an_actor_with_opendoors_activates_the_door_in_front() {
        let mut rt = door_runtime();
        let base = rt.things[0].clone();
        // Player at the origin, the actor at the origin facing -x (the port's forward is
        // `(-sin, cos)`), and a door thing 1.5 units along -x, linked to cog 0 as link 3.
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 90.0;
        let mut door = base.clone();
        door.pos = [-1.5, 0.0, 0.0];
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

        rt.ai_tick(0, 0.1);
        assert!(
            rt.pending_messages.iter().any(|(instance, label, _, sender)| {
                *instance == 0 && *label == "activate" && *sender == Some(3)
            }),
            "the actor did not activate the door in front: {:?}",
            rt.pending_messages
        );

        // Without the instinct the same body does nothing to the door.
        let mut quiet = door_runtime();
        let base = quiet.things[0].clone();
        let mut actor = base.clone();
        actor.pos = [0.0, 0.0, 0.0];
        actor.yaw = 90.0;
        let mut door = base.clone();
        door.pos = [-1.5, 0.0, 0.0];
        quiet.things = vec![base, actor, door];
        quiet.templates = vec!["walkplayer".into(), "tuskan".into(), "door".into()];
        quiet.types = vec![10, 2, 0];
        quiet.ai.insert(1, crate::ai::parse("alignment=-1.0, rank=0.5, fov=180\n\nLookForTarget 500.0\n"));
        quiet.thing_links.push((0, 3, 2));
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
        // The body at the origin, an actor two units along +x facing +x (i.e. away from the body).
        let mut actor = rt.things[0].clone();
        actor.pos = [2.0, 0.0, 0.0];
        actor.yaw = 0.0;
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

        // Facing away: outside the 90-degree cone, so the scan must not find the body even though the
        // range (2.0) is well inside `sightdist` (5.0).
        rt.ai_tick(0, 0.1);
        assert!(
            !rt.ai_last_seen.contains_key(&1),
            "the scan acquired a body outside the class's view cone"
        );

        // Turn the actor onto it: same range, now inside the cone, so the find lands.
        rt.things[1].yaw = 180.0;
        rt.ai_scan.clear();
        rt.ai_last_seen.clear();
        rt.ai_tick(0, 0.1);
        assert!(
            rt.ai_last_seen.contains_key(&1),
            "the scan did not acquire a body inside the class's view cone"
        );
    }
}
