//! Sector collision: the level's own convex cells, used to stop movement.
//!
//! A `.jkl` sector is a convex hull: its `SURFACES first count` range lists the
//! polygons that bound it, and a surface whose `adjoin` is `>= 0` is a *portal* into
//! the neighbouring sector rather than a wall. That is exactly the rule a walker needs:
//! collide with solid surfaces (`adjoin < 0`), walk through openings.
//!
//! Surface records carry no plane normal, and the renderer deliberately ignores winding
//! (the data is not consistent), so the normal is derived: take it from the face's first
//! two edges, then orient it by **majority vote over the sector's own vertices** — for a
//! convex hull almost every vertex of the sector lies behind each of its faces. The
//! face's own vertices sit on the plane and do not vote, so this needs no assumed
//! interior point. (Using the average of the sector's vertices as an interior point
//! inverted planes in sectors whose surface set is not convex, and a flipped floor then
//! pins a walker in place.)
//!
//! The solver is the classic "push out of any plane you are inside": iterate a few
//! times, move the point out along each plane it penetrates. For a convex cell that
//! both stops the walker at walls and lets it slide along them.

/// One solid face of a sector, as an outward plane.
#[derive(Clone, Copy)]
pub struct Plane {
    pub normal: [f32; 3],
    /// Any point on the plane.
    pub point: [f32; 3],
    /// The `Level::surfaces` entry this plane came from, so the walker can read its
    /// surface flags (water, metal, earth) back out. `usize::MAX` when a test builds one.
    pub surface: usize,
}

/// A placed thing the walker collides with.
///
/// `half` is `Some` only for a `collide=3` thing with a loaded model: the model's bounds give the
/// leaves a thickness the template `size=` does not. Otherwise the engine's movement sphere applies,
/// whose radius the caller already tightens to the model's plan footprint for furniture.
#[derive(Clone, Copy, Debug)]
pub struct ThingSolid {
    /// Collision centre in plan, and the thing's own z for the span's origin.
    pub centre: [f32; 3],
    pub radius: f32,
    pub z_min: f32,
    pub z_max: f32,
    pub sector: usize,
    pub half: Option<[f32; 2]>,
    /// Degrees, the engine's `rdMatrix_Build34` yaw.
    pub yaw: f32,
}

impl ThingSolid {
    /// World point to the thing's own plan frame.
    fn local(&self, p: [f32; 2]) -> [f32; 2] {
        let (sin, cos) = self.yaw.to_radians().sin_cos();
        let (x, y) = (p[0] - self.centre[0], p[1] - self.centre[1]);
        [x * cos + y * sin, -x * sin + y * cos]
    }

    /// World direction to the thing's own plan frame.
    fn local_dir(&self, v: [f32; 2]) -> [f32; 2] {
        let (sin, cos) = self.yaw.to_radians().sin_cos();
        [v[0] * cos + v[1] * sin, -v[0] * sin + v[1] * cos]
    }

    /// A direction in the thing's plan frame to world.
    fn to_world(&self, v: [f32; 2]) -> [f32; 2] {
        let (sin, cos) = self.yaw.to_radians().sin_cos();
        [v[0] * cos - v[1] * sin, v[0] * sin + v[1] * cos]
    }

    /// Sweep a walking sphere against this thing. `None` for a sphere is the engine's cylinder;
    /// `Some` is the model's oriented plan box extruded over the model's z span.
    fn sweep_plan(
        &self,
        at: [f32; 3],
        dir: [f32; 3],
        left: f32,
        radius: f32,
    ) -> Option<(f32, [f32; 3])> {
        let Some(half) = self.half else {
            let span = self.radius + radius;
            let (dx, dy) = (at[0] - self.centre[0], at[1] - self.centre[1]);
            let b = 2.0 * (dir[0] * dx + dir[1] * dy);
            let c = dx * dx + dy * dy - span * span;
            // Already overlapping. A body walking *out* -- the escape direction -- is left free;
            // one walking in is stopped at distance 0 instead of tunnelling through.
            if c <= 0.0 {
                let out_len = (dx * dx + dy * dy).sqrt().max(1e-6);
                let out = [dx / out_len, dy / out_len, 0.0];
                if dir[0] * out[0] + dir[1] * out[1] < 0.0 {
                    return Some((0.0, out));
                }
                return None;
            }
            let disc = b * b - 4.0 * c;
            if disc < 0.0 {
                return None;
            }
            let t = (-b - disc.sqrt()) / 2.0;
            if !(0.0..=left).contains(&t) {
                return None;
            }
            let (hx, hy) = (dx + dir[0] * t, dy + dir[1] * t);
            let len = (hx * hx + hy * hy).sqrt().max(1e-6);
            return Some((t, [hx / len, hy / len, 0.0]));
        };
        let local = self.local([at[0], at[1]]);
        let dir_local = self.local_dir([dir[0], dir[1]]);
        let ext = [half[0] + radius, half[1] + radius];
        if local[0].abs() <= ext[0] && local[1].abs() <= ext[1] {
            // Starting inside the inflated box. A body that has just stopped against a face sits
            // within float dust of it and must stay stopped; one whose recorded pose starts deeper
            // (the elevator spawn overlaps two leaves by 0.014) has to be free to walk out, or the
            // sweep pins it at the spawn for the whole leg. Deep overlaps are resolved by the push
            // pass after the move.
            let (px, py) = (ext[0] - local[0].abs(), ext[1] - local[1].abs());
            let normal = if px <= py {
                [if local[0] >= 0.0 { 1.0 } else { -1.0 }, 0.0]
            } else {
                [0.0, if local[1] >= 0.0 { 1.0 } else { -1.0 }]
            };
            if px.min(py) < 1e-3
                && dir_local[0] * normal[0] + dir_local[1] * normal[1] < 0.0
            {
                let n = self.to_world(normal);
                return Some((0.0, [n[0], n[1], 0.0]));
            }
            return None;
        }
        let mut near = 0.0f32;
        let mut far = left;
        let mut normal = [0.0f32; 2];
        for axis in 0..2 {
            let d = dir_local[axis];
            let l = local[axis];
            if d.abs() < 1e-8 {
                if l.abs() > ext[axis] {
                    return None;
                }
                continue;
            }
            let t1 = (-ext[axis] - l) / d;
            let t2 = (ext[axis] - l) / d;
            let (enter, exit, sign) = if t1 <= t2 {
                (t1, t2, -1.0)
            } else {
                (t2, t1, 1.0)
            };
            if enter > near {
                near = enter;
                normal = [0.0; 2];
                normal[axis] = sign;
            }
            far = far.min(exit);
            if near > far {
                return None;
            }
        }
        if near > left || normal == [0.0; 2] {
            return None;
        }
        let n = self.to_world(normal);
        Some((near, [n[0], n[1], 0.0]))
    }
}

#[derive(Clone, Default)]
pub struct LevelGeometry {
    /// Per sector, the planes of its solid surfaces.
    pub sectors: Vec<Vec<Plane>>,
    /// Per sector, only the planes that **bound** the cell: every vertex the sector claims is behind
    /// them. `sectors` carries every claimed surface's plane, because a wall patch inside the room
    /// still has to stop a body; only this list may answer "is the body inside", which is what
    /// `sithIntersect_IsSphereInSector` (JK.EXE `0x507f30`) asks.
    pub hull: Vec<Vec<Plane>>,
    /// Per sector, its `HASCOLLIDEBOX` bounds (`SITH_SECTOR_HASCOLLIDEBOX`, sector flag `0x1000`),
    /// from the file's `COLLIDEBOX` row (not the enclosing `BOUNDBOX`). `IsSphereInSector`
    /// answers "inside" immediately when the sphere is within this box, without looking at a
    /// single surface — which is why the engine's containment check almost never fires, and
    /// why a port that skips the box and tests every plane sees bodies "outside" their sector.
    pub boxes: Vec<Option<([f32; 3], [f32; 3])>>,
    /// Per sector, the openings in its hull: a portal surface names the sector on the
    /// other side in its `adjoin` field, which is what makes a doorway a doorway.
    pub portals: Vec<Vec<Portal>>,
    /// Per sector, a point inside its hull and the radius that encloses that hull from
    /// there. Derived from the sector's own vertices, so it is tighter than the file's
    /// bounding radius.
    pub spheres: Vec<([f32; 3], f32)>,
    /// The bodies a placed *model* makes solid: `_structure` and `_walkstruct` things, whose
    /// `collide=` the template chain carries. Retail stops Kyle at the cantina's columns and tables;
    /// without these he walks through both, which is what a player notices first.
    ///
    /// `sithIntersect_CheckSphereThingIntersection` (`0x5080d0`) uses the model's mesh only when
    /// either thing has `collide=3` (`sithThing_ParseThingArg` case 2, `+0x28`); every other
    /// collidable thing is a sphere from `size=`. A door leaf is `collide=3`, so its 0.025-thick
    /// box is the contact — a `size=` cylinder stops the body 0.185 early. The sector is part of the
    /// record because `sithCollision_SearchForThingCollisions` walks the **moving body's** sector
    /// thing list: a thing standing in another room cannot touch it.
    pub things: std::sync::Arc<std::sync::RwLock<Vec<ThingSolid>>>,
    /// Every downward-facing solid surface with its polygon: the floors a body can stand on.
    pub floors: Vec<Face>,
    /// Every vertical solid surface with its polygon: the walls. A wall only pushes while the
    /// body's own span overlaps the polygon's, which is what makes a low ledge steppable and a
    /// high wall solid.
    pub walls: Vec<Face>,
    /// Surface id → index into `walls`, so a sector's plane can be matched to its polygon.
    pub wall_of: std::collections::HashMap<usize, usize>,
    /// The same for `floors`: the sweep needs a floor's polygon, not just its plane.
    pub floor_of: std::collections::HashMap<usize, usize>,
    /// **Every** non-adjoined face that has a polygon, whatever its orientation, and the surface → index
    /// map for it. The sweep needs this: a walkway slab seen from above is upward-facing, so it is in
    /// neither `floors` (downward) nor `walls` (vertical) — and sector 218 on level 1 is exactly that,
    /// which is why a downward sweep from its floor plane found nothing and the body fell.
    pub faces: Vec<Face>,
    pub face_of: std::collections::HashMap<usize, usize>,
    /// The surface id -> its owning sector. The floor probe needs this to stay inside the body's room
    /// graph; it is not a draw ownership claim (several sectors can share a surface).
    /// A portal that owns no solid polygon (level 1's ceiling doorway 3300/3301 is an
    /// adjoin on both sides) still needs containment geometry for floor reachability.
    pub portal_faces: Vec<(usize, Face)>,
    pub surface_owner: Vec<usize>,
    /// `World adjoins` flags by link id. Fixtures that do not provide them default to passable.
    pub adjoin_flags: Vec<u16>,
    /// Every sector that claims each surface, derived once with the collision build. Drawing needs
    /// all claimants; `surface_owner` stays the first one used by collision reachability.
    pub surface_claims: Vec<Vec<usize>>,
    /// Surface -> the last sector that lists it, the runtime's script lookup.
    pub surface_sectors: Vec<usize>,
    /// Adjoin row -> the surface that carries it.
    pub adjoin_surface: Vec<usize>,
    /// Sector-to-sector links with the adjoin row's attenuation, derived once from the mirrors.
    pub sector_adjoins: Vec<Vec<(i32, f32)>>,
}

/// The collision-facing name for the one geometry record.
pub type Solids = LevelGeometry;

impl LevelGeometry {
    /// Replace the dynamic solid things. Doors and lifts move, so this is the one mutable slice of
    /// the record; it lives behind a lock so every owner shares the update without cloning geometry.
    /// Callers must not hold a read guard from [`Self::things`] here, or the write would wait on itself.
    pub fn set_things(&self, things: Vec<ThingSolid>) {
        *self
            .things
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = things;
    }

    pub fn things(&self) -> std::sync::RwLockReadGuard<'_, Vec<ThingSolid>> {
        self.things
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// A surface with its polygon, for the queries that need coverage rather than a plane.
///
/// A sector's planes are infinite, which is why a wall blocks at every height and why a sector
/// that merely *shares an edge* with the floor cannot be told from one that is above it. The
/// polygon is the only thing that answers "is the body over this face".
#[derive(Clone)]
pub struct Face {
    pub normal: [f32; 3],
    pub point: [f32; 3],
    pub verts: Vec<[f32; 3]>,
    pub surface: usize,
}

/// An opening in a sector's hull, and the sector behind it.
#[derive(Clone)]
pub struct Portal {
    pub adjoin: usize,
    /// The `World adjoins` row that governs crossing *this* way, which is where the flag word
    /// lives. A mirrored link uses the mirror's row, as the engine's per-side
    /// `SithSurfaceAdjoin` does.
    pub link: usize,
    pub verts: Vec<[f32; 3]>,
    pub normal: [f32; 3],
    pub point: [f32; 3],
}

// The bits the scripts switch, as the engine names them (`sithSurface.h`, `types_enums.h`).
/// `SithSurfaceAdjoin::flags` 0x1: draw (and cull) through this opening.
pub const ADJOIN_VISIBLE: u16 = 0x1;
/// 0x2: the body may pass. `ClearAdjoinFlags(x, 2)` is a force field going up.
pub const ADJOIN_MOVE: u16 = 0x2;
/// 0x10: AI-only. The engine's move path (`RAYCAST_200`, which `MoveThing` always sets) treats
/// an opening with this bit as solid, which is what the shipped "block rear-door access" does.
pub const ADJOIN_AI_ONLY: u16 = 0x10;
/// `SithSectorFlag` 0x80: `SetSectorAdjoins(sector, 0)` hides every opening *of* that sector —
/// how every shipped door script closes its own doorway.
pub const SECTOR_ADJOINS_OFF: u32 = 0x80;
/// `SithSectorFlag` 0x2: the sector is a water volume. `sithPhysics.c:43` halves movement inside
/// one, which is what `SetSectorFlags(sector, 2)` sets up.
pub const SECTOR_UNDERWATER: u32 = 0x2;
/// `SITH_SECTOR_FALLDEATH` 0x40: falling faster than 3 u/s in one of these kills the player
/// (`sithPlayer.c:255`, `decomp/004c89d0_sithPlayer_Update.c:82`). The engine's own answer to
/// falling out of the world.
pub const SECTOR_FALLDEATH: u32 = 0x40;

/// What the level's scripts have switched: each sector's flag word and each adjoin row's.
///
/// Borrowed rather than copied, so the runtime stays the one place the state lives and the
/// solver reads whatever the scripts set last frame. Empty slices mean "nothing switched",
/// which is what a caller with no runtime (`--walk`, a spawn probe over bare geometry) wants.
#[derive(Clone, Copy, Default)]
pub struct Gates<'a> {
    pub sectors: &'a [u32],
    pub adjoins: &'a [u16],
}

impl Gates<'_> {
    /// Can a body move from `from` into `to` through the opening `link`?
    ///
    /// `sithCollision_SearchForSurfaceCollisions` tests the adjoin's own bits, and a sector with
    /// `SITH_SECTOR_ADJOINSOFF` has hidden all of its openings. The engine lets a body step *into*
    /// such a sector — the door leaf is a thing, and things collide; this port has no thing-on-thing
    /// collision, so a shut sector is treated as impassable from both sides. Otherwise a closed
    /// door's sector can be entered and leaves a body with no way out.
    ///
    /// ponytail: that missing collision is the ceiling. With it, only `from` would need the test and
    /// the door leaf would stop the body inside instead.
    pub fn passable(&self, from: usize, to: usize, link: usize) -> bool {
        let shut = |s: usize| self.sectors.get(s).is_some_and(|f| f & SECTOR_ADJOINS_OFF != 0);
        let flags = self.adjoins.get(link).copied().unwrap_or(ADJOIN_MOVE);
        !shut(from) && !shut(to) && flags & ADJOIN_MOVE != 0 && flags & ADJOIN_AI_ONLY == 0
    }
    /// Does the cull walk through the opening `link`?
    pub fn visible(&self, link: usize) -> bool {
        self.adjoins.get(link).copied().unwrap_or(ADJOIN_VISIBLE) & ADJOIN_VISIBLE != 0
    }
}

/// Does this polygon span a plane at all? Used by `--collide` to prove that no solid surface with
/// a real polygon fell out of the collision set.
pub fn has_plane(pts: &[[f32; 3]]) -> bool {
    polygon_normal(pts).is_some()
}

/// Whether `pos` is over the polygon, counting a point within `radius` of an edge as over it.
///
/// Winding is not trusted (the `.jkl` faces do not agree on it), so the test only asks that the
/// point be on the same side of every edge, whichever side that is.
fn over_face(face: &Face, pos: [f32; 3], radius: f32) -> bool {
    let n = face.verts.len();
    if n < 3 {
        return false;
    }
    let mut sign = 0.0f32;
    for i in 0..n {
        let a = face.verts[i];
        let b = face.verts[(i + 1) % n];
        let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
        let len = (ex * ex + ey * ey).sqrt();
        if len < 1e-6 {
            continue;
        }
        let dist = (ex * (pos[1] - a[1]) - ey * (pos[0] - a[0])) / len;
        if dist.abs() <= radius {
            continue;
        }
        if sign == 0.0 {
            sign = dist.signum();
        } else if dist.signum() != sign {
            return false;
        }
    }
    true
}

impl Face {
    /// The polygon's lowest and highest point.
    pub fn min_z(&self) -> Option<f32> {
        self.verts.iter().map(|v| v[2]).reduce(f32::min)
    }
    fn max_z(&self) -> Option<f32> {
        self.verts.iter().map(|v| v[2]).reduce(f32::max)
    }
}

/// Whether a wall face stops a body: its vertical span must overlap the face's, and the body must be
/// beside the face's outline in plan. Anything else passes — that is what makes a low ledge walkable
/// once the feet clear it, and what keeps a wall from pushing a body past the end of its polygon.
///
/// This is the whole lateral rule, in one place, because both halves were wrong at different times:
/// the span test fixed "the wall blocks at every height", and the outline test fixes "the infinite
/// plane still pushes beyond the wall".
pub fn face_blocks(face: &Face, feet: f32, head: f32, pos: [f32; 3], tolerance: f32) -> bool {
    let (Some(low), Some(high)) = (face.min_z(), face.max_z()) else { return false };
    if feet >= high - 0.01 || head <= low + 0.01 {
        return false;
    }
    beside(face, pos, tolerance)
}

/// Whether `pos` is next to the face **in plan**: inside its horizontal projection, or within
/// `tolerance` of it. A wall is vertical, so its projection is the line the body has to be on to
/// be stopped by it — the same "is the body over this face" question `support` asks of floors.
fn beside(face: &Face, pos: [f32; 3], tolerance: f32) -> bool {
    let n = face.verts.len();
    if n < 3 {
        return false;
    }
    let mut sign = 0.0f32;
    let mut outside = false;
    let mut nearest = f32::INFINITY;
    for i in 0..n {
        let a = face.verts[i];
        let b = face.verts[(i + 1) % n];
        let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
        let len = (ex * ex + ey * ey).sqrt();
        // An edge that is vertical in world space collapses in plan: it says nothing about
        // which side the body is on, but its distance is the distance to the face's line.
        if len < 1e-6 {
            continue;
        }
        let dist = (ex * (pos[1] - a[1]) - ey * (pos[0] - a[0])) / len;
        if sign == 0.0 {
            sign = dist.signum();
        } else if dist.signum() != sign {
            outside = true;
        }
        // Distance to the edge as a **segment**, clamped to its ends. Using the distance to the
        // infinite line let a body past the end of a wall still be pushed by it, which is the
        // lateral gap this function used to have.
        let (px, py) = (pos[0] - a[0], pos[1] - a[1]);
        let along = ((px * ex + py * ey) / (len * len)).clamp(0.0, 1.0);
        let (cx, cy) = (px - ex * along, py - ey * along);
        nearest = nearest.min((cx * cx + cy * cy).sqrt());
    }
    // Inside the projection, or within a body radius of the face's outline.
    !outside || nearest <= tolerance
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = dot(v, v).sqrt();
    (len > 1e-6).then(|| [v[0] / len, v[1] / len, v[2] / len])
}

/// A polygon's plane normal, from the widest-spanning triple of its vertices.
///
/// The first three vertices are not enough: shipped `.jkl` faces routinely *start* with
/// collinear points, and `normalize(cross(p1-p0, p2-p0))` is then `None` — which used to drop
/// the whole surface from collision. Level 1's `01narshadda` surface 3510 is exactly that: the
/// 8-vertex floor slab at `z = -1.325` in sector 361, whose first three vertices are on one
/// line. Losing it left a hole in the street that the body fell through, and the same rule was
/// dropping walls, so bodies could walk into them.
fn polygon_normal(pts: &[[f32; 3]]) -> Option<[f32; 3]> {
    let a = *pts.first()?;
    let mut best: Option<[f32; 3]> = None;
    let mut best_len = 1e-6f32;
    // ponytail: every triple, so O(n^2) per polygon, at level load only (largest face here is
    // 17 vertices). A convex hull or a fan would be O(n), and nothing needs it.
    for i in 1..pts.len() {
        for j in (i + 1)..pts.len() {
            let c = cross(sub(pts[i], a), sub(pts[j], a));
            let len = dot(c, c).sqrt();
            if len > best_len {
                best_len = len;
                best = normalize(c);
            }
        }
    }
    best
}

/// Which sector owns each surface: `sector.surfaces` is a `(first, count)` range into the
/// surface table, so this is a flat expansion of it. `usize::MAX` for a surface no sector
/// claims.
fn surface_sectors(level: &crate::jkl::Level) -> Vec<usize> {
    let mut surface_sector: Vec<usize> = vec![usize::MAX; level.surfaces.len()];
    for (i, sector) in level.sectors.iter().enumerate() {
        let (first, count) = sector.surfaces;
        for s in first..first + count {
            if let Some(slot) = surface_sector.get_mut(s) {
                *slot = i;
            }
        }
    }
    surface_sector
}

/// Every sector that claims each surface.
///
/// More than one sector can list the same surface: level 1's sector ranges overlap, so a
/// floor can be claimed by the room *and* by a thin trigger volume inside it. Taking only
/// the last claimant (as a single `surface_sectors` slot must) puts that floor in the
/// trigger's batch, and hiding the trigger then hides the floor. Drawing needs all claims.
fn surface_claims(level: &crate::jkl::Level) -> Vec<Vec<usize>> {
    let mut claims: Vec<Vec<usize>> = vec![Vec::new(); level.surfaces.len()];
    for (i, sector) in level.sectors.iter().enumerate() {
        let (first, count) = sector.surfaces;
        for s in first..first + count {
            if let Some(slot) = claims.get_mut(s)
                && !slot.contains(&i)
            {
                slot.push(i);
            }
        }
    }
    claims
}

/// Build the solid-plane set for every sector.
pub fn build(level: &crate::jkl::Level) -> Solids {
    // Which sector each surface belongs to, and which surface carries each adjoin index:
    // both are needed to turn "this surface is adjoined to #123" into "the room on the
    // other side is sector N".
    let surface_sectors = surface_sectors(level);
    let surface_claims = surface_claims(level);
    let mut adjoin_surface: Vec<usize> = vec![usize::MAX; level.adjoins.len()];
    for (i, surface) in level.surfaces.iter().enumerate() {
        if let Ok(a) = usize::try_from(surface.adjoin) {
            if let Some(slot) = adjoin_surface.get_mut(a) {
                if *slot == usize::MAX {
                    *slot = i;
                }
            }
        }
    }
    // Sector-to-sector links with the attenuation the AI graph uses. The mirror row is the cost, as
    // in `sithAIAwareness`'s `ProcessEvent`; one link per opening.
    let mut sector_adjoins: Vec<Vec<(i32, f32)>> = vec![Vec::new(); level.sectors.len()];
    for (adjoin, row) in level.adjoins.iter().enumerate() {
        let Ok(mirror) = usize::try_from(row.mirror) else { continue };
        let (Some(first), Some(second)) = (
            adjoin_surface.get(adjoin).copied().filter(|s| *s != usize::MAX),
            adjoin_surface.get(mirror).copied().filter(|s| *s != usize::MAX),
        ) else {
            continue;
        };
        if mirror <= adjoin {
            continue;
        }
        let (a, b) = (
            surface_sectors.get(first).copied().unwrap_or(usize::MAX),
            surface_sectors.get(second).copied().unwrap_or(usize::MAX),
        );
        if a == usize::MAX || b == usize::MAX || a == b {
            continue;
        }
        let cost = level.adjoins.get(mirror).map(|a| a.dist).unwrap_or(0.0).max(0.0);
        sector_adjoins[a].push((b as i32, cost));
        sector_adjoins[b].push((a as i32, cost));
    }
    // The sector across an adjoin: its mirror's surface, and that surface's sector.
    let across = |adjoin: i32| -> Option<usize> {
        let a = usize::try_from(adjoin).ok()?;
        let mirror = usize::try_from(level.adjoins.get(a)?.mirror).ok()?;
        let surface = *adjoin_surface.get(mirror)?;
        let sector = *surface_sectors.get(surface)?;
        (sector != usize::MAX).then_some(sector)
    };

    let mut floors: Vec<Face> = Vec::new();
    let mut walls: Vec<Face> = Vec::new();
    let mut faces: Vec<Face> = Vec::new();
    let mut sectors = Vec::with_capacity(level.sectors.len());
    let mut hulls = Vec::with_capacity(level.sectors.len());
    let mut boxes = Vec::with_capacity(level.sectors.len());
    let mut portal_sets = Vec::with_capacity(level.sectors.len());
    let mut spheres = Vec::with_capacity(level.sectors.len());
    for sector in &level.sectors {
        let (first, count) = sector.surfaces;
        let owned: Vec<(usize, &crate::jkl::Surface)> = level
            .surfaces
            .iter()
            .enumerate()
            .skip(first)
            .take(count)
            .collect();

        // Every vertex of the sector, used both to orient each face and to bound it.
        let mut hull: Vec<[f32; 3]> = Vec::new();
        for (_, surface) in &owned {
            for &(v, _) in &surface.verts {
                if let Some(p) = level.vertices.get(v as usize) {
                    hull.push(*p);
                }
            }
        }
        let (mut sum, n) = ([0.0f32; 3], hull.len());
        for p in &hull {
            sum = [sum[0] + p[0], sum[1] + p[1], sum[2] + p[2]];
        }
        let centre = if n > 0 {
            [sum[0] / n as f32, sum[1] / n as f32, sum[2] / n as f32]
        } else {
            sector.center
        };

        let mut planes = Vec::new();
        let mut hull_planes = Vec::new();
        let mut portals = Vec::new();
        for (surface_of, surface) in &owned {
            if surface.verts.len() < 3 {
                continue;
            }
            let pts: Vec<[f32; 3]> = surface
                .verts
                .iter()
                .filter_map(|&(v, _)| level.vertices.get(v as usize).copied())
                .collect();
            if pts.len() < 3 {
                continue;
            }
            // The plane the level **ships**: `sithSurface_ReadSurfacesListText` (JK.EXE `0x4e5c00`)
            // reads a second block after the surface rows — one `idx: nx ny nz` line each — into
            // `face.normal`, and every engine consumer uses that field. The file's normals point
            // *into* the room (surface 3275 on level 1 is the floor slab at `z = -1.300` and ships
            // `(0, 0, 1)`, up), while this port's solver was built on the opposite, *outward*
            // convention, so the authored one is only used here to settle the **sign** — the plane
            // itself still comes from the polygon, which is what the collision code expects, and the
            // vertices are what a degenerate or missing block leaves as the fallback.
            let normal = {
                let Some(mut normal) = polygon_normal(&pts) else {
                    continue;
                };
                if dot(surface.normal, surface.normal) > 0.25 {
                    // Outward is the negation of the file's inward normal.
                    if dot(normal, [-surface.normal[0], -surface.normal[1], -surface.normal[2]]) < 0.0 {
                        normal = [-normal[0], -normal[1], -normal[2]];
                    }
                } else {
                    // No shipped normal: majority vote of the sector's vertices, where this face's
                    // own vertices lie on the plane and do not vote and almost everything else is
                    // behind.
                    let (mut behind, mut front) = (0usize, 0usize);
                    for v in &hull {
                        let d = dot(sub(*v, pts[0]), normal);
                        if d < -1e-4 {
                            behind += 1;
                        } else if d > 1e-4 {
                            front += 1;
                        }
                    }
                    if front > behind {
                        normal = [-normal[0], -normal[1], -normal[2]];
                    }
                }
                normal
            };
            // The hull's own orientation: a plane bounds a cell when the cell's inside is behind it,
            // so the normal is taken outward *from this sector's centre*. The authored normal is
            // right for the surface, but one surface can be claimed by two sectors and only ever be
            // authored for one of them — the sector that claims it must still see it as its own wall.
            // `planes` keeps the authored sign (the sweep orients each face per query anyway).
            let own = if dot(sub(centre, pts[0]), normal) > 0.0 {
                [-normal[0], -normal[1], -normal[2]]
            } else {
                normal
            };
            // An adjoined surface is an opening: the walker goes through it into the
            // sector its *mirror* adjoin belongs to, so it is remembered as a portal
            // rather than a wall.
            match across(surface.adjoin) {
                Some(adjoin) => {
                    let point = pts[0];
                    let link = usize::try_from(surface.adjoin).unwrap_or(usize::MAX);
                    // `sithIntersect_IsSphereInSector` (`decomp/00507f30:29`) counts a surface into
                    // the hull when it has `HAS_COLLISION` **or** is an adjoin whose link allows
                    // movement (`pAdjoin->flags & 2`) — a body in a doorway is inside both sectors.
                    if level.adjoins.get(link).is_some_and(|a| a.flags & 0x2 != 0) {
                        hull_planes.push(Plane { normal: own, point, surface: *surface_of });
                    }
                    // The row that governs this side's crossing: the one this surface names.
                    portals.push(Portal { adjoin, link, verts: pts, normal, point });
                }
                // The engine's own test, `sithCollision_SearchForSurfaceCollisions`:
                //     if ( (v12->flags & SITH_SURFACE_HAS_COLLISION) == 0 && !v15 ) continue;
                // A surface is solid only when its flags carry `SITH_SURFACE_HAS_COLLISION` (0x4), unless
                // it is an adjoin, which is a doorway. Trigger brushes and marker surfaces do not, and
                // colliding with them puts invisible walls through the level — and walls 0.08 units from a
                // spawn point that the level placed in a trigger volume.
                None if surface.surfflags & 0x4 == 0 => {}
                None => {
                    // Downward-facing and solid: a floor a body can stand on, kept with its
                    // polygon so "over this face" can be asked later.
                    if normal[2] < -0.7 {
                        floors.push(Face { normal, point: pts[0], verts: pts.clone(), surface: *surface_of });
                    } else if normal[2].abs() < 0.7 {
                        walls.push(Face { normal, point: pts[0], verts: pts.clone(), surface: *surface_of });
                    }
                    // The sweep needs this one whatever its orientation: an upward-facing slab is
                    // still what a body stands on, and a downward sweep reaches it from above.
                    faces.push(Face { normal, point: pts[0], verts: pts.clone(), surface: *surface_of });
                    hull_planes.push(Plane { normal: own, point: pts[0], surface: *surface_of });
                    planes.push(Plane { normal, point: pts[0], surface: *surface_of })
                }
            }
        }
        let mut radius = 0.0f32;
        for p in &hull {
            let d = sub(*p, centre);
            radius = radius.max(dot(d, d).sqrt());
        }
        // The box `IsSphereInSector` short-circuits on: the level's own `BOUNDBOX` (the engine's
        // `collidebox_onecorner/othercorner`), used only when the sector claims `HASCOLLIDEBOX`
        // (0x1000), which is what the engine tests first.
        let box_of = if sector.flags & 0x1000 != 0 {
            sector.boundbox
        } else {
            None
        };
        sectors.push(planes);
        hulls.push(hull_planes);
        boxes.push(box_of);
        portal_sets.push(portals);
        spheres.push((centre, radius.max(0.1)));
    }
    // The portal graph is **symmetric**. A link is discovered from whichever side owns the surface,
    // and a shared surface is not always claimed by both sectors: level 1's bar rooms pointed into
    // the hall the player stands in and not back out of it, so the walk from sector 218 reached 97
    // rooms and none of the 21 in the frustum. Mirroring every link costs one pass and makes the
    // walk independent of the claim table.
    for from in 0..portal_sets.len() {
        let outgoing: Vec<Portal> = portal_sets[from].clone();
        for portal in outgoing {
            let to = portal.adjoin;
            if to >= portal_sets.len() || to == from {
                continue;
            }
            if portal_sets[to].iter().any(|p| p.adjoin == from) {
                continue;
            }
            portal_sets[to].push(Portal {
                adjoin: from,
                // The far side's own row, which is the mirror of this one.
                link: level
                    .adjoins
                    .get(portal.link)
                    .and_then(|a| usize::try_from(a.mirror).ok())
                    .unwrap_or(portal.link),
                verts: portal.verts.clone(),
                normal: [-portal.normal[0], -portal.normal[1], -portal.normal[2]],
                point: portal.point,
            });
        }
    }
    if std::env::var_os("BK_PORTAL_TRACE").is_some() {
        let empty: Vec<usize> = (0..portal_sets.len())
            .filter(|s| portal_sets[*s].is_empty())
            .collect();
        println!(
            "portals: {} sectors, {} with no link at all, first empty {:?}",
            portal_sets.len(),
            empty.len(),
            empty.iter().take(8).collect::<Vec<_>>()
        );
    }
    let wall_of: std::collections::HashMap<usize, usize> =
        walls.iter().enumerate().map(|(i, face)| (face.surface, i)).collect();
    let floor_of: std::collections::HashMap<usize, usize> =
        floors.iter().enumerate().map(|(i, face)| (face.surface, i)).collect();
    let face_of: std::collections::HashMap<usize, usize> =
        faces.iter().enumerate().map(|(i, face)| (face.surface, i)).collect();
    // Which room a polygon belongs to. A level can list one surface from several sectors (a trigger
    // volume shares the room's floor), so the first claimant is enough for reachability; it is not a
    // statement about who draws it. The old floor probe repeated this scan for every candidate face.
    let mut surface_owner = vec![usize::MAX; level.surfaces.len()];
    for (room, planes) in sectors.iter().enumerate() {
        for plane in planes {
            if plane.surface != usize::MAX && surface_owner[plane.surface] == usize::MAX {
                surface_owner[plane.surface] = room;
            }
        }
    }
    // A portal is an opening, not a solid face, so `faces` deliberately excludes it. Keep one
    // polygon per opening separately: a downward sweep may use it to decide which room actually
    // contains the probe, without turning the opening into a collision surface.
    let portal_faces = portal_sets
        .iter()
        .enumerate()
        .flat_map(|(room, portals)| {
            portals.iter().map(move |portal| {
                (
                    room,
                    Face {
                        normal: portal.normal,
                        point: portal.point,
                        verts: portal.verts.clone(),
                        surface: usize::MAX,
                    },
                )
            })
        })
        .collect();
    // The solid things are filled by the caller, which has the models: see `solid_things` in `main`.
    Solids {
        sectors,
        hull: hulls,
        boxes,
        portals: portal_sets,
        spheres,
        floors,
        walls,
        wall_of,
        floor_of,
        faces,
        face_of,
        portal_faces,
        surface_owner,
        adjoin_flags: level.adjoins.iter().map(|a| a.flags).collect(),
        surface_claims,
        surface_sectors,
        adjoin_surface,
        sector_adjoins,
        things: std::sync::Arc::new(std::sync::RwLock::new(Vec::new())),
    }
}

/// One swept-sphere contact: where a body of `radius` first touches, in the direction it moved.
#[derive(Clone, Copy, Debug)]
pub struct Sweep {
    /// Travel distance along the sweep direction at which the sphere touches.
    pub distance: f32,
    /// The surface it touched, as `Level::surfaces` indexes them.
    pub surface: usize,
    /// The face normal, pointing out of the solid.
    pub normal: [f32; 3],
    /// The sector the contact is in (after crossing any portals along the way).
    pub sector: usize,
    /// Where the sphere's centre is at the contact, in world space. A sweep that crossed a doorway
    /// spends part of its distance there, so this — not `distance` — is what a caller can place a body by.
    pub point: [f32; 3],
    /// The touched face's highest vertex. A step-up may lift the body over a face whose **top edge** is
    /// within the step allowance — a stair riser's top is the next tread — while a wall whose top is
    /// high is not a step, whatever height its contact point happens to sit at. Without this the step
    /// test can only ask about the contact point, which cannot tell a riser from a wall.
    pub top: f32,
}

/// `sithIntersect_CheckSphereHit`: the travel distance at which a sphere first meets a plane.
///
/// A body already closer than its radius (`gap < 0`) is touching, so the answer is `0`. A body moving
/// away from the plane never hits it. This is the engine's routine line for line, including the
/// `gap / approach` distance, because the whole collision behaviour is built on it.
fn sphere_plane(
    start: [f32; 3],
    dir: [f32; 3],
    dist: f32,
    radius: f32,
    normal: [f32; 3],
    point: [f32; 3],
) -> Option<f32> {
    let above = dot(sub(start, point), normal);
    if above < 0.0 {
        return None;
    }
    let gap = above - radius;
    if gap > dist {
        return None;
    }
    if gap < 0.0 {
        return Some(0.0);
    }
    let approach = -dot(dir, normal);
    if approach <= 0.0 || approach * dist < gap {
        return None;
    }
    Some((gap / approach).max(0.0))
}

/// `sithIntersect_TestSphereFaceHit`: is the sphere touching the polygon — inside it, or within
/// `radius` of one of its edges?
///
/// The engine projects to the two axes the normal is not dominant on and walks the edges. Because the
/// `.jkl` winding cannot be trusted, this asks the polygon's own centre which side is inside and
/// then measures how far outside the sphere reaches — the same test with no winding assumption.
fn sphere_face(pos: [f32; 3], radius: f32, face: &Face) -> bool {
    let n = face.verts.len();
    if n < 3 {
        return false;
    }
    let mut centre = [0.0f32; 3];
    for v in &face.verts {
        centre = [centre[0] + v[0], centre[1] + v[1], centre[2] + v[2]];
    }
    let centre = [centre[0] / n as f32, centre[1] / n as f32, centre[2] / n as f32];
    // The two axes to work in: drop the one the normal is largest on.
    let axis = {
        let n = face.normal;
        let (ax, ay, az) = (n[0].abs(), n[1].abs(), n[2].abs());
        if ax >= ay && ax >= az {
            0
        } else if ay >= az {
            1
        } else {
            2
        }
    };
    let flat = |p: [f32; 3]| -> [f32; 2] {
        match axis {
            0 => [p[1], p[2]],
            1 => [p[0], p[2]],
            _ => [p[0], p[1]],
        }
    };
    let p = flat(pos);
    let c = flat(centre);
    let mut outside = 0.0f32;
    for i in 0..n {
        let a = flat(face.verts[i]);
        let b = flat(face.verts[(i + 1) % n]);
        let edge = [b[0] - a[0], b[1] - a[1]];
        let len = (edge[0] * edge[0] + edge[1] * edge[1]).sqrt();
        if len < 1e-6 {
            continue;
        }
        // Signed distance from the edge line, positive on the side the polygon centre is on.
        let side = |q: [f32; 2]| (edge[0] * (q[1] - a[1]) - edge[1] * (q[0] - a[0])) / len;
        let inside_sign = if side(c) < 0.0 { -1.0 } else { 1.0 };
        let gap = -inside_sign * side(p);
        if gap > outside {
            outside = gap;
        }
    }
    outside <= radius + 1e-5
}

impl LevelGeometry {
    /// The engine's collision search: sweep a sphere from `start` along `dir` for `dist`, walking
    /// through adjoins, and report the first solid surface it meets.
    ///
    /// `sithCollision_SearchForSurfaceCollisions` does exactly this — for each surface of the sector it
    /// tests the swept sphere against the polygon (`CheckSphereFaceIntersection`) and, when the surface
    /// is an adjoin the movement is allowed through, it *recurse*s into the sector on the other side
    /// with the remaining distance. The first solid hit stops the walk. This is the same shape with an
    /// explicit visited list, which the engine keeps in `apSearchedSectors`.
    ///
    /// `ponytail:` the depth limit and the "already searched" rule stop a sweep bouncing between two
    /// rooms; a chain of more than 24 adjoins in a straight line would report no hit rather than the
    /// far one, which no shipped level does.
    pub fn sweep(
        &self,
        sector: usize,
        start: [f32; 3],
        dir: [f32; 3],
        dist: f32,
        radius: f32,
        gates: Gates<'_>,
    ) -> Option<Sweep> {
        self.sweep_rooms(sector, start, dir, dist, radius, gates).0
    }

    /// [`Self::sweep`], and the sector the walk ended in.
    ///
    /// The second value is not decoration: a sweep that walks *through* a doorway and hits nothing
    /// returns no contact, and the caller still has to know it is now in the next room. Asking again
    /// afterwards (by polygon or by distance to the plane) cannot answer it — each room's doorway carries
    /// its own outward normal, so both rooms claim the same doorway space and a body sitting in one was
    /// swapped back and forth every frame (17,651 crossings over 34 levels, 97 units walked).
    /// [`Self::sweep`], and the sector and position the walk ended at.
    ///
    /// The extra values are not decoration: a sweep that walks *through* a doorway and hits nothing
    /// returns no contact, and the caller still has to know it is now in the next room — and where. The
    /// position matters as much as the room: the crossing advance is what carries the origin past the far
    /// room's doorway plane, and discarding it left the body a radius short, refused by the containment
    /// rule on every frame (523 refusals in 700).
    pub fn sweep_rooms(
        &self,
        sector: usize,
        start: [f32; 3],
        dir: [f32; 3],
        dist: f32,
        radius: f32,
        gates: Gates<'_>,
    ) -> (Option<Sweep>, usize, [f32; 3]) {
        let mut sector = sector;
        let mut from = start;
        let mut left = dist;
        let mut visited: Vec<usize> = Vec::new();
        // The nearest solid contact found *before* a doorway that was nearer still. If the walk dead-ends
        // through an opening, this is what stops the body — the engine's stack does the same, keeping a
        // wall hit from a room it passed through.
        let mut closest: Option<Sweep> = None;
        for _ in 0..24 {
            if visited.contains(&sector) {
                return (closest, sector, [from[0] + dir[0] * left, from[1] + dir[1] * left, from[2]]);
            }
            visited.push(sector);
            let mut wall: Option<Sweep> = None;
            let mut door: Option<(f32, usize)> = None;

            let Some(planes_here) = self.sectors.get(sector) else { return (closest, sector, [from[0] + dir[0] * left, from[1] + dir[1] * left, from[2]]) };
            for plane in planes_here {
                let face = self
                    .face_of
                    .get(&plane.surface)
                    .and_then(|i| self.faces.get(*i))
                    .or_else(|| self.floor_of.get(&plane.surface).and_then(|i| self.floors.get(*i)))
                    .or_else(|| self.wall_of.get(&plane.surface).and_then(|i| self.walls.get(*i)));
                let Some(face) = face else {
                    continue;
                };
                // This codebase's face normals were built for the walker's own queries, not "out of the
                // solid"; flip toward the sweep so the engine's arithmetic applies.
                let normal = if dot(sub(from, face.point), face.normal) < 0.0 {
                    [-face.normal[0], -face.normal[1], -face.normal[2]]
                } else {
                    face.normal
                };
                // The engine only records a surface it is moving *into*: `SearchForSurfaceCollisions`
                // tests `Dot3(moveNorm, pushVel) < 0` before pushing the hit. Without that, a body flush
                // against a face reports a zero-distance hit for every direction — including the slide
                // along the wall — and the move wedges. This is the guard that makes sliding work.
                // `>= -1e-4`, not `>= 0`: a body resting *flush* against a face has a normal with a few
                // millionths of tangential dust, so a slide along that face reads as `dot = -6e-6` and
                // the face blocks it at distance 0 — the body could never slide out of a corner it was
                // touching (level 4's doorway panel plus the portal beside it). The engine's
                // `Dot3(moveNorm, pushVel) < 0` test carries the same intent; the tolerance keeps
                // floating-point dust from turning "tangent" into "into".
                if dot(dir, normal) >= -1e-4 {
                    continue;
                }
                let Some(t) = sphere_plane(from, dir, left, radius, normal, face.point) else {
                    continue;
                };
                let at = [from[0] + dir[0] * t, from[1] + dir[1] * t, from[2] + dir[2] * t];
                if !sphere_face(at, radius, face) {
                    continue;
                }
                if wall.is_none_or(|w| t < w.distance) {
                    wall = Some(Sweep {
                        distance: t,
                        surface: face.surface,
                        normal,
                        sector,
                        point: at,
                        top: face.verts.iter().map(|v| v[2]).fold(f32::MIN, f32::max),
                    });
                }
            }

            // A shut opening's own sector **is** the door leaf: the level's scripts closed it
            // (`00_door.cog`'s `startup` calls `SectorAdjoins(doorsector, 0)`), and the leaf's faces are
            // what retail stops the body at -- 11gorc's sector 9 leaf between 15 and 14 is the shipped
            // case (retail stops at y 13.19). The walkable passage is often a different sector beside
            // the leaf, so refusing the crossing alone still lets the body through the leaf's space;
            // its faces are tested here as walls, inside the opening they seal.
            for portal in self.portals.get(sector).into_iter().flatten() {
                if gates.passable(sector, portal.adjoin, portal.link) {
                    continue;
                }
                let Some(leaf) = self.sectors.get(portal.adjoin) else { continue };
                let mouth = Face {
                    normal: portal.normal,
                    point: portal.point,
                    verts: portal.verts.clone(),
                    surface: usize::MAX,
                };
                for plane in leaf {
                    let face = self
                        .face_of
                        .get(&plane.surface)
                        .and_then(|i| self.faces.get(*i))
                        .or_else(|| self.floor_of.get(&plane.surface).and_then(|i| self.floors.get(*i)))
                        .or_else(|| self.wall_of.get(&plane.surface).and_then(|i| self.walls.get(*i)));
                    let Some(face) = face else { continue };
                    if let Some(hit) = self.face_contact(face, from, dir, left, radius, sector) {
                        // The leaf's face stands *behind* the opening, so compare it with the mouth
                        // projected onto the opening's own plane -- not the raw 3D point.
                        let off = dot(sub(hit.point, portal.point), portal.normal);
                        let flat = [
                            hit.point[0] - portal.normal[0] * off,
                            hit.point[1] - portal.normal[1] * off,
                            hit.point[2] - portal.normal[2] * off,
                        ];
                        if sphere_face(flat, radius, &mouth) && wall.is_none_or(|w| hit.distance < w.distance) {
                            wall = Some(hit);
                        }
                    }
                }
            }

            // Adjoins are openings, never contacts: the sweep goes through them. A doorway behind the
            // direction of travel, or one leading back into a room already searched, is not taken.
            for portal in self.portals.get(sector).into_iter().flatten() {
                if visited.contains(&portal.adjoin) {
                    continue;
                }
                let face = Face {
                    normal: portal.normal,
                    point: portal.point,
                    verts: portal.verts.clone(),
                    surface: usize::MAX,
                };
                let normal = if dot(sub(from, face.point), face.normal) < 0.0 {
                    [-face.normal[0], -face.normal[1], -face.normal[2]]
                } else {
                    face.normal
                };
                // `>= -1e-4`, not `>= 0`: a body resting *flush* against a face has a normal with a few
                // millionths of tangential dust, so a slide along that face reads as `dot = -6e-6` and
                // the face blocks it at distance 0 — the body could never slide out of a corner it was
                // touching (level 4's doorway panel plus the portal beside it). The engine's
                // `Dot3(moveNorm, pushVel) < 0` test carries the same intent; the tolerance keeps
                // floating-point dust from turning "tangent" into "into".
                if dot(dir, normal) >= -1e-4 {
                    continue;
                }
                let Some(t) = sphere_plane(from, dir, left, radius, normal, face.point) else {
                    continue;
                };
                let at = [from[0] + dir[0] * t, from[1] + dir[1] * t, from[2] + dir[2] * t];
                if !sphere_face(at, radius, &face) {
                    continue;
                }
                // A body *already* at the opening's plane crossed it on the frame it arrived. Counting
                // that as a crossing again makes a body pinned in a doorway swap rooms every frame
                // (13,151 crossings over 34 levels). Only a crossing that advances through the plane
                // counts, which is also the engine's reading: the contact pushes the body on.
                if t <= 0.0 {
                    continue;
                }
                // A closed opening is not an opening: `SearchForSurfaceCollisions` falls through to
                // its plain-surface test when the adjoin's MOVE bit is clear (or the sector's adjoins
                // are hidden), so a force field stops a body exactly like a wall does.
                //
                // An opening whose far side cannot hold the body is a wall too. A door's *slab* is a
                // sector like any other, and a closed one is thinner than the body's radius (level 1's
                // `337` is 0.05 wide against 0.065), so no origin position inside it is legal: crossing
                // in put the body outside its own sector, `MoveThing`'s restore undid it, and the body
                // re-crossed on the next frame — 713 times in 700.
                let advance = self.doorway_advance(from, dir, t, radius, portal.adjoin);
                let lands_inside = gates.passable(sector, portal.adjoin, portal.link) && advance.is_some();
                if std::env::var_os("BK_SECTOR_TRACE").is_some() && t < 0.5 {
                    // *Every* portal evaluated, not just the accepted ones: each room keeps its own copy
                    // of a doorway's plane, and a rejected copy that becomes a wall is otherwise silent.
                    eprintln!(
                        "sector trace: portal into {} from sector {sector} t {t:.6} dir {dir:?} \
                         normal ({:.4},{:.4},{:.4}) point ({:.4},{:.4},{:.4}) advance {advance:?} inside {lands_inside}",
                        portal.adjoin,
                        normal[0], normal[1], normal[2],
                        face.point[0], face.point[1], face.point[2]
                    );
                }
                if lands_inside {
                    if door.is_none_or(|(dt, _)| t < dt) {
                        door = Some((t, portal.adjoin));
                    }
                } else if wall.is_none_or(|w| t < w.distance) {
                    wall = Some(Sweep { distance: t, surface: usize::MAX, normal, sector, point: at, top: f32::MAX });
                }
            }

            match (wall, door) {
                (Some(hit), Some((dt, _))) if hit.distance <= dt => return (Some(hit), sector, from),
                (Some(hit), None) => return (Some(hit), sector, from),
                (Some(hit), Some((dt, adjoin))) => {
                    closest.get_or_insert(hit);
                    // Step to the far side of the opening's plane, not just past the sphere's contact
                    // point. `IsSphereInSector` is an infinite-plane test at the body's *origin*, and the
                    // far room's own copy of the doorway sits on that plane: an origin left a radius short
                    // of it is outside the far room, so `MoveThing`'s restore undoes the crossing and the
                    // body re-crosses for ever (level 1: 524 refused frames in 700, 0.35 units walked).
                    //
                    // The advance is *not* capped by the remaining distance. A crossing is a sector
                    // transition, and the origin has to clear the far room's plane even when the step has
                    // almost run out — measured: with the cap the body came to rest 0.009 past the contact
                    // point, still 0.056 short of the plane, and was refused on every frame. Clearing one
                    // radius is below the collision radius, so it cannot put the body through anything.
                    let advance = self.doorway_advance_logged(from, dir, dt, radius, adjoin).unwrap_or(dt + 1e-4);
                    from = [from[0] + dir[0] * advance, from[1] + dir[1] * advance, from[2] + dir[2] * advance];
                    left = (left - dt).max(0.0);
                    sector = adjoin;
                }
                (None, Some((dt, adjoin))) => {
                    let advance = self.doorway_advance_logged(from, dir, dt, radius, adjoin).unwrap_or(dt + 1e-4);
                    from = [from[0] + dir[0] * advance, from[1] + dir[1] * advance, from[2] + dir[2] * advance];
                    left = (left - dt).max(0.0);
                    sector = adjoin;
                }
                (None, None) => return (closest, sector, [from[0] + dir[0] * left, from[1] + dir[1] * left, from[2]]),
            }
        }
        (closest, sector, [from[0] + dir[0] * left, from[1] + dir[1] * left, from[2]])
    }
    /// One surface's contact for a sphere of `radius` sweeping `dir` over `left` from `from`.
    fn face_contact(
        &self,
        face: &Face,
        from: [f32; 3],
        dir: [f32; 3],
        left: f32,
        radius: f32,
        sector: usize,
    ) -> Option<Sweep> {
        let normal = if dot(sub(from, face.point), face.normal) < 0.0 {
            [-face.normal[0], -face.normal[1], -face.normal[2]]
        } else {
            face.normal
        };
        if dot(dir, normal) >= -1e-4 {
            return None;
        }
        let t = sphere_plane(from, dir, left, radius, normal, face.point)?;
        let at = [from[0] + dir[0] * t, from[1] + dir[1] * t, from[2] + dir[2] * t];
        if !sphere_face(at, radius, face) {
            return None;
        }
        Some(Sweep {
            distance: t,
            surface: face.surface,
            normal,
            sector,
            point: at,
            top: face.verts.iter().map(|v| v[2]).fold(f32::MIN, f32::max),
        })
    }

    /// How far to step a body that is crossing a doorway, from its contact point.
    ///
    /// Two constraints, and they disagree on thin geometry:
    /// * the origin has to end up **past** the doorway's plane, because `IsSphereInSector` is an
    ///   infinite-plane test and a radius short of it is "outside" — that wants `radius`;
    /// * the origin has to end up **inside** the far room, and a door sector can be thinner than the
    ///   body's radius (level 1's `sector 337` is `0.05` wide against a `0.065` radius, so the contact
    ///   point is already past its far face) — that wants no more than half the room's depth.
    ///
    /// So: clear the plane, but never more than halfway into a shallow room where the two would
    /// otherwise contradict. And because each room keeps its **own copy** of the opening's plane —
    /// level 1's copies differ by 3 mm — a couple of longer reaches are tried before the opening is
    /// called unusable.
    ///
    /// `None` means nowhere the body can stand: a closed door's slab, thinner than the body's radius.
    /// The caller treats such an opening as a wall (see `sweep_rooms`).
    fn doorway_advance(
        &self,
        from: [f32; 3],
        dir: [f32; 3],
        contact: f32,
        radius: f32,
        adjoin: usize,
    ) -> Option<f32> {
        // The doorway plane sits one radius beyond the contact point.
        let plane = [
            from[0] + dir[0] * (contact + radius),
            from[1] + dir[1] * (contact + radius),
            from[2] + dir[2] * (contact + radius),
        ];
        // The far room's nearest plane ahead of that point is its far wall (or the other side of a door
        // slab): travelling toward a plane means moving along its outward normal.
        let mut depth = f32::INFINITY;
        for plane_of in self.sectors.get(adjoin).into_iter().flatten() {
            let along = dot(dir, plane_of.normal);
            if along <= 1e-4 {
                continue;
            }
            let to_plane = -dot(sub(plane, plane_of.point), plane_of.normal) / along;
            if to_plane > 1e-4 {
                depth = depth.min(to_plane);
            }
        }
        // A room shallower than the body cannot hold it, however legal its origin's position looks: a
        // closed door's slab is exactly that (level 1's `337` is 0.05 deep against a 0.13 body). Such an
        // opening is a wall — otherwise the body stands *inside the door*.
        //
        // Relaxing this to the engine's rule (adopt the far sector and re-search from the contact) was
        // measured: `--walk all` containment went 27 -> 29 of 34 and crossings 59 -> 78, but `walk18_spawn`
        // wedged — its 0.103-deep doorway 721 let the body 0.023 short of the far plane, where the sweep
        // stops it (0.27 units against retail's 0.88). The engine's crossing sets the sector **at the
        // contact** and re-searches; this path advances the body a radius past the plane, so a slab needs
        // that through-move before the depth test can be relaxed.
        if std::env::var_os("BK_SECTOR_TRACE").is_some() {
            eprintln!(
                "sector trace: doorway {adjoin} depth {depth:.4} (needs {:.4}) radius {radius:.3}",
                2.0 * radius
            );
        }
        if depth < 2.0 * radius {
            return None;
        }
        let base = contact + (radius + 1e-3).min(depth * 0.5);
        for extra in [0.0f32, 0.01, radius] {
            let advance = base + extra;
            let landing = [
                from[0] + dir[0] * advance,
                from[1] + dir[1] * advance,
                from[2] + dir[2] * advance,
            ];
            // The body may stand one **step** higher on the far side. This port's level data tiles the
            // ground with slanted triangles whose floors differ slightly at the seams: level 3's doorway
            // into sector 369 has its floor ~0.02 *above* the body's feet, so the strict test called the
            // opening a wall and the body slid along a seam it should walk over. The engine re-tests the
            // hull after the move and its floor pass steps the body up, so a step's worth of rise is legal
            // here — the same allowance `move_body` uses (`moveSize + 0.01`).
            const STEP: f32 = 0.07;
            let inside = self.inside_sector(adjoin, [landing[0], landing[1], landing[2] - radius], 0.0);
            let stepped = self.inside_sector(adjoin, [landing[0], landing[1], landing[2] - radius + STEP], 0.0);
            if std::env::var_os("BK_SECTOR_TRACE").is_some() {
                eprintln!(
                    "sector trace:   advance {advance:.4} landing {landing:?} inside {inside} stepped {stepped}"
                );
            }
            if inside || stepped {
                return Some(advance);
            }
        }
        None
    }

    /// [`Self::doorway_advance`], with a trace on `BK_SECTOR_TRACE`.
    fn doorway_advance_logged(&self, from: [f32; 3], dir: [f32; 3], contact: f32, radius: f32, adjoin: usize) -> Option<f32> {
        let advance = self.doorway_advance(from, dir, contact, radius, adjoin);
        if std::env::var_os("BK_SECTOR_TRACE").is_some() {
            eprintln!(
                "sector trace: doorway into {adjoin} from {from:?} dir {dir:?} contact {contact:.4} -> advance {advance:?}"
            );
        }
        advance
    }

    /// `sithCollision_MoveThing` for a walking body: sweep horizontally, slide along what was hit, and
    /// lift over a low step.
    ///
    /// The engine's loop pops a collision, lets a handler decide (a wall stops, a doorway is entered,
    /// a thing is pushed) and then re-sweeps the remaining distance at the angle the hit allows. Three
    /// bounces is enough for a corner; a fourth would only re-find the same wall.
    ///
    /// A step is tried as the engine does it — the same sweep from a raised start, see
    /// `sithPhysics_FindFloor`'s second pass at `range = moveSize` — and the vertical sweep then settles
    /// the body onto whatever it landed on.
    /// The nearest **solid model** the body would run into: `(travel, normal)` in plan.
    ///
    /// `sithCollision` treats a collidable thing's model as solid surfaces, so a walking body is
    /// **blocked** by a crate or a table and slides along it. A `collide=3` thing is its model's
    /// oriented bounds box, and the rest are cylinders sized from the model's own plan footprint
    /// (see `solid_things`): the circle test solves `|P + d t - C| = R + r`, while the box test sweeps
    /// the sphere in the thing's own frame. Both return the face normal as the slide frame.
    /// Whether a segment crosses a **solid thing** — the bar counter, a crate, a column.
    ///
    /// The engine's sight and shot rays go through the collision search, so a table between two
    /// bodies blocks both; `segment_blocked` alone tests world surfaces, which let level 1's cantina
    /// patron see the player across the bar counter (round 307). A segment that *starts* inside a
    /// solid is not blocked by it: that is the actor's own body.
    pub fn segment_thing(&self, from: [f32; 3], to: [f32; 3]) -> bool {
        let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len < 1e-4 {
            return false;
        }
        let dir = [d[0] / len, d[1] / len, d[2] / len];
        // The circle test runs in the floor plane, so its quadratic's `a` is the ray's horizontal
        // speed squared, not 1: a sloped ray crosses a cylinder over a longer stretch of `t`.
        let a = dir[0] * dir[0] + dir[1] * dir[1];
        if a < 1e-8 {
            return false;
        }
        let things = self.things();
        for thing in things.iter() {
            let (centre, radius, z_min, z_max) = (thing.centre, thing.radius, thing.z_min, thing.z_max);
            if let Some(half) = thing.half {
                let from_local = thing.local([from[0], from[1]]);
                let to_local = thing.local([to[0], to[1]]);
                // A ray that starts or ends inside the leaf is an actor's own body, not an obstacle.
                if (from_local[0].abs() <= half[0] && from_local[1].abs() <= half[1])
                    || (to_local[0].abs() <= half[0] && to_local[1].abs() <= half[1])
                {
                    continue;
                }
                let d_local = [to_local[0] - from_local[0], to_local[1] - from_local[1]];
                let mut near = 0.0f32;
                let mut far = len;
                for axis in 0..2 {
                    if d_local[axis].abs() < 1e-8 {
                        if from_local[axis].abs() > half[axis] {
                            near = f32::MAX;
                            break;
                        }
                        continue;
                    }
                    let t1 = (-half[axis] - from_local[axis]) / d_local[axis];
                    let t2 = (half[axis] - from_local[axis]) / d_local[axis];
                    near = near.max(t1.min(t2));
                    far = far.min(t1.max(t2));
                    if near > far {
                        break;
                    }
                }
                if near > 0.0 && near <= len {
                    let (z0, z1) = (from[2] + dir[2] * near, from[2] + dir[2] * far);
                    if z0.min(z1) <= z_max && z0.max(z1) >= z_min {
                        return true;
                    }
                }
                continue;
            }
            let (dx, dy) = (from[0] - centre[0], from[1] - centre[1]);
            let b = 2.0 * (dir[0] * dx + dir[1] * dy);
            let c = dx * dx + dy * dy - radius * radius;
            // The ray starts inside this cylinder: it is the actor's own body, not an obstacle.
            if c <= 0.0 {
                continue;
            }
            // The ray **ends** inside it: that is the target's own body, which never blocks the
            // sight of itself.
            let (tx, ty) = (to[0] - centre[0], to[1] - centre[1]);
            if tx * tx + ty * ty <= radius * radius && to[2] >= z_min && to[2] <= z_max {
                continue;
            }
            let disc = b * b - 4.0 * a * c;
            if disc < 0.0 {
                continue;
            }
            // Entry and exit of the infinite cylinder, clipped to the segment: the ray is inside the
            // circle for that whole stretch, so it is blocked when its height there meets the span —
            // including a ray that comes in over the top and dips into the counter.
            let enter = (-b - disc.sqrt()) / (2.0 * a);
            let leave = ((-b + disc.sqrt()) / (2.0 * a)).min(len);
            if enter <= 0.0 || enter > len {
                continue;
            }
            let (z0, z1) = (from[2] + dir[2] * enter, from[2] + dir[2] * leave);
            if z0.min(z1) <= z_max && z0.max(z1) >= z_min {
                return true;
            }
        }
        false
    }

    /// Whether `other` is in the collision search of a sphere of `radius` swept `left` along `dir` from
    /// `at` in `sector`. `sithCollision_SearchForCollisions` (`0x4e8160`) adds a sector only when the
    /// sweep **reaches one of its adjoins**, so a crate in the next room (or in a trigger sliver inside
    /// this one) collides once the body gets to the opening — while a body resting on a floor opening
    /// and walking level (`18Ascend`'s spawn over its shaft) never reaches the leaves below it.
    pub fn in_search(
        &self,
        sector: usize,
        other: usize,
        at: [f32; 3],
        dir: [f32; 3],
        left: f32,
        radius: f32,
    ) -> bool {
        if other == sector {
            return true;
        }
        self.portals.get(sector).is_some_and(|portals| {
            portals.iter().any(|portal| {
                if portal.adjoin != other {
                    return false;
                }
                let now = dot(sub(at, portal.point), portal.normal);
                let then = now + dot(dir, portal.normal) * left;
                // Crossing the plane, or overlapping it by more than float dust on the way. A sphere
                // that only touches it (a body standing on it) is not a contact.
                now.signum() != then.signum() || now.abs().min(then.abs()) < radius - 1e-4
            })
        })
    }

    fn sweep_things(
        &self,
        at: [f32; 3],
        dir: [f32; 3],
        left: f32,
        radius: f32,
        sector: usize,
    ) -> Option<(f32, [f32; 3])> {
        let mut best: Option<(f32, [f32; 3])> = None;
        let things = self.things();
        for thing in things.iter() {
            // `SearchForThingCollisions` walks the sector lists of the sectors the sweep reaches: a
            // crate in the room next door is not a contact until the body gets to the opening.
            let next_to = |from: usize, to: usize| {
                self.portals
                    .get(from)
                    .is_some_and(|portals| portals.iter().any(|portal| portal.adjoin == to))
            };
            let reachable = self.in_search(sector, thing.sector, at, dir, left, radius)
                || next_to(sector, thing.sector)
                || self.portals.get(sector).is_some_and(|portals| {
                    portals.iter().any(|portal| next_to(portal.adjoin, thing.sector))
                });
            if !reachable {
                continue;
            }
            if at[2] + radius < thing.z_min || at[2] - radius > thing.z_max {
                continue;
            }
            let Some((t, normal)) = thing.sweep_plan(at, dir, left, radius) else {
                continue;
            };
            if best.is_none_or(|(near, _)| t < near) {
                best = Some((t, normal));
            }
        }
        best
    }
    /// The component of `dir` that runs **along** the surface, kept in 3D.
    ///
    /// `sithCollision_MoveThing` slides the body along the plane it hit, so a walk into a ramp is
    /// carried *up* the ramp. Dropping the vertical part made every 45° ramp a wall: the body stayed
    /// flush with the plane and re-hit it at distance 0 every bounce (`16aEscapeShip`'s spawn ramp
    /// wedged after 0.09 units while retail walked 1.56).
    fn slide_along(dir: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
        let into = dir[0] * normal[0] + dir[1] * normal[1] + dir[2] * normal[2];
        [
            dir[0] - normal[0] * into,
            dir[1] - normal[1] * into,
            dir[2] - normal[2] * into,
        ]
    }

    pub fn move_body(
        &self,
        sector: usize,
        pos: [f32; 3],
        delta: [f32; 3],
        radius: f32,
        step: f32,
        gates: Gates<'_>,
    ) -> ([f32; 3], usize, bool) {
        let len = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
        if len < 1e-6 {
            return (pos, sector, false);
        }
        let mut at = pos;
        let mut here = sector;
        let mut left = len;
        let mut blocked = false;
        let mut dir = [delta[0] / len, delta[1] / len, 0.0];

        for bounce in 0..3 {
            if std::env::var_os("BK_MOVE_TRACE").is_some() {
                eprintln!("move: loop {bounce} at {:?} dir {:?} left {left:.5}", at, [dir[0], dir[1]]);
            }
            let room = self.sweep_rooms(here, at, dir, left, radius, gates);
            // A solid model is a contact like a wall: whichever comes first stops the move. A body that
            // walks into a crate stops against it (retail does: `walk3_spawn` and `walk6_spawn` both
            // halt at props), and the radial normal makes the slide round it.
            let body = self.sweep_things(at, dir, left, radius, here);
            if let Some((travel, normal)) = body
                && room.0.is_none_or(|hit| travel < hit.distance)
            {
                blocked = true;
                if std::env::var_os("BK_MOVE_TRACE").is_some() {
                    eprintln!(
                        "move: blocked by a solid model at {:?} normal {:?} after {travel:.3} of {left:.3}",
                        [at[0] + dir[0] * travel, at[1] + dir[1] * travel, at[2]],
                        normal
                    );
                }
                at = [at[0] + dir[0] * travel, at[1] + dir[1] * travel, at[2]];
                left -= travel.max(0.0);
                if left < 1e-4 {
                    break;
                }
                let slide = Self::slide_along(dir, normal);
                let mag = (slide[0] * slide[0] + slide[1] * slide[1] + slide[2] * slide[2]).sqrt();
                if mag < 1e-4 || bounce == 2 {
                    break;
                }
                dir = [slide[0] / mag, slide[1] / mag, slide[2] / mag];
                continue;
            }
            match room {

                (Some(hit), far, _) => {
                    blocked = true;
                    // `BK_MOVE_TRACE`: name the face that stopped the body. A body that reports
                    // blocked while a single-room probe sees nothing is being stopped by a face the
                    // portal-crossing sweep found in a *neighbouring* room, and this is the only
                    // place that knows which.
                    if std::env::var_os("BK_MOVE_TRACE").is_some() {
                        eprintln!(
                            "move: blocked by surface {} at {:?} normal {:?} after {:.6} of {:.6} (from sector {here} at {:?})",
                            hit.surface, hit.point, hit.normal, hit.distance, left, at
                        );
                    }
                    let travelled = (hit.point[0] - at[0]) * dir[0] + (hit.point[1] - at[1]) * dir[1];
                    at = hit.point;
                    // The contact point is `plane + normal * radius` in 3D, so a **steep** face leaves only
                    // `radius * |normal_xy|` clearance in plan (the cantina ramp's xy normal is 0.45 long,
                    // so the body sat 0.45 of a radius clear and crept through it a sub-step at a time).
                    // Horizontal motion needs a full radius of plan clearance: push the contact out along
                    // the horizontal normal by the shortfall. Vertical walls (|n_xy| ~ 1) need no push,
                    // floors (|n_xy| ~ 0) are skipped.
                    let nl = (hit.normal[0] * hit.normal[0] + hit.normal[1] * hit.normal[1]).sqrt();
                    // Only **steep** faces need it: a shallow slope (|n_xy| near 1) is what the step-up
                    // path climbs, and pushing there fights it - `walk_street90` crawled at 0.14 u/s
                    // past a shallow face at x 3.9 with an unbounded push. The cantina ramp is 0.45.
                    // ...but only when the push carries the body **along** its motion. An up-facing ramp's
                    // normal has a downhill plan component, so on a slope the same push that gives a steep
                    // wall its plan clearance shoves the body backwards: level 6's surface 2831 (a sloped
                    // floor, normal (−0.365, 0.183, 0.913), `nl` 0.41) pushed it 0.094 downhill — the
                    // measured 23.1509 → 23.1165 rewind against a +x move — and the body then slid away from
                    // the ramp mouth. A push opposite the motion is never a clearance, it is a rewind.
                    if nl > 1e-4 && nl < 0.6 {
                        let push = radius * (1.0 - nl) / nl;
                        let (mut px, mut py) = (hit.normal[0] * push, hit.normal[1] * push);
                        // Keep the push's *clearance* (its sideways component) but never its backwards
                        // part: an up-facing ramp's normal carries a downhill plan component, so on a
                        // slope the same push shoves the body back along its own motion — level 6's
                        // surface 2831 (normal (−0.365, 0.183, 0.913), `nl` 0.41) rewound it 0.094 against
                        // a +x move, and the body slid away from the ramp mouth instead of entering it.
                        // A plan push opposite the motion is never a clearance, it is a rewind.
                        let along = px * dir[0] + py * dir[1];
                        if along < 0.0 {
                            px -= dir[0] * along;
                            py -= dir[1] * along;
                        }
                        at = [at[0] + px, at[1] + py, at[2]];
                    }
                    here = far;
                    left -= travelled.max(0.0);
                    if left < 1e-4 {
                        break;
                    }
                    // A low obstruction is a step rather than a wall: retry the rest of the move from
                    // one step up. `hit.point` is the centre at contact, so the feet are `radius` below.
                    // A step candidate needs the *face itself* to be climbable, not just its contact point.
                    // A tall ledge's contact is at the body's feet (the body is standing at its base), so
                    // the contact-height test alone called it "low" and the walker could step up against
                    // it frame after frame — level 9's body climbed 0.34 up a 0.55-high ledge while retail
                    // simply stopped at its base. `Sweep::top` is the face's highest vertex; the step
                    // allowance is measured against that.
                    let low = hit.point[2] - radius < pos[2] - radius + step
                        && hit.top <= pos[2] - radius + step + 1e-3;
                    if std::env::var_os("BK_WALK_TRACE").is_some() {
                        eprintln!(
                            "walk trace:   step candidate low {low} contact z {:.3} feet {:.3} step {step:.3}",
                            hit.point[2],
                            pos[2] - radius
                        );
                    }
                    if low {
                        let raised = [at[0], at[1], at[2] + step];
                        // Retry the move one step up, the way `FindFloor` re-sweeps at `range = moveSize`.
                        // The destination's floor may be a frame away (a ledge whose top edge starts just
                        // past the body), so this cannot require a higher floor *here* — it lifts, and the
                        // vertical sweep settles the body onto whatever it ends up over. The four-way
                        // check is what keeps a lift from planting the body inside a wall.
    // `ponytail:` the four-way test rejects a body that merely *touches* the riser it was blocked by
    // (the sweep reports a hit at distance 0, and the contact is at exactly the body radius), so a step
    // only succeeds when the lift clears the riser's polygon top outright. The upgrade is to test the
    // rise against the polygon's own top edge rather than a swept sphere at the raised height.
    // A **stair riser** is the case this test used to reject: after the lift the body is still flush
    // against the *next* step, so a swept sphere at the raised height reports a hit at distance 0 and the
    // step fails on every stair (level 6: the port stopped at the foot of a staircase, z 1.020, while
    // retail climbed it). The old `ponytail:` note named the upgrade — judge the rise against the face's
    // own top edge: a stair riser's top is the next tread (`<= raised feet + step`), a wall's is high.
    let raised_feet = raised[2] - radius;
    let clears = |hit: Option<Sweep>| match hit {
        None => true,
        Some(h) => h.distance > radius * 0.5 || h.top <= raised_feet + step,
    };
    let clear = [[1.0f32, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0]]
        .iter()
        .all(|d| {
            let hit = self.sweep(here, raised, *d, radius * 2.0, radius, gates);
            if std::env::var_os("BK_WALK_TRACE").is_some()
                && let Some(h) = hit
                && !clears(Some(h))
            {
                eprintln!(
                    "walk trace:   step blocked by surface {} normal {:?} top {:.3} distance {:.3} point {:?}",
                    h.surface, h.normal, h.top, h.distance, h.point
                );
            }
            clears(hit)
        });
                        // A step must land *somewhere*: lifting the body at a spot where nothing is below it
                        // is how it walked off the walkway's seam and fell. Eight units is generous, so a
                        // step down into a room still works while a step over a void does not.
                        let lands = self
                            .floor_under(here, raised, 8.0, radius)
                            .is_some_and(|(_, travelled, _)| travelled <= 8.0);
                        // And the lifted body must still be inside its sector. A slanted surface's
                        // outward normal has a vertical component, so raising the body carries it
                        // *through* the plane: on `10cargo` sector 333's 45° corner the lift moved the
                        // centre 0.047 outward, which made the raised sweep read the wall as "behind
                        // me", pass, and walk the body out of the room — the engine never does this,
                        // because its step lives in the floor pass and `MoveThing` re-tests the hull
                        // afterwards (`decomp/004e7950_sithCollision_MoveThing.c:251`).
                        let inside = self.inside_sector(here, [raised[0], raised[1], raised[2] - radius], 0.0);

    let advance = clears(self.sweep(here, raised, dir, left, radius, gates));
    if std::env::var_os("BK_WALK_TRACE").is_some() {
        eprintln!("walk trace:   step predicates clear {clear} lands {lands} inside {inside} advance {advance}");
    }
    if clear && lands && inside && advance {
                            at = raised;
                            continue;
                        }
                    }                    // Slide: drop the velocity component that ran into the surface.
                    let slide = Self::slide_along(dir, hit.normal);
                    let mag = (slide[0] * slide[0] + slide[1] * slide[1] + slide[2] * slide[2]).sqrt();
                    if std::env::var_os("BK_MOVE_TRACE").is_some() {
                        eprintln!(
                            "move: bounce {bounce} hit t {travelled:.5} left {left:.5} dir {:?} slide {:?} mag {mag:.3} at {:?}",
                            [dir[0], dir[1], dir[2]], [slide[0], slide[1], slide[2]], at
                        );
                    }
                    // A **near-head-on** hit has almost no tangential component, and creeping along it is
                    // how a body rounds a wall the engine stops it at: level 13's wall (surface 2125,
                    // normal (−0.42, −0.91)) is hit at ~8° by the walking direction, so the slide is a few
                    // thousandths per frame — over a second it walks the body past the wall's left edge and
                    // off the ledge (health 72.6 from the fall). `mag` is `|dir × normal|` (the sine of the
                    // grazing angle), so a small `mag` is a head-on hit: stop rather than slide.
                    if mag < 0.2 || bounce == 2 {
                        break;
                    }
                    dir = [slide[0] / mag, slide[1] / mag, slide[2] / mag];
                }
                (None, far, end) => {
                    // A doorway's crossing advance runs past the plane by a radius, and that step is not
                    // a wall test: a solid standing *in* the opening (level 1's cantina leaves, two
                    // portal hops out) would be stepped over and the body would end up in the doorway's
                    // own sector, past the leaf it should stop at. The advance is clamped by the first
                    // solid contact along it.
                    let advanced = ((end[0] - at[0]).powi(2) + (end[1] - at[1]).powi(2)).sqrt();
                    if advanced > 1e-6
                        && let Some((travel, _)) = self.sweep_things(at, dir, advanced, radius, far)
                    {
                        blocked = true;
                        at = [at[0] + dir[0] * travel, at[1] + dir[1] * travel, at[2] + dir[2] * travel];
                        break;
                    }
                    // `sweep_rooms` reports its end point in plan and drops the vertical part; a slide
                    // along a ramp carries the body up, so the height is restored here — only on this
                    // path, because the vertical probes rely on the plan-only end.
                    at = [end[0], end[1], at[2] + dir[2] * left];
                    // Nothing was hit, but the sweep may have crossed a doorway and `far` is the room the
                    // body is standing in. This was rejected earlier and is right now: with the crossing
                    // advance uncapped, a body only reaches here after its origin has cleared the far
                    // room's doorway plane, so it is genuinely in that room. (With the capped advance the
                    // body stopped a radius short of the plane and adopting the room flipped it back and
                    // forth — 584 crossings became 13,151.)
                    here = far;
                    break;
                }
            }
        }
        // No hull projection here: `advance` snaps the body onto a sector plane, which can *lower* it
        // through the floor it is standing on. The sweep already walked any doorway, so `here` is the
        // sector the body ended in.
        //
        // The placed models come last because they are not sector planes: a `_structure` column or a
        // `_walkstruct` table is a body-sized cylinder the sweep knows nothing about, and pushing out
        // of it after the walls keeps the two solvers from fighting over the same step.
        // A solid model's cylinder is resolved **after** the walls, and the radial push it computes can
        // carry a body through the wall beside it — then every later slide is blocked and the body
        // wedges in the corner (level 4's `table2` against the doorway panel: the port stopped at
        // (-4.41, 1.885) where retail walks through the open half of the same doorway). So the push is
        // applied only where geometry allows: straight out first, then tangentially (which slides the
        // body along whatever it is pressed against), and not at all when every direction is blocked.
        let at = if std::env::var_os("BK_NO_THINGS").is_some() {
            at
        } else {
            let pushed = self.push_out_of_things(at, radius, here);
            if pushed == at {
                at
            } else {
                let out = [pushed[0] - at[0], pushed[1] - at[1]];
                let len = (out[0] * out[0] + out[1] * out[1]).sqrt();
                if len < 1e-6 {
                    at
                } else {
                    let unit = [out[0] / len, out[1] / len];
                    let clear = |dir: [f32; 2]| {
                        self.sweep(here, at, [dir[0], dir[1], 0.0], len, radius, gates).is_none()
                    };
                    // The push must not run **against** the frame's movement. A body squeezing past a
                    // model's cylinder gets a radial push that is mostly backwards along its own motion
                    // (level 4: two pieces of furniture either side of the spawn pushed the walking body
                    // straight back to where it started, every frame, for the whole leg); the engine's
                    // contact slides the body instead. So only pushes with a forward or sideways
                    // component are applied, and only where geometry is clear.
                    let want = [delta[0], delta[1]];
                    let want_len = (want[0] * want[0] + want[1] * want[1]).sqrt();
                    let forward = if want_len > 1e-6 {
                        (unit[0] * want[0] + unit[1] * want[1]) / want_len
                    } else {
                        1.0
                    };
                    if forward >= -0.1 && clear(unit) { pushed } else { at }
                }
            }
        };
        if std::env::var_os("BK_MOVE_TRACE").is_some() {
            eprintln!("move: end at {:?} here {here} blocked {} from {:?}", at, blocked, pos);
        }
        (at, here, blocked || at != pos)
    }

    /// Push a body out of any solid model it has ended up inside.
    ///
    /// A `collide=3` thing pushes out through its model box's nearest face; the rest use the
    /// template's own `size=` cylinder, the engine`s movement-sphere rule.
    fn push_out_of_things(&self, mut at: [f32; 3], radius: f32, sector: usize) -> [f32; 3] {
        let things = self.things();
        for thing in things.iter() {
            if !self.in_search(sector, thing.sector, at, [0.0; 3], 0.0, radius) {
                continue;
            }
            // The body's own radius against the thing's model span: a table blocks a walking body, a
            // lamp above head height does not.
            if at[2] + radius < thing.z_min || at[2] - radius > thing.z_max {
                continue;
            }
            if let Some(half) = thing.half {
                let local = thing.local([at[0], at[1]]);
                let ext = [half[0] + radius, half[1] + radius];
                let (px, py) = (ext[0] - local[0].abs(), ext[1] - local[1].abs());
                if px <= 0.0 || py <= 0.0 {
                    continue;
                }
                let normal = if px <= py {
                    [if local[0] >= 0.0 { 1.0 } else { -1.0 }, 0.0]
                } else {
                    [0.0, if local[1] >= 0.0 { 1.0 } else { -1.0 }]
                };
                let n = thing.to_world(normal);
                at[0] += n[0] * px.min(py);
                at[1] += n[1] * px.min(py);
                if std::env::var_os("BK_MOVE_TRACE").is_some() {
                    eprintln!(
                        "move: pushed out of the model box at {:?} half {half:?} yaw {:.3} to {:?}",
                        thing.centre, thing.yaw, [at[0], at[1], at[2]]
                    );
                }
                continue;
            }
            let (dx, dy) = (at[0] - thing.centre[0], at[1] - thing.centre[1]);
            let apart = (dx * dx + dy * dy).sqrt();
            let least = thing.radius + radius;
            if apart >= least {
                continue;
            }
            if apart > 1e-5 {
                let scale = least / apart;
                at[0] = thing.centre[0] + dx * scale;
                at[1] = thing.centre[1] + dy * scale;
            } else {
                // Dead centre: pick a direction rather than divide by zero.
                at[0] = thing.centre[0] + least;
            }
            // `BK_MOVE_TRACE`: a body pushed out of a solid thing every frame is a body walking
            // inside a model — `move_body` reports `blocked` only because of this push, which is why
            // the sweep said "nothing ahead" while the walker crawled.
            if std::env::var_os("BK_MOVE_TRACE").is_some() {
                eprintln!(
                    "move: pushed out of the collision cylinder at {:?} radius {:.3} (z {:.2}..{:.2}) to {:?}",
                    thing.centre, thing.radius, thing.z_min, thing.z_max, [at[0], at[1], at[2]]
                );
            }
        }
        at
    }

    /// `sithCamera_SearchSectorInRadius` (0x4c6050): pull a camera in to the first thing between it and
    /// the focus.
    ///
    /// The engine normalises `focus -> want`, calls `sithCollision_SearchForCollisions` with radius
    /// `0x3ca3d70a` (0.02) and flags `|0x800`, then **rewrites the camera position to the first hit**
    /// (`cam = hit_t * dir + focus`). That is what keeps a chase camera out of the wall behind the
    /// player; it is only used by the third-person modes, because in first person the near plane
    /// (1/64) already sits inside the body's own radius.
    ///
    /// This does the same cast through the sector walk, so a camera behind a doorway is not pulled in
    /// by the wall beside the door. `None` in the second slot means nothing was in the way and the
    /// requested point stands.
    pub fn camera_pull(
        &self,
        sector: usize,
        focus: [f32; 3],
        want: [f32; 3],
        radius: f32,
    ) -> ([f32; 3], Option<usize>) {
        let delta = [want[0] - focus[0], want[1] - focus[1], want[2] - focus[2]];
        let len = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        if len < 1e-5 {
            return (want, None);
        }
        let dir = [delta[0] / len, delta[1] / len, delta[2] / len];
        match self.sweep_rooms(sector, focus, dir, len, radius, Gates::default()).0 {
            // `Sweep::point` is where the sphere's centre contacts the plane, which is exactly where
            // the engine puts the camera — it does not back off by the radius again.
            Some(hit) => {
                if std::env::var_os("BK_CAMERA_TRACE").is_some() {
                    println!(
                        "camera pull: sector {sector}, contact at {:.4} of {len:.2}, normal {:?}, surface {}",
                        hit.distance, hit.normal, hit.surface
                    );
                }
                (hit.point, Some(hit.surface))
            }
            None => (want, None),
        }
    }
    /// The sector a body at `pos` can actually stand in, and the floor it would land on.
    ///
    /// This is the criterion the spawn needs: the level's recorded sector for a thing can be a trigger
    /// sliver with no planes and no doorways, where *no sweep can reach anything* — the body then falls
    /// for ever. `floor_below` cannot answer it either, because the floor may belong to the room below a
    /// doorway. So the answer is the sweep's: try every sector whose hull contains the point and take the
    /// first one a downward sweep finds a floor from.
    pub fn standable(
        &self,
        sector: usize,
        pos: [f32; 3],
        height: f32,
        radius: f32,
    ) -> Option<(usize, f32, f32)> {
        // `sector` itself comes first: a trigger sliver has no planes, so containment cannot name it,
        // yet it is a real sector a sweep can start from — and the level recorded it for a reason.
        std::iter::once(sector)
            .chain(self.sectors_at_exact(pos))
            .chain(self.sector_for_body(pos))
            .find_map(|sector| {
                self.floor_under(sector, pos, height, radius)
                    .map(|(floor, travelled, _)| (sector, floor, travelled))
            })
    }

    /// Does `pos` lie inside this room's actual geometry, rather than merely behind its
    /// infinite hull planes?
    ///
    /// Level 1's sector 340 is the shared sliver between the cantina floor and ceiling. Its
    /// planes contain the whole recorded spawn, but its solid polygons cover only the strips
    /// beside the tables. The hull therefore admits a body through a wall. Portals are openings,
    /// so their polygons count as room coverage without becoming solid obstacles.
    pub fn room_contains(&self, room: usize, pos: [f32; 3], radius: f32) -> bool {
        // `sithIntersect_IsSphereInSector` accepts a sphere wholly inside the sector's
        // collide box without testing a face. Level 5's stacked water/air volumes rely on
        // exactly this: their infinite hulls overlap, but the collide boxes separate them.
        if let Some(Some((lo, hi))) = self.boxes.get(room)
            && (0..3).all(|axis| pos[axis] - radius >= lo[axis] && pos[axis] + radius <= hi[axis])
        {
            return true;
        }
        let Some(planes) = self.sectors.get(room) else { return false };
        let hull = self.hull.get(room).filter(|p| !p.is_empty()).unwrap_or(planes);
        if !hull.iter().all(|plane| -dot(sub(pos, plane.point), plane.normal) >= -radius) {
            return false;
        }
        // A fixture or a sector with no polygon inventory has only its hull to answer with.
        // The real level build supplies every solid face and every portal polygon.
        if self.faces.is_empty() && self.portal_faces.is_empty() {
            return true;
        }
        self.faces
            .iter()
            .filter(|face| self.surface_owner.get(face.surface).copied().unwrap_or(room) == room)
            .chain(self.portal_faces.iter().filter(move |(owner, _)| *owner == room).map(|(_, face)| face))
            .any(|face| sphere_face(pos, radius, face))
    }

    /// `sithPhysics_FindFloor`: the floor under a body, found by sweeping a sphere straight down.
    ///
    /// The engine's range is `height * 1.1` (and `moveSize + 0.005` for a body that is not attached yet),
    /// which is what lets it find a floor it has just stepped off. The sweep crosses adjoins, so the
    /// floor of the room *below* a walkway's portal is found from above — that being exactly the case
    /// this port used to fall through.
    pub fn floor_under(
        &self,
        sector: usize,
        pos: [f32; 3],
        height: f32,
        radius: f32,
    ) -> Option<(f32, f32, usize)> {
        // `floor_candidates` is the one loop; this picks the highest landing, which is the surface the
        // sphere meets first falling from `pos`. Both callers share it so a probe cannot drift from
        // the walker's own answer.
        let mut best: Option<(f32, f32, usize)> = None;
        for (floor, travelled, owner, _, _) in self.floor_candidates(sector, pos, height, radius) {
            if best.is_none_or(|(near, _, _)| floor > near) {
                best = Some((floor, travelled, owner));
            }
        }
        // `BK_FLOOR_TRACE`: a landing *below the body's own sector floor* is how a body ends up standing on
        // something its room does not call a floor (level 5's plateau). Naming the face and both normals
        // there is the only way to tell a legitimate off-sector landing from the plateau.
        if std::env::var_os("BK_FLOOR_TRACE").is_some()
            && let Some((floor, _, owner)) = best
        {
            let here = self.floor_below(sector, pos);
            if here.is_none_or(|h| floor < h - 0.03)
                && let Some(&(_, _, fowner, normal, surface)) = self
                    .floor_candidates(sector, pos, height, radius)
                    .iter()
                    .find(|(f, _, _, _, _)| (*f - floor).abs() < 1e-6)
            {
                eprintln!(
                    "floor: landing {floor:.4} on surface {surface} owner {fowner} (body sector {sector} floor {here:?}) file normal {normal:?} room normal {:?} pos {pos:?}",
                    self.sectors
                        .get(fowner)
                        .and_then(|planes| planes.iter().find(|p| p.surface == surface))
                        .map(|p| p.normal),
                );
                let _ = owner;
            }
        }
        best
    }

    /// Every face a downward sphere sweep from `pos` lands on, as `(floor height, distance travelled,
    /// owning sector, the face's own normal, surface id)`. The walker's answer is the highest of these; the
    /// `--floor-at` probe prints them all, which is what makes a floor disagreement a table instead of
    /// an argument.
    pub fn floor_candidates(
        &self,
        sector: usize,
        pos: [f32; 3],
        height: f32,
        radius: f32,
    ) -> Vec<(f32, f32, usize, [f32; 3], usize)> {
        // The engine searches the body's room and its adjoins, not every face in the world. A global
        // scan lets a body land on an unrelated tabletop or ledge: level 1's start (sector 339) is the
        // table-top volume, whose correct downward contact is the room floor at -1.25, not the table
        // top at -0.95. Choosing the latter put the body on the tables, where their cylinders blocked
        // every walk direction.
        //
        // Start in the recorded room and expand through the level's portals. The build mirrors each
        // link so a one-sided claim still lets a body out of a trigger volume. Do not cross a portal
        // whose movement bit is shut: that is the same reachability rule `sweep_rooms` uses.
        let range = (height * 1.1).max(radius + 0.005);
        let mut reachable = vec![false; self.sectors.len()];
        let mut pending = Vec::new();
        if sector < self.sectors.len() {
            reachable[sector] = true;
            pending.push(sector);
        }
        while let Some(room) = pending.pop() {
            for portal in self.portals.get(room).into_iter().flatten() {
                let next = portal.adjoin;
                if next >= self.sectors.len() || reachable[next] {
                    continue;
                }
                if self.adjoin_flags.get(portal.link).is_some_and(|f| f & ADJOIN_MOVE == 0) {
                    continue;
                }
                // A link is a wall opening, not a floor connection: only the room the probe
                // actually lies in may supply a floor. Hull planes alone are not containment:
                // level 1's shared sliver 340 spans the tables but its polygons do not, so it
                // used to lend a floor to the tabletop volume through a solid wall.
                if !self.room_contains(next, pos, radius) {
                    continue;
                }
                reachable[next] = true;
                pending.push(next);
            }
        }
        let mut out = Vec::new();
        let scan = |scope: &[bool], out: &mut Vec<(f32, f32, usize, [f32; 3], usize)>| {
            for face in &self.faces {
                let owner = self.surface_owner.get(face.surface).copied().unwrap_or(sector);
                // An unowned face (a hand-built test fixture, or a surface no sector claims) is in
                // scope only when it belongs to the seed room, never to a neighbouring room.
                if !scope.get(owner).copied().unwrap_or(owner == sector) {
                    continue;
                }
                // A vertical face is not a floor, and a downward one is a ceiling.
                if face.normal[2].abs() < 0.2 {
                    continue;
                }
                // `.jkl` plane normals point **out of the room**, while
                // `sithCollision_SearchForSurfaceCollisions` tests into-room normals
                // (`dot(search_dir, normal) < 0` accepts a floor for a downward sweep). Flipping the
                // face toward the body is that normalisation, not a deviation.
                let normal = if dot(sub(pos, face.point), face.normal) < 0.0 {
                    [-face.normal[0], -face.normal[1], -face.normal[2]]
                } else {
                    face.normal
                };
                if normal[2] < 0.2 {
                    continue;
                }
                let Some(t) = sphere_plane(pos, [0.0, 0.0, -1.0], range, radius, normal, face.point)
                else {
                    continue;
                };
                let at = [pos[0], pos[1], pos[2] - t];
                if !sphere_face(at, radius, face) {
                    continue;
                }
                let floor = at[2] - radius;
                out.push((floor, pos[2] - at[2], owner, face.normal, face.surface));
            }
        };
        scan(&reachable, &mut out);
        if out.is_empty() {
            // The shipped data does not always tile: a walkway slab or a trigger sliver can sit in a
            // sector with no portal to the room below it. Only then fall back to the global scan; it
            // is a rescue for a disconnected room, never a preferred landing while a reachable face
            // exists. The body's vertical pass still refuses a contact above its current height.
            out.clear();
            let all = vec![true; self.sectors.len()];
            scan(&all, &mut out);
        }
        out
    }
}

/// What a body's feet come to rest on: the single floor answer every motion caller shares.
///
/// `height` is the **feet** height, not the centre: a swept sphere stops with its centre one
/// `radius` above a face, a sector plane and a solid thing's top are already feet heights, so every
/// caller inverts it the same way (`height + radius`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rest {
    /// The height the body's feet rest at.
    pub height: f32,
    /// How far below the body's centre the contact sits (`pos[2] - (height + radius)`).
    pub travelled: f32,
    /// The contact normal turned toward the body, so a ramp gives the direction it slides down.
    /// A sector plane and a solid thing's top have none.
    pub normal: Option<[f32; 3]>,
}

impl LevelGeometry {
    /// The one floor rule: the player, AI settling, actor grounding and gravity landing all ask
    /// this, and no caller keeps a second support rule behind it.
    ///
    /// In the engine's own order, highest answer wins:
    ///
    /// 1. the downward sphere sweep's faces — a catwalk slab, a ramp, a face-only floor;
    /// 2. a sector's own floor **plane** where its data has no face inside the engine's floor
    ///    reach (`18Ascend`'s shaft opening), and the planes of the sectors around a body whose
    ///    sector carries no floor plane at all (`19A`'s 723, level 18's spawn);
    /// 3. a solid thing's model top (`collide=`, `ThingSolid::z_max`) — a crate, a catwalk, a lift.
    ///
    /// `reach` is how far below the feet a surface may be and still count: the walker passes this
    /// frame's fall, so a body in the air is not caught by a floor it has not reached. Motion
    /// callers that want the floor under a body wherever it is (actor grounding) pass
    /// `f32::INFINITY` and compare the answer with their own reach.
    pub fn resting_floor(
        &self,
        sector: usize,
        pos: [f32; 3],
        reach: f32,
        radius: f32,
    ) -> Option<Rest> {
        let mut best: Option<Rest> = None;
        for (floor, travelled, _owner, raw, surface) in
            self.floor_candidates(sector, pos, reach + radius, radius)
        {
            // The fall may only reach a floor whose contact is inside this frame's travel, and the
            // contact must not be above where the body already is.
            if travelled > reach + 1e-3 || floor + radius > pos[2] + 1e-3 {
                continue;
            }
            if best.is_none_or(|near| floor > near.height) {
                // `.jkl` plane normals point out of the room, while the walker wants the normal
                // turned toward the body; a ramp's horizontal part is the downslope direction.
                let normal = self
                    .face_of
                    .get(&surface)
                    .and_then(|i| self.faces.get(*i))
                    .map(|face| {
                        if dot(sub(pos, face.point), face.normal) < 0.0 {
                            [-face.normal[0], -face.normal[1], -face.normal[2]]
                        } else {
                            face.normal
                        }
                    })
                    .unwrap_or(raw);
                best = Some(Rest {
                    height: floor,
                    travelled,
                    normal: Some(normal),
                });
            }
        }
        if best.is_none() {
            // The shipped data does not always tile, so the plane is the port's own data-gap
            // repair, not engine behaviour (`sithPhysics_FindFloor` sweeps surfaces only).
            let plane = match self.floor_below(sector, pos) {
                // A sector with no floor plane of its own is held by the floor planes of the
                // sectors around it, which may not be walkable from it at all.
                None => self
                    .sectors_around(pos)
                    .into_iter()
                    .filter_map(|other| self.floor_below(other, pos))
                    // Never lift: contact must be at or below where the body already is.
                    .filter(|floor| floor + radius <= pos[2] + 1e-3)
                    .fold(None, |best: Option<f32>, floor| {
                        Some(best.map_or(floor, |near| near.max(floor)))
                    }),
                // A sector whose plane is its only floor answers with it where the body is over
                // the room and no face is inside the engine's floor reach: a walkway slab 0.42
                // below a ramp still leaves the plane in charge (`walk15`'s 212-of-243 ungrounded
                // frames), while level 2's ledge plane 3.0 under a 3.065 body is not support.
                Some(plane) => {
                    let contact = plane + radius;
                    (self.room_contains(sector, pos, radius)
                        && self.floor_under(sector, pos, 1.0, radius).is_none()
                        && contact <= pos[2] + 1e-3
                        && pos[2] - contact <= reach + 1e-3)
                        .then_some(plane)
                }
            };
            if let Some(height) = plane {
                best = Some(Rest {
                    height,
                    travelled: pos[2] - height - radius,
                    normal: None,
                });
            }
        }
        if let Some(top) = self.prop_floor(pos, radius, reach)
            && best.is_none_or(|near| top > near.height)
        {
            best = Some(Rest {
                height: top,
                travelled: pos[2] - top - radius,
                normal: None,
            });
        }
        best
    }

    /// The top of the highest solid thing the body is over, within `reach` below the feet.
    ///
    /// `sithCollision` treats a thing's own model as solid for every body, so a catwalk slab, a
    /// crate or a lift platform is a floor. Tops are already feet heights, and a top above the
    /// feet is something the body is beside, not standing on.
    fn prop_floor(&self, pos: [f32; 3], radius: f32, reach: f32) -> Option<f32> {
        let feet = pos[2] - radius;
        let mut best: Option<f32> = None;
        for thing in self.things().iter() {
            let over = match thing.half {
                Some(half) => {
                    let local = thing.local([pos[0], pos[1]]);
                    local[0].abs() <= half[0] + radius && local[1].abs() <= half[1] + radius
                }
                None => {
                    let (dx, dy) = (pos[0] - thing.centre[0], pos[1] - thing.centre[1]);
                    dx * dx + dy * dy <= (thing.radius + radius).powi(2)
                }
            };
            if !over {
                continue;
            }
            let top = thing.z_max;
            if top > feet + 0.02 || feet - top > reach + 1e-3 {
                continue;
            }
            if best.is_none_or(|near| top > near) {
                best = Some(top);
            }
        }
        best
    }
}

/// How far along `dir` the plane of `surface` is from `from`: what a sphere sweep is measured against.
///
/// Exposed for the `--collide` gate, which checks that a sweep stops a radius short of the face; the
/// walker itself never needs it, since it only wants the contact point.
pub fn distance_to_surface(solids: &Solids, surface: usize, from: [f32; 3], dir: [f32; 3]) -> f32 {
    let face = solids
        .face_of
        .get(&surface)
        .and_then(|i| solids.faces.get(*i))
        .or_else(|| solids.wall_of.get(&surface).and_then(|i| solids.walls.get(*i)))
        .or_else(|| solids.floor_of.get(&surface).and_then(|i| solids.floors.get(*i)));
    let Some(face) = face else {
        return f32::INFINITY;
    };
    let normal = if dot(sub(from, face.point), face.normal) < 0.0 {
        [-face.normal[0], -face.normal[1], -face.normal[2]]
    } else {
        face.normal
    };
    let approach = -dot(dir, normal);
    if approach <= 1e-6 {
        return f32::INFINITY;
    }
    dot(sub(from, face.point), normal) / approach
}

/// Is `p` inside the convex polygon `verts`, measured in the plane with `normal`?
/// The test only needs the signs to agree, so it does not depend on winding.
fn in_polygon(verts: &[[f32; 3]], normal: [f32; 3], p: [f32; 3]) -> bool {
    let mut positive = false;
    let mut negative = false;
    for i in 0..verts.len() {
        let a = verts[i];
        let b = verts[(i + 1) % verts.len()];
        let s = dot(cross(sub(b, a), sub(p, a)), normal);
        if s > 1e-5 {
            positive = true;
        } else if s < -1e-5 {
            negative = true;
        }
        if positive && negative {
            return false;
        }
    }
    true
}

/// Whether the segment `from -> to` passes through a surface's polygon.
///
/// `SITH_MESSAGE_CROSSED` fires when a thing crosses a surface, which is how a trigger plane or a
/// force field script knows the player went through it. The test is the plain one: the endpoints are
/// on opposite sides of the surface's plane and the crossing point is inside the polygon.
pub fn crosses(level: &crate::jkl::Level, surface: usize, from: [f32; 3], to: [f32; 3]) -> bool {
    let Some(s) = level.surfaces.get(surface) else { return false };
    let verts: Vec<[f32; 3]> = s
        .verts
        .iter()
        .filter_map(|(v, _)| level.vertices.get(*v as usize).copied())
        .collect();
    if verts.len() < 3 {
        return false;
    }
    // The same widest-triple rule as the collision build: a trigger face whose first three
    // vertices are collinear still has a plane, and `crossed` has to find it.
    let Some(normal) = polygon_normal(&verts) else {
        return false;
    };
    let (d0, d1) = (dot(sub(from, verts[0]), normal), dot(sub(to, verts[0]), normal));
    // Same side (or sitting exactly on the plane both times) is not a crossing.
    if d0.signum() == d1.signum() || (d0 - d1).abs() < 1e-6 {
        return false;
    }
    let t = d0 / (d0 - d1);
    let p = [
        from[0] + (to[0] - from[0]) * t,
        from[1] + (to[1] - from[1]) * t,
        from[2] + (to[2] - from[2]) * t,
    ];
    in_polygon(&verts, normal, p)
}

impl LevelGeometry {
    /// Whether the segment `from -> to` runs into any of the level's wall faces.
///
/// A position-only resolver cannot see a mover that steps clean over a thin wall between two frames,
/// which is why a fast projectile passed through geometry in `--triggers`. This is the swept test:
/// the segment against each wall face's plane, inside that face's polygon.
///
/// `ponytail:` it scans every wall face, so it is O(walls) per moving thing per frame. With a handful
/// of movers that is nothing; a level with hundreds of projectiles would want the sector's own faces
/// instead of the whole list.
pub fn segment_blocked(&self, from: [f32; 3], to: [f32; 3]) -> bool {
    self.segment_blocked_at(from, to).is_some()
}

/// The face that stops `from -> to`, if one does: the same test as [`Self::segment_blocked`], with
/// the surface and the crossing point so a trace can name the wall instead of guessing.
pub fn segment_blocked_at(&self, from: [f32; 3], to: [f32; 3]) -> Option<(usize, [f32; 3])> {
    for face in &self.walls {
        if face.verts.len() < 3 {
            continue;
        }
        let (d0, d1) = (dot(sub(from, face.point), face.normal), dot(sub(to, face.point), face.normal));
        if d0.signum() == d1.signum() || (d0 - d1).abs() < 1e-6 {
            continue;
        }
        // Only a crossing from the front of the face counts: walking away from a wall is not a hit.
        if d0 < 0.0 {
            continue;
        }
        let t = d0 / (d0 - d1);
        let p = [
            from[0] + (to[0] - from[0]) * t,
            from[1] + (to[1] - from[1]) * t,
            from[2] + (to[2] - from[2]) * t,
        ];
        if in_polygon(&face.verts, face.normal, p) {
            return Some((face.surface, p));
        }
    }
    None
}
}

impl LevelGeometry {
    /// The sector a point is in: the smallest hull sphere that contains it and whose
    /// solid planes put it inside. Returns `None` outside every sector.
    pub fn sector_at(&self, pos: [f32; 3], slack: f32) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, (centre, radius)) in self.spheres.iter().enumerate() {
            if dot(sub(pos, *centre), sub(pos, *centre)) > radius * radius {
                continue;
            }
            // A hull's interior is on the negative side of every outward plane; the
            // slack absorbs faces that meet exactly at the hull's edge.
            if self.sectors[i]
                .iter()
                .all(|p| dot(sub(pos, p.point), p.normal) <= slack)
            {
                match best {
                    Some((_, r)) if *radius >= r => {}
                    _ => best = Some((i, *radius)),
                }
            }
        }
        best.map(|(i, _)| i)
    }

    /// Every sector whose bounding sphere contains `pos` — the seed set for drawing.
    ///
    /// `sector_at` prefers the smallest hull, which is right for walking but wrong for
    /// drawing. Level 1 has 462 sectors including thin trigger slivers that sit inside a
    /// room, and its rooms are not all convex, so the *hull plane test* misses the room
    /// the camera is standing in (its floor and ceiling then vanish). The sphere test is
    /// looser on purpose: seeding from a few candidate sectors only ever adds rooms.
    pub fn sectors_around(&self, pos: [f32; 3]) -> Vec<usize> {
        self.spheres
            .iter()
            .enumerate()
            .filter(|(_, (centre, radius))| dot(sub(pos, *centre), sub(pos, *centre)) <= radius * radius)
            .map(|(i, _)| i)
            .collect()
    }

    /// The floor under `pos` in `sector`, with the `Level::surfaces` entry it belongs to.
    ///
    /// The walker needs the surface itself, not just the height: the surface flags say
    /// whether the player is standing on water (`0x20000`), deep water (`0x100000`), earth
    /// or metal, which the engine uses for movement speed and landing sounds.
    pub fn floor_surface(&self, sector: usize, pos: [f32; 3]) -> Option<(f32, usize)> {
        let planes = self.sectors.get(sector)?;
        planes
            .iter()
            .filter(|p| p.normal[2] < -0.7)
            .map(|p| {
                let d = dot(sub(pos, p.point), p.normal);
                (pos[2] - p.normal[2] * d, p.surface)
            })
            .filter(|(z, _)| *z <= pos[2] + 0.05)
            .max_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// How far inside a sector a point is: the smallest distance to any plane that bounds the cell.
    ///
    /// `sithIntersect_IsSphereInSector` (JK.EXE `0x507f30`) tests the sector's *hull*, and the hull is
    /// the bounding planes only — a wall patch in the middle of a room is not part of it. `hull`
    /// carries exactly those (every vertex the sector claims is behind them), so this is the engine's
    /// question rather than "every surface this sector happens to claim".
    ///
    /// `None` means the sector claims no bounding plane at all (a trigger sliver), which callers
    /// should read as "inside": there is nothing to be outside of.
    pub fn clearance(&self, sector: usize, pos: [f32; 3]) -> Option<f32> {
        // A sector with no bounding plane at all (a trigger sliver, or a synthetic cell in a test)
        // falls back to its whole plane list: there is no hull to be outside of, and the fallback
        // keeps such a cell behaving as it did before `hull` existed.
        let planes = match self.hull.get(sector) {
            Some(planes) if !planes.is_empty() => planes,
            _ => self.sectors.get(sector)?,
        };
        planes.iter().map(|p| -dot(sub(pos, p.point), p.normal)).reduce(f32::min)
    }

    /// `sithIntersect_IsSphereInSector` (JK.EXE `0x507f30`, `sithIntersect.c:58`): is a body of
    /// `radius` at `pos` still inside `sector`?
    ///
    /// Two parts, in the engine's order. First the sector's collide box (`HASCOLLIDEBOX`, flag
    /// `0x1000`): if the sphere fits inside it the answer is **yes**, without testing a single
    /// surface. Only a sphere outside that box is tested against the sector's bounding planes, where
    /// a distance below the radius means outside. `sithCollision_MoveThing` calls this with radius 0
    /// to decide whether to undo a move.
    pub fn inside_sector(&self, sector: usize, pos: [f32; 3], radius: f32) -> bool {
        if let Some(Some((lo, hi))) = self.boxes.get(sector) {
            let fits = (0..3).all(|k| pos[k] - radius >= lo[k] && pos[k] + radius <= hi[k]);
            if fits {
                return true;
            }
        }
        self.clearance(sector, pos).is_none_or(|c| {
            // `sithIntersect_IsSphereInSector` snaps a plane distance within `1e-5` of zero to zero
            // (`_DAT_00523288` is 1e-5, `_DAT_00523284` is 0) before the radius test, so a body
            // resting exactly on its own floor is inside it. Without the band, `floor + radius` as
            // f32 lands a ten-millionth below the plane and the containment check rejects a body
            // standing on solid ground (`18Ascend`'s spawn did exactly that, every frame).
            let c = if c.abs() <= 1e-5 { 0.0 } else { c };
            c >= radius
        })
    }

    /// The sector a *body* of `radius` should start in.
    ///
    /// `sector_at` prefers the smallest hull, which is right for "which room is this
    /// point in" but wrong for a body of finite size: level 1 puts thin trigger slivers
    /// inside rooms, and a sliver that contains the point cannot contain the body — the
    /// solver then pushes the walker out of the sliver and pins it. This picks the
    /// sector with the most room to spare instead.
    pub fn sector_for_body(&self, pos: [f32; 3]) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for sector in self.sectors_at_exact(pos) {
            let Some(clearance) = self.clearance(sector, pos) else { continue };
            if best.is_none_or(|(_, c)| clearance > c) {
                best = Some((sector, clearance));
            }
        }
        // Fall back to the ordinary containment test so a point on a boundary still
        // starts somewhere.
        best.map(|(sector, _)| sector).or_else(|| self.sector_at(pos, 0.05))
    }

    /// Every sector whose planes all have `pos` on their inside.
    pub fn sectors_at_exact(&self, pos: [f32; 3]) -> Vec<usize> {
        (0..self.sectors.len())
            .filter(|&i| self.clearance(i, pos).is_some_and(|c| c >= 0.0))
            .collect()
    }

    /// Breadth-first route from one sector to another, as the **portal centres** to walk through.
    ///
    /// `sithAI.c` moves a puppet along the level's own path graph; this world's graph is the adjoin
    /// table, so the shortest chain of openings is the route. Only openings the gates allow are taken
    /// (`Gates::passable`: a shut door, a force field or an AI-only opening is not a route). An empty
    /// vector means "already there"; `None` means unreachable, and the caller keeps its straight-line
    /// behaviour rather than freezing.
    pub fn route(&self, from: usize, to: usize, gates: Gates<'_>) -> Option<Vec<[f32; 3]>> {
        let n = self.portals.len();
        if from >= n || to >= n {
            return None;
        }
        if from == to {
            return Some(Vec::new());
        }
        let mut prev: Vec<Option<(usize, [f32; 3])>> = vec![None; n];
        let mut seen = vec![false; n];
        seen[from] = true;
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(from);
        while let Some(sector) = queue.pop_front() {
            for portal in self.portals.get(sector).into_iter().flatten() {
                let next = portal.adjoin;
                if next >= n || seen[next] || !gates.passable(sector, next, portal.link) {
                    continue;
                }
                seen[next] = true;
                // The middle of the opening: a body walks at the doorway, not at a corner of it.
                let mut centre = [0.0f32; 3];
                for v in &portal.verts {
                    for k in 0..3 {
                        centre[k] += v[k];
                    }
                }
                let count = portal.verts.len().max(1) as f32;
                for k in 0..3 {
                    centre[k] /= count;
                }
                prev[next] = Some((sector, centre));
                if next == to {
                    let mut out = Vec::new();
                    let mut at = to;
                    while let Some((back, centre)) = prev[at] {
                        out.push(centre);
                        at = back;
                    }
                    out.reverse();
                    return Some(out);
                }
                queue.push_back(next);
            }
        }
        None
    }

    /// The sectors reachable from `start` through openings the camera can see.
    ///
    /// This is the engine's visibility walk in its simplest safe form: breadth-first over
    /// the level's own `adjoin` links, admitting the sector behind a portal only while
    /// `sees` says that portal's polygon is on screen. It is **conservative by
    /// construction** — everything it returns is drawn, so a wrong `sees` can only add a
    /// room, never hide one that should show.
    ///
    /// `depth` bounds the walk: retail's recursion stops on portal count and area, this
    /// stops at a fixed number of rooms, which is enough for a 462-sector level and keeps
    /// a pathological chain of portals from walking the whole level every frame.
    /// `gates` is what the scripts switched: an opening whose adjoin row has lost `VISIBLE`
    /// (`ClearAdjoinFlags(visFace, 1)`, how `16a_crowadjoin.cog` hides the docking port) is not
    /// walked through. Closing an opening for movement does *not* hide it — a force field still
    /// shows the room behind it — so this reads the adjoin's own bit, not `SECTOR_ADJOINS_OFF`.
    pub fn visible_sectors(
        &self,
        seeds: &[usize],
        gates: Gates<'_>,
        sees: &mut dyn FnMut(&[[f32; 3]]) -> bool,
    ) -> Vec<usize> {
        const DEPTH: usize = 24;
        let mut seen = vec![false; self.portals.len()];
        let mut out = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        for &start in seeds {
            if start < seen.len() && !seen[start] {
                seen[start] = true;
                out.push(start);
                queue.push_back((start, 0usize));
            }
        }
        while let Some((sector, depth)) = queue.pop_front() {
            if depth >= DEPTH {
                continue;
            }
            for portal in self.portals.get(sector).into_iter().flatten() {
                if seen[portal.adjoin] || !gates.visible(portal.link) || !sees(&portal.verts) {
                    continue;
                }
                seen[portal.adjoin] = true;
                out.push(portal.adjoin);
                queue.push_back((portal.adjoin, depth + 1));
            }
        }
        out
    }
    /// Step the walker through a doorway if it has reached one. Returns the new sector.
    ///
    /// The walker stays in its sector until it crosses one of that sector's portals. That
    /// matters because sector hulls overlap (level 1 has 462 sectors, many of them thin
    /// trigger slivers), so "which hull am I inside" is not a reliable way to move — the
    /// level's own `adjoin` links are.
    pub fn advance(&self, sector: usize, pos: [f32; 3], radius: f32, gates: Gates<'_>) -> (usize, [f32; 3]) {
        for portal in self.portals.get(sector).into_iter().flatten() {
            if !gates.passable(sector, portal.adjoin, portal.link) {
                continue;
            }
            // The walker is at the opening when it is within its own radius of the
            // portal's plane. The sign is not trusted: `adjoin` gives the direction the
            // level intends, and a door is passed in both directions anyway.
            let d = dot(sub(pos, portal.point), portal.normal);
            if d.abs() > radius + 0.3 {
                continue;
            }
            if in_polygon(&portal.verts, portal.normal, pos) {
                let (resolved, _) = self.resolve(portal.adjoin, pos, radius);
                return (portal.adjoin, resolved);
            }
        }
        (sector, pos)
    }

    /// Push `pos` out of every solid plane of `sector` within `radius`. Returns the
    /// corrected position and whether anything was touched.
    /// Lateral collision against the sector's planes, exactly as before faces existed.
    ///
    /// This is the baseline the `--walk` sweeps measure (34/34 levels, 1077 crossings), so it
    /// stays available unchanged; [`Self::resolve_stepped`] is the variant the window uses.
    pub fn resolve(&self, sector: usize, pos: [f32; 3], radius: f32) -> ([f32; 3], bool) {
        self.resolve_with(sector, pos, radius, false)
    }

    /// Lateral collision that lets a body pass a face once its feet are above that face's top
    /// edge and it is standing beside it. The engine's rule, and what makes step-up possible.
    pub fn resolve_stepped(&self, sector: usize, pos: [f32; 3], radius: f32) -> ([f32; 3], bool) {
        self.resolve_with(sector, pos, radius, true)
    }

    fn resolve_with(&self, sector: usize, pos: [f32; 3], radius: f32, faces: bool) -> ([f32; 3], bool) {
        // Walls are polygons, not infinite planes. Three things have to hold before a face
        // pushes the body: the body's vertical span must overlap the face's, the body must be
        // beside it in plan, and it must be penetrating it. The first two are what make a low
        // ledge passable once the feet clear it — the engine's own rule, `moveSize + 0.01`
        // above the feet — where the old infinite-plane test blocked at every height.
        let Some(planes) = self.sectors.get(sector) else { return (pos, false) };
        let feet = pos[2] - radius;
        let head = pos[2] + radius;
        let mut out = pos;
        let mut hit = false;
        for _ in 0..3 {
            let mut moved = false;
            for plane in planes {
                // The sector's planes still bound the room — that is what keeps a body inside
                // it — but a plane whose surface has a polygon stops pushing as soon as the
                // body's feet are above that polygon's top edge, or the body is not beside it.
                // That exception is the whole of step-up: a 0.5-unit riser is a wall until the
                // feet clear 0.5, and open sky above it.
                if faces
                    && let Some(face) = self.wall_of.get(&plane.surface).and_then(|i| self.walls.get(*i))
                    && !face_blocks(face, feet, head, out, radius + 0.02)
                {
                    continue;
                }
                let d = dot(sub(out, plane.point), plane.normal);
                if d > -radius {
                    let push = d + radius;
                    out = [
                        out[0] - plane.normal[0] * push,
                        out[1] - plane.normal[1] * push,
                        out[2] - plane.normal[2] * push,
                    ];
                    moved = true;
                    hit = true;
                }
            }
            if !moved {
                break;
            }
        }
        (out, hit)
    }

    /// The floor a body at `pos` rests on, from the floor **polygons** rather than sector
    /// planes: the highest downward-facing face whose area the body is over.
    ///
    /// This is the query that a plane list cannot answer. Level 1's walkway is sector `218`,
    /// which owns no downward plane; the slab is a face at `-0.75` belonging to other sectors,
    /// and under the same point lie a wall's plane at `-1.12` and a pipe's inside at `-1.30`.
    /// Coverage — *is the body over this face* — tells them apart, which is exactly what the
    /// engine's own surface trace does.
    ///
    /// `drop` is how far below the feet a face may be and still count as support; a body in the
    /// air above a floor should not be caught by it.
    pub fn support(&self, pos: [f32; 3], radius: f32, drop: f32) -> Option<f32> {
        let feet = pos[2] - radius;
        let mut best: Option<f32> = None;
        for face in &self.floors {
            if face.normal[2].abs() < 0.7 {
                continue;
            }
            let d = dot(sub(pos, face.point), face.normal);
            let z = pos[2] - face.normal[2] * d;
            if z > feet + 0.02 || z < feet - drop {
                continue;
            }
            if !over_face(face, pos, radius) {
                continue;
            }
            if best.is_none_or(|near| z > near) {
                best = Some(z);
            }
        }
        best
    }

    /// The ceiling above `pos` in `sector`: the lowest upward-facing plane the point is below.
    ///
    /// The mirror of [`Self::floor_below`], and what the eye height has to fit under: a camera
    /// placed above the ceiling renders the inside of a solid, which is a black screen.
    pub fn ceiling_above(&self, sector: usize, pos: [f32; 3]) -> Option<f32> {
        let planes = self.sectors.get(sector)?;
        planes
            .iter()
            .filter(|p| p.normal[2] > 0.7)
            .map(|p| {
                let d = dot(sub(pos, p.point), p.normal);
                pos[2] - p.normal[2] * d
            })
            .filter(|z| *z >= pos[2] - 0.05)
            .min_by(f32::total_cmp)
    }

    /// Where a rising body's centre stops under a ceiling: the mirror of the floor contact
    /// (`ceiling - radius`), and never below where the body already is (a body above its own ceiling keeps
    /// its height). `None` when the rise fits, or when there is no ceiling to hit.
    pub fn rise_stop(ceiling: Option<f32>, start: f32, rise: f32, radius: f32) -> Option<f32> {
        let ceiling = ceiling?;
        (start + rise + radius > ceiling + 1e-4).then(|| (ceiling - radius).max(start))
    }

    /// The lowest upward-facing polygon above `pos`, within `height`. The mirror of
    /// `floor_under`, and polygon-precise on purpose: `ceiling_above` answers for the sector's planes as
    /// infinite surfaces, so a step beside the body reports a "ceiling" at its own height and stops a jump.
    pub fn ceiling_over(&self, pos: [f32; 3], height: f32, radius: f32) -> Option<f32> {
        let range = (height * 1.1).max(radius + 0.005);
        let mut best: Option<f32> = None;
        for face in &self.faces {
            if face.normal[2].abs() < 0.2 {
                continue;
            }
            let normal = if dot(sub(pos, face.point), face.normal) < 0.0 {
                [-face.normal[0], -face.normal[1], -face.normal[2]]
            } else {
                face.normal
            };
            if normal[2] > -0.2 {
                continue;
            }
            let Some(t) = sphere_plane(pos, [0.0, 0.0, 1.0], range, radius, normal, face.point) else {
                continue;
            };
            let at = [pos[0], pos[1], pos[2] + t];
            if !sphere_face(at, radius, face) {
                continue;
            }
            let ceiling = at[2] + radius;
            if std::env::var_os("BK_CEIL_TRACE").is_some() {
                eprintln!(
                    "ceiling: surface {} at z {ceiling:.3} (pos {:.3}) verts {:?} normal {:?}",
                    face.surface, pos[2], face.verts.first(), face.normal
                );
            }
            if best.is_none_or(|near| ceiling < near) {
                best = Some(ceiling);
            }
        }
        best
    }

    /// The floor under `pos` in `sector`: the highest downward-facing plane the point is
    /// above. A floor's outward normal points down, away from the room, and "down" is
    /// `-Z` because the world is Z-up (`CEILING SKY Z`, and 144 of `04escapehouse`'s 164
    /// sky faces are flat in Z).
    pub fn floor_below(&self, sector: usize, pos: [f32; 3]) -> Option<f32> {
        let planes = self.sectors.get(sector)?;
        planes
            .iter()
            .filter(|p| p.normal[2] < -0.7)

            // The plane's height directly under `pos`: exact for a sloped floor too,
            // unlike the plane's own point, which can be a corner far away.
            .map(|p| {
                let d = dot(sub(pos, p.point), p.normal);
                pos[2] - p.normal[2] * d
            })
            .filter(|z| *z <= pos[2] + 0.05)
            .max_by(f32::total_cmp)
    }
}

/// An 8 x 4 x 8 room, for tests: floor at z = -1, ceiling at z = 1, walls at |x|, |y| = 2.
/// Normals point out of the room, which is the convention `.jkl` data gives. Shared so a test outside this
/// module (main.rs's `walker_step`) can exercise the same geometry.
#[cfg(test)]
pub(crate) fn test_room() -> Solids {
    Solids {
        things: Default::default(),
        boxes: Vec::new(),
        hull: Vec::new(),
        floor_of: std::collections::HashMap::new(),
        sectors: vec![vec![
            Plane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, -1.0], surface: usize::MAX },
            Plane { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, 1.0], surface: usize::MAX },
            Plane { normal: [1.0, 0.0, 0.0], point: [2.0, 0.0, 0.0], surface: usize::MAX },
            Plane { normal: [-1.0, 0.0, 0.0], point: [-2.0, 0.0, 0.0], surface: usize::MAX },
            Plane { normal: [0.0, 1.0, 0.0], point: [0.0, 2.0, 0.0], surface: usize::MAX },
            Plane { normal: [0.0, -1.0, 0.0], point: [0.0, -2.0, 0.0], surface: usize::MAX },
        ]],
        spheres: vec![([0.0, 0.0, 0.0], 3.0)],
        portals: vec![Vec::new()],
        // The floor face, 4 x 4 at z = -1, so the polygon queries have something to cover.
        floors: vec![Face {
            normal: [0.0, 0.0, -1.0],
            point: [0.0, 0.0, -1.0],
            verts: vec![[-2.0, -2.0, -1.0], [2.0, -2.0, -1.0], [2.0, 2.0, -1.0], [-2.0, 2.0, -1.0]],
            surface: 0,
        }],
        // The four vertical faces, so `resolve` has polygons to push against.
        walls: vec![
            Face {
                normal: [1.0, 0.0, 0.0],
                point: [2.0, 0.0, 0.0],
                verts: vec![[2.0, -2.0, -1.0], [2.0, 2.0, -1.0], [2.0, 2.0, 1.0], [2.0, -2.0, 1.0]],
                surface: 1,
            },
            Face {
                normal: [-1.0, 0.0, 0.0],
                point: [-2.0, 0.0, 0.0],
                verts: vec![[-2.0, 2.0, -1.0], [-2.0, -2.0, -1.0], [-2.0, -2.0, 1.0], [-2.0, 2.0, 1.0]],
                surface: 2,
            },
        ],
        wall_of: [(1usize, 0usize), (2, 1)].into_iter().collect(),
        // The ceiling as a **face**, because the body step's ceiling test is polygon-precise.
        faces: vec![Face {
            normal: [0.0, 0.0, 1.0],
            point: [0.0, 0.0, 1.0],
            verts: vec![[-2.0, -2.0, 1.0], [2.0, -2.0, 1.0], [2.0, 2.0, 1.0], [-2.0, 2.0, 1.0]],
            surface: 3,
        }],
        face_of: std::collections::HashMap::new(),
        portal_faces: Vec::new(),
        surface_owner: Vec::new(),
        adjoin_flags: Vec::new(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_room as room;

    /// A floor made of two patches that meet at `x = 0`, with the **upward** normal the shipped
    /// `18Ascend`/`19A` floors carry (surface flags 0x80005, `n (0,0,+1)`) and the patches only in the
    /// `faces` list — `build()` files only `normal[2] < -0.7` faces as `floors`, so these are the
    /// landing probe's `faces` alone. Level 17's spawn stands at exactly such a seam.
    fn seam_floor() -> Solids {
        let patch = |x0: f32, x1: f32, surface: usize| Face {
            normal: [0.0, 0.0, 1.0],
            point: [x0, -2.0, 0.0],
            verts: vec![[x0, -2.0, 0.0], [x1, -2.0, 0.0], [x1, 2.0, 0.0], [x0, 2.0, 0.0]],
            surface,
        };
        Solids {
            things: Default::default(),
            boxes: Vec::new(),
            hull: Vec::new(),
            floor_of: std::collections::HashMap::new(),
            sectors: vec![vec![Plane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, 0.0], surface: usize::MAX }]],
            spheres: vec![([0.0, 0.0, 0.0], 8.0)],
            portals: vec![Vec::new()],
            floors: Vec::new(),
            walls: Vec::new(),
            wall_of: std::collections::HashMap::new(),
            faces: vec![patch(0.0, 2.0, 5618), patch(-2.0, 0.0, 5619)],
            face_of: std::collections::HashMap::new(),
        portal_faces: Vec::new(),
        surface_owner: Vec::new(),
        adjoin_flags: Vec::new(),
        ..Default::default()
        }
    }

    /// `sithIntersect_IsSphereInSector` snaps a plane distance within `1e-5` of zero to zero before
    /// comparing it with the radius (`_DAT_00523288` is 1e-5, `_DAT_00523284` is 0). A body resting on
    /// its own floor computes `floor + radius` in f32, which lands a ten-millionth outside the plane:
    /// the engine calls that inside, and so must the port (`18Ascend`'s spawn rejected every frame
    /// before this band existed, which flipped its sector into the shaft below).
    #[test]
    fn float_dust_outside_a_plane_is_still_inside_the_sector() {
        let room = seam_floor();
        assert!(
            room.inside_sector(0, [0.5, 0.0, 1e-7], 0.0),
            "a point a ten-millionth inside the plane is outside"
        );
        assert!(
            room.inside_sector(0, [0.5, 0.0, -1e-6], 0.0),
            "a point inside the engine's 1e-5 band was rejected"
        );
        assert!(
            !room.inside_sector(0, [0.5, 0.0, -2e-5], 0.0),
            "the band is not a licence to stand 2e-5 outside the room"
        );
    }

    #[test]
    fn a_body_on_a_two_patch_floor_seam_finds_the_floor() {
        // Level 17: body centre 0.063 above the floor, radius 0.065, standing on the shared edge of
        // two patches. The landing probe must find the floor; the 1e-3 "never lift" bound then keeps
        // it from being placed on it (the resting height is 0.120114 above the floor).
        let room = seam_floor();
        let floor = room.floor_under(0, [0.0, 0.0, 0.063], 0.12, 0.065);
        assert!(
            floor.is_some(),
            "the landing probe found no floor where two patches meet: {floor:?}"
        );
        let (height, _travelled, _owner) = floor.unwrap();
        assert!(
            (height - 0.0).abs() < 0.01,
            "the floor under a seam body is {height}, not the patch plane"
        );
    }
    /// Two rooms joined by one opening, and a third room reachable only through the second.
    fn corridor() -> Solids {
        let quad = |z: f32| {
            vec![[-1.0, 0.0, z], [1.0, 0.0, z], [1.0, 0.0, z + 1.0], [-1.0, 0.0, z + 1.0]]
        };
        Solids {
        things: Default::default(),
            boxes: Vec::new(),
            hull: Vec::new(),
            sectors: vec![Vec::new(), Vec::new(), Vec::new()],
            spheres: vec![([0.0, 0.0, 0.0], 4.0), ([0.0, 8.0, 0.0], 4.0), ([0.0, 16.0, 0.0], 4.0)],
            floors: Vec::new(),
            walls: Vec::new(),
            wall_of: std::collections::HashMap::new(),
        faces: Vec::new(),
        face_of: std::collections::HashMap::new(),
        portal_faces: Vec::new(),
        surface_owner: Vec::new(),
        adjoin_flags: Vec::new(),
            floor_of: std::collections::HashMap::new(),
            portals: vec![
                vec![Portal { adjoin: 1, link: 0, verts: quad(0.0), normal: [0.0, 1.0, 0.0], point: [0.0, 0.0, 0.0] }],
                vec![
                    Portal { adjoin: 0, link: 1, verts: quad(0.0), normal: [0.0, -1.0, 0.0], point: [0.0, 0.0, 0.0] },
                    Portal { adjoin: 2, link: 2, verts: quad(8.0), normal: [0.0, 1.0, 0.0], point: [0.0, 8.0, 0.0] },
                ],
                vec![Portal { adjoin: 1, link: 3, verts: quad(8.0), normal: [0.0, -1.0, 0.0], point: [0.0, 8.0, 0.0] }],
            ],
            ..Default::default()
        }
    }

    /// The engine's door rule, on a body rather than on flags: a doorway a script has **shut** is a wall
    /// to the walker, and one it has **opened** is a doorway.
    ///
    /// `Gates` is the whole of the state — `SECTOR_ADJOINS_OFF` on either room (what
    /// `SetSectorAdjoins(sector, 0)` sets, and what every shipped door script calls on startup) and the
    /// link's `ADJOIN_MOVE` — and `move_body` is the body's own mover, so this is the same decision the
    /// player's step makes. The fixture is the three-room corridor: rooms at `y = 0, 8, 16`, one opening
    /// between each pair.
    #[test]
    fn a_shut_doorway_stops_the_body_and_an_open_one_lets_it_through() {
        let solids = corridor();
        let radius = 0.065;
        // In the first room, facing the opening at y = 0, with a step long enough to carry the body through.
        let start = [0.0, -0.5, 0.5];
        let step = [0.0, 1.2, 0.0];
        let links = [ADJOIN_MOVE | ADJOIN_VISIBLE; 4];

        let shut = Gates { sectors: &[SECTOR_ADJOINS_OFF, 0, 0], adjoins: &links };
        let (_pos, sector, blocked) = solids.move_body(0, start, step, radius, 0.066, shut);
        assert_eq!(sector, 0, "a shut doorway must leave the body in its own room");
        assert!(blocked, "the body should report the contact that stopped it");

        let open = Gates { sectors: &[0, 0, 0], adjoins: &links };
        let (pos, sector, _) = solids.move_body(0, start, step, radius, 0.066, open);
        assert_eq!(sector, 1, "an open doorway must let the body into the next room, ended at {pos:?}");
        assert!(pos[1] > start[1], "the body should be past the opening, not at it: {pos:?}");

        // And the other side of the same rule: an opening a script shut with `ClearAdjoinFlags(x, 2)`.
        let no_move = [ADJOIN_VISIBLE; 4];
        let barred = Gates { sectors: &[0, 0, 0], adjoins: &no_move };
        let (_pos, sector, blocked) = solids.move_body(0, start, step, radius, 0.066, barred);
        assert_eq!(sector, 0, "an opening without `ADJOIN_MOVE` must stop the body");
        assert!(blocked, "a force field should report the contact");
    }

    /// Four walls and a floor, each with a polygon and the surface id its plane carries: the shape
    /// every sweep test needs, and what a sector looks like to `sweep`.
    fn boxed() -> Solids {
        let (x, y, z) = (2.0f32, 2.0f32, 1.0f32);
        let floors = vec![Face {
            normal: [0.0, 0.0, -1.0],
            point: [0.0, 0.0, -z],
            verts: vec![[-x, -y, -z], [x, -y, -z], [x, y, -z], [-x, y, -z]],
            surface: 0,
        }];
        let side = |n: [f32; 3], p: [f32; 3], verts: Vec<[f32; 3]>, surface: usize| Face {
            normal: n,
            point: p,
            verts,
            surface,
        };
        let walls = vec![
            side([1.0, 0.0, 0.0], [x, 0.0, 0.0], vec![[x, -y, -z], [x, y, -z], [x, y, z], [x, -y, z]], 1),
            side([-1.0, 0.0, 0.0], [-x, 0.0, 0.0], vec![[-x, -y, -z], [-x, y, -z], [-x, y, z], [-x, -y, z]], 2),
        ];
        let mut sectors = vec![Vec::new()];
        for face in floors.iter().chain(walls.iter()) {
            sectors[0].push(Plane { normal: face.normal, point: face.point, surface: face.surface });
        }
        // `faces` is what the floor probe scans; `build` fills it with every solid polygon, so a fixture
        // that leaves it empty cannot answer `floor_under` at all.
        let mut faces = floors.clone();
        faces.extend(walls.iter().cloned());
        let face_of = faces.iter().enumerate().map(|(i, f)| (f.surface, i)).collect();
        Solids {
        things: Default::default(),
            boxes: Vec::new(),
            hull: Vec::new(),
            sectors,
            portals: vec![Vec::new()],
            spheres: vec![([0.0, 0.0, 0.0], 4.0)],
            floors,
            walls,
            faces,
            face_of,
            portal_faces: Vec::new(),
            surface_owner: Vec::new(),
            adjoin_flags: Vec::new(),
            wall_of: [(1usize, 0usize), (2, 1)].into_iter().collect(),
            floor_of: [(0usize, 0usize)].into_iter().collect(),
            ..Default::default()
        }
    }

    #[test]
    fn the_sweep_stops_a_radius_short_of_a_wall() {
        // `sithIntersect_CheckSphereHit`: a body of radius 0.25 moving at the +x wall (x = 2) from the
        // origin travels 2 - 0.25 before it touches, and that is the whole point of the radius.
        let solids = boxed();
        let hit = solids
            .sweep(0, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 10.0, 0.25, Gates::default())
            .expect("the +x wall");
        assert!((hit.distance - 1.75).abs() < 1e-3, "distance {}", hit.distance);
        assert!((hit.normal[0] + 1.0).abs() < 1e-3, "normal {:?} should face the body", hit.normal);
    }

    #[test]
    fn the_sweep_misses_a_wall_the_sphere_never_reaches() {
        // Aiming down the middle of the room must not report either side wall.
        let solids = boxed();
        let hit = solids.sweep(0, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 1.0, 0.25, Gates::default());
        assert!(hit.is_none(), "reported {hit:?} along an open axis");
    }

    #[test]
    fn an_infinite_hull_does_not_import_a_floor_through_a_wall() {
        // The upper room's hull extends past its one solid wall. A lower room's hull also
        // contains that outside point, but none of its polygons does: this is level 1's
        // tabletop sliver, not a walkable connection to the cantina floor.
        let mut solids = boxed();
        solids.faces = solids.floors.clone();
        solids.faces.extend(solids.walls.iter().cloned());
        solids.surface_owner = vec![0, 0, 0];
        // No solid floor in the seed room: its only downward connection is the portal.
        solids.sectors[0].retain(|plane| plane.normal[2] > -0.7);
        solids.faces.retain(|face| face.normal[2].abs() < 0.7);
        solids.floors.clear();
        solids.sectors.push(vec![Plane {
            normal: [0.0, 0.0, -1.0],
            point: [0.0, 0.0, -1.5],
            surface: 3,
        }]);
        solids.spheres.push(([0.0, 0.0, -1.5], 4.0));
        let lower = Face {
            normal: [0.0, 0.0, -1.0],
            point: [0.0, 0.0, -1.5],
            verts: vec![[-2.0, -2.0, -1.5], [2.0, -2.0, -1.5], [2.0, 2.0, -1.5], [-2.0, 2.0, -1.5]],
            surface: 3,
        };
        solids.faces.push(lower);
        solids.surface_owner.push(1);
        let portal = Face {
            normal: [0.0, 1.0, 0.0],
            point: [0.0, -2.0, 0.0],
            verts: vec![[-1.0, -2.0, -1.0], [1.0, -2.0, -1.0], [1.0, -2.0, 1.0], [-1.0, -2.0, 1.0]],
            surface: usize::MAX,
        };
        solids.portals[0].push(Portal {
            adjoin: 1,
            link: 0,
            verts: portal.verts.clone(),
            normal: portal.normal,
            point: portal.point,
        });
        solids.portals.push(vec![Portal {
            adjoin: 0,
            link: 1,
            verts: portal.verts.clone(),
            normal: [-portal.normal[0], -portal.normal[1], -portal.normal[2]],
            point: portal.point,
        }]);
        solids.portal_faces = vec![(0, portal.clone()), (1, portal)];

        let outside = [3.0, 0.0, 0.0];
        assert!(
            solids.floor_under(0, outside, 1.0, 0.065).is_none(),
            "a neighbouring hull imported its floor outside the room's polygons"
        );
        let inside = [0.0, -2.0, 0.0];
        let (floor, _, owner) = solids
            .floor_under(0, inside, 2.0, 0.065)
            .expect("the actual portal polygon connects the two rooms");
        assert_eq!(owner, 1);
        assert!((floor + 1.5).abs() < 1e-3, "floor {floor}");
    }

    #[test]
    fn the_floor_is_found_through_a_portal() {
        // Two sectors: the upper one's floor is a *portal*, so it owns no downward plane. The slab a
        // body stands on belongs to the sector below — the case that made the old walker fall out of
        // `01narshadda`'s walkway.
        let mut solids = boxed();
        // Move the floor face down into a second sector and make the first sector's bottom a doorway.
        solids.floors[0].point = [0.0, 0.0, -1.5];
        solids.floors[0].verts = solids.floors[0]
            .verts
            .iter()
            .map(|v| [v[0], v[1], -1.5])
            .collect();
        solids.sectors.push(vec![Plane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, -1.5], surface: 0 }]);
        // Ownership follows the face's move, or the scoped probe correctly refuses to look in room 1.
        solids.surface_owner = vec![1];
        solids.spheres.push(([0.0, 0.0, -1.5], 4.0));
        solids.sectors[0].retain(|p| p.normal[2] > -0.7);
        // `faces` is what the floor probe scans and it was cloned before the slab moved: keep it in step,
        // or the probe answers with the slab's old position.
        solids.faces = solids.floors.clone();
        solids.faces.extend(solids.walls.iter().cloned());
        solids.portals[0].push(Portal {
            adjoin: 1,
            link: 0,
            verts: vec![[-2.0, -2.0, -1.0], [2.0, -2.0, -1.0], [2.0, 2.0, -1.0], [-2.0, 2.0, -1.0]],
            normal: [0.0, 0.0, -1.0],
            point: [0.0, 0.0, -1.0],
        });
        solids.portals.push(vec![Portal {
            adjoin: 0,
            link: 1,
            verts: vec![[-2.0, -2.0, -1.0], [2.0, -2.0, -1.0], [2.0, 2.0, -1.0], [-2.0, 2.0, -1.0]],
            normal: [0.0, 0.0, 1.0],
            point: [0.0, 0.0, -1.0],
        }]);
        let (floor, gap, sector) = solids
            .floor_under(0, [0.0, 0.0, -0.5], 1.0, 0.25)
            .expect("the floor below the doorway");
        assert_eq!(sector, 1, "the floor belongs to the lower sector");
        assert!((floor + 1.5).abs() < 1e-3, "floor {floor}");
        assert!((gap - 0.75).abs() < 1e-3, "gap {gap}");
    }
    #[test]
    fn the_floor_knows_which_surface_it_is() {
        let mut room = room();
        // The first plane is the -Z floor; tag it with a surface index and read it back.
        room.sectors[0][0].surface = 7;
        let (height, surface) = room.floor_surface(0, [0.0, 0.0, -0.95]).expect("floor");
        assert_eq!(surface, 7);
        assert!((height + 1.0).abs() < 1e-4, "height {height}");
    }

    /// The one floor rule answers the same resting height for the four shapes the motion callers
    /// used to resolve with different queries: a sector whose floor is only its plane, a walkway
    /// that is only a face, a ramp, and a solid thing's model top. The same body, at the same pose,
    /// rests at the same height on every one of them.
    #[test]
    fn a_plane_only_sector_a_face_only_walkway_a_ramp_and_a_prop_rest_at_one_height() {
        const R: f32 = 0.065;
        let body = [0.0, 0.0, R];
        let rest = |solids: &Solids| solids.resting_floor(0, body, 0.1, R);

        // 1. Plane only: the cell's planes are its whole floor, with no polygon anywhere.
        let plane_only = Solids {
            sectors: vec![vec![
                Plane { normal: [0.0, 0.0, -1.0], point: [0.0; 3], surface: usize::MAX },
                Plane { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, 2.0], surface: usize::MAX },
            ]],
            spheres: vec![([0.0; 3], 4.0)],
            portals: vec![Vec::new()],
            ..Default::default()
        };
        let plane = rest(&plane_only).expect("a plane-only sector has a floor");
        assert_eq!(plane.height, 0.0);
        assert!(plane.normal.is_none(), "a plane carries no face normal");

        // 2. Face only: `floor_below` is `None`, and the slab polygon is the whole floor.
        let walkway = Face {
            normal: [0.0, 0.0, 1.0],
            point: [0.0; 3],
            verts: vec![[-2.0, -2.0, 0.0], [2.0, -2.0, 0.0], [2.0, 2.0, 0.0], [-2.0, 2.0, 0.0]],
            surface: 1,
        };
        let face_only = Solids {
            sectors: vec![vec![Plane { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, 2.0], surface: 2 }]],
            spheres: vec![([0.0; 3], 4.0)],
            portals: vec![Vec::new()],
            faces: vec![walkway],
            face_of: [(1usize, 0usize)].into_iter().collect(),
            surface_owner: vec![0, 0, 0],
            ..Default::default()
        };
        assert!(face_only.floor_below(0, body).is_none(), "the fixture must carry no floor plane");
        let face = rest(&face_only).expect("a face-only walkway has a floor");
        assert_eq!(face.height, 0.0);

        // 3. Ramp: a 45-degree face, wound with the file's out-of-room normal. The body already
        // touches it at this pose, so the resting height matches; the normal is what tells the walker
        // which way the slope falls.
        let up = [std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2];
        let slant = Face {
            normal: [-up[0], 0.0, -up[2]],
            point: [0.0; 3],
            verts: vec![
                [-2.0, -2.0, 2.0],
                [2.0, -2.0, -2.0],
                [2.0, 2.0, -2.0],
                [-2.0, 2.0, 2.0],
            ],
            surface: 1,
        };
        let ramp = Solids {
            sectors: vec![vec![Plane { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, 2.0], surface: 2 }]],
            spheres: vec![([0.0; 3], 4.0)],
            portals: vec![Vec::new()],
            faces: vec![slant],
            face_of: [(1usize, 0usize)].into_iter().collect(),
            surface_owner: vec![0, 0, 0],
            ..Default::default()
        };
        let ramp = rest(&ramp).expect("a ramp is a floor");
        assert_eq!(ramp.height, 0.0);
        let normal = ramp.normal.expect("a ramp keeps its slope");
        assert!((normal[2] - up[2]).abs() < 1e-4, "the ramp normal must point up: {normal:?}");

        // 4. Solid prop: no level geometry at all, only the thing's model top.
        let prop = Solids::default();
        prop.set_things(vec![ThingSolid {
            centre: [0.0; 3],
            radius: 0.5,
            z_min: -1.0,
            z_max: 0.0,
            sector: 0,
            half: None,
            yaw: 0.0,
        }]);
        let top = rest(&prop).expect("a solid thing's top is a floor");
        assert_eq!(top.height, 0.0);
        assert!(top.normal.is_none(), "a thing's top carries no level normal");

        // One primitive, four surfaces, one resting height.
        assert_eq!(
            [plane.height, face.height, ramp.height, top.height],
            [0.0; 4],
            "the motion callers' surfaces must come to rest at the same height"
        );
    }

    #[test]
    fn walks_portals_only_where_the_camera_can_see() {
        let level = corridor();
        // Every portal on screen: the whole corridor, starting from the first room.
        let mut all = |_: &[[f32; 3]]| true;
        assert_eq!(level.visible_sectors(&[0], Gates::default(), &mut all), vec![0, 1, 2]);
        // Nothing on screen: just the room the camera is in.
        let mut none = |_: &[[f32; 3]]| false;
        assert_eq!(level.visible_sectors(&[0], Gates::default(), &mut none), vec![0]);
        // Only the first opening: the room behind it, but not the one past that.
        let mut first = |verts: &[[f32; 3]]| verts[0][2] < 4.0;
        assert_eq!(level.visible_sectors(&[0], Gates::default(), &mut first), vec![0, 1]);
        // The walk is a graph search, so a cycle back to the start adds nothing.
        let mut again = |_: &[[f32; 3]]| true;
        assert_eq!(level.visible_sectors(&[1], Gates::default(), &mut again).len(), 3);
        // Two seeds in the same room: the union, without duplicates.
        assert_eq!(level.visible_sectors(&[0, 1], Gates::default(), &mut again), vec![0, 1, 2]);
    }

    #[test]
    fn a_closed_adjoin_is_a_wall_and_a_hidden_one_is_not_walked() {
        let level = corridor();
        // Walk +y from the middle of room 0 into the first opening.
        let dir = [0.0, 1.0, 0.0];
        let from = [0.0, -1.0, 0.5];
        let open = level.sweep(0, from, dir, 3.0, 0.25, Gates::default());
        assert!(
            open.is_none_or(|h| h.sector == 1),
            "an open doorway is crossed, not hit: {open:?}"
        );

        // `ClearAdjoinFlags(link, 2)`: the force field is a wall at the opening.
        let mut adjoins = vec![ADJOIN_VISIBLE | ADJOIN_MOVE; 4];
        adjoins[0] &= !ADJOIN_MOVE;
        let gates = Gates { sectors: &[0; 3], adjoins: &adjoins };
        let hit = level
            .sweep(0, from, dir, 3.0, 0.25, gates)
            .expect("a closed adjoin stops the sweep");
        assert_eq!(hit.sector, 0, "the body never left its room");
        assert!(hit.distance < 1.0, "stopped at the opening: {}", hit.distance);

        // `SetAdjoinFlags(link, 16)`: AI-only, which the engine's move path treats as solid.
        let mut ai_only = vec![ADJOIN_VISIBLE | ADJOIN_MOVE; 4];
        ai_only[0] |= ADJOIN_AI_ONLY;
        let gates = Gates { sectors: &[0; 3], adjoins: &ai_only };
        assert!(level.sweep(0, from, dir, 3.0, 0.25, gates).is_some_and(|h| h.sector == 0));

        // `SetSectorAdjoins(0, 0)`: the sector's own adjoins are hidden, so nothing is crossed.
        let gates = Gates { sectors: &[SECTOR_ADJOINS_OFF, 0, 0], adjoins: &[0x7; 4] };
        assert!(level.sweep(0, from, dir, 3.0, 0.25, gates).is_some_and(|h| h.sector == 0));
        // ... and the doorway is shut from the far side too: this port has no thing collision for
        // the door leaf that would stop a body once it is inside the shut sector.
        let back = level.sweep(1, [0.0, 1.0, 0.5], [0.0, -1.0, 0.0], 3.0, 0.25, gates);
        assert!(back.is_some_and(|h| h.sector == 1), "{back:?}");

        // `ClearAdjoinFlags(link, 1)` hides the room behind it from the cull walk.
        let mut hidden = vec![0x7; 4];
        hidden[0] &= !ADJOIN_VISIBLE;
        let gates = Gates { sectors: &[0; 3], adjoins: &hidden };
        let mut all = |_: &[[f32; 3]]| true;
        assert_eq!(level.visible_sectors(&[0], gates, &mut all), vec![0]);
        // The far side keeps its own row, so the hidden opening is only one-way.
        assert_eq!(level.visible_sectors(&[1], gates, &mut all), vec![1, 0, 2]);
    }

    /// A body already touching a wall must still slide along it. The sweep reports a touching face at
    /// `distance: 0` for *any* direction that does not move away from it, so every loop iteration
    /// re-reports the same plane with no travel and three bounces end the move with the body wedged —
    /// level 18's body against surface 7332's plate at a doorway edge was exactly this (blocked true on
    /// every move while retail slid past it). A grazing contact is a slide, not a stop.
    #[test]
    fn a_body_touching_a_wall_slides_along_it() {
        let mut room = room();
        // The sweep walks faces; `room()` keeps its polygons in `walls`/`floors`.
        room.faces = room.walls.clone();
        room.faces.extend(room.floors.iter().cloned());
        let gates = Gates { sectors: &[], adjoins: &[] };
        // The +x wall is x = 2 and the body is one radius (0.3) clear of it at x = 1.7.
        let (pos, _sector, blocked) =
            room.move_body(0, [1.7, 0.0, -0.5], [0.0, 1.0, 0.0], 0.3, 0.066, gates);
        assert!(
            pos[1] > 0.5,
            "a body touching a wall did not slide along it: y {} (blocked {blocked})",
            pos[1]
        );
    }
    /// A walk into a 45° ramp is carried **up** it: the slide has to keep the direction's vertical
    /// component, and the sweep end its height. Dropping them left the body flush with the plane,
    /// re-hitting it at distance 0 every bounce — `16aEscapeShip`'s spawn ramp wedged after 0.09
    /// units while retail walked 1.56.
    #[test]
    fn a_body_walks_up_a_45_degree_ramp() {
        let mut room = room();
        let h = std::f32::consts::FRAC_1_SQRT_2;
        // The sweep resolves each hull plane's `surface` through `face_of`/`floor_of`/`wall_of`, so the
        // ramp needs both its plane and its polygon, the way the loader registers a level face.
        room.sectors[0].push(Plane { normal: [0.0, -h, h], point: [0.0, 1.0, 0.0], surface: 9 });
        room.faces.push(Face {
            normal: [0.0, -h, h],
            point: [0.0, 1.0, 0.0],
            verts: vec![[-1.0, 0.2, -0.8], [1.0, 0.2, -0.8], [1.0, 1.4, 0.4], [-1.0, 1.4, 0.4]],
            surface: 9,
        });
        room.face_of.insert(9, room.faces.len() - 1);
        let gates = Gates { sectors: &[], adjoins: &[] };
        let (pos, _sector, _blocked) =
            room.move_body(0, [0.0, 0.3, 0.0], [0.0, 1.0, 0.0], 0.1, 0.066, gates);
        assert!(pos[1] > 0.9, "the body did not advance up the ramp: {pos:?}");
        assert!(pos[2] > 0.05, "the body did not climb the ramp: {pos:?}");
    }

    #[test]
    fn pushes_out_of_walls_and_finds_the_floor() {
        let room = room();
        assert_eq!(room.sector_at([0.0, 0.0, 0.0], 0.05), Some(0));
        assert_eq!(room.sector_at([9.0, 0.0, 0.0], 0.05), None);

        // Walking into +X: the point is held one radius inside the wall at x = 2.
        let (out, hit) = room.resolve(0, [1.9, 0.5, 0.0], 0.3);
        assert!(hit);
        assert!((out[0] - 1.7).abs() < 1e-4, "x = {}", out[0]);

        // `resolve` is the plane baseline and still lifts the body off the floor; `support` is
        // what the walker uses for standing, and it reports the same floor.
        let (out, hit) = room.resolve(0, [0.0, 0.0, -0.95], 0.3);
        assert!(hit);
        assert!((out[2] + 0.7).abs() < 1e-4, "z = {}", out[2]);
        assert_eq!(room.support([0.0, 0.0, -0.5], 0.3, 1.0), Some(-1.0));

        // Away from every plane, nothing moves.
        let (out, hit) = room.resolve(0, [0.0, 0.0, 0.0], 0.3);
        assert!(!hit);
        assert_eq!(out, [0.0, 0.0, 0.0]);

        // The floor is the only downward-facing plane, at z = -1.
        let floor = room.floor_below(0, [0.0, 0.0, 0.5]).unwrap();
        assert!((floor + 1.0).abs() < 1e-4, "floor = {floor}");
        // A point below the floor is not standing on it.
        assert!(room.floor_below(0, [0.0, 0.0, -2.0]).is_none());

        // The ceiling is the mirror image, at z = +1, and it is what the eye must fit under.
        let ceiling = room.ceiling_above(0, [0.0, 0.0, -0.5]).unwrap();
        // A rise stops at the sphere's ceiling contact, never pushes a body down, and no ceiling means
        // no clamp. This is the jump-through-the-ceiling bug in predicate form.
        let radius = 0.065;
        assert_eq!(ceiling, 1.0);
        assert_eq!(Solids::rise_stop(Some(ceiling), 0.0, 0.5, radius), None, "a rise that fits under it");
        assert_eq!(
            Solids::rise_stop(Some(ceiling), 0.9, 0.2, radius),
            Some(1.0 - radius),
            "a bonk stops at the sphere contact"
        );
        assert_eq!(Solids::rise_stop(Some(ceiling), 1.5, 0.2, radius), Some(1.5), "a body above it keeps its height");
        assert_eq!(Solids::rise_stop(None, 0.0, 5.0, radius), None, "no ceiling, no clamp");
        // The ceiling query is polygon-precise: a step *beside* the body is not a ceiling over it.
        let mut with_step = test_room();
        with_step.faces.push(Face {
            normal: [0.0, 0.0, 1.0],
            point: [0.0, 0.0, 0.2],
            verts: vec![[1.5, -2.0, 0.2], [2.0, -2.0, 0.2], [2.0, 2.0, 0.2], [1.5, 2.0, 0.2]],
            surface: 4,
        });
        assert_eq!(
            with_step.ceiling_over([0.0, 0.0, 0.0], 1.2, radius),
            Some(1.0),
            "a step beside the body was taken for a ceiling"
        );
        assert_eq!(
            with_step.ceiling_over([1.75, 0.0, 0.0], 1.2, radius),
            Some(0.2),
            "under the step it is the ceiling"
        );
        assert!((ceiling - 1.0).abs() < 1e-4, "ceiling = {ceiling}");
        // A point above the ceiling has nothing above it.
        assert!(room.ceiling_above(0, [0.0, 0.0, 2.0]).is_none());
        // Headroom is the difference, which is the number the camera uses.
        let headroom = ceiling - (-0.5);
        assert!((headroom - 1.5).abs() < 1e-4, "headroom = {headroom}");
    }

    /// A neighbour sector joins the collision search only when the sweep reaches the opening into it:
    /// a body resting on a floor opening and walking level stays out of the room below (`18Ascend`'s
    /// spawn), while a body dropping through it, or walking into a wall opening, is searched there.
    #[test]
    fn a_neighbour_is_searched_only_when_the_sweep_reaches_its_opening() {
        let mut room = seam_floor();
        let opening = |normal: [f32; 3], adjoin: usize| Portal {
            adjoin,
            link: 0,
            verts: Vec::new(),
            normal,
            point: [0.0; 3],
        };
        // Sector 0's floor opens onto sector 1 below; its wall at x = 0 opens onto sector 2.
        room.portals = vec![vec![opening([0.0, 0.0, 1.0], 1), opening([1.0, 0.0, 0.0], 2)]];
        let r = 0.065;
        let level = [1.0, 0.0, 0.0];
        assert!(room.in_search(0, 0, [5.0, 0.0, 5.0], level, 0.1, r), "a body's own sector");
        assert!(
            !room.in_search(0, 1, [5.0, 0.0, r], level, 0.1, r),
            "a body standing on the floor opening searched the room below it"
        );
        assert!(
            room.in_search(0, 1, [5.0, 0.0, r], [0.0, 0.0, -1.0], 0.1, r),
            "a body dropping through the opening did not search the room below"
        );
        assert!(
            room.in_search(0, 2, [0.1, 0.0, 1.0], [-1.0, 0.0, 0.0], 0.1, r),
            "a body walking into the wall opening did not search the next room"
        );
        assert!(
            !room.in_search(0, 2, [3.0, 0.0, 1.0], [-1.0, 0.0, 0.0], 0.1, r),
            "a body far from the wall opening searched the next room"
        );
    }

    /// A **shut** opening's sector is the door leaf, and its faces stop the body even when the passage
    /// the body crosses is a different sector beside it (level 11's sector 9 leaf between 15 and 14;
    /// retail stops at y 13.19). An opening the scripts left passable stays a contact-free crossing.
    #[test]
    fn a_shut_opening_leaf_blocks_the_body_through_its_own_faces() {
        let mut room = seam_floor();
        let opening = |normal: [f32; 3], adjoin: usize| Portal {
            adjoin,
            link: 0,
            verts: vec![[-1.0, 0.0, -1.0], [1.0, 0.0, -1.0], [1.0, 0.0, 1.0], [-1.0, 0.0, 1.0]],
            normal,
            point: [0.0, 0.0, 0.0],
        };
        // Sector 0 opens into 1; 1 is shut, and its own wall sits just past the opening.
        room.portals = vec![vec![opening([0.0, 1.0, 0.0], 1)], Vec::new()];
        room.sectors = vec![
            vec![Plane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, 0.0], surface: usize::MAX }],
            vec![
                Plane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, 0.0], surface: usize::MAX },
                Plane { normal: [0.0, -1.0, 0.0], point: [0.0, 0.1, 0.0], surface: 77 },
            ],
        ];
        room.spheres = vec![([0.0, 0.0, 0.0], 8.0), ([0.0, 0.3, 0.0], 8.0)];
        room.walls = vec![Face {
            normal: [0.0, -1.0, 0.0],
            point: [0.0, 0.1, 0.0],
            verts: vec![[-1.0, 0.1, -1.0], [1.0, 0.1, -1.0], [1.0, 0.1, 1.0], [-1.0, 0.1, 1.0]],
            surface: 77,
        }];
        room.wall_of = [(77usize, 0usize)].into_iter().collect();
        let r = 0.065;
        let shut = Gates { sectors: &[0, crate::collide::SECTOR_ADJOINS_OFF], adjoins: &[0, 0] };
        // Passable: the body crosses into 1; the wall there is its own room's, so it is a contact too.
        // Shut: the *leaf's* faces stop it before the crossing is taken.
        let dir = [0.0, 1.0, 0.0];
        let before = room.sweep_rooms(0, [0.0, -0.2, 0.0], dir, 0.5, r, shut);
        let leaf_hit = before.0.map(|hit| hit.point[1]);
        assert!(
            leaf_hit.is_some_and(|y| y <= 0.1),
            "the shut leaf did not stop the body at its face: {leaf_hit:?}"
        );
    }

    /// A body that walks into a solid thing it already touches is **stopped**, not tunnelled
    /// through. Two door leaves stand in their own thin sector beside the opening they seal (level 1's
    /// cantina doors are in 260, two portal hops from the street the body walks), so the search has to
    /// see one hop past the rooms this sector opens into -- and the first contact leaves the body
    /// overlapping, which used to be skipped every frame after it.
    #[test]
    fn a_body_walks_into_an_overlapped_solid_thing_and_is_stopped() {
        let mut room = seam_floor();
        let opening = |normal: [f32; 3], adjoin: usize| Portal {
            adjoin,
            link: 0,
            verts: Vec::new(),
            normal,
            point: [0.0; 3],
        };
        // Sector 0 opens into 1, and the leaf's room 2 hangs off 1 only: two hops from the body.
        room.portals = vec![vec![opening([0.0, 1.0, 0.0], 1)], vec![opening([0.0, 1.0, 0.0], 2)], Vec::new()];
        room.sectors = vec![
            vec![Plane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, 0.0], surface: usize::MAX }],
            vec![Plane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, 0.0], surface: usize::MAX }],
            vec![Plane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, 0.0], surface: usize::MAX }],
        ];
        room.spheres = vec![([0.0, 0.0, 0.0], 8.0), ([0.0, 0.5, 0.0], 8.0), ([0.0, 1.0, 0.0], 8.0)];
        room.set_things(vec![ThingSolid {
            centre: [0.0, 0.3, -0.5],
            radius: 0.2,
            z_min: -1.0,
            z_max: 0.0,
            sector: 2,
            half: None,
            yaw: 0.0,
        }]);
        let r = 0.065;
        let dir = [0.0, 1.0, 0.0];
        // Touching already (the cylinder's span is 0.265): the sweep must still report the contact.
        let contact = room.sweep_things([0.0, 0.045, -0.5], dir, 0.5, r, 0);
        assert!(contact.is_some(), "a body already touching a leaf walked through it");
        let (travel, normal) = contact.unwrap();
        assert_eq!(travel, 0.0, "an overlapped solid let the body advance {travel}");
        assert!(normal[1] < 0.0, "the contact did not push the body back: {normal:?}");
        // Walking *out* of an overlap is the escape direction, not a new contact.
        assert!(
            room.sweep_things([0.0, 0.045, -0.5], [0.0, -1.0, 0.0], 0.5, r, 0).is_none(),
            "walking out of an overlap was called a collision"
        );
    }

    /// A solid thing's cylinder stops a sight ray: a bar counter between two bodies blocks the
    /// view the way the engine's collision ray does, while a ray that *starts* inside a cylinder
    /// (the actor's own body) is not blocked by it.
    #[test]
    fn a_solid_thing_blocks_the_sight_segment_but_not_from_inside_it() {
        let mut room = seam_floor();
        room.set_things(vec![ThingSolid {
            centre: [1.0, 0.0, -0.5],
            radius: 0.5,
            z_min: -1.0,
            z_max: 0.0,
            sector: 0,
            half: None,
            yaw: 0.0,
        }]);
        assert!(
            room.segment_thing([0.0, 0.0, -0.5], [2.0, 0.0, -0.5]),
            "a counter between the bodies did not block sight"
        );
        assert!(
            !room.segment_thing([1.0, 0.0, -0.5], [2.0, 0.0, -0.5]),
            "the ray started inside the cylinder and was called blocked"
        );
        assert!(
            !room.segment_thing([0.0, 0.0, 1.5], [2.0, 0.0, 1.5]),
            "a ray above the counter was blocked"
        );
        // A segment that ends inside a cylinder is aimed at that body: it is not an obstacle to it.
        assert!(
            !room.segment_thing([0.0, 0.0, -0.5], [1.0, 0.0, -0.5]),
            "the target's own body blocked the sight of itself"
        );
        // A sloped ray (an actor on a ledge looking down) is blocked too: the circle test must use
        // the ray's horizontal speed, not assume it is level.
        assert!(
            room.segment_thing([0.0, 0.0, 0.5], [2.0, 0.0, -1.5]),
            "a downward ray through the counter was not blocked"
        );
        // A ray that enters the circle above the counter's top and dips into it before leaving.
        assert!(
            room.segment_thing([0.4, 0.0, 0.1], [1.6, 0.0, -0.3]),
            "a ray dipping into the counter from above was not blocked"
        );
    }

    /// A `collide=3` door leaf is its model's 0.025-thick box, not the template's 0.2 `size=`
    /// cylinder: retail lets the body stand much closer to the leaf before it stops.
    #[test]
    fn a_thin_model_leaf_collides_as_its_box() {
        let mut room = seam_floor();
        room.set_things(vec![ThingSolid {
            centre: [1.0, 0.0, -0.5],
            radius: 0.2,
            z_min: -1.0,
            z_max: 0.0,
            sector: 0,
            half: Some([0.2, 0.0125]),
            yaw: 0.0,
        }]);
        let r = 0.065;
        // Broad side at y = -0.025: the body stops at 0.0125 + 0.065 from the centre, where the
        // old 0.2-radius cylinder stopped at 0.265.
        let (travel, normal) = room
            .sweep_things([1.0, -0.3, -0.5], [0.0, 1.0, 0.0], 0.5, r, 0)
            .expect("the leaf did not stop the body");
        assert!((travel - 0.2225).abs() < 1e-4, "the leaf stopped at {travel}");
        assert!(normal[1] < -0.99, "the leaf's normal was {normal:?}");
        // A path just past the leaf's 0.2 half width misses it; the old radius caught it.
        assert!(
            room.sweep_things([1.3, -0.3, -0.5], [0.0, 1.0, 0.0], 0.5, r, 0)
                .is_none(),
            "a body beside the leaf was stopped"
        );
        // The box turns with the thing's yaw: at 90 degrees the thin side faces world x.
        room.things.write().expect("things lock")[0].yaw = 90.0;
        let (travel, normal) = room
            .sweep_things([1.3, 0.0, -0.5], [-1.0, 0.0, 0.0], 0.5, r, 0)
            .expect("the turned leaf did not stop the body");
        assert!((travel - 0.2225).abs() < 1e-4, "the turned leaf stopped at {travel}");
        assert!(normal[0] > 0.99, "the turned leaf's normal was {normal:?}");
        // A body inside the leaf's inflated box is pushed out through the thin face.
        room.things.write().expect("things lock")[0].yaw = 0.0;
        let pushed = room.push_out_of_things([1.0, 0.0, -0.5], r, 0);
        assert!(
            (pushed[1].abs() - 0.0775).abs() < 1e-4,
            "the push put the body at {pushed:?}"
        );
        // Sight and shots use the same box.
        assert!(
            room.segment_thing([1.0, -0.3, -0.5], [1.0, 0.3, -0.5]),
            "the leaf did not block a ray across it"
        );
        assert!(
            !room.segment_thing([1.0, 0.0, -0.5], [1.0, 0.3, -0.5]),
            "the leaf blocked the ray of a body already inside it"
        );
    }

    /// `support` answers the question a plane list cannot: is the body *over* this face?
    #[test]
    fn support_needs_the_body_to_be_over_the_face() {
        let room = room();
        // Over the 4x4 floor at z = -1, from half a unit up.
        let over = room.support([0.0, 0.0, -0.5], 0.35, 1.0);
        assert_eq!(over, Some(-1.0), "support was {over:?}");
        // Beside it: the same height, no coverage, so nothing to stand on.
        assert_eq!(room.support([3.0, 0.0, -0.5], 0.35, 1.0), None);
        // Within a body radius of the edge still counts, or a walker on a lip would drop.
        assert_eq!(room.support([2.1, 0.0, -0.5], 0.35, 1.0), Some(-1.0));
        // Far above the floor, outside the `drop` window, it is not support.
        assert_eq!(room.support([0.0, 0.0, 4.0], 0.35, 1.0), None);
        // A face above the feet is never support: that is what made the walker climb.
        assert_eq!(room.support([0.0, 0.0, -1.4], 0.35, 1.0), None);
    }

    /// One build carries the surface maps, the runtime takes them, and the dynamic solid things are
    /// shared through the record instead of being copied with it.
    #[test]
    fn geometry_maps_and_things_are_shared_records() {
        let mut level = crate::jkl::Level::default();
        level.vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        level.surfaces = vec![crate::jkl::Surface {
            material: 0,
            surfflags: 0,
            geo: 4,
            adjoin: -1,
            verts: vec![(0, 0), (1, 1), (2, 2)],
            intensities: vec![1.0; 3],
            normal: [0.0, 0.0, 1.0],
        }];
        level.sectors = vec![
            crate::jkl::Sector { surfaces: (0, 1), ..Default::default() },
            crate::jkl::Sector { surfaces: (0, 1), ..Default::default() },
        ];

        let mut geometry = build(&level);
        assert_eq!(geometry.surface_claims[0], vec![0, 1], "all claimants were not kept");
        assert_eq!(geometry.surface_sectors[0], 1, "the last claimant was not recorded");

        // The runtime takes the supplied map rather than deriving its own.
        geometry.surface_sectors[0] = 7;
        let runtime = crate::runtime::Runtime::new_with_geometry(&level, &geometry);
        assert_eq!(runtime.surface_sector[0], 7);

        // `set_things` is the one mutable slice; a clone of the record sees it without a deep clone.
        let shared = geometry.clone();
        shared.set_things(vec![ThingSolid {
            centre: [1.0, 2.0, 3.0],
            radius: 0.2,
            z_min: -1.0,
            z_max: 0.0,
            sector: 0,
            half: None,
            yaw: 0.0,
        }]);
        assert_eq!(geometry.things().len(), 1, "the geometry copy did not share its things");
    }

    /// The sector-adjoin graph is built from the same mirrors the level ships, once.
    #[test]
    fn geometry_pairs_the_mirrored_adjoins() {
        let face = |adjoin: i32| crate::jkl::Surface {
            material: 0,
            surfflags: 0,
            geo: 4,
            adjoin,
            verts: vec![(0, 0), (1, 1), (2, 2)],
            intensities: vec![1.0; 3],
            normal: [0.0, 0.0, 1.0],
        };
        let mut level = crate::jkl::Level::default();
        level.vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        level.surfaces = vec![face(0), face(1)];
        level.sectors = vec![
            crate::jkl::Sector { surfaces: (0, 1), ..Default::default() },
            crate::jkl::Sector { surfaces: (1, 1), ..Default::default() },
        ];
        level.adjoins = vec![
            crate::jkl::Adjoin { flags: 0x7, mirror: 1, dist: 2.0 },
            crate::jkl::Adjoin { flags: 0x7, mirror: 0, dist: 3.0 },
        ];

        let geometry = build(&level);
        assert_eq!(geometry.adjoin_surface, vec![0, 1]);
        assert_eq!(
            geometry.sector_adjoins,
            vec![vec![(1, 3.0)], vec![(0, 3.0)]],
            "one opening carries the mirror row's distance for both directions"
        );
    }
}
