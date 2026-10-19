//! `.jkl` level parser.
//!
//! Recovered from the shipped levels. The format is plain ASCII with
//! `SECTION: <name>` blocks. Only the blocks needed to draw the world are
//! parsed here:
//!
//!   SECTION: HEADER      - gravity, sky Z, mipmap/LOD distances
//!   SECTION: MATERIALS   - `<idx>: <file>.mat <xTile> <yTile>`
//!   SECTION: GEORESOURCE - colormaps, world vertices, texture vertices,
//!                          adjoins, surfaces
//!   Section: Things      - placed objects
//!
//! Everything else (SOUNDS, AICLASS, cog*, TEMPLATES, ...) is skipped.

use crate::rows::{fields, floats_in, read_names, read_rows};

/// A named template: `<name> <class> key=value ...`
pub struct Template {
    pub name: String,
    /// The template this one is `Based On:`, if any: `walkplayer` derives from `_actor`,
    /// which is where `jumpspeed` and `eyeoffset` live.
    pub base: Option<String>,
    /// Every `key=value` pair in the row, in file order.
    pub params: Vec<(String, String)>,
    /// The `model3d=` value, i.e. the `.3do` a placed object of this template uses.
    pub model3d: Option<String>,
    /// The `sprite=` value. Effect templates (`+laserhit`, `+grenade_exp`) use this
    /// instead of a model; nothing *placed* in the shipped levels does, because
    /// sprites are spawned at runtime by the scripts.
    pub sprite: Option<String>,
}

/// One placed `.cog`: the script and the values bound to its symbols, in the
/// declaration order of the script's non-message, non-local symbols.
#[derive(Clone)]
pub struct CogInstance {
    pub script: String,
    pub values: Vec<String>,
}

/// An asset the level references by name, resolved through the archive index.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Model,
    Sprite,
    Keyframe,
    Puppet,
    AiClass,
    Cog,
    Sound,
}

impl AssetKind {
    /// File extension this kind resolves to, when the reference omits one.
    pub fn ext(self) -> &'static str {
        match self {
            AssetKind::Model => "3do",
            AssetKind::Sprite => "spr",
            AssetKind::Keyframe => "key",
            AssetKind::Puppet => "pup",
            AssetKind::AiClass => "ai",
            AssetKind::Cog => "cog",
            AssetKind::Sound => "wav",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AssetKind::Model => "model",
            AssetKind::Sprite => "sprite",
            AssetKind::Keyframe => "keyframe",
            AssetKind::Puppet => "puppet",
            AssetKind::AiClass => "aiclass",
            AssetKind::Cog => "cog",
            AssetKind::Sound => "sound",
        }
    }
}

pub struct Asset {
    pub kind: AssetKind,
    pub name: String,
}

#[derive(Default)]
pub struct Level {
    pub vertices: Vec<[f32; 3]>,
    pub texverts: Vec<[f32; 2]>,
    pub surfaces: Vec<Surface>,
    /// `World adjoins`: the portal links between surfaces, in file order. A surface's
    /// `adjoin` indexes this table; `mirror` names the adjoin on the other side.
    pub adjoins: Vec<Adjoin>,
    pub materials: Vec<String>,
    pub colormaps: Vec<String>,
    pub sectors: Vec<Sector>,
    pub things: Vec<Thing>,
    pub templates: Vec<Template>,
    pub assets: Vec<Asset>,
    /// Placed script instances: which `.cog` and the symbol values bound to it.
    pub cogs: Vec<CogInstance>,
    pub gravity: f32,
    pub sky_z: f32,
}

/// A convex room. `center` is inside it by construction, which makes it the one
/// reliable place to put a camera when the surrounding geometry is unknown.
#[derive(Default, Clone)]
pub struct Sector {
    pub flags: u32,
    pub center: [f32; 3],
    pub radius: f32,
    pub colormap: u32,
    /// `(first, count)` into `Level::surfaces`, from the sector's own `SURFACES` row.
    pub surfaces: (usize, usize),
    /// The sector's ambient loop: `SOUND <wav> <volume>`, played on loop while the
    /// player is in this sector. Most sectors have none.
    pub sound: Option<(String, f32)>,
}

/// One row of the `World adjoins` table: a portal link.
#[derive(Clone, Default)]
pub struct Adjoin {
    /// Index of the adjoin on the other side of the same opening, or `-1`.
    pub mirror: i32,
    /// Mirror distance, used by the engine's reflection pass. Parsed so the field order
    /// is documented; nothing here reflects yet.
    #[allow(dead_code)]
    pub dist: f32,
}

#[derive(Clone)]
pub struct Surface {
    pub material: i32,
    /// Raw `surfflags` word. Which bit marks a sky face is not yet established.
    pub surfflags: u32,
    pub geo: i32,
    pub adjoin: i32,
    /// (world vertex, texture vertex or -1)
    pub verts: Vec<(u32, i32)>,
    /// Per-vertex shading scalar baked by the level editor's lightmapper.
    pub intensities: Vec<f32>,
}

pub struct Thing {
    pub template: String,
    pub pos: [f32; 3],
    /// The thing's full orientation in degrees: `pitch` about X, `yaw` about Z, `roll` about Y — the three
    /// numbers the row carries after the position (`rdMatrix_Build34(pyr)` in the engine, whose `x/y/z` are
    /// these in order). Only `yaw` was read for a long time, which draws every tilted thing flat.
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    pub sector: i32,
    /// `thingflags=` — the engine's per-thing state bits.
    pub flags: u32,
    /// `frame=(x/y/z:pitch/yaw/roll)…` — the thing's keyframe path, written straight
    /// into the level. Doors and lifts are exactly these lists: `MoveToFrame`
    /// interpolates them, and frame 0 is the position above. `.key` files are for
    /// actors.
    pub frames: Vec<[f32; 6]>,
    /// `vel=(x/y/z)`.
    pub vel: [f32; 3],
}

impl Thing {
    /// `frame=(x/y/z:p/y/r)` → six numbers.
    fn parse_frame(s: &str) -> Option<[f32; 6]> {
        let inner = s.strip_prefix("frame=(")?.trim_end_matches(')');
        let (pos, rot) = inner.split_once(':')?;
        let p: Vec<f32> = pos.split('/').filter_map(|v| v.trim().parse().ok()).collect();
        let r: Vec<f32> = rot.split('/').filter_map(|v| v.trim().parse().ok()).collect();
        if p.len() != 3 || r.len() != 3 {
            return None;
        }
        Some([p[0], p[1], p[2], r[0], r[1], r[2]])
    }
}

impl Level {
    /// A template's raw parameter, following its `Based On:` chain.
    pub fn template_param(&self, name: &str, key: &str) -> Option<&str> {
        let mut current = name;
        // The shipped chains are at most three deep (`walkplayer` -> `_actor`), so a
        // fixed cut-off is enough and also stops a malformed file from looping.
        for _ in 0..8 {
            let template = self
                .templates
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(current))?;
            if let Some((_, value)) = template
                .params
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
            {
                return Some(value);
            }
            current = template.base.as_deref()?;
        }
        None
    }

    /// A numeric template parameter, following the same chain.
    pub fn template_f32(&self, name: &str, key: &str) -> Option<f32> {
        self.template_param(name, key).and_then(|v| v.parse().ok())
    }

    /// Axis-aligned bounds of the world geometry.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for v in &self.vertices {
            for a in 0..3 {
                lo[a] = lo[a].min(v[a]);
                hi[a] = hi[a].max(v[a]);
            }
        }
        if self.vertices.is_empty() {
            ([0.0; 3], [0.0; 3])
        } else {
            (lo, hi)
        }
    }
}

pub fn parse(text: &str) -> Level {
    let mut level = Level { gravity: 4.0, sky_z: 20.0, ..Default::default() };
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0usize;

    while i < lines.len() {
        let line = lines[i].trim();
        let lower = line.to_ascii_lowercase();

        if lower.starts_with("section:") {
            i += 1;
            continue;
        }

        // ---- HEADER scalars ------------------------------------------------
        if lower.starts_with("world gravity") {
            if let Some(v) = nth_f32(line, 2) {
                level.gravity = v;
            }
        } else if lower.starts_with("ceiling sky z") {
            if let Some(v) = nth_f32(line, 3) {
                level.sky_z = v;
            }
        }
        // ---- colormaps -----------------------------------------------------
        else if lower.starts_with("world colormaps") {
            let n = nth_f32(line, 2).unwrap_or(0.0) as usize;
            i += 1;
            for row in read_rows(&lines, &mut i, n) {
                if let Some(name) = indexed_name(row) {
                    level.colormaps.push(name);
                }
            }
            continue;
        }
        // ---- world vertices -------------------------------------------------
        else if lower.starts_with("world vertices") {
            let n = nth_f32(line, 2).unwrap_or(0.0) as usize;
            i += 1;
            for row in read_rows(&lines, &mut i, n) {
                let f = floats_in(row, 1, 3);
                if f.len() >= 3 {
                    level.vertices.push([f[0], f[1], f[2]]);
                }
            }
            continue;
        }
        // ---- texture vertices -----------------------------------------------
        else if lower.starts_with("world texture vertices") {
            let n = nth_f32(line, 3).unwrap_or(0.0) as usize;
            i += 1;
            for row in read_rows(&lines, &mut i, n) {
                let f = floats_in(row, 1, 2);
                if f.len() >= 2 {
                    level.texverts.push([f[0], f[1]]);
                }
            }
            continue;
        }
        // ---- adjoins ---------------------------------------------------------
        // `World adjoins N` then `num: flags mirror dist` per row. A surface's `adjoin`
        // field indexes this table, and `mirror` names the adjoin on the *other* side of
        // the same portal — which is how two rooms are linked.
        else if lower.starts_with("world adjoins") {
            let n = nth_f32(line, 2).unwrap_or(0.0) as usize;
            i += 1;
            for row in read_rows(&lines, &mut i, n) {
                let f = fields(row);
                let mirror: i32 = f.get(2).and_then(|v| v.parse().ok()).unwrap_or(-1);
                let dist: f32 = f.get(3).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                level.adjoins.push(Adjoin { mirror, dist });
            }
            continue;
        }
        // ---- surfaces --------------------------------------------------------
        else if lower.starts_with("world surfaces") {
            let n = nth_f32(line, 2).unwrap_or(0.0) as usize;
            i += 1;
            for row in read_rows(&lines, &mut i, n) {
                if let Some(s) = parse_surface(row) {
                    level.surfaces.push(s);
                }
            }
            continue;
        }
        // ---- materials -------------------------------------------------------
        else if lower.starts_with("world materials") {
            let n = nth_f32(line, 2).unwrap_or(0.0) as usize;
            i += 1;
            for row in read_rows(&lines, &mut i, n) {
                level.materials.push(indexed_name(row).unwrap_or_default());
            }
            continue;
        }
        // ---- things ----------------------------------------------------------
        else if lower.starts_with("world things") {
            let n = nth_f32(line, 2).unwrap_or(0.0) as usize;
            i += 1;
            for row in read_rows(&lines, &mut i, n) {
                if let Some(t) = parse_thing(row) {
                    level.things.push(t);
                }
            }
            continue;
        }

        // ---- placed scripts ---------------------------------------------------
        else if lower.starts_with("world cogs") {
            // `<index>: <tab> <script> <tab> <symbol values…>`, bound positionally to
            // the script's non-message, non-local symbols in declaration order. A
            // `-1` leaves a thing or sector unbound.
            let n = nth_f32(line, 2).unwrap_or(0.0) as usize;
            i += 1;
            for row in read_rows(&lines, &mut i, n) {
                let Some((_, rest)) = row.split_once(':') else { continue };
                let mut it = rest.split_whitespace();
                let Some(script) = it.next() else { continue };
                level.cogs.push(CogInstance {
                    script: script.to_string(),
                    values: it.map(|v| v.to_string()).collect(),
                });
            }
            continue;
        }

        // ---- referenced assets -----------------------------------------------
        else if let Some(kind) = asset_kind(&lower) {
            let n = nth_f32(line, 2).unwrap_or(0.0) as usize;
            i += 1;
            if kind == AssetKind::Sound {
                // Sound rows are bare filenames with no `<index>:` prefix.
                for row in read_names(&lines, &mut i, n) {
                    level.assets.push(Asset { kind, name: row.to_string() });
                }
            } else {
                for row in read_rows(&lines, &mut i, n) {
                    if let Some(name) = indexed_name(row) {
                        level.assets.push(Asset { kind, name });
                    }
                }
            }
            continue;
        }
        // ---- templates -------------------------------------------------------
        else if lower.starts_with("world templates") {
            let n = nth_f32(line, 2).unwrap_or(0.0) as usize;
            i += 1;
            for row in crate::rows::read_block(&lines, &mut i, n) {
                let f = fields(row);
                let Some(name) = f.first() else { continue };
                let mut model3d = None;
                let mut sprite = None;
                let mut params = Vec::new();
                let base = f
                    .get(1)
                    .map(|s| s.to_string())
                    .filter(|s| !s.eq_ignore_ascii_case("none"));
                for kv in f.iter().skip(2) {
                    if let Some((k, v)) = kv.split_once('=') {
                        params.push((k.to_string(), v.to_string()));
                        if k.eq_ignore_ascii_case("model3d") {
                            model3d = Some(v.to_string());
                        } else if k.eq_ignore_ascii_case("sprite") {
                            sprite = Some(v.to_string());
                        }
                    }
                }
                level.templates.push(Template {
                    name: name.to_string(),
                    base,
                    params,
                    model3d,
                    sprite,
                });
            }
            continue;
        }
        // ---- sectors ---------------------------------------------------------
        else if lower.starts_with("sector") {
            level.sectors.push(Sector::default());
        } else if lower.starts_with("flags") {
            if let Some(s) = level.sectors.last_mut() {
                s.flags = u32::from_str_radix(
                    fields(line).get(1).unwrap_or(&"0").trim_start_matches("0x"),
                    16,
                )
                .unwrap_or(0);
            }
        } else if lower.starts_with("center") {
            if let Some(s) = level.sectors.last_mut() {
                let f = floats_in(line, 1, 3);
                if f.len() == 3 {
                    s.center = [f[0], f[1], f[2]];
                }
            }
        } else if lower.starts_with("radius") {
            if let Some(s) = level.sectors.last_mut() {
                s.radius = nth_f32(line, 1).unwrap_or(0.0);
            }
        } else if lower.starts_with("colormap") {
            if let Some(s) = level.sectors.last_mut() {
                s.colormap = nth_f32(line, 1).unwrap_or(0.0) as u32;
            }
        } else if lower.starts_with("surfaces") {
            // `SURFACES <first> <count>`: the sector's slice of the world surface list.
            if let Some(s) = level.sectors.last_mut() {
                let f = fields(line);
                let first = f.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let count = f.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
                s.surfaces = (first, count);
            }
        } else if lower.starts_with("sound") {
            // `SOUND <wav> [volume]` inside a sector block. (Section headers start
            // with `Section:`, which the earlier arm matches first.)
            if let Some(s) = level.sectors.last_mut() {
                let f = fields(line);
                if let Some(wav) = f.get(1) {
                    let volume = f.get(2).and_then(|v| v.parse().ok()).unwrap_or(1.0);
                    s.sound = Some((wav.to_string(), volume));
                }
            }
        }

        i += 1;
    }

    level
}


/// Map a `World <list> <n>` header to the asset kind it enumerates.
fn asset_kind(lower: &str) -> Option<AssetKind> {
    // The geometry and sector headers (`world vertices`, `world surfaces`,
    // `world materials`, `world sectors`) must not match here.
    let rest = lower.strip_prefix("world ")?;
    Some(match rest.split_whitespace().next()? {
        "models" => AssetKind::Model,
        "sprites" => AssetKind::Sprite,
        "keyframes" => AssetKind::Keyframe,
        "puppets" => AssetKind::Puppet,
        "aiclasses" => AssetKind::AiClass,
        "scripts" | "cogs" => AssetKind::Cog,
        "sounds" => AssetKind::Sound,
        _ => return None,
    })
}

/// `n`-th whitespace field parsed as f32 (`n == 0` is the `idx:` token).
fn nth_f32(line: &str, n: usize) -> Option<f32> {
    fields(line).get(n)?.parse().ok()
}

/// `<idx>:\t<name>\t...` -> `name`
fn indexed_name(line: &str) -> Option<String> {
    let f = fields(line);
    let name = f.get(1)?;
    if *name == "mat:" {
        return f.get(2).map(|s| s.to_string());
    }
    Some(name.trim().to_string())
}

/// Surface record:
/// `num: mat: surfflags: faceflags: geo: light: tex: adjoin: extralight:
///  nverts: <vertex,texvert>...  <intensity>...`
fn parse_surface(line: &str) -> Option<Surface> {
    let f = fields(line);
    if f.len() < 11 {
        return None;
    }
    let material: i32 = f[1].parse().ok()?;
    let surfflags = u32::from_str_radix(f[2].trim_start_matches("0x"), 16).unwrap_or(0);
    let geo: i32 = f[4].parse().ok()?;
    let adjoin: i32 = f[7].parse().ok()?;
    let nverts: usize = f[9].parse().ok()?;
    let start = 10;
    if start + nverts * 2 > f.len() {
        return None;
    }

    let mut verts = Vec::with_capacity(nverts);
    for k in 0..nverts {
        let pair = f[start + k];
        let (v, t) = pair.split_once(',').unwrap_or((pair, "-1"));
        verts.push((v.parse().ok()?, t.parse().unwrap_or(-1)));
    }

    let intensities = (0..nverts)
        .filter_map(|k| f.get(start + nverts + k).and_then(|s| s.parse().ok()))
        .collect();

    Some(Surface { material, surfflags, geo, adjoin, verts, intensities })
}

/// `num: template name X Y Z Pitch Yaw Roll Sector [extra...]`
fn parse_thing(line: &str) -> Option<Thing> {
    let f = fields(line);
    if f.len() < 10 {
        return None;
    }
    // Per-thing state follows the sector column as `key=value` items; `frame=` may
    // repeat, one entry per keyframe.
    let mut flags = 0;
    let mut frames = Vec::new();
    let mut vel = [0.0; 3];
    for word in f[9..].iter() {
        if let Some(v) = word.strip_prefix("thingflags=0x") {
            flags = u32::from_str_radix(v, 16).unwrap_or(0);
        } else if word.starts_with("frame=") {
            if let Some(fr) = Thing::parse_frame(word) {
                frames.push(fr);
            }
        } else if let Some(v) = word.strip_prefix("vel=(") {
            let nums: Vec<f32> =
                v.trim_end_matches(')').split('/').filter_map(|n| n.trim().parse().ok()).collect();
            if nums.len() == 3 {
                vel = [nums[0], nums[1], nums[2]];
            }
        }
    }
    Some(Thing {
        template: f[1].to_string(),
        pos: [f[3].parse().ok()?, f[4].parse().ok()?, f[5].parse().ok()?],
        pitch: f[6].parse().unwrap_or(0.0),
        yaw: f[7].parse().unwrap_or(0.0),
        roll: f[8].parse().unwrap_or(0.0),
        sector: f[9].parse().unwrap_or(-1),
        flags,
        frames,
        vel,
    })
}