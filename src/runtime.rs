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

    pub fn borrow(&self) -> std::sync::MutexGuard<'_, Runtime> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn borrow_mut(&self) -> std::sync::MutexGuard<'_, Runtime> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
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
pub const SAVE_SECTION: u16 = 0x1001;
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

/// [`Runtime::carry`]: the player's own state, which survives a level change.
#[derive(Clone, Debug, Default)]
pub struct CarryOver {
    pub inv: BTreeMap<i32, f32>,
    pub weapon: Option<String>,
    pub cur_bin: i32,
    pub saber: Option<SaberInfo>,
    pub jk_flags: u32,
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
fn template_insert(template: &str) -> [f32; 3] {
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
pub struct Runtime {
    pub things: Vec<ThingState>,
    /// Template name per thing, for `GetThingTemplate`.
    pub templates: Vec<String>,
    /// `GetThingType` per thing, from the template's `type=` (`SITH_THING_*` values).
    pub types: Vec<i32>,
    /// Template name -> thing type, for `CreateThing` to give the new thing its type.
    pub template_types: BTreeMap<String, i32>,
    /// A template's `health=`/`maxhealth=`, so a created thing starts with real health.
    pub template_health: BTreeMap<String, f32>,
    /// A template's `vel=` — a projectile's launch speed — and its `damage=`.
    pub template_vel: BTreeMap<String, [f32; 3]>,
    pub template_damage: BTreeMap<String, f32>,
    /// A projectile's `explode=` template: the thing the engine spawns where it lands.
    pub template_explode: BTreeMap<String, String>,
    /// A projectile's `mindamage=`, its `rate=` (damage lost per second while flying) and
    /// whether its typeflags carry `SITH_WF_DAMAGE_DECAY` (0x4000).
    pub template_min_damage: BTreeMap<String, f32>,
    /// `maxhealth=`, the ceiling `HealThing` clamps to.
    pub template_max_health: BTreeMap<String, f32>,
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
    /// so it queues `(cog instance, label, params)` and a system drains it. The params are what
    /// `GetParam(0)`/`GetParam(1)` answer inside the handler — `damaged` carries the amount and the
    /// damage type, which is how `force_absorb.cog` decides how much of it to take.
    pub pending_messages: Vec<(usize, &'static str, [f64; 2])>,
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
    /// Actors that steered through a portal on the last `ai_tick`, so a check can see the pathing.
    pub via_portal: Vec<usize>,
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
    /// `SetAutoSwitch`/`GetAutoSwitch`: the player's weapon-auto-switch bit field.
    pub auto_switch: u32,
    /// `SetAutoPickup`/`GetAutoPickup`: the same, for items.
    pub auto_pickup: u32,
    /// `SetWeaponMesh`/`jkSetPOVModel`/`jkSetWaggle`/`jkPlayPOVKey`: the first-person weapon's own
    /// data. This build draws no viewmodel, so these are recorded for the pass that will.
    pub weapon_mesh: BTreeMap<usize, String>,
    pub pov_model: BTreeMap<usize, String>,
    pub pov_key: BTreeMap<usize, String>,
    pub waggle: BTreeMap<usize, [f32; 3]>,
    /// `.key` name -> frame count, for `GetKeyLen`. The loader fills it.
    pub key_lengths: BTreeMap<String, f32>,
    /// The level time each actor last fired, so the renderer can play its `fire` track. The engine
    /// picks the action track from `sithPuppet_UpdateThingMove`, which this build has no analogue of.
    pub last_shot: BTreeMap<usize, f32>,
    /// `SetArmedMode`/`SetMajorMode`: the puppet's armed state and its major mode.
    pub armed_mode: BTreeMap<usize, u32>,
    pub major_mode: BTreeMap<usize, u32>,
    /// `SetCurWeapon`'s weapon thing, when a script named one.
    pub cur_weapon_thing: Option<usize>,
    /// `GetThingClassCog`: the cog a thing's class names.
    pub thing_class_cog: BTreeMap<usize, i32>,
    /// `jkEnableSaber`/`jkDisableSaber`, and the saber camera the scripts switch to.
    pub saber_enabled: bool,
    pub saber_cam: i32,
    /// `misc\items.dat`'s `cog=` column: inventory bin -> the script that bin runs. The loader fills
    /// it once, because the runtime has no file access.
    pub item_cog: BTreeMap<i32, String>,
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
    /// How long an actor waits between shots. The engine's instinct scheduler decides with
    /// the file's weights; this build uses a fixed cadence, which is a documented stand-in.
    pub ai_shot_interval: f32,
    /// The last hold time `DeactivateBin` reported, kept so a caller can inspect it after
    /// the entry has been popped.
    pub last_hold: f32,
    /// `AddDynamicTint`: the last screen tint a script asked for.
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
            prints: Vec::new(),
            hud_message: None,
            string_buffer: String::new(),
            loaded_sounds: Vec::new(),
            weapon_cog: None,
            weapon_rate: 0.0,
            auto_switch: 1,
            auto_pickup: 1,
            weapon_mesh: BTreeMap::new(),
            pov_model: BTreeMap::new(),
            pov_key: BTreeMap::new(),
            waggle: BTreeMap::new(),
            key_lengths: BTreeMap::new(),
            last_shot: BTreeMap::new(),
            armed_mode: BTreeMap::new(),
            major_mode: BTreeMap::new(),
            cur_weapon_thing: None,
            thing_class_cog: BTreeMap::new(),
            saber_enabled: true,
            saber_cam: 0,
            item_cog: BTreeMap::new(),
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
            template_explode,
            template_timer,
            thing_timer: BTreeMap::new(),
            player_shields: 100.0,
            player_dead: false,
            ir_mode: false,
            map_mode_flags: 0,
            walked: Vec::new(),
            walk_vel: BTreeMap::new(),
            via_portal: Vec::new(),
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
            bin_wait: BTreeMap::new(),
            time: 0.0,
            fire_wait: 0.0,
            next_fire: 0.0,
            master_cog: None,
            multi_mode_flags: 0,
            play_key: BTreeMap::new(),
            face_geomode: BTreeMap::new(),
            camera_focus: [None; 7],
            camera_secondary: [None; 7],
            current_camera: 0,
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
            ai_shot_interval: 1.0,
            ai_standoff: 0.25,
            ai_move: BTreeMap::new(),
            ai_chase: BTreeMap::new(),
            ai_look: BTreeMap::new(),
            ai_route: BTreeMap::new(),
            ai_last_seen: BTreeMap::new(),
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
        self.decay_damage(dt);
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
                self.pending_messages.push((instance, "timer", [0.0; 2]));
            }
        }
        // Force mana regenerates. `force_heal.cog` spends `20 * rank` of bin 14 and the shipped
        // scripts expect the pool to come back; the *rate* is calibrated (§16.5) because the engine's
        // own tick lives in `jkPlayer.c`, which this source tree does not carry.
        if let Some(mana) = self.inv.get_mut(&14) {
            *mana = (*mana + self.mana_regen * dt).min(self.mana_max);
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
                self.pending_messages.push((instance, "arrived", [0.0; 2]));
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
        for (i, t) in self.things.iter_mut().enumerate() {
            if let Some(drive) = usize::try_from(t.sector).ok().and_then(|s| thrust.get(s)).copied()
                && drive != [0.0; 3]
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
                let (resolved, hit) = solids.resolve_stepped(sector, t.pos, 0.4);
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
        }
        for index in blocked {
            for instance in self.thing_instances(index) {
                self.pending_messages.push((instance, "blocked", [0.0; 2]));
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

    /// Fire `template` from the shooter: spawn it just in front of the shooter's eye and
    /// give it the template's own `vel=`, rotated into the shooter's facing.
    ///
    /// The engine's projectile is a `SITH_THING_WEAPON` created by `CreateThing` from the
    /// weapon template, launched along the barrel; the speed in the shipped templates is
    /// `vel=(0/4/0)` for `+bryarbolt`, i.e. 4 units per second down the model's +Y.
    /// One AI tick: every actor that can see the player fires its own `weapon=`.
    ///
    /// The engine runs an instinct scheduler over the `.ai` file's weights; this build uses
    /// the fields whose meaning is not in doubt — `sightdist=`, `fov=` — and a fixed
    /// `ai_shot_interval` cadence between shots. Returns how many shots were fired.
    pub fn ai_tick(&mut self, player: usize, dt: f32) -> usize {
        let Some(target) = self.things.get(player).map(|t| t.pos) else { return 0 };
        let mut fired = 0;
        let mut walked = 0;
        self.walked.clear();
        self.walk_vel.clear();
        self.via_portal.clear();
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
            let Some(thing) = self.things.get_mut(index) else { continue };
            if thing.destroyed || self.types.get(index).copied() != Some(2) {
                continue;
            }
            // The firing cadence gates the *shot*, not the legs: an actor that has just
            // fired keeps walking, which is what makes it close the distance instead of
            // stopping for a second every shot.
            thing.ai_wait = (thing.ai_wait - dt).max(0.0);
            let ready = thing.ai_wait <= 0.0;
            let (from, yaw) = (thing.pos, thing.yaw);
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
            let remembered = self.ai_last_seen.get(&index).copied();
            if scripted.is_none() && !sighted && aim.is_none() && remembered.is_none() {
                // Out of sight and nothing remembered: the actor stands where it is, turning to a
                // scripted look target.
                if let Some(thing) = self.things.get_mut(index) {
                    thing.ai_target = None;
                    if let Some(yaw) = look_yaw {
                        thing.yaw = yaw;
                    }
                }
                continue;
            }
            // `AIFlee(actor, from)`: the actor runs away from that thing and does not shoot, which
            // is a whole mode in the engine (`sithAI_Flee`); here the goal is the opposite
            // direction, ten units out.
            let fleeing = self
                .ai_flee
                .get(&index)
                .and_then(|t| self.things.get(*t).map(|t| t.pos));
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
                        thing.yaw = away[1].atan2(away[0]).to_degrees();
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
            let want = scripted.unwrap_or(chase);
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
                * self.actor_speed.get(&index).copied().unwrap_or(1.0);
            // A scripted destination is walked all the way onto; the chase stops at its standoff, and
            // so does each doorway on a route.
            let standoff = if scripted.is_some() || route_goal.is_some() {
                0.0
            } else {
                self.ai_standoff
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
            let route_goal = self.ai_route.get(&index).and_then(|r| r.first().copied());
            let goal = route_goal.unwrap_or(want);
            let dx = goal[0] - from[0];
            let dy = goal[1] - from[1];
            let reach = (dx * dx + dy * dy).sqrt();
            // Arrival is measured against the **destination**, not the doorway the actor is aiming
            // at: a waypoint is a place on the way, and stopping at one dropped a script's order.
            let destination_reach =
                ((want[0] - from[0]).powi(2) + (want[1] - from[1]).powi(2)).sqrt();

            if reach > standoff && speed > 0.0 {
                let step = (speed * dt).min(reach - standoff);
                let next = [
                    from[0] + dx / reach * step,
                    from[1] + dy / reach * step,
                    from[2],
                ];
                let mut via_portal = false;
                let heading = (dx, dy);
                if let Some(thing) = self.things.get_mut(index) {
                    thing.pos = next;
                    thing.ai_target = Some(goal);
                    thing.yaw = heading.1.atan2(heading.0).to_degrees();
                    if via_portal {
                        self.via_portal.push(index);
                    }
                }
                walked += 1;
                self.walked.push(index);
                // The velocity this body is walking at, which is what the renderer needs to pick
                // its puppet state — a position delta cannot tell a stride from a collision push.
                let dt = dt.max(1e-4);
                self.walk_vel.insert(
                    index,
                    [(next[0] - from[0]) / dt, (next[1] - from[1]) / dt],
                );
            } else if let Some(thing) = self.things.get_mut(index) {
                thing.ai_target = None;
                if let Some(yaw) = look_yaw {
                    thing.yaw = yaw;
                } else if reach > 0.001 {
                    thing.yaw = dy.atan2(dx).to_degrees();
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
            let shooting_at = aim.unwrap_or(target);
            let can_see = if aim.is_some() { true } else { sighted };
            if !can_see || !ready {
                continue;
            }
            let Some(primary) = ai.as_ref().and_then(|ai| ai.instinct("PrimaryFire")).cloned() else {
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
            let (delay_ms, min_dot, max_dist, error, min_dist) =
                (arg(0), arg(1), arg(2), arg(3), arg(4));
            let (burst, burst_ms) = (arg(8), arg(9));
            let (sx, sy) = (shooting_at[0] - from[0], shooting_at[1] - from[1]);
            let range = (sx * sx + sy * sy).sqrt();
            if range > max_dist.max(0.001) || range < min_dist {
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
        for (index, weapon, error) in shots {
            if self.fire_spread(index as i64, &weapon, error).is_some() {
                // The renderer plays a puppet's `fire` track off this: the engine's
                // `sithPuppet_UpdateThingMove` picks the action track while the actor is shooting.
                self.last_shot.insert(index, self.time);
                fired += 1;
            }
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

    /// [`Self::fire`] with the actor's aim error: `sithAICmd_PrimaryFire` passes
    /// `percentageErrorInAim` and the engine spreads the shot inside that fraction of a right
    /// angle. A missed shot is a shot that flies wide, which is what makes a firefight survivable.
    pub fn fire_spread(&mut self, shooter: i64, template: &str, error: f32) -> Option<usize> {
        let spread = if error > 0.0 {
            // A fresh heading inside +/- `error` of a right angle, jittered around the aim line.
            let r = crate::cog::pseudo_random() as f32;
            ((r - 0.5) * 2.0) * error * std::f32::consts::FRAC_PI_2
        } else {
            0.0
        };
        self.fire_with_spread(shooter, template, spread)
    }

    pub fn fire(&mut self, shooter: i64, template: &str) -> Option<usize> {
        self.fire_with_spread(shooter, template, 0.0)
    }

    fn fire_with_spread(
        &mut self,
        shooter: i64,
        template: &str,
        spread: f32,
    ) -> Option<usize> {
        let template = template.to_ascii_lowercase();
        let src = self.thing(shooter)?;
        let (pos, yaw, sector) = (src.pos, src.yaw, src.sector);
        let speed = self.template_vel.get(&template).copied().unwrap_or([0.0; 3]);
        // The template's velocity is in the thing's own frame: forward is +Y there, and
        // a JK yaw of `a` faces `(cos a, sin a, 0)`. `spread` is the aim error an AI instinct
        // carries: it turns the whole shot, so a missed shot is a shot that flies wide.
        let (sin, cos) = (yaw.to_radians() + spread).sin_cos();
        let forward = [
            speed[1] * cos - speed[0] * sin,
            speed[1] * sin + speed[0] * cos,
            speed[2],
        ];
        let health = self.template_health.get(&template).copied().unwrap_or(1.0);
        let kind = self.template_types.get(&template).copied().unwrap_or(3);
        // The shot starts at full damage and loses it in flight when the weapon decays.
        let damage = self.template_damage.get(&template).copied().unwrap_or(0.0);
        if std::env::var_os("BK_HIT_TRACE").is_some() {
            eprintln!("fire: {template} dmg {damage:.1} vel {forward:?} kind {kind}");
        }
        let key = template.clone();
        self.things.push(ThingState {
            // Out of the body, so the projectile does not start inside the shooter. The
            // engine's muzzle is the template's `fireoffset=`; this world's stand-in is a unit
            // ahead of the eye. It has to clear the shooter's own body radius (0.6 + the bolt's
            // 0.15), or the shot lands on the thing that fired it on the frame it is created —
            // which is exactly what `--hit` caught.
            pos: {
                let len = (forward[0] * forward[0] + forward[1] * forward[1]).sqrt().max(1e-4);
                [
                    pos[0] + forward[0] / len * 1.0,
                    pos[1] + forward[1] / len * 1.0,
                    pos[2] + 0.1,
                ]
            },
            insert: [0.0; 3],
            yaw,
            pitch: 0.0,
            roll: 0.0,
            sector,
            flags: 0,
            health,
            frame: 0.0,
            target: None,
            frames: Vec::new(),
            destroyed: false,
            vel: forward,
            damage,
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
            min_damage: self.template_min_damage.get(&key).copied().unwrap_or(damage),
            decay: if self.template_decay.contains(&key) {
                self.template_rate.get(&key).copied().unwrap_or(0.0)
            } else {
                0.0
            },
        });
        self.templates.push(template);
        self.types.push(kind);
        let index = self.things.len() - 1;
        self.arm_life(index);
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
        // Exact-ish names, not bare substrings: a "st" hint happily matched `+concblast2`
        // and a "saber" hint matched the `+force_saber` particle effect.
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

    /// `SetCurInvWeapon` / `SelectWeapon`: the bin the player has armed, and — when this
    /// build can tell which template that bin means — the projectile it fires.
    pub fn select_bin(&mut self, bin: i32) -> bool {
        if !(0..=26).contains(&bin) {
            return false;
        }
        self.cur_bin = bin;
        if let Some(weapon) = self.weapon_for_bin(bin) {
            self.weapon = Some(weapon);
        }
        true
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
            if flat > reach {
                continue;
            }
            // Behind the player is not in reach: the engine's use is a look, not a touch.
            if flat > 0.01 && (dx / flat) * facing[0] + (dy / flat) * facing[1] < 0.3 {
                continue;
            }
            if best.is_none_or(|(_, d)| flat < d) {
                best = Some((*surface, flat));
            }
        }
        let (surface, _) = best?;
        for (instance, _, linked) in &self.surface_links {
            if *linked == surface {
                self.pending_messages.push((*instance, "activate", [0.0; 2]));
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
            if flat > reach {
                continue;
            }
            if flat > 0.01 && (dx / flat) * facing[0] + (dy / flat) * facing[1] < 0.3 {
                continue;
            }
            if best.is_none_or(|(_, d)| flat < d) {
                best = Some((*target, flat));
            }
        }
        let (target, _) = best?;
        for (instance, _, linked) in &self.thing_links {
            if *linked == target {
                // `activate`, not `used`: no shipped cog declares a `used` label (0 of 540), while 82 of them -- every door, switch and item -- answer `activate`. It is also what the surface half of this action already sends.
                self.pending_messages.push((*instance, "activate", [0.0; 2]));
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
    pub fn sector_messages(&self, old: Option<usize>, new: Option<usize>) -> Vec<(usize, &'static str)> {
        let mut out = Vec::new();
        for (instance, _link, sector) in &self.sector_links {
            if Some(*sector) == new {
                out.push((*instance, "entered"));
            } else if Some(*sector) == old {
                out.push((*instance, "exited"));
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
                self.pending_messages.push((*instance, "sighted", [0.0; 2]));
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
                self.pending_messages.push((*instance, "sighted", [0.0; 2]));
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
        let name = self.templates.get(target).cloned().unwrap_or_default().to_ascii_lowercase();
        let (radius, height) = self.template_body.get(&name).copied().unwrap_or((0.5, 1.8));
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
        let Ok(index) = usize::try_from(target) else { return 0.0 };
        if self.invulnerable(index, kind) {
            return self.things.get(index).map(|t| t.health).unwrap_or(0.0);
        }
        let mut died = false;
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
            self.pending_messages.push((instance, "damaged", params));
            if died {
                self.pending_messages.push((instance, "killed", params));
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

    /// What the player keeps across a level change.
    ///
    /// The engine's `jkPlayer_*` state survives the level load — inventory, the armed bin, the
    /// selected weapon, the saber, the `jkFlags` the force scripts set. This build rebuilds the whole
    /// runtime from the incoming `.jkl`, so those pieces have to be handed over explicitly instead of
    /// coming out of the thing list.
    pub fn carry(&self) -> CarryOver {
        let player = self.player_thing();
        CarryOver {
            inv: self.inv.clone(),
            weapon: self.weapon.clone(),
            cur_bin: self.cur_bin,
            saber: self.saber.get(&player).cloned(),
            jk_flags: self.things.get(player).map(|t| t.jk_flags).unwrap_or(0),
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
        if let Some(thing) = self.things.get_mut(player) {
            thing.jk_flags = carry.jk_flags;
        }
    }

    /// The thing the view follows, when a script has pointed a camera away from the player.
    ///
    /// The engine's camera table is seven slots (`sithCamera_g_aCameras[7]`); `SetCameraFocus(slot,
    /// thing)` points one at a thing and `SetCurrentCamera(slot)` picks it. `None` means the player's
    /// own eyes, which is both the default and what a focus on the player means.
    pub fn camera_focus_thing(&self) -> Option<usize> {
        let focus = self.camera_focus.get(self.current_camera).copied().flatten()?;
        (focus != self.player_thing()).then_some(focus)
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
            // `SectorThrust(sector, vector)`: the force the sector applies to things in
            // it. Stored, not simulated — nothing is in flight yet (combat, stage 5).
            "sectorthrust" => {
                let v = args.get(1).map(vec3).unwrap_or([0.0; 3]);
                if let Some(s) = self.sector_mut(idx(0)) {
                    s.thrust = v;
                }
                Value::Nil
            }
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
            "aiflee" => {
                if let (Ok(actor), Ok(from)) = (usize::try_from(idx(0)), usize::try_from(idx(1))) {
                    self.ai_flee.insert(actor, from);
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
                    Some(Value::Str(name)) if self.template_damage.contains_key(&name.to_ascii_lowercase()) => {
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
                self.bin_active.insert(bin as i32, self.time);
                Value::Nil
            }
            "deactivatebin" => {
                let held = self.bin_active.remove(&(idx(1) as i32)).map(|at| self.time - at).unwrap_or(0.0);
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
                let cog = idx(1) as i32;
                let rate = num(2) as f32;
                let mode = idx(3) as i32;
                if (0..2).contains(&mode) {
                    self.weapon_cog = Some((cog, rate, mode));
                    self.weapon_rate = rate;
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
                self.waggle.insert(ti(0), args.get(1).map(vec3).unwrap_or([0.0; 3]));
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
            // `jkPrintUNIString(player, key)`: the engine puts the string in the message area, the
            // same place `jkStringOutput` sends a composed line. The key is resolved by the app.
            "jkprintunistring" => {
                let line = match args.first() {
                    Some(Value::Str(s)) => s.clone(),
                    _ if !args.is_empty() => format!("COG_{:05}", num(0) as i64),
                    _ => String::new(),
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
            // How many of a bin the player can hold. The engine's table is per item class
            // (`jkInventory`); without it, a bin is treated as roomy, which is what lets a
            // powerup script's `if (amount < GetInvMax(...))` guard pass and `TakeItem` run.
            "getinvmax" => Value::Num(100.0),
            "setinvavailable" => {
                self.inv_available.insert(idx(0) as i32, idx(1) != 0);
                Value::Nil
            }
            // `AddDynamicTint(player, r, g, b)`: the screen tint a powerup flashes. Recorded
            // as the player's tint so the renderer can use it later.
            "adddynamictint" => {
                if let Some(t) = self.thing_mut(idx(0)) {
                    let _ = t;
                }
                self.tint = [num(1) as f32, num(2) as f32, num(3) as f32];
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
        Runtime {
            things: vec![ThingState {
                pos: [0.0; 3],
                insert: [0.0; 3],
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
            }],
            templates: vec!["walkplayer".into()],
            types: vec![10],
            template_types: BTreeMap::from([("walkplayer".to_string(), 10)]),
            template_health: BTreeMap::from([("walkplayer".to_string(), 100.0)]),
            template_vel: BTreeMap::new(),
            template_damage: BTreeMap::new(),
            template_inv: BTreeMap::new(),
            template_count: BTreeMap::new(),
            template_bins: BTreeMap::new(),
            prints: Vec::new(),
            hud_message: None,
            string_buffer: String::new(),
            loaded_sounds: Vec::new(),
            weapon_cog: None,
            weapon_rate: 0.0,
            auto_switch: 1,
            auto_pickup: 1,
            weapon_mesh: BTreeMap::new(),
            pov_model: BTreeMap::new(),
            pov_key: BTreeMap::new(),
            waggle: BTreeMap::new(),
            key_lengths: BTreeMap::new(),
            last_shot: BTreeMap::new(),
            armed_mode: BTreeMap::new(),
            major_mode: BTreeMap::new(),
            cur_weapon_thing: None,
            thing_class_cog: BTreeMap::new(),
            saber_enabled: true,
            saber_cam: 0,
            item_cog: BTreeMap::new(),
            pending_cogs: Vec::new(),
            spawned_cogs: std::collections::BTreeSet::new(),
            messages: Vec::new(),
            caller_cog: -1,
            messages_sent: 0,
            sectors: vec![SectorState { thrust: [0.0; 3], light: 0.0, light_from: 0.0, light_end: 0.0, light_rate: 0.0, tint: [0.0; 3], colormap: 0 }],
            sector_flags: vec![0],
            adjoin_flags: vec![0x7],
            surface_adjoin: vec![0],
            surface_geomode: Vec::new(),
            surfaces: Vec::new(),
            sounds: Vec::new(),
            timers: 0.0,
            inv: BTreeMap::new(),
            inv_available: BTreeMap::new(),
            inv_activated: BTreeMap::new(),
            bin_active: BTreeMap::new(),
            bin_wait: BTreeMap::new(),
            time: 0.0,
            fire_wait: 0.0,
            next_fire: 0.0,
            master_cog: None,
            multi_mode_flags: 0,
            play_key: BTreeMap::new(),
            face_geomode: BTreeMap::new(),
            camera_focus: [None; 7],
            camera_secondary: [None; 7],
            current_camera: 0,
            shake_pos: [0.0; 3],
            shake_angle: [0.0; 3],
            shake_pos_rate: 0.0,
            shake_angle_rate: 0.0,
            surface_epoch: 0,
            surface_cels: Vec::new(),
            material_cels: BTreeMap::new(),
            material_names: BTreeMap::new(),
            material_anims: Vec::new(),
            material_cel: BTreeMap::new(),
            face_light_mode: BTreeMap::new(),
            thing_geomode: BTreeMap::new(),
            geomode_epoch: 0,
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
            template_weapon: BTreeMap::new(),
            ai_shot_interval: 1.0,
            ai_standoff: 0.25,
            ai_move: BTreeMap::new(),
            ai_chase: BTreeMap::new(),
            ai_look: BTreeMap::new(),
            ai_route: BTreeMap::new(),
            ai_last_seen: BTreeMap::new(),
            ai_fire_target: BTreeMap::new(),
            ai_flee: BTreeMap::new(),
            ai_classes: BTreeMap::new(),
            level_complete: false,
            autosave: false,
            music: None,
            music_volume: 1.0,
            template_speed: BTreeMap::new(),
            last_hold: 0.0,
            tint: [0.0; 3],
            color_effects: BTreeMap::new(),
            next_color_effect: 1,
            thing_model: BTreeMap::new(),
            force_target: None,
            force_target_tint: Vec::new(),
            saber: BTreeMap::new(),
            saber_wait: 0.0,
            weapon: None,
            cur_bin: 0,
            default_weapon: "+bryarbolt".into(),
            respawning: Vec::new(),
            template_respawn: BTreeMap::new(),
            template_min_damage: BTreeMap::new(),
            template_max_health: BTreeMap::new(),
            template_timer: BTreeMap::new(),
            thing_timer: BTreeMap::new(),
            template_body: BTreeMap::new(),
            template_explode: BTreeMap::new(),
            player_shields: 100.0,
            player_dead: false,
            ir_mode: false,
            map_mode_flags: 0,
            walked: Vec::new(),
            walk_vel: BTreeMap::new(),
            via_portal: Vec::new(),
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
            template_rate: BTreeMap::new(),
            template_decay: std::collections::BTreeSet::new(),
            template_sprite_anim: std::collections::BTreeSet::new(),

            adjoins_changes: 0,
            things_moved: 0,
        }
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
}
