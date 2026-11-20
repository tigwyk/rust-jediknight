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

/// One surface of a sector's own `SURFACES` range, as `sithIntersect_IsSphereInSector`
/// (JK.EXE `0x507f30`) walks it.
#[derive(Clone, Copy, Debug)]
pub struct ContainPlane {
    pub normal: [f32; 3],
    pub point: [f32; 3],
    pub surfflags: u32,
    /// The `World adjoins` row the surface names, `-1` when it is not an opening.
    pub adjoin: i32,
}

/// A placed thing the collision search meets (`sithCollision_SearchForThingCollisions`, 0x4e8430).
///
/// `half` is the oriented plan box of a `collide=3` thing's loaded model — used by the sight
/// segment test only; movement uses the mesh (`0x5080d0`) or the `size=` sphere (`+0x30`).
#[derive(Clone, Debug)]
pub struct ThingSolid {
    /// Index in the level/runtime thing array, before filtering non-solid things.
    pub index: usize,
    /// Collision centre in plan, and the thing's own z for the span's origin.
    pub centre: [f32; 3],
    /// Authored thing position (`+0xb8`), independent of the plan box's offset centre.
    pub origin: [f32; 3],
    pub radius: f32,
    pub z_min: f32,
    pub z_max: f32,
    pub sector: usize,
    pub half: Option<[f32; 2]>,
    /// Degrees, the engine's `rdMatrix_Build34` yaw.
    pub yaw: f32,
    /// `collide=3`: the model's collision faces in world space (outward normals), placed under the
    /// thing's full pitch/yaw/roll. `sithIntersect_CheckSphereThingIntersection` (`0x5080d0`) sweeps
    /// these instead of a sphere, so a door leaf laid flat (`18Ascend` things 323/324) is a floor.
    pub mesh: Option<std::sync::Arc<[Vec<Face>]>>,
    /// The thingflags word (`+0x0`): the search gates read `0x80` (flag 8), `0x40` (flag `0x10`)
    /// and `0x80002` straight off it (`0x4e8430:33-35`).
    pub thing_flags: u32,
    /// The `collide=` word (`+0x28`): 3 selects the mesh arm of `0x5080d0`, 0 is not admitted.
    pub collide: u32,
    /// The engine's thing type (`+0xc`; `_decor` is `type=cog` = 7, so furniture is 7), for the
    /// `0x2000` type gate and the pair matrix.
    pub kind: u32,
    /// The owner thing (`+0x80`) a spawned weapon remembers; placed things have none. The
    /// owner-chain exclusion (`0x4e8430:79-88`) compares these.
    pub owner: Option<usize>,
    /// Type 5 (`item`): the (5,10) pair handler `0x4fbe10` always returns 0 — a walker meets an
    /// item's volume, records the contact and passes through; items never stop a move.
    pub item: bool,
}

impl ThingSolid {
    /// World point to the thing's own plan frame.
    fn local(&self, p: [f32; 2]) -> [f32; 2] {
        let (sin, cos) = self.yaw.to_radians().sin_cos();
        let (x, y) = (p[0] - self.centre[0], p[1] - self.centre[1]);
        [x * cos + y * sin, -x * sin + y * cos]
    }


}

#[derive(Clone, Default)]
pub struct LevelGeometry {
    /// Per sector, the planes of its solid surfaces.
    pub sectors: Vec<Vec<Plane>>,
    /// Per sector, only the planes that **bound** the cell: every vertex the sector claims is behind
    /// them. `sectors` carries every claimed surface's plane, because a wall patch inside the room
    /// still has to stop a body. Containment is not asked of either list: `containment` carries
    /// the sector's own surfaces as `sithIntersect_IsSphereInSector` (JK.EXE `0x507f30`) walks
    /// them, and `clearance` reads this hull.
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
    /// The level's surface flag words by surface id. `sithPhysics_FindFloor` sets its search's
    /// `0x10` bit for a thing without physflag `0x80` — the `walkplayer` inherits `_actor`'s
    /// `physflags=0x4a4f`, which has `0x80` clear — so the player's floor search counts only
    /// surfaces with flag bit 1 (`sithCollision_SearchForSurfaceCollisions`, `0x4e86d0` lines
    /// 125-126). Wall-only faces carry `0x4` without the bit and are never floors:
    /// `09fuelstation`'s 40-degree pit-mouth face (surface 9278, flags `0x4`).
    pub surface_flags: Vec<u32>,
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
    /// Per sector, its own `SURFACES` range as `sithIntersect_IsSphereInSector` (JK.EXE
    /// `0x507f30`) walks it: every surface's plane (normal pointing out of this sector), flag
    /// word and `World adjoins` row. The walk counts a surface when the flags carry
    /// `HAS_COLLISION` (`0x4`) or the adjoin row allows movement (`0x2`) — decided at call
    /// time, because doors clear the move flag at runtime.
    pub containment: Vec<Vec<ContainPlane>>,
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

    /// `sithCollision_HasLOS` (0x4e7500) as the item handler `0x4fbe10` uses it: with flags
    /// `0x2122` the search admits surfaces and **cog-type things only** (`0x4e8430`'s `0x2000`
    /// filter), and the segment is clear when nothing meets it before the target — the first
    /// non-self hit must be the player itself. Bit `0x1` skips the port's thing arm; the type-7
    /// blockers are tested directly here, as their own radius-0 sphere sweep.
    pub fn item_has_los(
        &self, sector: usize, from: [f32; 3], to: [f32; 3], gates: Gates<'_>,
    ) -> bool {
        let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let dist = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        if dist <= 1e-5 {
            return true;
        }
        let dir = [delta[0] / dist, delta[1] / dist, delta[2] / dist];
        let mut search = CollisionSearch::new();
        self.search_for_collisions(sector, None, from, dir, dist, 0.0, gates, 0x1 | 0x22, &mut search);
        for index in 0..search.count {
            if let Some(record) = search.records[index] {
                if record.distance() < dist - 1e-4 {
                    return false;
                }
            }
        }
        let things = self.things();
        for thing in things.iter() {
            if thing.kind != 7 {
                continue;
            }
            if let Some(entry) = sphere_intersection(from, dir, dist, 0.0, thing.centre, thing.radius, false, 0)
            {
                if entry < dist - 1e-4 {
                    return false;
                }
            }
        }
        true
    }
}

/// A surface with its polygon, for the queries that need coverage rather than a plane.
///
/// A sector's planes are infinite, which is why a wall blocks at every height and why a sector
/// that merely *shares an edge* with the floor cannot be told from one that is above it. The
/// polygon is the only thing that answers "is the body over this face".
#[derive(Clone, Debug)]
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
    let mut contains = Vec::with_capacity(level.sectors.len());
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
        let mut contain = Vec::new();
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
            // `(0, 0, 1)`, up). This port stores the opposite, outward convention.
            // Fixtures without normals fall back to their polygon.
            let normal = {
                if dot(surface.normal, surface.normal) > 0.25 {
                    [-surface.normal[0], -surface.normal[1], -surface.normal[2]]
                } else {
                    let Some(mut normal) = polygon_normal(&pts) else {
                        continue;
                    };
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
                    normal
                }
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
            // `sithIntersect_IsSphereInSector` (`0x507f30`) walks the sector's own surface list
            // whatever the adjoin mirror table says, so every surface with a plane lands here and
            // the counting rule (`0x4` or a move-flagged adjoin row) is applied at call time.
            contain.push(ContainPlane {
                normal: own,
                point: pts[0],
                surfflags: surface.surfflags,
                adjoin: surface.adjoin,
            });
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
                    if surface.surfflags & 0x4 != 0 || level.adjoins.get(link).is_some_and(|a| a.flags & 0x2 != 0) {
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
        contains.push(contain);
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
                verts: portal.verts.iter().rev().copied().collect(),
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
    let surface_flags: Vec<u32> = level.surfaces.iter().map(|s| s.surfflags).collect();
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
        containment: contains,
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
        surface_flags,
        adjoin_flags: level.adjoins.iter().map(|a| a.flags).collect(),
        surface_claims,
        surface_sectors,
        adjoin_surface,
        sector_adjoins,
        things: std::sync::Arc::new(std::sync::RwLock::new(Vec::new())),
    }
}

/// What a body is attached to (`sithThing` `+0x34`/`+0x38`/`+0x44`): the floor's into-room normal and a
/// point on its plane, the surface when it is one (a thing face has none), and the drag scale its surface
/// flags give (`0x4f7430`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attach {
    pub normal: [f32; 3],
    pub point: [f32; 3],
    pub surface: Option<usize>,
    pub thing: Option<ThingHit>,
    pub friction: f32,
}

/// The selected target and its optional mesh/face (`sithCollision` record `+4/+0xc/+0x10`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThingHit {
    pub thing: usize,
    pub mesh_face: Option<(usize, usize)>,
    /// Set from [`ThingSolid::item`] at search time, so the move arm can pass an item through
    /// without a solid lookup.
    pub item: bool,
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
    kind: u32,
    pub thing: Option<ThingHit>,
}

#[derive(Clone, Copy)]
enum CollisionRecord {
    Solid(Sweep),
    Crossing { distance: f32, sector: usize, surface: usize },
    Adjoin { distance: f32, sector: usize },
}

impl CollisionRecord {
    fn distance(self) -> f32 {
        match self {
            Self::Solid(hit) => hit.distance,
            Self::Crossing { distance, .. } | Self::Adjoin { distance, .. } => distance,
        }
    }

    fn kind(self) -> u32 {
        match self {
            Self::Solid(hit) => hit.kind,
            Self::Crossing { .. } => 0x20,
            Self::Adjoin { .. } => 0x40,
        }
    }
}

/// `sithCollision_PopStack` (0x4e7120): nearest first, preferring a face over an edge or vertex on a tie.
fn pop_collision(records: &mut [Option<CollisionRecord>]) -> Option<CollisionRecord> {
    let mut best: Option<usize> = None;
    for (index, candidate) in records.iter().enumerate() {
        let Some(candidate) = candidate else { continue };
        if best.is_none_or(|previous| {
            let current = records[previous].unwrap();
            candidate.distance() < current.distance()
                || (candidate.distance() == current.distance()
                    && current.kind() & 0x18 != 0
                    && candidate.kind() & 4 != 0)
        }) {
            best = Some(index);
        }
    }
    records[best?].take()
}

/// `sithCollision`'s 128 records and 64 searched sectors, owned by one search frame.
pub struct CollisionSearch {
    records: [Option<CollisionRecord>; 128],
    count: usize,
    sectors: [usize; 64],
    sector_count: usize,
    excluded_thing: Option<usize>,
}

impl CollisionSearch {
    fn new() -> Self {
        Self { records: [None; 128], count: 0, sectors: [0; 64], sector_count: 0, excluded_thing: None }
    }

    pub(crate) fn exclude_thing(&mut self, index: usize) {
        self.excluded_thing = Some(index);
    }

    fn push(&mut self, record: CollisionRecord) {
        if self.count != self.records.len() {
            self.records[self.count] = Some(record);
            self.count += 1;
        }
    }

    fn add_sector(&mut self, sector: usize) -> bool {
        if self.sector_count == self.sectors.len() || self.sectors[..self.sector_count].contains(&sector) {
            return false;
        }
        self.sectors[self.sector_count] = sector;
        self.sector_count += 1;
        true
    }

    fn pop(&mut self) -> Option<CollisionRecord> {
        pop_collision(&mut self.records[..self.count])
    }
}

/// The mutable physics fields read and written by `sithCollision_MoveThing` (0x4e7950).
pub struct MoveState {
    pub pos: [f32; 3],
    pub sector: usize,
    pub vel: [f32; 3],
    pub remainder: [f32; 3],
    pub phys_flags: u32,
    pub kind: u32,
    pub move_type: u32,
    pub collide: u32,
    pub flags: u32,
    pub attach_flags: u32,
    pub attach: Option<Attach>,
    /// The owner thing (`+0x80`) a spawned weapon remembers; the walker has none. The
    /// owner-chain exclusion of `0x4e8430:79-88` compares it against each hit's owner.
    pub owner: Option<usize>,
    /// The water-surface distance `sithPhysics_FindFloor`'s water branch records (`thing+0x48`):
    /// how far up the nearest dry-side crossing sits. `None` is the engine's "no crossing found",
    /// which clears the in-water hold.
    pub water_surface: Option<f32>,
    /// Level indexes of item things (type 5) whose volume the sweep met this move — the
    /// `0x4fbe10` pass-through contact list the pickup gate consumes.
    pub touched: Vec<usize>,
}

impl MoveState {
    pub fn search_flags(&self, mut flags: u32) -> u32 {
        if self.collide == 0 { flags |= 5; }
        if self.move_type == 2 { flags |= 4; }
        if self.kind == 10 { flags |= 0x200; }
        if flags & 1 == 0 { flags |= 0x800; }
        flags
    }
}

/// `sithCollision_Startup`'s pair-handler table (`DAT_00847f38`, indexed `mover*12 + hit`):
/// whether the installed handler cell is non-zero. `sithCollision_AddCollisionHandler`
/// (`0x4e6f40`) mirrors an asymmetric pair into the transposed cell with the swapped flag, so
/// `(5,10)` also admits `(10,5)` — the player meets an item's volume.
///
/// Handlers, for the ledger: `LAB_004ed210` (2,2)/(2,10)/(10,2)/(2,7)/(7,2);
/// `sithCollision_ThingCollisionHandler` (7,10)/(10,7)/(10,10)/(4,4);
/// `ParticleAndActorCollisionHandler` (4,2)/(2,4)/(4,10)/(10,4); `LAB_004d48c0` the type-3
/// pairs; `LAB_004fbe10` (5,10)/(10,5).
fn pair_handler_present(mover: u32, hit: u32) -> bool {
    matches!((mover, hit),
        (2, 2) | (2, 10) | (10, 2) | (2, 7) | (7, 2)
        | (7, 10) | (10, 7) | (10, 10) | (4, 4)
        | (4, 2) | (2, 4) | (4, 10) | (10, 4)
        | (3, 2) | (2, 3) | (3, 10) | (10, 3) | (3, 4) | (4, 3) | (3, 7) | (7, 3)
        | (5, 10) | (10, 5))
}

/// The mover-half gates of `0x4e8430:76-95` (a `mover != 0` search only):
/// - `:77-78` the pair-handler cell (`mover*12 + hit`) must be non-zero, and neither thing may
///   carry thingflag `0x200`;
/// - `:79-88` the weapon/owner chain: a type-3 mover (or hit) with the `+0x130` weapon bit skips
///   a hit that shares its owner (`+0x80`) and master (`+0x84`). Every ported mover is the
///   type-10 walker (no owner) and placed things have none, so the `+0x84`/`+0x74` comparison
///   never sees a non-zero word — the owner equality below is the whole chain for shipped data;
/// - `:89-94` the attach exclusion: a mover attached (`+0x34 & 6`) to this very thing is skipped
///   when its attach flags carry `8` or the search carries `0x40`.
fn mover_thing_admitted(mover: &MoveState, hit: &ThingSolid, flags: u32) -> bool {
    if !pair_handler_present(mover.kind, hit.kind) {
        return false;
    }
    if mover.flags & 0x200 != 0 || hit.thing_flags & 0x200 != 0 {
        return false;
    }
    if (mover.kind == 3 || hit.kind == 3) && mover.owner.is_some() && mover.owner == hit.owner {
        return false;
    }
    if mover.attach_flags & 6 != 0
        && mover.attach.is_some_and(|attach| attach.thing.is_some_and(|t| t.thing == hit.index))
        && (mover.attach_flags & 8 != 0 || flags & 0x40 != 0)
    {
        return false;
    }
    true
}

/// `sithIntersect_CheckSphereIntersection` (0x508540): the sphere gate before a thing's mesh test.
#[allow(clippy::too_many_arguments)]
fn sphere_intersection(
    start: [f32; 3],
    dir: [f32; 3],
    dist: f32,
    radius: f32,
    centre: [f32; 3],
    thing_radius: f32,
    mesh: bool,
    flags: u32,
) -> Option<f32> {
    let span = thing_radius + radius;
    let offset = sub(centre, start);
    if dist != 0.0 {
        let along = dir[0] * offset[0] + dir[1] * offset[1] + dir[2] * offset[2];
        if 0.0 <= along && along <= span + dist {
            let offset = sub(centre, [
                dir[0] * along + start[0],
                dir[1] * along + start[1],
                dir[2] * along + start[2],
            ]);
            let perpendicular = (offset[2] * offset[2] + offset[1] * offset[1] + offset[0] * offset[0]).sqrt();
            if span <= perpendicular {
                return None;
            }
            let entry = along - (span * span - perpendicular * perpendicular).sqrt();
            return Some(if entry <= dist && 0.0 <= entry { entry } else { 0.0 });
        }
        if !mesh {
            return None;
        }
    }
    let distance = (offset[2] * offset[2] + offset[1] * offset[1] + offset[0] * offset[0]).sqrt();
    if span <= distance {
        return None;
    }
    Some(if flags & 0x400 == 0 { 0.0 } else { distance })
}

/// `sithIntersect_CheckSphereHit` (0x508be0): the travel distance at which a sphere first meets a plane.
///
/// An overlapping sphere returns zero, except flag `0x400` adds its radius to the caller's output.
/// The output survives rejected faces, matching the engine's shared scratch-slot protocol.
fn sphere_plane(
    start: [f32; 3],
    dir: [f32; 3],
    dist: f32,
    radius: f32,
    normal: [f32; 3],
    point: [f32; 3],
    flags: u32,
    out_distance: &mut f32,
) -> Option<f32> {
    let p = sub(start, point);
    let above = p[0] * normal[0] + p[2] * normal[2] + p[1] * normal[1];
    let above = if above.abs() <= 1e-5 { 0.0 } else { above };
    if above < 0.0 {
        return None;
    }
    let gap = above - radius;
    if gap > dist {
        return None;
    }
    if gap < 0.0 {
        *out_distance = if flags & 0x400 != 0 { *out_distance + radius } else { 0.0 };
        return Some(*out_distance);
    }
    let approach = -(dir[0] * normal[0] + dir[2] * normal[2] + dir[1] * normal[1]);
    if approach <= 0.0 || approach * dist < gap {
        return None;
    }
    *out_distance = (gap / approach).max(0.0);
    Some(*out_distance)
}

/// `sithIntersect_TestSphereFaceHit`: is the sphere touching the polygon — inside it, or within
/// `radius` of one of its edges?
///
/// The engine projects to the two axes the normal is not dominant on and walks the edges. Because the
/// `.jkl` winding cannot be trusted, this asks the polygon's own centre which side is inside and
/// then measures how far outside the sphere reaches — the same test with no winding assumption.
fn sphere_face(pos: [f32; 3], radius: f32, face: &Face) -> bool {
    sphere_face_parts(pos, radius, face.normal, &face.verts)
}

fn sphere_face_parts(pos: [f32; 3], radius: f32, normal: [f32; 3], verts: &[[f32; 3]]) -> bool {
    let n = verts.len();
    if n < 3 {
        return false;
    }
    let mut centre = [0.0f32; 3];
    for v in verts {
        centre = [centre[0] + v[0], centre[1] + v[1], centre[2] + v[2]];
    }
    let centre = [centre[0] / n as f32, centre[1] / n as f32, centre[2] / n as f32];
    // The two axes to work in: drop the one the normal is largest on.
    let axis = {
        let n = normal;
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
        let a = flat(verts[i]);
        let b = flat(verts[(i + 1) % n]);
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
    outside <= radius
}

/// `sithIntersect_TestSphereFaceHit` (0x508750): return the outside-edge mask.
fn face_edge_mask(pos: [f32; 3], radius: f32, inward: [f32; 3], verts: &[[f32; 3]]) -> Option<u32> {
    let abs = [inward[0].abs(), inward[1].abs(), inward[2].abs()];
    let axis = if abs[0] <= abs[1] {
        if abs[2] < abs[1] { 1 } else { 2 }
    } else if abs[2] < abs[0] {
        0
    } else {
        2
    };
    let axes = [[2, 1], [0, 2], [1, 0]][axis];
    let (x, y) = if inward[axis] <= 0.0 { (axes[1], axes[0]) } else { (axes[0], axes[1]) };
    let mut mask = 0u32;
    for (i, a) in verts.iter().enumerate() {
        let b = verts[(i + 1) % verts.len()];
        let edge = [b[x] - a[x], b[y] - a[y]];
        let cross = (pos[x] - a[x]) * edge[1] - (pos[y] - a[y]) * edge[0];
        if cross < 0.0 {
            let length = (edge[0] * edge[0] + edge[1] * edge[1]).sqrt();
            if radius == 0.0
                || (pos[x] - a[x]) * (edge[1] / length) - (pos[y] - a[y]) * (edge[0] / length) < -radius
            {
                return None;
            }
            mask |= 1u32.checked_shl(i as u32).unwrap_or(0);
        }
    }
    Some(mask)
}

impl LevelGeometry {
    /// Search touched sectors and return the nearest solid contact.
    pub fn sweep(
        &self,
        sector: usize,
        start: [f32; 3],
        dir: [f32; 3],
        dist: f32,
        radius: f32,
        gates: Gates<'_>,
    ) -> Option<Sweep> {
        let mut search = CollisionSearch::new();
        self.search_for_collisions(sector, None, start, dir, dist, radius, gates, 1, &mut search);
        let mut here = sector;
        while let Some(record) = search.pop() {
            match record {
                CollisionRecord::Crossing { sector, .. } => here = sector,
                CollisionRecord::Solid(mut hit) => {
                    hit.sector = here;
                    return Some(hit);
                }
                CollisionRecord::Adjoin { .. } => unreachable!(),
            }
        }
        None
    }

    /// `sithCollision_SearchForCollisions` (0x4e8160): one shared thing/surface/adjoin stack.
    /// `mover` is the engine's `param_2`: the moving thing whose pair gates the thing search
    /// applies, or `None` for a mover-less probe (`FindFloor` `0x4f5550` passes 0).
    #[allow(clippy::too_many_arguments)]
    pub fn search_for_collisions(
        &self, sector: usize, mover: Option<&MoveState>, start: [f32; 3], dir: [f32; 3], dist: f32, radius: f32,
        gates: Gates<'_>, flags: u32, search: &mut CollisionSearch,
    ) -> f32 {
        search.count = 0;
        search.sector_count = 0;
        search.add_sector(sector);
        let things = self.things();
        let excluded = search.excluded_thing;
        if flags & 1 == 0 {
            self.thing_contacts(&things, mover, start, dir, dist, radius, sector, flags, excluded, |_, hit| search.push(CollisionRecord::Solid(hit)));
        }
        self.search_for_surface_collisions(sector, start, dir, dist, radius, gates, flags, search);
        let mut index = 0;
        while index < search.count {
            if let Some(CollisionRecord::Adjoin { distance, sector }) = search.records[index] {
                if (flags & 0x400 != 0 || distance <= dist) && search.add_sector(sector) {
                    if flags & 1 == 0 {
                        self.thing_contacts(&things, mover, start, dir, dist, radius, sector, flags, excluded, |_, hit| search.push(CollisionRecord::Solid(hit)));
                    }
                    self.search_for_surface_collisions(sector, start, dir, dist, radius, gates, flags, search);
                }
                search.records[index] = None;
            }
            index += 1;
        }
        // Startup installs no distance-reduction predicates (0x4e6d90); thing hits retain `dist`.
        if dist != 0.0 && flags & 0x800 != 0 {
            let count = search.sector_count;
            for index in 0..count {
                let here = search.sectors[index];
                for portal in self.portals.get(here).into_iter().flatten() {
                    let adjoin_flags = gates.adjoins.get(portal.link).or_else(|| self.adjoin_flags.get(portal.link))
                        .copied().unwrap_or(ADJOIN_MOVE);
                    if adjoin_flags & ADJOIN_MOVE != 0 && things.iter().any(|thing| thing.sector == portal.adjoin)
                        && search.add_sector(portal.adjoin)
                    {
                        self.thing_contacts(&things, mover, start, dir, dist, radius, portal.adjoin, flags, excluded, |_, hit| search.push(CollisionRecord::Solid(hit)));
                    }
                }
            }
        }
        dist
    }

    /// `sithCollision_SearchForSurfaceCollisions` (`0x4e86d0`): one pass over the sector's own
    /// surface list, in file order. A surface enters the pass only when it carries `HAS_COLLISION`
    /// (`surfflags & 4`) or its adjoin column names a row (`0x4e86d0:47`).
    ///
    /// - `0x4e86d0:32-41` — the whole pass is skipped only when the sector has a collidebox
    ///   (flag `0x1000`) and the sphere at the end of the move sits strictly inside it on every
    ///   axis; every other sector runs the pass.
    /// - `0x4e86d0:48-122` — adjoin flag routing: search flag `4` tests every row, otherwise the
    ///   row's `1`/`2`/`0x10` bits against the search's `0x1100`/`0x100`/`0x200` bits decide
    ///   doorway (test path) vs wall (solid path) for the surface.
    /// - `0x4e86d0:53-75` — the radius leg (`sithIntersect_CheckSphereFaceIntersection` at the
    ///   body's radius; a non-zero radius returns kind 4 **without coverage**) records the `0x40`
    ///   touch — unless the search is deferred (`flags & 4 && flags & 1`) or the far sector is
    ///   already in the searched list.
    /// - `0x4e86d0:77-112` — unless `flags & 2`, a second intersection at radius 0 whose contact
    ///   centre must land inside the portal polygon (`sithIntersect_TestSphereFaceHit`) records
    ///   the `0x20` crossing, after the deferred touch when that leg also hits.
    /// - `0x4e86d0:124-151` — the solid leg: a plane reject (`dot(end − v0, n) <= radius`), then
    ///   `sithIntersect_CheckSphereFaceIntersectionEx` with coverage and the vertex refinement,
    ///   then the directional gate `flags & 0x400 || dot(dir, hit_normal) < 0` (`_DAT_00523008`
    ///   reads 0.0 from the shipped image), recording kind `hit | 2`.
    ///
    /// The port keeps solids and portals in two tables, so the file order is a merge by surface
    /// id; the claims test drops the mirrored portals the engine's per-sector list never contains.
    /// Both out-distance scratch slots (`local_30`/`local_1c`) survive rejected faces, so they are
    /// never reset inside the pass.
    #[allow(clippy::too_many_arguments)]
    fn search_for_surface_collisions(
        &self, here: usize, start: [f32; 3], dir: [f32; 3], dist: f32, radius: f32,
        gates: Gates<'_>, flags: u32, search: &mut CollisionSearch,
    ) {
        let end = [start[0] + dir[0] * dist, start[1] + dir[1] * dist, start[2] + dir[2] * dist];
        if self.boxes.get(here).is_some_and(|bounds| {
            bounds.is_some_and(|(lo, hi)| (0..3).all(|k| end[k] - radius > lo[k] && end[k] + radius < hi[k]))
        }) {
            return;
        }
        let planes = self.sectors.get(here).map_or(&[][..], Vec::as_slice);
        let portals = self.portals.get(here).map_or(&[][..], Vec::as_slice);
        let (mut pi, mut ai) = (0, 0);
        // Retail leaves these scratch slots uninitialized on entry; do not invent a finite
        // distance for an initial overlapping `0x400` query. Current movement/floor callers omit it.
        let mut touch_distance = f32::NAN;
        let mut face_distance = f32::NAN;
        while pi < planes.len() || ai < portals.len() {
            let surface = portals.get(ai).and_then(|portal| self.adjoin_surface.get(portal.link))
                .copied().unwrap_or(usize::MAX);
            if pi < planes.len() && (ai == portals.len() || planes[pi].surface <= surface) {
                let plane = &planes[pi];
                pi += 1;
                if flags & 4 == 0 && (flags & 0x10 == 0 || self.surface_flags.get(plane.surface).is_none_or(|f| f & 1 != 0)) {
                    if let Some(face) = self.face_of.get(&plane.surface).and_then(|i| self.faces.get(*i))
                        .or_else(|| self.floor_of.get(&plane.surface).and_then(|i| self.floors.get(*i)))
                        .or_else(|| self.wall_of.get(&plane.surface).and_then(|i| self.walls.get(*i)))
                    {
                        self.surface_contact(face.normal, face.point, &face.verts, face.surface, here, start, dir, dist, radius, flags, search, &mut touch_distance);
                    }
                }
                continue;
            }
            let portal = &portals[ai];
            ai += 1;
            if self.surface_claims.get(surface).is_some_and(|claims| !claims.contains(&here)) {
                continue;
            }
            let adjoin_flags = gates.adjoins.get(portal.link).or_else(|| self.adjoin_flags.get(portal.link))
                .copied().unwrap_or(ADJOIN_MOVE);
            let passable = flags & 4 != 0 || ((flags & 0x1100 == 0 || adjoin_flags & 1 != 0)
                && !(flags & 0x200 != 0 && flags & 0x100 == 0 && adjoin_flags & 0x10 != 0)
                && (flags & 0x100 != 0 || adjoin_flags & 2 != 0));
            if passable {
                let normal = [-portal.normal[0], -portal.normal[1], -portal.normal[2]];
                if let Some(touch) = sphere_plane(start, dir, dist, radius, normal, portal.point, flags, &mut touch_distance) {
                    let deferred = flags & 5 == 5;
                    if !deferred && !search.sectors[..search.sector_count].contains(&portal.adjoin) {
                        search.push(CollisionRecord::Adjoin { distance: touch, sector: portal.adjoin });
                    }
                    if flags & 2 == 0 && let Some(distance) = sphere_plane(start, dir, dist, 0.0, normal, portal.point, flags, &mut face_distance) {
                        let point = [start[0] + dir[0] * distance, start[1] + dir[1] * distance, start[2] + dir[2] * distance];
                        // `CheckSphereFaceIntersection` (0x5090b0) checks coverage only for radius zero.
                        if face_edge_mask(point, 0.0, normal, &portal.verts).is_some() {
                            if deferred && !search.sectors[..search.sector_count].contains(&portal.adjoin) {
                                search.push(CollisionRecord::Adjoin { distance: touch, sector: portal.adjoin });
                            }
                            if std::env::var_os("BK_CROSS_TRACE").is_some() {
                                eprintln!(
                                    "cross: surface {surface} -> sector {} dist {distance:.4} center {:?}",
                                    portal.adjoin,
                                    [start[0] + dir[0] * distance, start[1] + dir[1] * distance, start[2] + dir[2] * distance]
                                );
                            }
                            search.push(CollisionRecord::Crossing { distance, sector: portal.adjoin, surface });
                        }
                    }
                }
            } else if flags & 4 == 0 && (flags & 0x10 == 0 || self.surface_flags.get(surface).is_none_or(|f| f & 1 != 0)) {
                self.surface_contact(portal.normal, portal.point, &portal.verts, surface, here, start, dir, dist, radius, flags, search, &mut touch_distance);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn surface_contact(
        &self, normal: [f32; 3], point: [f32; 3], verts: &[[f32; 3]], surface: usize, sector: usize,
        start: [f32; 3], dir: [f32; 3], dist: f32, radius: f32, flags: u32, search: &mut CollisionSearch, out_distance: &mut f32,
    ) {
        if -dot(sub([start[0] + dir[0] * dist, start[1] + dir[1] * dist, start[2] + dir[2] * dist], point), normal) > radius {
            return;
        }
        if let Some(mut hit) = Self::face_contact(normal, point, verts, surface, start, dir, dist, radius, sector, flags, out_distance) {
            if flags & 0x400 != 0 || dir[0] * hit.normal[0] + dir[2] * hit.normal[2] + dir[1] * hit.normal[1] < 0.0 {
                hit.kind |= 2;
                search.push(CollisionRecord::Solid(hit));
            }
        }
    }

    /// `sithPhysics_FindFloor`'s water branch (`0x4f5550`): from a centre in an underwater sector, search
    /// 0.05 straight up (`0x3d4ccccd`, radius 0) and take the nearest adjoin crossing whose far sector is
    /// dry. The distance is `thing+0x48`, the depth the underwater update holds the body at; `None`
    /// clears physflag `0x100000`.
    pub fn water_surface(&self, sector: usize, pos: [f32; 3], gates: Gates<'_>, dry: impl Fn(usize) -> bool) -> Option<f32> {
        const UP: [f32; 3] = [0.0, 0.0, 1.0];
        let mut searched = vec![sector];
        let mut here = sector;
        loop {
            let nearest = self
                .portals
                .get(here)?
                .iter()
                .filter(|portal| !searched.contains(&portal.adjoin) && gates.passable(here, portal.adjoin, portal.link))
                .filter_map(|portal| {
                    let normal = [-portal.normal[0], -portal.normal[1], -portal.normal[2]];
                    let t = sphere_plane(pos, UP, 0.05, 0.0, normal, portal.point, 0, &mut 0.0)?;
                    face_edge_mask([pos[0], pos[1], pos[2] + t], 0.0, normal, &portal.verts)?;
                    Some((t, portal.adjoin))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))?;
            if dry(nearest.1) {
                return Some(nearest.0);
            }
            searched.push(nearest.1);
            here = nearest.1;
        }
    }

    /// One surface's contact for a sphere of `radius` sweeping `dir` over `left` from `from`.
    #[allow(clippy::too_many_arguments)]
    fn face_contact(
        face_normal: [f32; 3],
        point: [f32; 3],
        verts: &[[f32; 3]],
        surface: usize,
        from: [f32; 3],
        dir: [f32; 3],
        left: f32,
        radius: f32,
        sector: usize,
        flags: u32,
        out_distance: &mut f32,
    ) -> Option<Sweep> {
        let normal = [-face_normal[0], -face_normal[1], -face_normal[2]];
        let t = sphere_plane(from, dir, left, radius, normal, point, flags, out_distance)?;
        if flags & 0x400 == 0 && dir[0] * normal[0] + dir[2] * normal[2] + dir[1] * normal[1] >= 0.0 {
            return None;
        }
        let at = [from[0] + dir[0] * t, from[1] + dir[1] * t, from[2] + dir[2] * t];
        let distance = dot(sub(at, point), normal);
        let distance = if distance.abs() <= 1e-5 { 0.0 } else { distance };
        let projected = [at[0] - normal[0] * distance, at[1] - normal[1] * distance, at[2] - normal[2] * distance];
        let mask = face_edge_mask(projected, radius, normal, verts)?;
        let (normal, kind) = if mask == 0 {
            (normal, 4)
        } else {
            // `sithIntersect_CheckSphereFaceHitVerticesIntersection` (0x508990).
            let mut nearest = radius + 1.0;
            let mut closest = None;
            for (i, a) in verts.iter().enumerate() {
                if mask & 1u32.checked_shl(i as u32).unwrap_or(0) == 0 {
                    continue;
                }
                let edge = sub(verts[(i + 1) % verts.len()], *a);
                let length = dot(edge, edge).sqrt();
                if length == 0.0 {
                    continue;
                }
                let unit = [edge[0] / length, edge[1] / length, edge[2] / length];
                let along = dot(sub(projected, *a), unit);
                if along < -radius || along - radius > length {
                    continue;
                }
                let kind = if (0.0..=length).contains(&along) { 8 } else { 0x10 };
                let along = along.clamp(0.0, length);
                let point = [a[0] + unit[0] * along, a[1] + unit[1] * along, a[2] + unit[2] * along];
                let separation = sub(projected, point);
                let distance = dot(separation, separation).sqrt();
                if distance <= radius && distance < nearest {
                    nearest = distance;
                    closest = Some((point, kind));
                }
            }
            let (point, kind) = closest?;
            (normalize(sub(from, point))?, kind)
        };
        Some(Sweep {
            distance: t,
            surface,
            normal,
            sector,
            point: at,
            kind,
            thing: None,
        })
    }

    /// `sithIntersect_CheckSphereThingIntersection` (0x5080d0): one thing's contact for a swept
    /// sphere, both arms of the engine function.
    ///
    /// - `0x5080d0:36-40` — the mesh gate: `flags & 0x80 == 0` and a `collide=3` partner. The
    ///   second disjunct also admits a `collide=3` **mover** against a sphere thing; that branch
    ///   (`bVar14`, the mover's own mesh with the direction negated and the normal negated back)
    ///   needs the mover's model, and every mover this port sweeps is the `walkplayer`
    ///   (`collide=1` from `_actor`), so it cannot run here — the walker is never the collide-3
    ///   side. Remainder noted in the #100 ledger row.
    /// - `0x5080d0:41-46` — the gate is the thing's `size=` sphere at `+0xb8` with radius
    ///   `+0x30` (`sithIntersect_CheckSphereIntersection` 0x508540), even when model vertices
    ///   reach outside it.
    /// - `0x5080d0:108-119` — a non-mesh thing: the normal is `normalize(start + dir*out − pos)`,
    ///   pointing from the thing's centre to the contact, and the distance is the gate's own
    ///   entry value.
    /// - the mesh arm: the engine transforms the sweep into the thing's frame, tests model-space
    ///   faces, and rotates the normal back (`rdMatrix_InvertOrtho34` 0x43fae0,
    ///   `rdMatrix_TransformPoint34Acc` 0x4421d0). This port pre-bakes the faces under the same
    ///   matrix (`main::solid_things_at`), so the identical arithmetic runs in world space; the
    ///   rotation is orthonormal, so distances and normals agree. The mesh list is `model+0x24`
    ///   count / `model+0x28` base (`0x5080d0:75`), and `rdModel3_LoadEntry` (`0x443e80:125-135`)
    ///   stores one count/pointer pair per **geoset**, so this reads geoset 0 only.
    fn check_sphere_thing_intersection(
        thing: &ThingSolid,
        meshes: &[Vec<Face>],
        at: [f32; 3],
        dir: [f32; 3],
        left: f32,
        radius: f32,
        sector: usize,
        flags: u32,
    ) -> Option<Sweep> {
        let mesh = flags & 0x80 == 0 && thing.collide == 3;
        let t = sphere_intersection(at, dir, left, radius, thing.origin, thing.radius, mesh, flags)?;
        let point = [at[0] + dir[0] * t, at[1] + dir[1] * t, at[2] + dir[2] * t];
        if !mesh {
            let normal = normalize([
                point[0] - thing.origin[0],
                point[1] - thing.origin[1],
                point[2] - thing.origin[2],
            ])?;
            return Some(Sweep {
                distance: t, normal, point, sector, surface: usize::MAX, kind: 1,
                thing: Some(ThingHit { thing: thing.index, mesh_face: None, item: false }),
            });
        }
        // `0x5080d0:76-104` keeps the last successful mesh at the current shortened distance, not every
        // face that can intersect the ray.
        let mut selected = None;
        let mut limit = left;
        // Rejected faces still overwrite the shared plane distance (0x508400 -> 0x508d20).
        let mut distance = 0.0;
        for (mesh, faces) in meshes.iter().enumerate() {
            if let Some((mut hit, face)) = Self::sphere_mesh_intersection(faces, at, dir, limit, radius, sector, &mut distance) {
                limit = distance;
                hit.kind |= 1;
                hit.thing = Some(ThingHit { thing: thing.index, mesh_face: Some((mesh, face)), item: false });
                selected = Some(hit);
            }
        }
        let mut hit = selected?;
        hit.distance = distance;
        hit.point = [at[0] + dir[0] * distance, at[1] + dir[1] * distance, at[2] + dir[2] * distance];
        Some(hit)
    }

    /// `sithIntersect_CheckSphereMeshIntersection` (0x508400): one face per mesh, shrinking travel.
    fn sphere_mesh_intersection(
        faces: &[Face], at: [f32; 3], dir: [f32; 3], mut left: f32, radius: f32, sector: usize,
        out_distance: &mut f32,
    ) -> Option<(Sweep, usize)> {
        let (mut kind, mut alignment) = (0, 1.0);
        let mut selected = None;
        for (index, face) in faces.iter().enumerate() {
            let into = [-face.normal[0], -face.normal[1], -face.normal[2]];
            if let Some(hit) = Self::face_contact(into, face.point, &face.verts, usize::MAX, at, dir, left, radius, sector, 0, out_distance) {
                if hit.distance < left || (kind != 4 && hit.kind == 4)
                    || face.normal[0] * dir[0] + face.normal[1] * dir[1] + face.normal[2] * dir[2] < alignment
                {
                    left = hit.distance;
                    kind = hit.kind;
                    alignment = face.normal[0] * dir[0] + face.normal[1] * dir[1] + face.normal[2] * dir[2];
                    selected = Some((hit, index));
                }
            }
        }
        selected
    }

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
    /// `at` in `sector`, **through every room the sweep reaches**.
    ///
    /// `sithCollision_SearchForCollisions` (`0x4e8160`) adds a sector when the sphere reaches one of
    /// its adjoins, and each added sector adds *its* adjoins to the same search (`uVar8` grows while
    /// the loop runs), so the scope is the transitive closure over crossed portals. Level 1's door
    /// leaf sits two portal hops from the body that uses it while the 0.165 use sweep touches every
    /// portal on the way, which is why one hop is not enough.
    pub fn in_search_chain(
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
        let mut seen = vec![sector];
        let mut next = 0;
        while next < seen.len() {
            let node = seen[next];
            next += 1;
            let Some(portals) = self.portals.get(node) else { continue };
            for portal in portals {
                let target = portal.adjoin;
                if seen.contains(&target) {
                    continue;
                }
                let now = dot(sub(at, portal.point), portal.normal);
                let then = now + dot(dir, portal.normal) * left;
                // The engine adds the adjoined sector only when the sphere reaches that portal, so
                // the target behind it is in the search only on this same crossing test.
                if now.signum() != then.signum() || now.abs().min(then.abs()) < radius - 1e-4 {
                    if target == other {
                        return true;
                    }
                    seen.push(target);
                }
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

    /// `sithCollision_SearchForThingCollisions` (`0x4e8430`): the searched sector's thing list,
    /// admitted in the engine's order and intersected by `0x5080d0`. `mover` is the moving thing
    /// (`param_2`); with `None` (FindFloor, probes) every admitted intersecting thing records.
    /// Mesh/face identity and feature kind survive the per-mesh and per-thing reductions; a
    /// non-mesh contact has no face.
    ///
    /// - `0x4e8430:33-36` — search flag `8` needs thingflag `0x80`; `0x10` needs thingflag `0x40`
    ///   (the type-7 half of the floor gate is the separate `0x2000` bit); `collide (+0x28)` must
    ///   be non-zero; `flags & 0x80002` must be clear; `0x2000` needs type 7.
    /// - `0x4e8430:76-95` — with a mover, a thing must also pass the pair matrix, the `0x200`
    ///   mutual exclusion, the owner chain and the attach exclusion. The engine skips the mover
    ///   itself (`puVar2 != puVar7`); runtime movers set their index on the search.
    /// - `0x4e8430:37-75` — the record's kind is `hit | 1`; `Startup` installs no distance
    ///   predicates (`0x4e6d90`), so the hit distance survives unchanged.
    fn thing_contacts(
        &self,
        things: &[ThingSolid],
        mover: Option<&MoveState>,
        at: [f32; 3],
        dir: [f32; 3],
        left: f32,
        radius: f32,
        sector: usize,
        flags: u32,
        excluded: Option<usize>,
        mut each: impl FnMut(&ThingSolid, Sweep),
    ) {
        let mut each = move |solid: &ThingSolid, mut hit: Sweep| {
            if let Some(contact) = &mut hit.thing {
                contact.item = solid.item;
            }
            each(solid, hit);
        };
        for thing in things.iter() {
            if thing.sector != sector || excluded == Some(thing.index) {
                continue;
            }
            if flags & 8 != 0 && thing.thing_flags & 0x80 == 0 {
                continue;
            }
            if flags & 0x10 != 0 && thing.thing_flags & 0x40 == 0 {
                continue;
            }
            if thing.collide == 0 || thing.thing_flags & 0x80002 != 0 {
                continue;
            }
            if flags & 0x2000 != 0 && thing.kind != 7 {
                continue;
            }
            if let Some(mover) = mover
                && !mover_thing_admitted(mover, thing, flags)
            {
                continue;
            }
            let hit = if let Some(meshes) = &thing.mesh {
                Self::check_sphere_thing_intersection(thing, meshes, at, dir, left, radius, sector, flags)
            } else if thing.collide == 3 {
                // A `collide=3` thing whose model did not load: the engine's mesh loop runs over
                // a zero count (`model+0x24`) and the function returns 0 — no contact at all.
                None
            } else {
                Self::check_sphere_thing_intersection(thing, &[], at, dir, left, radius, sector, flags)
            };
            if let Some(hit) = hit {
                each(thing, hit);
            }
        }
    }

    #[cfg(test)]
    fn sweep_things(&self, at: [f32; 3], dir: [f32; 3], left: f32, radius: f32, sector: usize) -> Option<(f32, [f32; 3])> {
        let mut search = CollisionSearch::new();
        self.search_for_collisions(sector, None, at, dir, left, radius, Gates::default(), 0x800, &mut search);
        while let Some(record) = search.pop() {
            if let CollisionRecord::Solid(hit) = record && hit.thing.is_some() {
                return Some((hit.distance, hit.normal));
            }
        }
        None
    }

    /// `sithCollision_MoveThing` (0x4e7950) for a geometry-only physics probe.
    pub fn move_thing(
        &self, sector: usize, pos: [f32; 3], delta: [f32; 3], radius: f32, gates: Gates<'_>, vel: &mut [f32; 3],
    ) -> ([f32; 3], usize, f32) {
        let mut state = MoveState {
            pos, sector, vel: *vel, remainder: delta, phys_flags: 0x4a4f,
            kind: 10, move_type: 1, collide: 1, flags: 0, attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        let distance = self.move_thing_with(&mut state, delta, radius, 0,
            |state, dir, dist, radius, flags, search| {
                self.search_for_collisions(state.sector, Some(state), state.pos, dir, dist, radius, gates, flags, search);
            },
            |state, _, far| {
                if state.sector != far && gates.sectors.get(far).is_some_and(|flags| flags & SECTOR_UNDERWATER != 0) {
                    state.attach = None;
                    state.vel[2] *= 0.25;
                }
                state.sector = far;
            },
            |state| self.inside_sector_with_gates(state.sector, state.pos, 0.0, gates),
            |_| {},
            |state, hit, crease| self.probe_contact(state, hit, crease),
        );
        *vel = state.vel;
        (state.pos, state.sector, distance)
    }

    /// `sithCollision_MoveThing` (`0x4e7950`): the whole move loop, transliterated from
    /// `decomp/004e7950_sithCollision_MoveThing.c` — same constants, same branches, same order.
    ///
    /// - `0x4e7950:40-45` — a zero-length move returns 0; `:46-57` the search flags take the
    ///   engine's ORs (`5` for a thing that cannot collide, `4` for a path-rider, `0x200` for
    ///   kind 10, `0x800` when nothing set `1`) through `MoveState::search_flags`.
    /// - `0x4e7950:90-103` — while distance remains and fewer than four rounds ran: save the
    ///   sub-step's start pose and sector, then ask `search_world` (the
    ///   `sithCollision_SearchForCollisions` `0x4e8160` call).
    /// - `0x4e7950:104-127` — consume the record stack nearest first (`sithCollision_PopStack`
    ///   `0x4e7120`): a kind-`0x18` record loses an equal distance to a kind-`4` one.
    /// - `0x4e7950:128-134` — a non-zero hit distance advances the thing to `start + hit * dir`,
    ///   measured from the sub-step start, not the previous record's contact.
    /// - `0x4e7950:135-139` — the remaining move is `dir * (dist - hit)`, or zero when the hit
    ///   ate the move. The physflag-`0x20` impulse bleed of `:140-155`/`:197-204` (`+0x228..0x230`,
    ///   fields only `sithPhysics_UpdatePlayerPhysics` `0x4f6860:30-32,138-149` fills, for
    ///   flag-`0x20` bodies) is elided: no ported body carries the flag.
    /// - `0x4e7950:156-196` — dispatch: a `0x20` crossing saves the position, messages a
    ///   flag-`2` surface and calls `SetSector` (`:165-185`), continuing only when the position
    ///   changed; surface and thing solids take their handler's answer as continue/stop
    ///   (`:158-163`); an item contact answers 0 (`0x4fbe10`).
    /// - `0x4e7950:206-236` — a clean round runs the whole distance and zeroes the remainder; a
    ///   stopped one renormalizes the remainder into the direction, snaps a `1e-5` (`0x52300c`)
    ///   length to zero and counts the round. The travelled total takes the last hit (clean: the
    ///   whole distance); `thingflags & 2` returns at once (`:235`).
    /// - `0x4e7950:238-250` — past four rounds a physics thing's movement resets
    ///   (`sithPhysics_ResetThingMovement` `0x4f61a0`: the port's `vel`/`remainder` fields), and
    ///   a `1e-5` total snaps to zero.
    /// - `0x4e7950:251-264` — a colliding physics thing that ended outside its sector
    ///   (`sithIntersect_IsSphereInSector` at radius 0, `0x507f30`) restores the LAST sub-step's
    ///   start pose and sector through `crossing(None, sector)`; the engine never searches for
    ///   another sector.
    /// - `0x4e7950:279-296` — the tail: a physics thing that travelled re-runs FindFloor through
    ///   `floor_tail` unless the caller passed `0x40`; an unattached (or attach-flag-8) body must
    ///   also carry physflag `0x40` and a vertical speed in `[-2.0, 0.2]` (`0x523018`/`0x52301c`).
    ///
    /// The rider recursion of `:61-88`/`:266-278` (a thing's children move with it, `0x34 & 2`
    /// damage) has no ported equivalent: no ported thing carries children. Callbacks run
    /// synchronously; each search and final containment read the current world.
    #[allow(clippy::too_many_arguments)]
    pub fn move_thing_with(
        &self, state: &mut MoveState, delta: [f32; 3], radius: f32, flags: u32,
        mut search_world: impl FnMut(&MoveState, [f32; 3], f32, f32, u32, &mut CollisionSearch),
        mut crossing: impl FnMut(&mut MoveState, Option<usize>, usize),
        mut contains: impl FnMut(&MoveState) -> bool,
        mut floor_tail: impl FnMut(&mut MoveState),
        mut solid_contact: impl FnMut(&mut MoveState, Sweep, &mut Option<[f32; 3]>) -> bool,
    ) -> f32 {
        let mut left = (delta[0] * delta[0] + delta[2] * delta[2] + delta[1] * delta[1]).sqrt();
        if left <= 0.0 { return 0.0; }
        let flags = state.search_flags(flags);
        let mut dir = [delta[0] / left, delta[1] / left, delta[2] / left];
        let mut search = CollisionSearch::new();
        let mut crease = None;
        let mut total = 0.0;
        let mut rounds = 0;
        let mut saved = (state.pos, state.sector);
        while left != 0.0 && rounds <= 3 {
            saved = (state.pos, state.sector);
            search_world(state, dir, left, radius, flags, &mut search);
            let mut accepted = false;
            let mut hit_distance = 0.0;
            while let Some(record) = search.pop() {
                hit_distance = record.distance();
                if hit_distance != 0.0 {
                    state.pos = [
                        dir[0] * hit_distance + saved.0[0],
                        dir[1] * hit_distance + saved.0[1],
                        dir[2] * hit_distance + saved.0[2],
                    ];
                }
                state.remainder = if left <= hit_distance { [0.0; 3] } else {
                    let remaining = left - hit_distance;
                    [dir[0] * remaining, dir[1] * remaining, dir[2] * remaining]
                };
                accepted = match record {
                    CollisionRecord::Crossing { sector, surface, distance } => {
                        // Consumed crossing (the push-site print in the search lists records a
                        // nearer surface hit may still discard).
                        if std::env::var_os("BK_CROSS_TRACE").is_some() {
                            eprintln!(
                                "cross: sector {} -> {} surface {surface} dist {distance:.4} pos {:?} attach {}",
                                state.sector, sector, state.pos, state.attach.is_some()
                            );
                        }
                        // `0x4e7950:166-171`: the position is saved before the surface message;
                        // a surface carrying flag 2 is messaged (`sithCog_SurfaceSendMessage`
                        // mode 8) before `SetSector` — both live in the `crossing` callback.
                        let before = state.pos.map(f32::to_bits);
                        crossing(state, Some(surface), sector);
                        state.pos.map(f32::to_bits) != before
                    }
                    CollisionRecord::Solid(hit) if hit.thing.is_some_and(|t| t.item) => {
                        // `0x4fbe10` (pair (5,10), invoked swapped as (item, player)) always returns
                        // 0: the contact is recorded and the sweep keeps going — the walker passes
                        // through the item's volume. The touch gates run at the consumer.
                        if let Some(contact) = hit.thing {
                            if !state.touched.contains(&contact.thing) {
                                state.touched.push(contact.thing);
                            }
                        }
                        false
                    }
                    CollisionRecord::Solid(hit) => {
                        // What stopped the move: the seed-decay window on #30 needs the face.
                        if std::env::var_os("BK_SOLID_TRACE").is_some() {
                            let what = hit.thing.map_or_else(
                                || format!("surface {}", hit.surface),
                                |t| format!("thing {}", t.thing),
                            );
                            eprintln!(
                                "solid: {what} normal {:?} dist {hit_distance:.4} pos {:?}",
                                hit.normal, state.pos
                            );
                        }
                        solid_contact(state, hit, &mut crease)
                    }
                    CollisionRecord::Adjoin { .. } => unreachable!(),
                };
                if accepted { break; }
            }
            if !accepted {
                state.pos = [
                    dir[0] * left + saved.0[0],
                    dir[1] * left + saved.0[1],
                    dir[2] * left + saved.0[2],
                ];
                total += left;
                state.remainder = [0.0; 3];
                left = 0.0;
            } else {
                total += hit_distance;
                left = (state.remainder[0] * state.remainder[0] + state.remainder[2] * state.remainder[2]
                    + state.remainder[1] * state.remainder[1]).sqrt();
                if left != 0.0 {
                    dir = [state.remainder[0] / left, state.remainder[1] / left, state.remainder[2] / left];
                }
                if left.abs() <= 1e-5 { left = 0.0; }
                rounds += 1;
            }
            if state.flags & 2 != 0 { return total; }
        }
        if rounds > 3 && state.move_type == 1 {
            state.vel = [0.0; 3];
            state.remainder = [0.0; 3];
        }
        if total.abs() <= 1e-5 { total = 0.0; }
        if state.collide != 0 && state.move_type == 1 && !contains(state) {
            state.pos = saved.0;
            crossing(state, None, saved.1);
        }
        // `0x4e7950:279-296` — the FindFloor tail.
        if state.move_type == 1 {
            if total == 0.0 { return total; }
            if flags & 0x40 == 0 {
                let attached = (state.attach.is_some() || state.attach_flags != 0)
                    && state.attach_flags & 8 == 0;
                if !attached {
                    if state.phys_flags & 0x40 == 0 { return total; }
                    if !(-2.0..=0.2).contains(&state.vel[2]) { return total; }
                }
                floor_tail(state);
            }
        }
        total
    }

    /// Geometry-only sweeps have no runtime body or sound receiver; their contacts are immovable.
    pub(crate) fn probe_contact(&self, state: &mut MoveState, hit: Sweep, crease: &mut Option<[f32; 3]>) -> bool {
        let flags = if hit.thing.is_none() { self.surface_flags.get(hit.surface).copied().unwrap_or(0) } else { 0 };
        Self::collide_hurt(state, hit.normal, hit.distance, crease, flags & 0x80 != 0, |_, _| {})
    }

    /// `sithCollision_CollideHurt` (`0x4e9090`): the slide response a solid contact answers with,
    /// transliterated from `decomp/004e9090_sithCollision_CollideHurt.c` — same constants, same
    /// branches, same order.
    ///
    /// The engine passes the thing, the contact normal, the hit record's distance (`record+0x20`, as
    /// `sithCollision_HandleThingHitSurface` `0x4e8b40` hands it over) and a damage gate (the
    /// surface's flags `& 0x80`; the thing-thing handler `0x4e8c50` passes 0). Fields: `+0x10`
    /// moveType (physics = 1), `+0x1d0` physflags, `+0x1d4` velocity, `+0x268` the remaining-move
    /// vector. The crease is the engine's static normal (`0x8b4bf0`) plus its active flag
    /// (`0x8b4be4`), which `sithCollision_MoveThing` (`0x4e7950:89`) clears on entry — one crease
    /// per move, the port's `crease` argument.
    ///
    /// Constants read from the shipped image's `.rdata`: the approach epsilon is `0x52300c`
    /// (1e-5); the collision charge uses the same curve and cutoff (`0x523034`, `0x523038`,
    /// `0x523014`) as `sithCollision_FallHurt` (`0x4e9550`), but runs here before updating the
    /// remainder. The damage gate suppresses it when set.
    ///
    /// A fresh face (`distance != 0`, or no crease yet) stores the normal and cancels the
    /// into-normal components of velocity and remainder scaled by 1.0001 — 1.9 under physflag
    /// `0x20` — preserving the tangential component exactly. A second face met at distance 0
    /// cancels plainly (no restitution), folds the stored crease against the new face
    /// (`rdVector_Normalize3Acc` `0x449b97`), and cancels into that crease too.
    pub(crate) fn collide_hurt(
        state: &mut MoveState, n: [f32; 3], distance: f32, crease: &mut Option<[f32; 3]>, suppress_charge: bool,
        charge: impl FnOnce(&mut MoveState, f32),
    ) -> bool {
        if state.move_type != 1 {
            return false;
        }
        let raw = -(state.remainder[1] * n[1] + state.remainder[2] * n[2] + state.remainder[0] * n[0]);
        let mut into = raw;
        if raw.abs() <= 1e-5 {
            into = 0.0;
        }
        if into <= 0.0 {
            return false;
        }
        let restitution = if state.phys_flags & 0x20 != 0 { 1.9 } else { 1.0001 };
        if distance != 0.0 || crease.is_none() {
            *crease = Some(n);
            let v_into = -(n[1] * state.vel[1] + n[2] * state.vel[2] + state.vel[0] * n[0]);
            if v_into > 0.0 {
                let m = restitution * v_into;
                for k in 0..3 {
                    state.vel[k] = n[k] * m + state.vel[k];
                }
                if !suppress_charge {
                    let damage = crate::fall_damage(v_into);
                    if damage > 0.0 {
                        charge(state, damage);
                    }
                }
            }
            let m = restitution * into;
            for k in 0..3 {
                state.remainder[k] = n[k] * m + state.remainder[k];
            }
        } else {
            for k in 0..3 {
                state.remainder[k] = n[k] * raw + state.remainder[k];
            }
            let v_into = -(n[1] * state.vel[1] + n[2] * state.vel[2] + state.vel[0] * n[0]);
            if v_into > 0.0 {
                for k in 0..3 {
                    state.vel[k] = n[k] * v_into + state.vel[k];
                }
            }
            // `rdVector_Normalize3Acc` (`0x449b97`): scale by 1/length unless the length is 0.0.
            let stored = crease.unwrap();
            let fold = -(n[0] * stored[0] + n[2] * stored[2] + n[1] * stored[1]);
            let mut c = [n[0] * fold + stored[0], n[1] * fold + stored[1], n[2] * fold + stored[2]];
            let length = (c[0] * c[0] + c[2] * c[2] + c[1] * c[1]).sqrt();
            if length != 0.0 {
                let inverse = 1.0 / length;
                for component in &mut c {
                    *component *= inverse;
                }
            }
            *crease = Some(c);
            let v_crease = -(state.vel[1] * c[1] + state.vel[2] * c[2] + state.vel[0] * c[0]);
            if v_crease > 0.0 {
                for k in 0..3 {
                    state.vel[k] = v_crease * c[k] + state.vel[k];
                }
            }
            let into_crease =
                -(state.remainder[2] * c[2] + state.remainder[1] * c[1] + state.remainder[0] * c[0]);
            if into_crease > 0.0 {
                for k in 0..3 {
                    state.remainder[k] = into_crease * c[k] + state.remainder[k];
                }
            }
        }
        true
    }

    /// `sithPhysics_FindFloor` (`0x4f5550`), transliterated — the engine's one floor finder and
    /// seat, ray and radius passes, attach rules, water branch and detach.
    ///
    /// - `:27-29` a body with no sector does nothing (the port's sector is always valid).
    /// - `:30-44` the search direction is world down carrying `0x10`, so only floor-flagged
    ///   surfaces (`surfflags & 1`) and standable thing faces count; the physflag-`0x80` arm
    ///   orients the search by the body instead, and no ported body carries `0x80` (the flags
    ///   this port reads: 0x4a4f, 0x225, 0x201, 0x400000), so that arm is elided.
    /// - `:45-65` the reach: an unattached, unforced call searches `radius + 0.005`
    ///   (`0x523184` = −0.005); an attached or forced one searches
    ///   `max(GetThingHeight, radius + 0.005)` doubled under physflags `0xc0`
    ///   (`0x52318c` = 1.1 otherwise).
    /// - `:67-100` two passes: a radius-0 ray, then — only for kinds 2 (item) and 10 (player),
    ///   with a non-zero radius, and only when the ray attached nothing — the body's radius.
    /// - `:71-94` the pop loop takes records nearest first: a flag-2 record attaches to its
    ///   surface (`sithThing_AttachThingToSurface` `0x4cde80` records it without moving the
    ///   body); a thing record whose face normal's z is at least 0.6 (`0x523190`) attaches to
    ///   that face (`sithThing_AttachThingToThingFace`), and one with no mesh face stops the
    ///   search keeping the current attach; records failing the test keep popping.
    /// - `:102-104` exhausting both passes detaches (`sithThing_DetachThing` `0x4ce380`).
    /// - `:106-123` the water branch: a **player (kind 10)** in an underwater sector searches
    ///   0.05 up (`0x3d4ccccd`, radius 0, flag 1) for the nearest crossing whose far sector is
    ///   dry; the distance is the thing's water-surface field (`+0x48`) and finding none clears
    ///   it. Any other kind under water returns with the attach untouched.
    ///
    /// `force` is the engine's `param_2`: `sithThing_Initialize`'s creation attach passes 1,
    /// which takes the attached reach regardless of the current attach state.
    pub fn find_floor(
        &self, state: &mut MoveState, radius: f32, height: f32, gates: Gates<'_>, force: bool,
    ) {
        let underwater =
            gates.sectors.get(state.sector).is_some_and(|flags| flags & SECTOR_UNDERWATER != 0);
        if underwater {
            if state.kind == 10 {
                state.water_surface = self.water_surface(state.sector, state.pos, gates, |s| {
                    !gates.sectors.get(s).is_some_and(|flags| flags & SECTOR_UNDERWATER != 0)
                });
            }
            return;
        }
        const DOWN: [f32; 3] = [0.0, 0.0, -1.0];
        let flags = 0x2802 | 0x10;
        let reach = if !force && state.attach.is_none() {
            radius + 0.005
        } else if state.phys_flags & 0xc0 == 0 {
            height.max(radius + 0.005) * 1.1
        } else {
            height.max(radius + 0.005) * 2.0
        };
        if reach > 0.0 {
            let mut pass = 0.0f32;
            loop {
                let mut search = CollisionSearch::new();
                self.search_for_collisions(
                    state.sector, None, state.pos, DOWN, reach, pass, gates, flags, &mut search,
                );
                let pos = state.pos;
                while let Some(record) = search.pop() {
                    let CollisionRecord::Solid(hit) = record else { continue };
                    if let Some(target) = hit.thing {
                        let Some((mesh, face)) = target.mesh_face else {
                            // `:79-82` a thing record with no face stops the search.
                            return;
                        };
                        let things = self.things();
                        let solid = things.iter().find(|solid| solid.index == target.thing).unwrap();
                        let face = &solid.mesh.as_ref().unwrap()[mesh][face];
                        if face.normal[2] < 0.6 {
                            continue;
                        }
                        if std::env::var_os("BK_FLOOR_TRACE").is_some() {
                            eprintln!(
                                "floor: attach thing {} point {:?} pos {:?} r {pass} reach {reach}",
                                target.thing, face.point, pos
                            );
                        }
                        state.attach = Some(Attach {
                            normal: face.normal, point: face.point, surface: None, thing: Some(target), friction: 1.0,
                        });
                        return;
                    }
                    if let Some(face) = self.face_of.get(&hit.surface).and_then(|i| self.faces.get(*i))
                        .or_else(|| self.floor_of.get(&hit.surface).and_then(|i| self.floors.get(*i)))
                        .or_else(|| self.wall_of.get(&hit.surface).and_then(|i| self.walls.get(*i)))
                    {
                        // The attach normal (`+0x44`) points into the room, toward the body standing on it.
                        let normal = if dot(sub(pos, face.point), face.normal) < 0.0 {
                            [-face.normal[0], -face.normal[1], -face.normal[2]]
                        } else {
                            face.normal
                        };
                        let surface_flags = self.surface_flags.get(hit.surface).copied().unwrap_or(0);
                        // `0x4f7430`: surface flags `0x1000` and `0x2000` scale the drag to 0.3 and 0.1.
                        let friction =
                            if surface_flags & 0x3000 == 0 { 1.0 } else if surface_flags & 0x2000 == 0 { 0.3 } else { 0.1 };
                        if std::env::var_os("BK_FLOOR_TRACE").is_some() {
                            eprintln!(
                                "floor: attach surface {} owner {:?} normal {:?} point {:?} pos {:?} r {pass} reach {reach} friction {friction}",
                                hit.surface, self.surface_owner.get(hit.surface), normal, face.point, pos
                            );
                        }
                        state.attach =
                            Some(Attach { normal, point: face.point, surface: Some(hit.surface), thing: None, friction });
                        return;
                    }
                }
                // `:96-100` the radius pass, kinds 2 and 10 only, once.
                if pass != 0.0 || !matches!(state.kind, 2 | 10) || radius == 0.0 {
                    break;
                }
                pass = radius;
            }
        }
        // `:102-104` both passes exhausted: detach.
        state.attach = None;
    }

    /// One physics step of a body: `sithPhysics_UpdateThing`'s routing (`0x4f5900:22-34`) — the
    /// attached hold (`sithPhysics_UpdateAttachedThingPhysics` `0x4f7430`) while it is attached,
    /// gravity otherwise — then `sithThing_UpdateMove` (`0x4cc4c0`) -> one
    /// `sithCollision_MoveThing` (`0x4e7950`) and its `FindFloor` tail.
    ///
    /// `height` is `sithPhysics_GetThingHeight` (`0x4f6210`): the attached hold keeps the body that far
    /// above its floor. Returns the horizontal distance moved, whether the sector changed, and whether
    /// the body attached this step.
    pub fn step_body(
        &self,
        gates: Gates<'_>,
        sector: &mut usize,
        pos: &mut [f32; 3],
        vel: &mut [f32; 3],
        attach: &mut Option<Attach>,
        dt: f32,
        radius: f32,
        height: f32,
        gravity: f32,
    ) -> (f32, bool, bool) {
        let start = *pos;
        let initial_sector = *sector;
        // shortcut: the MoveState carries the player template's values (physflags 0x4a4f, kind
        // 10, move 1, collide 1) — `step_body`'s only callers simulate the walkplayer (the fall
        // probes and the walker tests), so the values are theirs; FindFloor's attached reach
        // doubles under physflags 0xc0 (`0x4f5550:59-64`), and 0x4a4f takes the doubling. A
        // non-player caller must thread its own flags through here instead.
        let mut state = MoveState {
            pos: *pos, sector: *sector, vel: *vel, remainder: [0.0; 3], phys_flags: 0x4a4f,
            kind: 10, move_type: 1, collide: 1, flags: 0, attach: *attach, attach_flags: 0,
            touched: Vec::new(), water_surface: None, owner: None,
        };
        let delta = Self::update_thing_physics(&mut state, dt, radius, height, gravity);
        let mut landed = false;
        let traveled = self.move_thing_with(&mut state, delta, radius, 0,
            |state, dir, dist, radius, flags, search| {
                self.search_for_collisions(state.sector, Some(state), state.pos, dir, dist, radius, gates, flags, search);
            },
            |state, _, far| {
                // `sithThing_EnterWater` (`0x4cd370`): a physics thing crossing into an
                // underwater sector halves its vertical speed (`0x522e8c` = 0.25); it does
                // not detach — FindFloor's water branch owns the seat under water.
                if state.sector != far && gates.sectors.get(far).is_some_and(|flags| flags & SECTOR_UNDERWATER != 0) {
                    state.vel[2] *= 0.25;
                }
                state.sector = far;
            },
            |state| self.inside_sector_with_gates(state.sector, state.pos, 0.0, gates),
            |state| { landed = self.finish_body(state, radius, height, gates); },
            |state, hit, crease| self.probe_contact(state, hit, crease),
        );
        *pos = state.pos;
        *sector = state.sector;
        *vel = state.vel;
        *attach = state.attach;
        let flat = ((state.pos[0] - start[0]).powi(2) + (state.pos[1] - start[1]).powi(2)).sqrt();
        (flat, state.sector != initial_sector, landed)
    }

    /// `sithPhysics_UpdateThing`'s routing (`0x4f5900:22-34`) for the move a body assembles this
    /// step: the attached hold while the body is attached (`0x34 & 3 != 0`), gravity otherwise
    /// (`sithPhysics_UpdatePlayerPhysics`'s arm for kind 10, `sithPhysics_UpdateThingPhysics`'s
    /// for the rest — both `vel.z -= gravity * dt`, `0x40000` halving read by the caller). The
    /// underwater model (`0x4f6d80`) is the player walker's own; a body routed there is handled
    /// by its caller before this.
    pub fn update_thing_physics(
        state: &mut MoveState, dt: f32, radius: f32, height: f32, gravity: f32,
    ) -> [f32; 3] {
        match state.attach {
            Some(_) => Self::update_attached_thing_physics(state, dt, radius, height, gravity),
            None => {
                state.vel[2] -= gravity * dt;
                let delta = [state.vel[0] * dt, state.vel[1] * dt, state.vel[2] * dt];
                if dot(delta, delta) <= 1e-10 { [0.0; 3] } else { delta }
            }
        }
    }

    /// `sithPhysics_UpdateAttachedThingPhysics` (`0x4f7430`), the half a ported attached body
    /// exercises: the attached move's normal rules and the seat hold.
    ///
    /// Elided with citations (no ported body reaches them):
    /// - attached drag (`:186-208`, `sithPhysics_ApplyDrag` `0x4f5d50`) is still handled by
    ///   `integrate_walk`; the normal component is now removed from persistent velocity here;
    /// - the look and orientation halves (`:43-93`, physflags 0x100/0x1000/0x8000) and the
    ///   angular integration (`:94-179`): the port's things carry no attached angular state;
    /// - the walk-thrust arm (`:209-246`, physflag 0x2): the port's walk integration is
    ///   `integrate_walk` on the control read issue #79 transliterated (`0x4d8a00`);
    /// - the gravity kick that frees a heavy body off its seat (`:247-421`, gated on
    ///   `mass != 0` and the sector's thrust holding it): the ported bodies that fall detach
    ///   through this finder's exhausted passes instead, which is how the player walks off a
    ///   ledge (no ported body relies on the thrust kick).
    ///
    /// What runs, in the engine's order:
    /// - the attached distance `d` along the surface normal, divided by the normal's z for a
    ///   physflag-`0x800` body on a tilt (`:76-80`, `_DAT_00523194` = 1.0);
    /// - the player's slope pull (`:428-438`): kind 10 with physflag 1, a tilted floor, and
    ///   either a slippery one (friction < 0.8) or a moving body decelerates `vel.z` by
    ///   `gravity * dt * clamp(1 - friction, 0.2, 0.8)` (`0x5231bc`/`0x5231c8`);
    /// - remove the velocity's along-normal component in place (`:439-455`), snap tiny velocity
    ///   components (`0x52319c` = 1e-5, `:456-485`), then assemble the move (`:486-489`);
    /// - the hold (`:491-535`): the rest is `GetThingHeight` — a ceiling attach under physflag
    ///   `0x10000` rests at `radius + 0.01` measured along the normal's rise (`0x5231cc` =
    ///   −0.01) instead — and an error past 1e-5 is corrected into the move by at most
    ///   `0.5 * dt` (`0x523198`) along the surface normal, negated: straight up for a
    ///   physflag-`0x800` body (`:517-529`).
    pub fn update_attached_thing_physics(
        state: &mut MoveState, dt: f32, radius: f32, height: f32, gravity: f32,
    ) -> [f32; 3] {
        let Some(a) = state.attach else { return [0.0; 3] };
        let n = a.normal;
        let mut d = dot(sub(state.pos, a.point), n);
        if state.phys_flags & 0x800 != 0 && n[2] < 1.0 { d /= n[2]; }
        let moving = state.vel != [0.0; 3];
        if state.kind == 10
            && state.phys_flags & 1 != 0
            && n[2] < 1.0
            && (a.friction < 0.8 || moving)
        {
            state.vel[2] -= gravity * dt * (1.0 - a.friction).clamp(0.2, 0.8);
        }
        let into = state.vel[2] * n[2] + state.vel[1] * n[1] + state.vel[0] * n[0];
        if into.abs() > 1e-5 {
            for (v, normal) in state.vel.iter_mut().zip(n) {
                *v += normal * -into;
            }
        }
        for v in &mut state.vel {
            if v.abs() <= 1e-5 { *v = 0.0; }
        }
        let mut delta = state.vel.map(|v| v * dt);
        let error = if state.phys_flags & 0x10000 == 0 {
            d - height
        } else {
            n[2] * d - (radius + 0.01)
        };
        if error.abs() > 1e-5 {
            let correction = error.clamp(-dt * 0.5, dt * 0.5);
            if state.phys_flags & 0x800 != 0 {
                delta[2] -= correction;
            } else {
                for k in 0..3 {
                    delta[k] -= n[k] * correction;
                }
            }
        }
        if dot(delta, delta) <= 1e-10 { [0.0; 3] } else { delta }
    }

    /// The FindFloor run `move_thing_with`'s tail gate (`0x4e7950:279-296`) hands its callback:
    /// `sithPhysics_FindFloor` (`0x4f5550`) itself, reach, passes, water branch and detach
    /// included. Returns whether the body attached this move.
    pub fn finish_body(&self, state: &mut MoveState, radius: f32, height: f32, gates: Gates<'_>) -> bool {
        let was = state.attach.is_some();
        self.find_floor(state, radius, height, gates, false);
        !was && state.attach.is_some()
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
        let mut search = CollisionSearch::new();
        self.search_for_collisions(sector, None, focus, dir, len, radius, Gates::default(), 0x800, &mut search);
        while let Some(record) = search.pop() {
            let CollisionRecord::Solid(hit) = record else { continue };
            // `Sweep::point` is where the sphere's centre contacts the plane, which is exactly where
            // the engine puts the camera — it does not back off by the radius again.
            {
                if std::env::var_os("BK_CAMERA_TRACE").is_some() {
                    println!(
                        "camera pull: sector {sector}, contact at {:.4} of {len:.2}, normal {:?}, surface {}",
                        hit.distance, hit.normal, hit.surface
                    );
                }
                return (hit.point, Some(hit.surface));
            }
        }
        (want, None)
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
                // The engine's floor search counts only floor-flagged surfaces: `FindFloor` sets
                // the search's `0x10` bit for a thing without physflag `0x80` (the `walkplayer`
                // inherits `_actor`'s `0x4a4f`), and `SearchForSurfaceCollisions` then requires
                // `surfflags & 1` (`0x4e86d0` lines 125-126). A wall-only face — `09fuelstation`'s
                // 40-degree pit-mouth 9278, flags `0x4` — is never a floor. A surface the table
                // does not know (a fixture face without a level surface) still passes.
                if self
                    .surface_flags
                    .get(face.surface)
                    .is_some_and(|flags| flags & 1 == 0)
                {
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
                let Some(t) = sphere_plane(pos, [0.0, 0.0, -1.0], range, radius, normal, face.point, 0, &mut 0.0)
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
    ///
    /// `lift` is the one exception a **placement** needs: `true` lets the answer sit up to the plane
    /// query's own 0.05 tolerance above the body, and the caller decides whether to raise it (the
    /// player's initial seat). Every motion caller passes `false`; a falling body never lifts.
    pub fn resting_floor(
        &self,
        sector: usize,
        pos: [f32; 3],
        reach: f32,
        radius: f32,
        lift: bool,
    ) -> Option<Rest> {
        // The highest floor this call may return: a landing takes a contact at or below the body's
        // centre, a placement reaches the plane query's own tolerance above it.
        let ceiling = if lift { pos[2] + 0.05 } else { pos[2] - radius + 1e-3 };
        let mut best: Option<Rest> = None;
        for (floor, travelled, _owner, raw, surface) in
            self.floor_candidates(sector, pos, reach + radius, radius)
        {
            // The fall may only reach a floor whose contact is inside this frame's travel, and the
            // contact must not be above where the body already is.
            if travelled > reach + 1e-3 || floor > ceiling {
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
                    // A landing may not lift the body; a seating call raises the ceiling itself.
                    .filter(|floor| *floor <= ceiling)
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
                        && plane <= ceiling
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
        // A placement seat (`lift`) resolves the **floor**, not the top of a table the body overlaps.
        // Level 1's spawn overlaps the bar's `00table`: its top is 0.026 above the body's feet, so an
        // unlimited-reach seat lifted the body onto it and the walker then wedged against the bar
        // instead of walking retail's route along the floor. A landing still takes a prop top.
        if !lift
            && let Some(top) = self.prop_floor(pos, radius, reach)
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
            if let Some(meshes) = &thing.mesh {
                // An upward face the body is over is a floor at the face's height under the body.
                for face in meshes.iter().flatten().filter(|f| f.normal[2] > 0.7) {
                    if !over_face(face, pos, radius) {
                        continue;
                    }
                    let top = face.point[2]
                        - (face.normal[0] * (pos[0] - face.point[0]) + face.normal[1] * (pos[1] - face.point[1]))
                            / face.normal[2];
                    if top > feet + 0.02 || feet - top > reach + 1e-3 {
                        continue;
                    }
                    if best.is_none_or(|near| top > near) {
                        best = Some(top);
                    }
                }
                continue;
            }
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
    ///
    /// A floor counts only where `pos` is over its polygon: `sithPhysics_FindFloor` (`0x4f5550`) finds
    /// floors through `sithCollision_SearchForSurfaceCollisions` (`0x4e86d0`), which tests each face's
    /// polygon, not its infinite plane. `06abarons` sector 35 is a ring of floor pieces at z 0.1 around
    /// an adjoin (surface 433) into the pool below; the ring's plane under that opening held the body
    /// up where retail drops through. A plane with no registered face (a plane-only fixture) still counts.
    pub fn floor_surface(&self, sector: usize, pos: [f32; 3]) -> Option<(f32, usize)> {
        let planes = self.sectors.get(sector)?;
        planes
            .iter()
            .filter(|p| p.normal[2] < -0.7)
            .filter(|p| {
                self.floor_of
                    .get(&p.surface)
                    .and_then(|&i| self.floors.get(i))
                    .is_none_or(|face| over_face(face, pos, 0.0))
            })
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
    /// `radius` at `pos` still inside `sector`? One transliteration, in the engine's order:
    ///
    /// 1. the sector's collide box (`HASCOLLIDEBOX`, sector flag `0x1000` — `Solids::build`
    ///    stores the box only for such sectors): a sphere that fits inside it is **in**, without
    ///    testing a single surface;
    /// 2. the sector's own surface list, where a surface counts when it has `HAS_COLLISION`
    ///    (`0x4`) or its adjoin row allows movement (`flags & 2`, read at call time so a shut
    ///    door stops counting): the plane distance `dot(pos - vertex0, normal)` snaps to zero
    ///    within `1e-5` of it, and a distance below `radius` means outside.
    ///
    /// `sithCollision_MoveThing` calls this with radius 0 to decide whether to undo a move.
    pub fn inside_sector(&self, sector: usize, pos: [f32; 3], radius: f32) -> bool {
        self.inside_sector_with_gates(sector, pos, radius, Gates::default())
    }

    pub(crate) fn inside_sector_with_gates(&self, sector: usize, pos: [f32; 3], radius: f32, gates: Gates<'_>) -> bool {
        // 0x507f30:18-24: the collide box, only when the sector flags carry 0x1000. The engine
        // walks the surfaces when the sphere is outside the box on any axis (`pos - radius <= lo`
        // or `hi <= pos + radius`), so "fits" is the strict complement.
        if let Some(Some((lo, hi))) = self.boxes.get(sector) {
            if (0..3).all(|k| pos[k] - radius > lo[k] && pos[k] + radius < hi[k]) {
                return true;
            }
        }
        // 0x507f30:29-30: a surface counts when its flags carry 0x4 or its adjoin row's flags
        // carry 0x2. The engine reads the adjoin flags through the surface's live pointer, so the
        // runtime gates win over the file's word.
        self.containment.get(sector).is_none_or(|surfaces| surfaces.iter().all(|s| {
            let adjoin_move = usize::try_from(s.adjoin).is_ok_and(|row| {
                gates.adjoins.get(row).copied()
                    .or_else(|| self.adjoin_flags.get(row).copied())
                    .unwrap_or(ADJOIN_MOVE)
                    & ADJOIN_MOVE != 0
            });
            if s.surfflags & 0x4 == 0 && !adjoin_move {
                return true;
            }
            // 0x507f30:33-45: `dot(pos - vertex0, normal)` (the port's normals point out, so the
            // sign is flipped), |distance| <= 1e-5 (`_DAT_00523288`) snaps to 0
            // (`_DAT_00523284`), and a distance below the radius means outside.
            let d = sub(pos, s.point);
            let mut c = -(d[0] * s.normal[0] + d[1] * s.normal[1] + d[2] * s.normal[2]);
            if c.abs() <= 1e-5 {
                c = 0.0;
            }
            c >= radius
        }))
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
            let Some(t) = sphere_plane(pos, [0.0, 0.0, 1.0], range, radius, normal, face.point, 0, &mut 0.0) else {
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
/// module (main.rs's `step_body` tests) can exercise the same geometry. The planes name their surfaces
/// and the maps file them, because the engine search reaches a polygon only through them.
#[cfg(test)]
pub(crate) fn test_room() -> Solids {
    Solids {
        things: Default::default(),
        boxes: Vec::new(),
        hull: Vec::new(),
        floor_of: [(0usize, 0usize)].into_iter().collect(),
        containment: vec![vec![
            ContainPlane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, -1.0], surfflags: 0x4, adjoin: -1 },
            ContainPlane { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, 1.0], surfflags: 0x4, adjoin: -1 },
            ContainPlane { normal: [1.0, 0.0, 0.0], point: [2.0, 0.0, 0.0], surfflags: 0x4, adjoin: -1 },
            ContainPlane { normal: [-1.0, 0.0, 0.0], point: [-2.0, 0.0, 0.0], surfflags: 0x4, adjoin: -1 },
            ContainPlane { normal: [0.0, 1.0, 0.0], point: [0.0, 2.0, 0.0], surfflags: 0x4, adjoin: -1 },
            ContainPlane { normal: [0.0, -1.0, 0.0], point: [0.0, -2.0, 0.0], surfflags: 0x4, adjoin: -1 },
        ]],
        sectors: vec![vec![
            Plane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, -1.0], surface: 0 },
            Plane { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, 1.0], surface: 3 },
            Plane { normal: [1.0, 0.0, 0.0], point: [2.0, 0.0, 0.0], surface: 1 },
            Plane { normal: [-1.0, 0.0, 0.0], point: [-2.0, 0.0, 0.0], surface: 2 },
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
            verts: vec![[-2.0, 2.0, 1.0], [2.0, 2.0, 1.0], [2.0, -2.0, 1.0], [-2.0, -2.0, 1.0]],
            surface: 3,
        }],
        face_of: [(3usize, 0usize)].into_iter().collect(),
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

    /// `0x4fbe10` (pair (5,10)) returns 0 for every item contact: the sweep records the item and
    /// keeps going — an item never stops the walker, and the recorded index is the touch.
    #[test]
    fn an_item_is_passed_through_and_recorded() {
        let solids = boxed();
        // An item mid-room, on the body's path along +x.
        solids.set_things(vec![ThingSolid {
            index: 3,
            centre: [1.0, 0.0, 0.0],
            origin: [1.0, 0.0, 0.0],
            radius: 0.1,
            z_min: -0.1,
            z_max: 0.1,
            sector: 0,
            half: None,
            yaw: 0.0,
            mesh: None,
            thing_flags: 0,
            collide: 1,
            owner: None,
            kind: 5,
            item: true,
        }]);
        let mut state = MoveState {
            pos: [0.0, 0.0, 0.0], sector: 0, vel: [0.0; 3], remainder: [1.8, 0.0, 0.0],
            phys_flags: 0x4a4f, kind: 10, move_type: 1, collide: 1, flags: 0,
            attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        let travel = solids.move_thing_with(
            &mut state, [1.8, 0.0, 0.0], 0.065, 0,
            |state, dir, distance, radius, flags, search| {
                solids.search_for_collisions(
                    state.sector, Some(state), state.pos, dir, distance, radius,
                    Gates::default(), flags, search,
                );
            },
            |_, _, _| {},
            |_| true,
            |_| {},
            |state, hit, crease| solids.probe_contact(state, hit, crease),
        );
        assert!((travel - 1.8).abs() < 1e-4, "the walker passes through the item, travel {travel}");
        assert_eq!(state.touched, vec![3], "the contact is recorded for the touch gate");
    }

    /// A walker-shaped mover for the admission tests: type 10, no owner, not attached.
    fn walker() -> MoveState {
        MoveState {
            pos: [0.0, 0.0, 0.0], sector: 0, vel: [0.0; 3], remainder: [1.0, 0.0, 0.0],
            phys_flags: 0x4a4f, kind: 10, move_type: 1, collide: 1, flags: 0,
            attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        }
    }

    /// One sphere thing mid-room, with the admission words the case under test needs.
    fn gate_thing(kind: u32, thing_flags: u32) -> ThingSolid {
        ThingSolid {
            index: 4,
            centre: [1.0, 0.0, 0.0],
            origin: [1.0, 0.0, 0.0],
            radius: 0.1,
            z_min: -0.1,
            z_max: 0.1,
            sector: 0,
            half: None,
            yaw: 0.0,
            mesh: None,
            thing_flags,
            collide: 1,
            owner: None,
            kind,
            item: false,
        }
    }

    /// The contacts a mover's own search records (`0x4e8430` with `param_2` set).
    fn mover_contacts(solids: &Solids, mover: &MoveState, flags: u32) -> Vec<Sweep> {
        let mut search = CollisionSearch::new();
        solids.search_for_collisions(
            0, Some(mover), [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 1.0, 0.065,
            Gates::default(), flags, &mut search,
        );
        let mut hits = Vec::new();
        while let Some(CollisionRecord::Solid(hit)) = search.pop() {
            if hit.thing.is_some() {
                hits.push(hit);
            }
        }
        hits
    }

    /// `0x4e6d90`'s pair matrix admits the walker against cog furniture (7) and an actor (2),
    /// but nothing against a camera (1): `DAT_00847f38[mover*12 + hit]` is zero there.
    #[test]
    fn the_pair_matrix_gates_the_movers_thing_search() {
        let solids = boxed();
        solids.set_things(vec![gate_thing(7, 0)]);
        assert_eq!(mover_contacts(&solids, &walker(), 0).len(), 1, "cog furniture meets the walker");
        solids.set_things(vec![gate_thing(2, 0)]);
        assert_eq!(mover_contacts(&solids, &walker(), 0).len(), 1, "an actor meets the walker (LAB_004ed210)");
        solids.set_things(vec![gate_thing(1, 0)]);
        assert!(mover_contacts(&solids, &walker(), 0).is_empty(), "a camera has no pair cell");
        // A mover-less search (FindFloor's mover=0) admits the camera all the same.
        let mut search = CollisionSearch::new();
        solids.search_for_collisions(
            0, None, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 1.0, 0.065, Gates::default(), 0, &mut search,
        );
        assert!(search.pop().is_some(), "mover 0 records any admitted intersecting thing");
    }

    /// `0x4e8430:78` — either side carrying thingflag `0x200` ends the pair.
    #[test]
    fn thingflag_0x200_mutually_excludes_the_pair() {
        let solids = boxed();
        solids.set_things(vec![gate_thing(7, 0x200)]);
        assert!(mover_contacts(&solids, &walker(), 0).is_empty(), "a 0x200 thing is untouchable");
        solids.set_things(vec![gate_thing(7, 0)]);
        let mut mover = walker();
        mover.flags = 0x200;
        assert!(mover_contacts(&solids, &mover, 0).is_empty(), "a 0x200 mover touches nothing");
    }

    /// `0x4e8430:79-88` — a weapon thing skips the hit that shares its owner (`+0x80`).
    #[test]
    fn the_owner_chain_excludes_a_weapons_own_side() {
        let solids = boxed();
        let mut hit = gate_thing(7, 0);
        hit.owner = Some(9);
        solids.set_things(vec![hit]);
        let mut mover = walker();
        mover.kind = 3;
        mover.owner = Some(9);
        assert!(mover_contacts(&solids, &mover, 0).is_empty(), "same owner, no contact");
        mover.owner = Some(8);
        assert_eq!(mover_contacts(&solids, &mover, 0).len(), 1, "a stranger's thing still meets the bolt");
    }

    /// `0x4e8430:89-94` — a mover attached to the very thing is skipped while its attach flags
    /// carry `8` (or the search carries `0x40`).
    #[test]
    fn an_attached_mover_skips_the_thing_it_rides() {
        let solids = boxed();
        let hit = gate_thing(7, 0);
        let index = hit.index;
        solids.set_things(vec![hit]);
        let mut mover = walker();
        mover.attach_flags = 6;
        mover.attach = Some(Attach {
            normal: [0.0, 0.0, 1.0],
            point: [0.0, 0.0, 0.0],
            surface: None,
            thing: Some(ThingHit { thing: index, mesh_face: None, item: false }),
            friction: 1.0,
        });
        assert_eq!(mover_contacts(&solids, &mover, 0).len(), 1, "attach flag 8 clear and no search 0x40: the pair survives");
        mover.attach_flags = 0xe;
        assert!(mover_contacts(&solids, &mover, 0).is_empty(), "attach flag 8 skips the ridden thing");
        mover.attach_flags = 6;
        assert!(mover_contacts(&solids, &mover, 0x40).is_empty(), "search flag 0x40 skips it too");
    }

    /// `0x4e8430:33-36` — the search-flag and thingflag admission words.
    #[test]
    fn the_search_flags_admit_on_the_thingflags_word() {
        let solids = boxed();
        solids.set_things(vec![gate_thing(7, 0)]);
        assert!(mover_contacts(&solids, &walker(), 8).is_empty(), "flag 8 needs thingflag 0x80");
        assert!(mover_contacts(&solids, &walker(), 0x10).is_empty(), "flag 0x10 needs thingflag 0x40");
        assert!(mover_contacts(&solids, &walker(), 0x2000).len() == 1, "flag 0x2000 admits type 7");
        solids.set_things(vec![gate_thing(5, 0)]);
        assert!(mover_contacts(&solids, &walker(), 0x2000).is_empty(), "flag 0x2000 refuses an item");
        solids.set_things(vec![gate_thing(7, 0x80002)]);
        assert!(mover_contacts(&solids, &walker(), 0).is_empty(), "thingflags 0x80002 is never admitted");
    }

    /// `0x4fbe10`'s `HasLOS(item, player)` (`0x4e7500`): surfaces and cog-type things block, an
    /// item does not.
    #[test]
    fn item_touch_los_blocks_on_walls_and_cog_things_only() {
        let solids = boxed();
        assert!(
            solids.item_has_los(0, [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], Gates::default()),
            "the open room is clear"
        );
        // A cog thing between the endpoints blocks (flags 0x2122 admit type 7).
        solids.set_things(vec![ThingSolid {
            index: 7,
            centre: [0.0, 0.0, 0.0],
            origin: [0.0, 0.0, 0.0],
            radius: 0.2,
            z_min: -0.2,
            z_max: 0.2,
            sector: 0,
            half: None,
            yaw: 0.0,
            mesh: None,
            thing_flags: 0,
            collide: 1,
            owner: None,
            kind: 7,
            item: false,
        }]);
        assert!(
            !solids.item_has_los(0, [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], Gates::default()),
            "a cog thing between item and player blocks the touch"
        );
        // The same volume as an item does not block: only type 7 enters the 0x2000-filtered search.
        solids.set_things(vec![ThingSolid {
            index: 5,
            centre: [0.0, 0.0, 0.0],
            origin: [0.0, 0.0, 0.0],
            radius: 0.2,
            z_min: -0.2,
            z_max: 0.2,
            sector: 0,
            half: None,
            yaw: 0.0,
            mesh: None,
            thing_flags: 0,
            collide: 1,
            owner: None,
            kind: 5,
            item: true,
        }]);
        assert!(
            solids.item_has_los(0, [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], Gates::default()),
            "an item never blocks the touch ray"
        );
    }

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
            containment: vec![vec![
                ContainPlane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, 0.0], surfflags: 0x4, adjoin: -1 },
            ]],
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

    /// A placement seat resolves the floor, not a tabletop the body overlaps; a landing still stops
    /// on a prop's model top. Level 1's spawn overlaps the cantina bar's `00table`: its top is above
    /// the body's feet, so an unlimited-reach seat put the body **on the table** and the walker then
    /// wedged against the bar instead of walking retail's route at floor height.
    #[test]
    fn a_placement_seat_ignores_a_prop_top_but_a_landing_takes_it() {
        let mut room = seam_floor();
        room.set_things(vec![ThingSolid {
            index: 0,
            centre: [0.5, 0.0, 0.0],
            origin: [0.5, 0.0, 0.0],
            radius: 0.3,
            z_min: -0.1,
            z_max: 0.07,
            sector: 0,
            half: None,
            yaw: 0.0,
            mesh: None,
            thing_flags: 0,
            collide: 1,
            owner: None,
            kind: 7,
            item: false,
        }]);
        let radius = 0.065f32;
        // The seat probes one radius above the body's centre (0.12), which puts the tabletop below
        // the probe's feet, where `prop_floor` would otherwise accept it.
        let seat = room
            .resting_floor(0, [0.5, 0.0, 0.12 + radius], f32::INFINITY, radius, true)
            .expect("the seat found no floor");
        assert!(
            (seat.height - 0.0).abs() < 1e-4,
            "the seat lifted the body onto the table: height {}",
            seat.height
        );
        // A falling body still lands on the table, not through it onto the room floor.
        let land = room
            .resting_floor(0, [0.5, 0.0, 0.2], 0.3, radius, false)
            .expect("the landing found no floor");
        assert!(
            (land.height - 0.07).abs() < 1e-4,
            "the landing ignored the table top: height {}",
            land.height
        );
    }
    /// A non-mesh thing is the engine's `size=` sphere in 3D (`0x508540` gate, `0x5080d0:108-119`
    /// normal): a body above the centre stops farther out along the sphere's shoulder, and one
    /// outside the `size + radius` span passes clean over it. `sithCollision_MoveThing` (`0x4e7950`)
    /// has no step; the contact is answered by `sithCollision_CollideHurt` (`0x4e9090`) like a
    /// surface.
    #[test]
    fn a_solid_model_blocks_a_body_at_any_height() {
        let mut room = seam_floor();
        room.set_things(vec![ThingSolid {
            index: 0,
            centre: [0.0, 0.3, 0.0],
            origin: [0.0, 0.3, 0.0],
            radius: 0.1,
            z_min: 0.0,
            z_max: 0.08,
            sector: 0,
            half: Some([0.1, 0.1]),
            yaw: 0.0,
            mesh: None,
            thing_flags: 0,
            collide: 1,
            owner: None,
            kind: 7,
            item: false,
        }]);
        let radius = 0.065f32;
        // The sphere's shoulder: `entry = along - sqrt(span^2 - perp^2)` with `span = 0.1 + 0.065`
        // and `perp = 0.095` (the body's height over the centre), from `0x508540:24-31`.
        let entry = 0.6 - (0.165f32 * 0.165 - 0.095 * 0.095).sqrt();
        let (travel, normal) = room
            .sweep_things([0.0, -0.3, 0.095], [0.0, 1.0, 0.0], 1.0, radius, 0)
            .expect("the sphere gate missed the body");
        assert!((travel - entry).abs() < 1e-4, "the body left the sphere's shoulder: travel {travel}");
        // `0x5080d0:108-119`: the normal runs from the thing's centre to the contact.
        let contact = [0.0, -0.3 + entry, 0.095];
        let want = [contact[0], contact[1] - 0.3, contact[2]];
        let len = (want[1] * want[1] + want[2] * want[2]).sqrt();
        assert!((normal[1] - want[1] / len).abs() < 1e-4 && (normal[2] - want[2] / len).abs() < 1e-4,
            "the sphere's normal was {normal:?}");
        // Above the span the engine gate misses (`perp >= span`): the body passes clean over.
        assert!(
            room.sweep_things([0.0, -0.3, 0.2], [0.0, 1.0, 0.0], 1.0, radius, 0).is_none(),
            "a body outside the span was stopped"
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
                    Portal { adjoin: 0, link: 1, verts: quad(0.0).into_iter().rev().collect(), normal: [0.0, -1.0, 0.0], point: [0.0, 0.0, 0.0] },
                    Portal { adjoin: 2, link: 2, verts: quad(8.0), normal: [0.0, 1.0, 0.0], point: [0.0, 8.0, 0.0] },
                ],
                vec![Portal { adjoin: 1, link: 3, verts: quad(8.0).into_iter().rev().collect(), normal: [0.0, -1.0, 0.0], point: [0.0, 8.0, 0.0] }],
            ],
            containment: vec![
                vec![ContainPlane { normal: [0.0, 1.0, 0.0], point: [0.0, 0.0, 0.0], surfflags: 0, adjoin: 0 }],
                vec![
                    ContainPlane { normal: [0.0, -1.0, 0.0], point: [0.0, 0.0, 0.0], surfflags: 0, adjoin: 1 },
                    ContainPlane { normal: [0.0, 1.0, 0.0], point: [0.0, 8.0, 0.0], surfflags: 0, adjoin: 2 },
                ],
                vec![ContainPlane { normal: [0.0, -1.0, 0.0], point: [0.0, 8.0, 0.0], surfflags: 0, adjoin: 3 }],
            ],
            ..Default::default()
        }
    }

    /// 03katarn's thin light sector 429 as the shipped JKL places it (world 2 of the episode GOB's
    /// `jkl\03katarn.jkl`): a 0.10-deep strip (`y −0.70..−0.60`) whose south mouth (surface 3590,
    /// adjoin row 1159) spans `x 0.431..0.900`, whose north face at `y = −0.600` is the exit portal
    /// (surface 3593, adjoin row 1161, `x 0.513..0.812`, `z −0.200..0.100`) between two walls
    /// (surface 3594 `x 0.431..0.555`, surface 3592 `x 0.770..0.900`). Normals point out of the
    /// claiming sector, as `Solids::build` stores them.
    fn sliver() -> Solids {
        let mouth = vec![
            [0.431, -0.70, -0.20], [0.900, -0.70, -0.20], [0.900, -0.70, 0.80], [0.431, -0.70, 0.80],
        ];
        let exit = vec![
            [0.513, -0.60, -0.20], [0.812, -0.60, -0.20], [0.812, -0.60, 0.10], [0.513, -0.60, 0.10],
        ];
        let wall = |x0: f32, x1: f32, surface: usize| Plane {
            normal: [0.0, 1.0, 0.0],
            point: [0.5 * (x0 + x1), -0.60, -0.05],
            surface,
        };
        Solids {
            sectors: vec![
                Vec::new(),
                vec![wall(0.431, 0.555, 102), wall(0.770, 0.900, 103)],
                Vec::new(),
            ],
            faces: vec![
                Face {
                    normal: [0.0, 1.0, 0.0],
                    point: [0.493, -0.60, -0.05],
                    verts: vec![
                        [0.431, -0.60, -0.20], [0.555, -0.60, -0.20], [0.555, -0.60, 0.10], [0.431, -0.60, 0.10],
                    ],
                    surface: 102,
                },
                Face {
                    normal: [0.0, 1.0, 0.0],
                    point: [0.835, -0.60, -0.05],
                    verts: vec![
                        [0.770, -0.60, -0.20], [0.900, -0.60, -0.20], [0.900, -0.60, 0.10], [0.770, -0.60, 0.10],
                    ],
                    surface: 103,
                },
            ],
            face_of: std::collections::HashMap::from([(102, 0), (103, 1)]),
            portals: vec![
                vec![Portal { adjoin: 1, link: 0, verts: mouth.clone(), normal: [0.0, 1.0, 0.0], point: [0.665, -0.70, 0.30] }],
                vec![
                    Portal { adjoin: 0, link: 1, verts: mouth.into_iter().rev().collect(), normal: [0.0, -1.0, 0.0], point: [0.665, -0.70, 0.30] },
                    Portal { adjoin: 2, link: 2, verts: exit.clone(), normal: [0.0, 1.0, 0.0], point: [0.6625, -0.60, -0.05] },
                ],
                vec![Portal { adjoin: 1, link: 3, verts: exit.into_iter().rev().collect(), normal: [0.0, -1.0, 0.0], point: [0.6625, -0.60, -0.05] }],
            ],
            adjoin_surface: vec![100, 100, 101, 101],
            ..Default::default()
        }
    }

    /// `sithCollision_SearchForSurfaceCollisions` (`0x4e86d0`) on 03katarn's own sliver numbers:
    /// every distance below is the engine's arithmetic (`sithIntersect_CheckSphereHit`, `0x508be0`)
    /// on that geometry, at the player's radius 0.065 and the `MoveThing` search flags a player
    /// carries (`0x4e7950:46-57`: kind 10 sets `0x200`, no thing-attach leaves `0x1` clear so
    /// `0x800` is set — `0xA00`).
    #[test]
    fn sliver_crossings_follow_the_engine_search_arithmetic() {
        let solids = sliver();
        let north = [0.0f32, 1.0, 0.0];
        let mut search = CollisionSearch::new();
        let mut run = |sector: usize, start: [f32; 3], distance: f32, search: &mut CollisionSearch| {
            search.count = 0;
            solids.search_for_collisions(
                sector, None, start, north, distance, 0.065, Gates::default(), 0xA00, search,
            );
            let mut records = Vec::new();
            while let Some(record) = search.pop() {
                records.push(record);
            }
            records
        };
        let kinds = |records: &[CollisionRecord]| records.iter().map(|r| r.kind()).collect::<Vec<_>>();

        // The mouth, from the yard: the centre closes `gap0 = 0.02`, the radius leg already
        // overlaps (its `0x40` touch is consumed by the adjoin expansion, `0x4e8160:71`), and the
        // crossing (`0x20`) lands at `gap0`.
        let records = run(0, [0.657, -0.72, -0.08], 0.0258, &mut search);
        assert_eq!(kinds(&records), vec![0x20], "only the crossing survives the expansion");
        assert!((records[0].distance() - 0.02).abs() < 1e-6, "the crossing closes 0.02 of plane gap");
        let CollisionRecord::Crossing { sector, surface, .. } = records[0] else { panic!("not a crossing") };
        assert_eq!((sector, surface), (1, 100));

        // Under the exit portal's span the strip passes clean: the crossing lands at
        // `gap0 = 0.022`, and the flanking walls' contact points sit outside their polygons.
        let records = run(1, [0.657, -0.622, -0.08], 0.0258, &mut search);
        assert_eq!(kinds(&records), vec![0x20]);
        assert!((records[0].distance() - 0.022).abs() < 1e-6);
        let CollisionRecord::Crossing { sector, surface, .. } = records[0] else { panic!("not a crossing") };
        assert_eq!((sector, surface), (2, 101));

        // `0x508be0`'s own reject: parked 0.04 from the plane with only 0.0258 of move, the
        // radius-0 leg starves and no `0x20` is recorded — the engine keeps the same silence at
        // this pose. (The #70 repro parks here: 03katarn things 20/21, the shut `4x3door` leaves
        // of `03_FRONTDOOR.COG`, hold the body at exactly this gap while the trap surface 2107 —
        // which alone opens them, through `sithThing_AttachThingToSurface`'s `entered` at
        // `0x4cde80:50` — lies beyond the door.) Both portals' touches are consumed by the
        // expansion, so the record list is empty.
        let records = run(1, [0.657, -0.64, -0.08], 0.0258, &mut search);
        assert!(records.is_empty(), "both portals touch; neither crossing closes");

        // The corner pose, under wall 3594's span: the exit portal's contact point (x = 0.50)
        // misses its polygon, so beyond the consumed touches only the wall's overlapping-start
        // solid lands — at 0.
        let records = run(1, [0.50, -0.64, -0.08], 0.0258, &mut search);
        assert_eq!(kinds(&records), vec![0x2 | 0x4]);
        assert!(records[0].distance().abs() < 1e-6, "an overlapping start hits the wall at 0");
        let CollisionRecord::Solid(hit) = records[0] else { panic!("not a solid") };
        assert_eq!(hit.surface, 102);

        // A retail frame's accumulated delta (`1.29 u/s * 0.069 s`, `sithThing_UpdateMove` at
        // `0x4cc4c0` consuming one frame of the 50 Hz accumulator, `_DAT_005231ac`/`_DAT_005231b0`
        // = 50.0/0.02 in the shipped image) closes the whole band in one move: the crossing fires
        // from the deepest both-overlap pose.
        let records = run(1, [0.657, -0.66, -0.08], 0.089, &mut search);
        let crossing = records.iter().find_map(|record| match record {
            CollisionRecord::Crossing { distance, .. } => Some(*distance),
            _ => None,
        }).expect("a frame-sized move still records the crossing");
        assert!((crossing - 0.06).abs() < 1e-6, "the 0.1-wide strip is crossed in one frame's move");
    }

    #[test]
    fn a_portal_changes_sector_only_when_the_centre_crosses() {
        let room = corridor();
        let sweep = |y, distance| {
            room.move_thing(0, [0.0, y, 0.5], [0.0, distance, 0.0], 0.1, Gates::default(), &mut [0.0; 3])
        };
        let (end, touching, _) = sweep(-0.05, 0.02);
        assert_eq!(touching, 0);
        assert!((end[1] + 0.03).abs() < 1e-6);
        assert_eq!(sweep(-0.05, 0.1).1, 1);
        assert_eq!(sweep(0.0, 0.02).1, 1);
    }

    #[test]
    fn a_solid_contact_before_an_equal_distance_crossing_keeps_the_sector() {
        let mut room = corridor();
        let portal = &room.portals[0][0];
        room.faces.push(Face {
            normal: portal.normal,
            point: portal.point,
            verts: portal.verts.clone(),
            surface: 0,
        });
        room.sectors[0].push(Plane { normal: portal.normal, point: portal.point, surface: 0 });
        room.face_of.insert(0, 0);
        room.adjoin_surface = vec![1, 2, 3, 4];
        let hit = room.sweep(
            0, [0.0, -1.0, 0.5], [0.0, 1.0, 0.0], 2.0, 0.0, Gates::default(),
        ).unwrap();
        assert_eq!(hit.surface, 0);
        assert_eq!(hit.sector, 0);
    }

    #[test]
    fn collision_stack_prefers_a_face_over_an_equal_distance_edge_but_not_a_crossing() {
        let solid = |kind| CollisionRecord::Solid(Sweep {
            distance: 1.0,
            surface: kind as usize,
            normal: [0.0, 0.0, 1.0],
            sector: 0,
            point: [0.0; 3],
            kind,
            thing: None,
        });
        let crossing = CollisionRecord::Crossing { distance: 1.0, sector: 1, surface: 0 };
        let mut edge_cross_face = [Some(solid(8)), Some(crossing), Some(solid(4))];
        assert!(matches!(pop_collision(&mut edge_cross_face), Some(CollisionRecord::Solid(hit)) if hit.kind == 4));
        let mut cross_edge_face = [Some(crossing), Some(solid(8)), Some(solid(4))];
        assert!(matches!(pop_collision(&mut cross_edge_face), Some(CollisionRecord::Crossing { sector: 1, .. })));
        let mut vertex_face = [Some(solid(0x10)), Some(solid(4))];
        assert!(matches!(pop_collision(&mut vertex_face), Some(CollisionRecord::Solid(hit)) if hit.kind == 4));
        let mut sphere_face = [Some(solid(1)), Some(solid(5))];
        assert!(matches!(pop_collision(&mut sphere_face), Some(CollisionRecord::Solid(hit)) if hit.kind == 1));
        let mut thing_edge_face = [Some(solid(9)), Some(solid(5))];
        assert!(matches!(pop_collision(&mut thing_edge_face), Some(CollisionRecord::Solid(hit)) if hit.kind == 5));
    }

    #[test]
    fn sector_containment_uses_current_adjoin_movement_flags() {
        let room = corridor();
        let outside = [0.0, 0.1, 0.5];
        let open = Gates { sectors: &[], adjoins: &[ADJOIN_MOVE] };
        let closed = Gates { sectors: &[], adjoins: &[0] };
        assert!(!room.inside_sector_with_gates(0, outside, 0.0, open));
        assert!(room.inside_sector_with_gates(0, outside, 0.0, closed));
    }

    /// The floor `18Ascend`/`19A` ship: a slab with flags `0x80005` and file normal `(0, 0, 1)`
    /// (into the room). `0x507f30:37-42` snaps a plane distance within `1e-5` of zero to zero
    /// before the `fVar3 < param_2` test, so a body resting a ten-millionth *through* its own
    /// floor is still inside at radius 0 — `MoveThing`'s containment call.
    #[test]
    fn containment_snaps_a_resting_body_onto_its_own_floor() {
        let mut level = crate::jkl::Level::default();
        level.vertices = vec![
            [-2.0, -2.0, -1.3], [2.0, -2.0, -1.3], [2.0, 2.0, -1.3], [-2.0, 2.0, -1.3],
        ];
        level.surfaces = vec![crate::jkl::Surface {
            material: 0,
            surfflags: 0x80005,
            geo: 4,
            adjoin: -1,
            verts: vec![(0, 0), (1, 1), (2, 2), (3, 3)],
            intensities: vec![1.0; 4],
            normal: [0.0, 0.0, 1.0],
        }];
        level.sectors = vec![crate::jkl::Sector { surfaces: (0, 1), ..Default::default() }];
        let solids = build(&level);

        assert!(solids.inside_sector(0, [0.0, 0.0, -1.2], 0.0), "a body above the floor is inside");
        assert!(
            solids.inside_sector(0, [0.0, 0.0, -1.3 - 3e-6], 0.0),
            "3e-6 through the floor is inside the 1e-5 snap band"
        );
        assert!(
            !solids.inside_sector(0, [0.0, 0.0, -1.3 - 2e-5], 0.0),
            "2e-5 through the floor is past the band and outside"
        );
    }

    /// The one-sided adjoin pair `03katarn` 3383/3177 (rows 1088/983): the near side's surface
    /// carries the adjoin row, the far side's surface has adjoin `-1`. `0x507f30:29-30` counts the
    /// near surface (its row has `MOVE`), so a body past the plane is outside the near sector and
    /// the far sector's own list has nothing to test.
    #[test]
    fn a_body_past_a_one_sided_adjoin_plane_is_outside() {
        let mut level = crate::jkl::Level::default();
        level.vertices = vec![[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [-1.0, 0.0, 1.0]];
        let face = |adjoin: i32| crate::jkl::Surface {
            material: 0,
            surfflags: 0,
            geo: 4,
            adjoin,
            verts: vec![(0, 0), (1, 1), (2, 2), (3, 3)],
            intensities: vec![1.0; 4],
            normal: [0.0, -1.0, 0.0],
        };
        level.surfaces = vec![face(0), face(-1)];
        level.sectors = vec![
            crate::jkl::Sector { surfaces: (0, 1), ..Default::default() },
            crate::jkl::Sector { surfaces: (1, 1), ..Default::default() },
        ];
        level.adjoins = vec![
            crate::jkl::Adjoin { flags: 0x3, mirror: 1, dist: 0.0 },
            crate::jkl::Adjoin { flags: 0x3, mirror: 0, dist: 0.0 },
        ];
        let solids = build(&level);

        let past = [0.0, 0.1, 0.5];
        assert!(!solids.inside_sector(0, past, 0.0), "past the move-flagged plane, out of the near sector");
        assert!(solids.inside_sector(0, [0.0, -0.1, 0.5], 0.0), "on the near side, inside");
        assert!(
            solids.inside_sector(1, past, 0.0),
            "the far side's own surface carries no adjoin row, so nothing counts"
        );
        // The row's move flag is read at call time: a shut link stops the surface from counting.
        let shut = Gates { sectors: &[], adjoins: &[0] };
        assert!(solids.inside_sector_with_gates(0, past, 0.0, shut), "a shut link does not count the plane");
    }

    /// `03katarn` sector 429: `FLAGS 0x1000` with a collide box, the thin sliver of #70. A sphere
    /// that fits the box is in (`0x507f30:18-24`) without a single plane test — here a collision
    /// floor at z 0.5 that the point at z 0.3 sits below, which the plane walk alone would reject.
    #[test]
    fn a_collidebox_sector_accepts_without_plane_tests() {
        let mut level = crate::jkl::Level::default();
        level.vertices = vec![
            [0.430541, -0.70, 0.5], [0.90, -0.70, 0.5], [0.90, -0.60, 0.5], [0.430541, -0.60, 0.5],
        ];
        level.surfaces = vec![crate::jkl::Surface {
            material: 0,
            surfflags: 0x4,
            geo: 4,
            adjoin: -1,
            verts: vec![(0, 0), (1, 1), (2, 2), (3, 3)],
            intensities: vec![1.0; 4],
            normal: [0.0, 0.0, 1.0],
        }];
        level.sectors = vec![crate::jkl::Sector {
            flags: 0x1000,
            surfaces: (0, 1),
            boundbox: Some(([0.430541, -0.70, -0.199999], [0.90, -0.60, 0.80])),
            ..Default::default()
        }];
        let solids = build(&level);

        let at = [0.65, -0.65, 0.3];
        assert!(
            solids.inside_sector(0, at, 0.0),
            "the box accepts the point without the plane tests"
        );
        assert!(
            !solids.inside_sector(0, at, 0.9),
            "a sphere too big for the box falls through to the plane walk, which rejects it"
        );
    }

    /// `SearchForSurfaceCollisions` (0x4e86d0) checks the adjoin's movement flag, not sector visibility.
    #[test]
    fn sector_adjoin_visibility_does_not_replace_movement_flags() {
        let solids = corridor();
        let radius = 0.065;
        // In the first room, facing the opening at y = 0, with a step long enough to carry the body through.
        let start = [0.0, -0.5, 0.5];
        let step = [0.0, 1.2, 0.0];
        let links = [ADJOIN_MOVE | ADJOIN_VISIBLE; 4];

        let hidden = Gates { sectors: &[SECTOR_ADJOINS_OFF, 0, 0], adjoins: &links };
        let (pos, sector, _) = solids.move_thing(0, start, step, radius, hidden, &mut [0.0; 3]);
        assert_eq!(sector, 1, "a hidden movable adjoin still permits crossing: {pos:?}");
        assert!(pos[1] > 0.0, "the body should be past the opening: {pos:?}");

        // And the other side of the same rule: an opening a script shut with `ClearAdjoinFlags(x, 2)`.
        let no_move = [ADJOIN_VISIBLE; 4];
        let barred = Gates { sectors: &[0, 0, 0], adjoins: &no_move };
        let (pos, sector, traveled) = solids.move_thing(0, start, step, radius, barred, &mut [0.0; 3]);
        assert_eq!(sector, 0, "an opening without `ADJOIN_MOVE` must stop the body");
        assert!(pos[1] < 0.0 && traveled < step[1], "the force field did not stop the body: {pos:?}");
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
            side([1.0, 0.0, 0.0], [x, 0.0, 0.0], vec![[x, -y, z], [x, y, z], [x, y, -z], [x, -y, -z]], 1),
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

    /// A floor plane counts only over its polygon: `06abarons` sector 35's ring floor surrounds an
    /// adjoin into the pool, and the ring's plane must not hold up a body over that opening.
    #[test]
    fn a_floor_plane_is_not_a_floor_outside_its_polygon() {
        let mut room = boxed();
        room.floors[0].verts = vec![[1.0, -2.0, -1.0], [2.0, -2.0, -1.0], [2.0, 2.0, -1.0], [1.0, 2.0, -1.0]];
        assert!(room.floor_surface(0, [1.5, 0.0, -0.95]).is_some(), "over the strip");
        assert!(room.floor_surface(0, [0.0, 0.0, -0.95]).is_none(), "beside the strip, over its plane");
    }

    /// A `collide=3` mesh is collided as its faces: a door leaf laid flat (`18Ascend` things 323/324)
    /// holds a body up where its upward face is, and nowhere else, whatever its upright bounds were.
    #[test]
    fn a_flat_mesh_leaf_is_a_floor_over_its_face_only() {
        let leaf = Face {
            normal: [0.0, 0.0, 1.0],
            point: [0.0, 0.0, 1.0],
            verts: vec![[-0.5, -0.5, 1.0], [0.5, -0.5, 1.0], [0.5, 0.5, 1.0], [-0.5, 0.5, 1.0]],
            surface: usize::MAX,
        };
        let solids = Solids::default();
        solids.set_things(vec![ThingSolid {
            index: 0,
            centre: [0.0, 0.0, 1.0],
            origin: [0.0, 0.0, 1.0],
            radius: 0.2,
            z_min: 0.99,
            z_max: 1.0,
            sector: 0,
            // The upright bounds a yaw-only box would use: thin and tall, nothing like the leaf.
            half: Some([0.0125, 2.0]),
            yaw: 0.0,
            mesh: Some(vec![vec![leaf]].into()),
            thing_flags: 0,
            collide: 1,
            owner: None,
            kind: 7,
            item: false,
        }]);
        let r = 0.065;
        let on = solids.resting_floor(0, [0.3, 0.3, 1.0 + r], 0.1, r, false).expect("the leaf's top");
        assert!((on.height - 1.0).abs() < 1e-5, "height {}", on.height);
        assert!(solids.resting_floor(0, [1.0, 0.0, 1.0 + r], 0.1, r, false).is_none(), "beside the leaf");
    }

    /// The one floor rule answers the same resting height for the four shapes the motion callers
    /// used to resolve with different queries: a sector whose floor is only its plane, a walkway
    /// that is only a face, a ramp, and a solid thing's model top. The same body, at the same pose,
    /// rests at the same height on every one of them.
    #[test]
    fn a_plane_only_sector_a_face_only_walkway_a_ramp_and_a_prop_rest_at_one_height() {
        const R: f32 = 0.065;
        let body = [0.0, 0.0, R];
        let rest = |solids: &Solids| solids.resting_floor(0, body, 0.1, R, false);

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
            index: 0,
            centre: [0.0; 3],
            origin: [0.0; 3],
            radius: 0.5,
            z_min: -1.0,
            z_max: 0.0,
            sector: 0,
            half: None,
            yaw: 0.0,
            mesh: None,
            thing_flags: 0,
            collide: 1,
            owner: None,
            kind: 7,
            item: false,
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
    fn movement_and_visibility_use_their_respective_adjoin_flags() {
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

        // `0x10` blocks a player's `0x200` search, but not an untyped geometry query.
        let mut ai_only = vec![ADJOIN_VISIBLE | ADJOIN_MOVE; 4];
        ai_only[0] |= ADJOIN_AI_ONLY;
        let gates = Gates { sectors: &[0; 3], adjoins: &ai_only };
        assert!(level.sweep(0, from, dir, 3.0, 0.25, gates).is_none());
        let (stopped, sector, _) = level.move_thing(0, from, [0.0, 3.0, 0.0], 0.25, gates, &mut [0.0; 3]);
        assert_eq!(sector, 0);
        assert!(stopped[1] < 0.0);

        // Sector visibility does not make a movable adjoin solid in either direction.
        let gates = Gates { sectors: &[SECTOR_ADJOINS_OFF, 0, 0], adjoins: &[0x7; 4] };
        assert!(level.sweep(0, from, dir, 3.0, 0.25, gates).is_none());
        let back = level.sweep(1, [0.0, 1.0, 0.5], [0.0, -1.0, 0.0], 3.0, 0.25, gates);
        assert!(back.is_none(), "{back:?}");

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
            room.move_thing(0, [1.7, 0.0, -0.5], [0.0, 1.0, 0.0], 0.3, gates, &mut [0.0; 3]);
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
        room.sectors[0].push(Plane { normal: [0.0, h, -h], point: [0.0, 1.0, 0.0], surface: 9 });
        room.faces.push(Face {
            normal: [0.0, h, -h],
            point: [0.0, 1.0, 0.0],
            verts: vec![[-1.0, 0.2, -0.8], [1.0, 0.2, -0.8], [1.0, 1.4, 0.4], [-1.0, 1.4, 0.4]],
            surface: 9,
        });
        room.face_of.insert(9, room.faces.len() - 1);
        let gates = Gates { sectors: &[], adjoins: &[] };
        let (pos, _sector, _blocked) =
            room.move_thing(0, [0.0, 0.3, 0.0], [0.0, 1.0, 0.0], 0.1, gates, &mut [0.0; 3]);
        assert!(pos[1] > 0.9, "the body did not advance up the ramp: {pos:?}");
        assert!(pos[2] > 0.05, "the body did not climb the ramp: {pos:?}");
    }

    #[test]
    fn pushes_out_of_walls_and_finds_the_floor() {
        let room = room();
        assert_eq!(room.sector_at([0.0, 0.0, 0.0], 0.05), Some(0));
        assert_eq!(room.sector_at([9.0, 0.0, 0.0], 0.05), None);

        // `support` is what the walker uses for standing.
        assert_eq!(room.support([0.0, 0.0, -0.5], 0.3, 1.0), Some(-1.0));

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

    /// A chain hop is taken only when the sweep reaches that **portal**. Two rooms are always
    /// adjacent once an opening is crossed, so accepting a target on sight made the whole corridor
    /// visible to a sweep that never touched the second opening.
    #[test]
    fn a_chain_hop_is_taken_only_when_its_portal_is_reached() {
        let room = corridor();
        let r = 0.025;
        // Room 0 opens into 1 at y = 0, and 1 into 2 at y = 8. A body in room 0 walking level.
        assert!(
            !room.in_search_chain(0, 2, [0.0, -0.5, 0.5], [0.0, 1.0, 0.0], 1.0, r),
            "a sweep that stops inside room 1 searched the room past its far opening"
        );
        assert!(
            room.in_search_chain(0, 2, [0.0, -0.5, 0.5], [0.0, 1.0, 0.0], 9.0, r),
            "a sweep that crosses both openings did not search the second room"
        );
        assert!(
            room.in_search_chain(0, 1, [0.0, -0.5, 0.5], [0.0, 1.0, 0.0], 1.0, r),
            "the first hop's own portal was not reached"
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
        // A non-movable adjoin is itself a collision surface, before the far sector's wall.
        let dir = [0.0, 1.0, 0.0];
        let leaf_hit = room.sweep(0, [0.0, -0.2, 0.0], dir, 0.5, r, shut).map(|hit| hit.point[1]);
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
            verts: vec![[-1.0, 0.1, -1.0], [1.0, 0.1, -1.0], [1.0, 0.1, 1.0], [-1.0, 0.1, 1.0]],
            normal,
            point: [0.0, 0.1, 0.0],
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
            index: 0,
            centre: [0.0, 0.3, -0.5],
            origin: [0.0, 0.3, -0.5],
            radius: 0.2,
            z_min: -1.0,
            z_max: 0.0,
            sector: 2,
            half: None,
            yaw: 0.0,
            mesh: None,
            thing_flags: 0,
            collide: 1,
            owner: None,
            kind: 7,
            item: false,
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
            index: 0,
            centre: [1.0, 0.0, -0.5],
            origin: [1.0, 0.0, -0.5],
            radius: 0.5,
            z_min: -1.0,
            z_max: 0.0,
            sector: 0,
            half: None,
            yaw: 0.0,
            mesh: None,
            thing_flags: 0,
            collide: 1,
            owner: None,
            kind: 7,
            item: false,
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
    fn a_thin_model_leaf_collides_as_its_mesh() {
        // A `collide=3` leaf is its model's faces (`0x5080d0`): two 0.2 x 1.0 quads 0.025 apart,
        // the engine's own door-leaf proportions. The `size=` sphere is only the gate.
        let leaf = |cx: f32, faces: Vec<Face>| ThingSolid {
            index: 0,
            centre: [cx, 0.0, -0.5],
            origin: [cx, 0.0, -0.5],
            radius: 0.2,
            z_min: -1.0,
            z_max: 0.0,
            sector: 0,
            half: Some([0.2, 0.0125]),
            yaw: 0.0,
            mesh: Some(vec![faces].into()),
            thing_flags: 0,
            collide: 3,
            owner: None,
            kind: 7,
            item: false,
        };
        let broad = |cx: f32| vec![
            Face {
                normal: [0.0, -1.0, 0.0],
                point: [cx, -0.0125, -0.5],
                verts: vec![
                    [cx - 0.2, -0.0125, -1.0], [cx + 0.2, -0.0125, -1.0],
                    [cx + 0.2, -0.0125, 0.0], [cx - 0.2, -0.0125, 0.0],
                ],
                surface: usize::MAX,
            },
            Face {
                normal: [0.0, 1.0, 0.0],
                point: [cx, 0.0125, -0.5],
                verts: vec![
                    [cx + 0.2, 0.0125, -1.0], [cx - 0.2, 0.0125, -1.0],
                    [cx - 0.2, 0.0125, 0.0], [cx + 0.2, 0.0125, 0.0],
                ],
                surface: usize::MAX,
            },
        ];
        let mut room = seam_floor();
        room.set_things(vec![leaf(1.0, broad(1.0))]);
        let r = 0.065;
        // Broad side at y = -0.025: the body stops 0.0125 + 0.065 from the centre plane, where a
        // `size=` sphere alone would stop at 0.265.
        let (travel, normal) = room
            .sweep_things([1.0, -0.3, -0.5], [0.0, 1.0, 0.0], 0.5, r, 0)
            .expect("the leaf did not stop the body");
        assert!((travel - 0.2225).abs() < 1e-4, "the leaf stopped at {travel}");
        assert!(normal[1] < -0.99, "the leaf's normal was {normal:?}");
        // A path just past the leaf's 0.2 half width misses it: the projected centre is 0.1
        // outside the face edge and `TestSphereFaceHit` (`0x508750`) rejects past `radius`.
        assert!(
            room.sweep_things([1.3, -0.3, -0.5], [0.0, 1.0, 0.0], 0.5, r, 0)
                .is_none(),
            "a body beside the leaf was stopped"
        );
        // The mesh turns with the thing's placement: the same leaf placed at 90 degrees of yaw
        // has its thin side facing world x (the faces are baked under the thing's matrix,
        // `solid_things_at`, exactly as the engine's `InvertOrtho34` transform would place them).
        let turned = vec![
            Face {
                normal: [1.0, 0.0, 0.0],
                point: [1.0125, 0.0, -0.5],
                verts: vec![
                    [1.0125, -0.2, -1.0], [1.0125, 0.2, -1.0],
                    [1.0125, 0.2, 0.0], [1.0125, -0.2, 0.0],
                ],
                surface: usize::MAX,
            },
            Face {
                normal: [-1.0, 0.0, 0.0],
                point: [0.9875, 0.0, -0.5],
                verts: vec![
                    [0.9875, 0.2, -1.0], [0.9875, -0.2, -1.0],
                    [0.9875, -0.2, 0.0], [0.9875, 0.2, 0.0],
                ],
                surface: usize::MAX,
            },
        ];
        room.set_things(vec![leaf(1.0, turned)]);
        let (travel, normal) = room
            .sweep_things([1.3, 0.0, -0.5], [-1.0, 0.0, 0.0], 0.5, r, 0)
            .expect("the turned leaf did not stop the body");
        assert!((travel - 0.2225).abs() < 1e-4, "the turned leaf stopped at {travel}");
        assert!(normal[0] > 0.99, "the turned leaf's normal was {normal:?}");
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
            index: 0,
            centre: [1.0, 2.0, 3.0],
            origin: [1.0, 2.0, 3.0],
            radius: 0.2,
            z_min: -1.0,
            z_max: 0.0,
            sector: 0,
            half: None,
            yaw: 0.0,
            mesh: None,
            thing_flags: 0,
            collide: 1,
            owner: None,
            kind: 7,
            item: false,
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

    #[test]
    fn sphere_intersection_matches_the_engine_gate_branches() {
        let start = [0.0; 3];
        let dir = [1.0, 0.0, 0.0];
        // 0x508540:19-39: strict tangency rejection, entry distance, and zero for an out-of-range root.
        assert_eq!(sphere_intersection(start, dir, 2.0, 0.25, [2.0, 0.0, 0.0], 0.75, true, 0), Some(1.0));
        assert_eq!(sphere_intersection(start, dir, 2.0, 0.25, [1.0, 1.0, 0.0], 0.75, true, 0), None);
        assert_eq!(sphere_intersection(start, dir, 1.0, 0.25, [1.8, 0.9, 0.0], 0.75, true, 0), Some(0.0));
        // 0x508540:41-54: only meshes fall back to an initial overlap when the centre is behind.
        let behind = [-0.25, 0.0, 0.0];
        assert_eq!(sphere_intersection(start, dir, 1.0, 0.25, behind, 0.25, false, 0), None);
        assert_eq!(sphere_intersection(start, dir, 1.0, 0.25, behind, 0.25, true, 0), Some(0.0));
        assert_eq!(sphere_intersection(start, dir, 1.0, 0.25, behind, 0.25, true, 0x400), Some(0.25));
        assert_eq!(sphere_intersection(start, dir, 0.0, 0.25, behind, 0.25, false, 0), Some(0.0));
    }

    #[test]
    fn a_mesh_outside_its_size_sphere_cannot_deflect_the_body() {
        let solids = Solids::default();
        solids.set_things(vec![ThingSolid {
            index: 0,
            centre: [0.0; 3],
            origin: [0.0; 3],
            radius: 0.5,
            z_min: -1.0,
            z_max: 1.0,
            sector: 0,
            half: Some([1.0; 2]),
            yaw: 0.0,
            mesh: Some(vec![vec![Face {
                normal: [-1.0, 0.0, 0.0],
                point: [0.0; 3],
                verts: vec![[0.0, -1.0, -1.0], [0.0, -1.0, 1.0], [0.0, 1.0, 1.0], [0.0, 1.0, -1.0]],
                surface: usize::MAX,
            }]].into()),
            thing_flags: 0,
            collide: 3,
            owner: None,
            kind: 7,
            item: false,
        }]);
        let mut vel = [1.0, 0.0, 0.0];
        let (end, sector, _) = solids.move_thing(0, [-1.0, 0.61, 0.0], [2.0, 0.0, 0.0], 0.1, Gates::default(), &mut vel);
        assert_eq!(end, [1.0, 0.61, 0.0]);
        assert_eq!(sector, 0);
        assert_eq!(vel, [1.0, 0.0, 0.0], "a rejected mesh must not reach CollideHurt");
        let (end, _, _) = solids.move_thing(0, [-1.0, 0.5, 0.0], [2.0, 0.0, 0.0], 0.1, Gates::default(), &mut vel);
        assert!((end[0] + 0.1).abs() < 0.001, "a mesh inside the gate must still block: {end:?}");
        assert!(vel[0].abs() < 0.001, "the accepted contact must project velocity");
    }

    #[test]
    fn the_cantina_tables_second_edge_contact_fails_the_engine_sphere_gate() {
        // 00table's authored origin and size; the next search after the first accepted edge response.
        // 0x508540:24-31 gives perpendicular distance 0.14428146 > 0.070420 + 0.065.
        assert_eq!(sphere_intersection(
            [-10.431544, -5.433699, -1.1222925],
            [0.583979, -0.7167128, -0.3811709],
            0.001417,
            0.065,
            [-10.349864, -5.350090, -1.209447],
            0.070420,
            true,
            0xa00,
        ), None);
        assert!(sphere_intersection(
            [-10.435213, -5.439691, -1.131486],
            [0.50642204, 0.8575079, 0.09064813],
            0.017651, 0.065, [-10.349864, -5.350090, -1.209447], 0.070420, true, 0xa00,
        ).is_some(), "the first table edge remains a real collision candidate");
        assert_eq!(sphere_intersection(
            [-10.239994, -5.4296784, -1.129886],
            [-1.4550361e-5, 1.0, 0.0],
            0.008561, 0.065, [-10.349864, -5.350090, -1.209447], 0.070420, true, 0xa00,
        ), None, "the later wall/table crease must not receive a table record either");
    }

    fn contact_face(normal: [f32; 3], point: [f32; 3]) -> Face {
        let u = if normal[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] };
        let v = [
            normal[1] * u[2] - normal[2] * u[1],
            normal[2] * u[0] - normal[0] * u[2],
            normal[0] * u[1] - normal[1] * u[0],
        ];
        let verts = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .into_iter()
            .map(|(a, b)| [
                point[0] + a * u[0] + b * v[0],
                point[1] + a * u[1] + b * v[1],
                point[2] + a * u[2] + b * v[2],
            ]).collect();
        Face { normal, point, verts, surface: usize::MAX }
    }

    fn contact_model(meshes: Vec<Vec<Face>>) -> ThingSolid {
        ThingSolid {
            index: 77, centre: [0.0; 3], origin: [0.0; 3], radius: 4.0,
            z_min: -4.0, z_max: 4.0, sector: 0, half: None, yaw: 0.0,
            mesh: Some(meshes.into()), thing_flags: 0x40, collide: 3, owner: None, kind: 7, item: false,
        }
    }

    #[test]
    fn mesh_selection_prefers_a_face_then_authored_alignment_on_equal_travel() {
        let face = contact_face([-1.0, 0.0, 0.0], [0.0; 3]);
        let mut edge = face.clone();
        for vertex in &mut edge.verts {
            if vertex[1] < 0.0 { vertex[1] = 0.05; }
        }
        let (hit, index) = Solids::sphere_mesh_intersection(
            &[edge, face.clone()], [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], 2.0, 0.1, 0, &mut 0.0,
        ).unwrap();
        assert_eq!((hit.kind, index), (4, 1), "the equal-distance face must replace the edge");
        let angled = contact_face([-0.6, 0.8, 0.0], [0.0; 3]);
        let (_, index) = Solids::sphere_mesh_intersection(
            &[face.clone(), angled.clone()], [-0.05, 0.0, 0.0], [1.0, 0.0, 0.0], 0.2, 0.1, 0, &mut 0.0,
        ).unwrap();
        assert_eq!(index, 0, "equal feature kinds retain the more opposing authored normal");
        let (_, index) = Solids::sphere_mesh_intersection(
            &[angled, face], [-0.05, 0.0, 0.0], [1.0, 0.0, 0.0], 0.2, 0.1, 0, &mut 0.0,
        ).unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn rejected_polygon_planes_still_set_the_things_movement_distance() {
        let selected = contact_face([-1.0, 0.0, 0.0], [0.0; 3]);
        let missed = contact_face([-1.0, 0.0, 0.0], [-0.5, 2.0, 0.0]);
        let farther = contact_face([-0.6, 0.8, 0.0], [-0.1, 0.0, 0.0]);
        for meshes in [
            vec![vec![selected.clone(), missed.clone()]],
            vec![vec![selected.clone()], vec![missed.clone()]],
            vec![vec![selected, missed], vec![farther]],
        ] {
            let solids = Solids::default();
            solids.set_things(vec![contact_model(meshes)]);
            let mut vel = [1.0, 0.0, 0.0];
            let (end, _, _) = solids.move_thing(
                0, [-1.0, 0.0, 0.0], [2.0, 0.0, 0.0], 0.1, Gates::default(), &mut vel,
            );
            assert!((-0.61..-0.59).contains(&end[0]), "the last plane distance must survive polygon/mesh rejection: {end:?}");
            assert!(vel[0].abs() < 0.001 && vel[1] == 0.0, "the normal must remain the selected face's: {vel:?}");
        }
    }

    #[test]
    fn an_overlapping_backface_writes_distance_before_facing_rejection() {
        let selected = contact_face([-1.0, 0.0, 0.0], [0.2, 0.0, 0.0]);
        let backface = contact_face([1.0, 0.0, 0.0], [-0.05, 0.0, 0.0]);
        let solids = Solids::default();
        solids.set_things(vec![contact_model(vec![vec![selected, backface]])]);
        let mut vel = [1.0, 0.0, 0.0];
        let (end, _, _) = solids.move_thing(
            0, [0.0; 3], [0.3, 0.0, 0.0], 0.1, Gates::default(), &mut vel,
        );
        assert!(end[0].abs() < 0.001, "the rejected overlapping plane must overwrite travel with zero: {end:?}");
        assert!(vel[0].abs() < 0.001, "the retained face must still stop forward movement: {vel:?}");
    }

    #[test]
    fn a_refused_selected_edge_does_not_expose_the_same_things_farther_face() {
        let mut edge = contact_face([-1.0, 0.0, 0.0], [0.0; 3]);
        for vertex in &mut edge.verts {
            if vertex[1] < 0.0 { vertex[1] = 0.0; }
        }
        let farther = contact_face([-1.0, 0.0, 0.0], [0.1, 0.0, 0.0]);
        let solids = Solids::default();
        solids.set_things(vec![contact_model(vec![vec![edge, farther]])]);
        let mut vel = [1.0, -1.0, 0.0];
        let (end, _, _) = solids.move_thing(
            0, [-0.05, -0.05, 0.0], [0.2, -0.2, 0.0], 0.1, Gates::default(), &mut vel,
        );
        assert!((end[0] - 0.15).abs() < 1e-6 && (end[1] + 0.25).abs() < 1e-6, "a hidden alternative stopped the body: {end:?}");
        assert_eq!(vel, [1.0, -1.0, 0.0], "the selected tangent edge must be refused before response");
    }

    #[test]
    fn floor_eligibility_uses_the_last_selected_mesh_not_a_flattened_face_search() {
        let top = contact_face([0.0, 0.0, 1.0], [0.0; 3]);
        let side = contact_face([0.8660254, 0.0, 0.5], [0.0; 3]);
        let solids = Solids::default();
        solids.set_things(vec![contact_model(vec![vec![top], vec![side]])]);
        let mut state = floor_state([0.0, 0.0, 0.05], Some(FLAT), 10);
        solids.find_floor(&mut state, 0.065, 0.12, Gates::default(), false);
        let attach = state.attach;
        assert_eq!(attach, None, "the later mesh wins its equal-distance hit, then fails floor eligibility");
    }

    #[test]
    fn floor_search_cannot_choose_a_farther_top_face_of_the_same_mesh() {
        let side = contact_face([0.8660254, 0.0, 0.5], [0.0; 3]);
        let top = contact_face([0.0, 0.0, 1.0], [0.0, 0.0, -0.1]);
        let solids = Solids::default();
        solids.set_things(vec![contact_model(vec![vec![side, top]])]);
        let mut state = floor_state([0.0, 0.0, 0.5], Some(FLAT), 10);
        solids.find_floor(&mut state, 0.065, 0.12, Gates::default(), false);
        let attach = state.attach;
        assert_eq!(attach, None, "rejecting the selected side must not reveal another face of that mesh");
    }

    #[test]
    fn a_faceless_floor_record_keeps_attachment_instead_of_finding_a_lower_floor() {
        let solids = room();
        let mut sphere = contact_model(Vec::new());
        // `0x5080d0` records a faceless contact only for a non-mesh thing: a `collide=3` thing
        // whose mesh list is empty returns 0 outright.
        sphere.mesh = None;
        sphere.collide = 1;
        sphere.z_max = 0.2;
        solids.set_things(vec![sphere]);
        let mut state = floor_state([0.0, 0.0, 0.3], Some(FLAT), 10);
        solids.find_floor(&mut state, 0.065, 0.12, Gates::default(), false);
        assert_eq!(state.attach, Some(FLAT), "a faceless thing preserves the existing attachment");
        let mut state = floor_state([0.0, 0.0, 0.3], None, 10);
        solids.find_floor(&mut state, 0.065, 0.12, Gates::default(), false);
        assert_eq!(state.attach, None, "a faceless thing must not invent a top-face attachment");
    }

    #[test]
    fn floor_ties_keep_the_selected_things_mesh_face_identity() {
        let solids = room();
        solids.set_things(vec![contact_model(vec![vec![contact_face([0.0, 0.0, 1.0], [0.0, 0.0, -1.0])]])]);
        let mut state = floor_state([0.0, 0.0, -0.8], Some(FLAT), 10);
        solids.find_floor(&mut state, 0.065, 0.12, Gates::default(), false);
        let attach = state.attach.expect("the selected mesh face is a floor");
        assert_eq!(attach.thing, Some(ThingHit { thing: 77, mesh_face: Some((0, 0)), item: false }));
        assert_eq!(attach.surface, None, "the earlier thing face wins a tie with the sector floor");
        assert_eq!(attach.normal, [0.0, 0.0, 1.0]);
    }

    const FLAT: Attach = Attach { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, -1.0], surface: Some(0), thing: None, friction: 1.0 };

    /// `0x4f7430:439-491`: the velocity loses its normal component before the move is
    /// assembled, and the separate seat correction is clamped to `0.5 * dt` (`0x523198`).
    #[test]
    fn the_attached_hold_is_clamped_and_velocity_stays_projected() {
        let solids = room();
        let (mut sector, mut pos, mut vel, mut attach) = (0usize, [0.0f32, 0.0, -0.5], [1.0f32, 0.0, -3.0], Some(FLAT));
        solids.step_body(Gates::default(), &mut sector, &mut pos, &mut vel, &mut attach, 0.02, 0.065, 0.12, 4.0);
        assert_eq!(vel, [1.0, 0.0, 0.0]);
        assert!((pos[2] - (-0.5 - 0.01)).abs() < 1e-6, "the hold moved {} in one 0.02 step, not 0.01", -0.5 - pos[2]);
        assert!((pos[0] - 0.02).abs() < 1e-6, "the walk moved {}", pos[0]);
        // Within one step's clamp, the hold lands on the height exactly.
        let mut pos = [0.0f32, 0.0, -1.0 + 0.125];
        solids.step_body(Gates::default(), &mut sector, &mut pos, &mut [0.0; 3], &mut Some(FLAT), 0.02, 0.065, 0.12, 4.0);
        assert!((pos[2] - (-1.0 + 0.12)).abs() < 1e-6, "held at {}", pos[2]);
    }

    /// `0x4f7430:491-535`: the hold's rest is `GetThingHeight`, its correction runs along the
    /// surface normal — straight up for a physflag-`0x800` body on a tilt (`:76-80` divides the
    /// distance by the normal's rise first) — and a ceiling attach under physflag `0x10000` rests
    /// at `radius + 0.01` along the rise (`0x5231cc`).
    #[test]
    fn the_hold_corrects_along_the_normal_or_world_up_by_physflag() {
        let slope = Attach { normal: [0.0, -0.70710678, 0.70710678], point: [0.0, 0.0, 0.0], surface: Some(0), thing: None, friction: 1.0 };
        let mut state = floor_state([0.0, 0.0, 0.2], Some(slope), 0);
        state.phys_flags = 0;
        let delta = Solids::update_attached_thing_physics(&mut state, 0.02, 0.065, 0.12, 4.0);
        // d = 0.2 * cos45 = 0.14142; error = 0.02142; clamp 0.01 along the normal.
        assert!((delta[1] - 0.00710678).abs() < 1e-4 && (delta[2] + 0.00710678).abs() < 1e-4,
            "the correction follows the slope normal: {delta:?}");
        let mut state = floor_state([0.0, 0.0, 0.2], Some(slope), 0);
        state.phys_flags = 0x800;
        let delta = Solids::update_attached_thing_physics(&mut state, 0.02, 0.065, 0.12, 4.0);
        // d / cos45 = 0.2; error = 0.08; clamp 0.01 straight up.
        assert!((delta[1]).abs() < 1e-6 && (delta[2] + 0.01).abs() < 1e-6,
            "the 0x800 hold is vertical: {delta:?}");
        let ceiling = Attach { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, 1.0], surface: Some(0), thing: None, friction: 1.0 };
        let mut state = floor_state([0.0, 0.0, 0.9], Some(ceiling), 0);
        state.phys_flags = 0x10000;
        let delta = Solids::update_attached_thing_physics(&mut state, 0.02, 0.065, 0.12, 4.0);
        // rise * d - (radius + 0.01) = -0.1 - 0.075 = -0.175; clamp -0.01 along the normal.
        assert!((delta[2] + 0.01).abs() < 1e-6, "the ceiling rest hangs below its seat: {delta:?}");
    }

    /// A test body for the floor finder: the player's own shape (kind 10, physflags 0x4a4f)
    /// unless a case says otherwise.
    fn floor_state(pos: [f32; 3], attach: Option<Attach>, kind: u32) -> MoveState {
        MoveState {
            pos, sector: 0, vel: [0.0; 3], remainder: [0.0; 3], phys_flags: 0x4a4f,
            kind, move_type: 1, collide: 1, flags: 0, attach, attach_flags: 0,
            touched: Vec::new(), water_surface: None, owner: None,
        }
    }

    /// `0x4f5550:45-65`: an unattached body searches `moveSize + 0.005` (`0x523184`) down, an
    /// attached one `max(GetThingHeight, moveSize + 0.005)` — doubled under physflags `0xc0`
    /// (`0x52318c` = 1.1 otherwise) — and only a floor-flagged surface attaches.
    #[test]
    fn find_floor_reaches_by_attach_state_and_takes_only_floors() {
        let mut solids = room();
        let pos = [0.0f32, 0.0, -1.0 + 0.2];
        let mut state = floor_state(pos, None, 10);
        solids.find_floor(&mut state, 0.065, 0.12, Gates::default(), false);
        assert_eq!(state.attach, None, "0.2 is past the unattached reach of 0.07 to contact");
        let mut state = floor_state(pos, Some(FLAT), 10);
        solids.find_floor(&mut state, 0.065, 0.12, Gates::default(), false);
        let floor = state.attach.expect("the attached reach finds the floor");
        assert_eq!(floor.normal, [0.0, 0.0, 1.0]);
        assert_eq!(floor.surface, Some(0));
        solids.surface_flags = vec![0x4];
        let mut state = floor_state(pos, Some(FLAT), 10);
        solids.find_floor(&mut state, 0.065, 0.12, Gates::default(), false);
        assert_eq!(state.attach, None, "a surface without flag 1 attached");
        solids.surface_flags = vec![0x2001];
        let mut state = floor_state(pos, Some(FLAT), 10);
        solids.find_floor(&mut state, 0.065, 0.12, Gates::default(), false);
        assert_eq!(state.attach.map(|a| a.friction), Some(0.1));
    }

    /// `0x4f5550:96-100`: only kinds 2 and 10 retry the search at the body's radius — a floor the
    /// straight-down ray misses but the sphere reaches.
    #[test]
    fn the_radius_pass_is_for_items_and_players_only() {
        let solids = Solids::default();
        // A floor top whose polygon starts 0.03 east of the body: the radius-0 ray lands outside
        // it, the 0.065 sphere reaches its edge.
        solids.set_things(vec![ThingSolid {
            index: 77, centre: [0.0; 3], origin: [0.0; 3], radius: 4.0,
            z_min: -4.0, z_max: 4.0, sector: 0, half: None, yaw: 0.0,
            mesh: Some(vec![vec![Face {
                normal: [0.0, 0.0, 1.0],
                point: [0.0, 0.0, -0.1],
                verts: vec![[0.03, -2.0, -0.1], [2.0, -2.0, -0.1], [2.0, 2.0, -0.1], [0.03, 2.0, -0.1]],
                surface: usize::MAX,
            }]].into()),
            thing_flags: 0x40, collide: 3, owner: None, kind: 7, item: false,
        }]);
        let mut state = floor_state([0.0, 0.0, 0.0], None, 0);
        solids.find_floor(&mut state, 0.065, 0.12, Gates::default(), false);
        assert_eq!(state.attach, None, "a kind-0 body never takes the radius pass");
        let mut state = floor_state([0.0, 0.0, 0.0], None, 10);
        solids.find_floor(&mut state, 0.065, 0.12, Gates::default(), false);
        assert!(state.attach.is_some(), "the player's radius pass reaches the offset floor");
    }

    /// `0x4f5550:106-123`: under water the player (kind 10) runs the surface search and no other
    /// kind's attach is touched at all.
    #[test]
    fn under_water_only_the_player_runs_the_surface_branch() {
        let solids = room();
        let gates = Gates { sectors: &[SECTOR_UNDERWATER], adjoins: &[] };
        let mut state = floor_state([0.0, 0.0, -0.5], Some(FLAT), 3);
        solids.find_floor(&mut state, 0.065, 0.12, gates, false);
        assert_eq!(state.attach, Some(FLAT), "a non-player keeps its seat under water");
        let mut state = floor_state([0.0, 0.0, -0.5], Some(FLAT), 10);
        solids.find_floor(&mut state, 0.065, 0.12, gates, false);
        assert_eq!(state.attach, Some(FLAT), "the water branch never detaches");
        assert_eq!(state.water_surface, None, "no dry crossing above means no surface hold");
    }
    #[test]
    fn adjoin_discovery_finds_a_thing_outside_the_crossing_polygon() {
        let solids = corridor();
        let mut thing = contact_model(vec![vec![contact_face([0.0, -1.0, 0.0], [3.0, 0.2, 0.5])]]);
        thing.sector = 1;
        thing.origin = [3.0, 0.2, 0.5];
        solids.set_things(vec![thing]);
        let mut search = CollisionSearch::new();
        assert_eq!(solids.search_for_collisions(0, None, [3.0, -0.1, 0.5], [0.0, 1.0, 0.0], 0.5, 0.1, Gates::default(), 0, &mut search), 0.5);
        let Some(CollisionRecord::Solid(hit)) = search.pop() else { panic!("the radius-bearing discovery lost the neighboring thing") };
        assert_eq!(hit.thing.unwrap().thing, 77);
        assert!((hit.distance - 0.2).abs() < 1e-6);
        assert!(search.pop().is_none(), "a centre outside the polygon must not cross");
    }

    #[test]
    fn adjacent_thing_expansion_does_not_recurse_into_new_sectors() {
        let solids = corridor();
        let mut near = contact_model(vec![vec![contact_face([0.0, -1.0, 0.0], [0.0, -1.7, 0.5])]]);
        near.sector = 1;
        near.origin = [0.0, 0.2, 0.5];
        let mut far = contact_model(vec![vec![contact_face([0.0, -1.0, 0.0], [0.0, -1.8, 0.5])]]);
        far.index = 78;
        far.sector = 2;
        far.origin = [0.0, 8.2, 0.5];
        far.radius = 12.0;
        solids.set_things(vec![near, far]);
        let mut search = CollisionSearch::new();
        solids.search_for_collisions(0, None, [0.0, -2.0, 0.5], [0.0, 1.0, 0.0], 0.5, 0.1, Gates::default(), 0x800, &mut search);
        let Some(CollisionRecord::Solid(hit)) = search.pop() else { panic!("the adjacent thing was not searched") };
        assert_eq!(hit.thing.unwrap().thing, 77, "the closer thing two unsearched sectors away is not eligible");
        assert!((hit.distance - 0.2).abs() < 1e-6);
    }

    #[test]
    fn an_end_contact_is_refused_before_an_equal_distance_crossing() {
        let mut solids = corridor();
        let portal = &solids.portals[0][0];
        solids.faces.push(Face { normal: portal.normal, point: portal.point, verts: portal.verts.clone(), surface: 0 });
        solids.sectors[0].push(Plane { normal: portal.normal, point: portal.point, surface: 0 });
        solids.face_of.insert(0, 0);
        solids.adjoin_surface = vec![1, 2, 3, 4];
        let mut vel = [0.0, 1.0, 0.0];
        let (pos, sector, traveled) = solids.move_thing(0, [0.0, -1.0, 0.5], [0.0, 1.0, 0.0], 0.0, Gates::default(), &mut vel);
        assert_eq!(sector, 1, "CollideHurt refuses the zero remainder, so the equal-distance crossing still runs");
        assert_eq!(pos, [0.0, 0.0, 0.5]);
        assert_eq!(vel, [0.0, 1.0, 0.0]);
        assert_eq!(traveled, 1.0);
    }

    #[test]
    fn the_fourth_accepted_crossing_resets_velocity_even_with_no_remainder() {
        let solids = corridor();
        let mut state = MoveState {
            pos: [0.0, -1.0, 0.5], sector: 0, vel: [0.0, 1.0, 0.0], remainder: [0.0; 3],
            phys_flags: 0x4a4f, kind: 10, move_type: 1, collide: 1, flags: 0, attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        let traveled = solids.move_thing_with(&mut state, [0.0, 4.0, 0.0], 0.1, 0,
            |state, dir, distance, radius, flags, search| {
                solids.search_for_collisions(state.sector, Some(state), state.pos, dir, distance, radius, Gates::default(), flags, search);
            },
            |state, surface, far| {
                state.sector = far;
                if surface.is_some() {
                    state.pos = [0.0, -1.0, 0.5];
                    state.sector = 0;
                }
            },
            |state| solids.inside_sector_with_gates(state.sector, state.pos, 0.0, Gates::default()),
            |_| {},
            |state, hit, crease| solids.probe_contact(state, hit, crease),
        );
        assert_eq!(state.pos, [0.0, -1.0, 0.5]);
        assert_eq!(traveled, 4.0);
        assert_eq!(state.vel, [0.0; 3]);
        assert_eq!(state.remainder, [0.0; 3]);
    }

    /// `0x4e7950:104-134`: the stack is consumed nearest first, and each consumed record's hit
    /// distance advances the thing from the SUB-STEP start — not from the previous record's
    /// contact. The crossing at 0.3 only switches the sector (the position is unchanged, so the
    /// loop continues, `:177-184`); the solid at 0.6 stops the round with `start + 0.6 * dir`.
    #[test]
    fn records_are_consumed_nearest_first_with_the_advance_from_the_sub_step_start() {
        let solids = room();
        let mut state = MoveState {
            pos: [0.0, -1.0, 0.5], sector: 0, vel: [0.0, 1.0, 0.0], remainder: [0.0; 3],
            phys_flags: 0x4a4f, kind: 10, move_type: 1, collide: 1, flags: 0, attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        let mut first = true;
        let traveled = solids.move_thing_with(&mut state, [0.0, 1.0, 0.0], 0.1, 0,
            |_, _, _, _, _, search| {
                if first {
                    first = false;
                    search.push(CollisionRecord::Crossing { distance: 0.3, sector: 7, surface: 0 });
                    search.push(CollisionRecord::Solid(Sweep {
                        distance: 0.6, normal: [0.0, -1.0, 0.0], point: [0.0, 1.0, 0.5],
                        sector: 0, surface: 5, kind: 4, thing: None,
                    }));
                }
            },
            |state, surface, far| { if surface.is_some() { state.sector = far; } },
            |_| true,
            |_| {},
            |state, hit, crease| solids.probe_contact(state, hit, crease),
        );
        assert_eq!(state.sector, 7, "the nearer crossing was consumed before the solid");
        assert!(
            (state.pos[1] - -0.4).abs() < 1e-3,
            "the advance is start + 0.6 * dir, not 0.3 + 0.6: {}",
            state.pos[1]
        );
        assert!((traveled - 0.6).abs() < 1e-2, "the round's total is the hit distance: {traveled}");
    }

    /// `0x4e7950:251-264`: when the tail's containment (`IsSphereInSector` at radius 0) fails,
    /// the thing's pose AND sector are restored from the LAST sub-step's start — the engine
    /// never searches for another sector. The move itself still reports its travel.
    #[test]
    fn containment_failure_restores_the_last_sub_step_start() {
        let solids = corridor();
        let mut state = MoveState {
            pos: [0.0, -1.0, 0.5], sector: 0, vel: [0.0, 1.0, 0.0], remainder: [0.0; 3],
            phys_flags: 0x4a4f, kind: 10, move_type: 1, collide: 1, flags: 0, attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        let traveled = solids.move_thing_with(&mut state, [0.0, 2.0, 0.0], 0.1, 0,
            |state, dir, distance, radius, flags, search| {
                solids.search_for_collisions(state.sector, Some(state), state.pos, dir, distance, radius, Gates::default(), flags, search);
            },
            |state, _, far| { state.sector = far; },
            |_| false,
            |_| {},
            |state, hit, crease| solids.probe_contact(state, hit, crease),
        );
        assert_eq!(traveled, 2.0, "the move completed before the tail undid the pose");
        assert_eq!(state.pos, [0.0, -1.0, 0.5]);
        assert_eq!(state.sector, 0, "the restore returns the sub-step start sector");
    }

    /// `0x4e7950:279-296`: the tail runs FindFloor only for a physics thing that travelled,
    /// without caller flag `0x40`, and — unattached, including attach-flag 8 (`:284`) — only
    /// under physflag `0x40` with a vertical speed in `[-2.0, 0.2]` (`0x523018`/`0x52301c`);
    /// an attached body always re-finds its floor.
    #[test]
    fn the_find_floor_tail_gates_on_travel_attachment_and_landing_speed() {
        let solids = room();
        let run = |attach: Option<Attach>, attach_flags: u32, phys_flags: u32, vel: [f32; 3], flags: u32| {
            let mut state = MoveState {
                pos: [0.0, 0.0, 0.0], sector: 0, vel, remainder: [0.0; 3],
                phys_flags, kind: 10, move_type: 1, collide: 1, flags: 0,
                attach, attach_flags, touched: Vec::new(), water_surface: None, owner: None,
            };
            let mut ran = false;
            solids.move_thing_with(&mut state, [0.0, 1.0, 0.0], 0.1, flags,
                |_, _, _, _, _, _| {},
                |_, _, _| {},
                |_| true,
                |_| { ran = true; },
                |state, hit, crease| solids.probe_contact(state, hit, crease),
            );
            ran
        };
        assert!(run(None, 0, 0x4a4f, [0.0, 1.0, -1.0], 0), "physflag 0x40 and a landing speed find the floor");
        assert!(!run(None, 0, 0x4a4f & !0x40, [0.0, 1.0, -1.0], 0), "no physflag 0x40, no search");
        assert!(!run(None, 0, 0x4a4f, [0.0, 1.0, -3.0], 0), "falling faster than -2.0, no search");
        assert!(!run(None, 0, 0x4a4f, [0.0, 1.0, -1.0], 0x40), "caller flag 0x40 skips the tail");
        assert!(!run(Some(FLAT), 8, 0, [0.0, 1.0, -1.0], 0), "attach flag 8 takes the unattached branch");
        assert!(run(Some(FLAT), 8, 0x40, [0.0, 1.0, -1.0], 0), "attach flag 8 with physflag 0x40 and the band searches");
        assert!(run(Some(FLAT), 0, 0, [0.0, 1.0, -9.0], 0), "an attached body always re-finds its floor");
    }

    /// `0x4e7950` has no lateral step: `sithThing_UpdateMove` (`0x4cc4c0`) hands the sphere one
    /// move, and the slide (`0x4e9090`) is the only wall answer, so a riser is a wall to a body
    /// whose span meets it (`0x508be0`: the plane gap closes at `above - radius`) and open air to
    /// one whose contact centre sits above its top edge (the coverage test, `0x508990`). Nothing
    /// in the loop may lift the body over it.
    #[test]
    fn a_riser_blocks_a_body_below_its_top_and_passes_one_above_it() {
        let mut solids = room();
        solids.sectors[0].push(Plane {
            normal: [0.0, 1.0, 0.0], point: [0.0, 1.0, -0.75], surface: 9,
        });
        solids.faces.push(Face {
            normal: [0.0, 1.0, 0.0],
            point: [0.0, 1.0, -0.75],
            verts: vec![[-1.0, 1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 1.0, -0.5], [-1.0, 1.0, -0.5]],
            surface: 9,
        });
        solids.face_of.insert(9, solids.faces.len() - 1);
        let gates = Gates { sectors: &[], adjoins: &[] };
        // Below the top: the sphere's span meets the face, so the plane gap (1.0 - 0.5 - 0.1)
        // stops the body at the contact, sliding, with z untouched.
        let (pos, _, _) = solids.move_thing(0, [0.0, 0.5, -0.7], [0.0, 1.0, 0.0], 0.1, gates, &mut [0.0, 1.0, 0.0]);
        assert!(
            (pos[1] - 0.9).abs() < 1e-3,
            "the riser stopped the body at the sphere contact: {}",
            pos[1]
        );
        assert_eq!(pos[2], -0.7, "no lateral step: the body was not lifted over the riser");
        // Above the top: the contact centre is past the face's polygon, so no record at all.
        let (pos, _, _) = solids.move_thing(0, [0.0, 0.5, -0.3], [0.0, 1.0, 0.0], 0.1, gates, &mut [0.0, 1.0, 0.0]);
        assert!((pos[1] - 1.5).abs() < 1e-5, "a body above the riser's top edge walks over it: {}", pos[1]);
        assert_eq!(pos[2], -0.3);
    }

    #[test]
    fn a_tiny_initial_move_occurs_even_when_the_returned_travel_snaps_to_zero() {
        let (pos, _, traveled) = room().move_thing(0, [0.0; 3], [1e-6, 0.0, 0.0], 0.065, Gates::default(), &mut [0.0; 3]);
        assert_eq!(pos[0], 1e-6);
        assert_eq!(traveled, 0.0);
    }

    /// The FindFloor run itself (the gate lives in `move_thing_with` now): an attached body
    /// re-searches at `2 * GetThingHeight` and can move to a new floor; the unattached reach is
    /// `moveSize + 0.005`.
    #[test]
    fn the_floor_tail_run_respects_the_attach_state() {
        let solids = room();
        let mut state = MoveState {
            pos: [0.0, 0.0, -0.94], sector: 0, vel: [0.0; 3], remainder: [0.0; 3],
            phys_flags: 0, kind: 10, move_type: 1, collide: 1, flags: 0, attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        let prior = Some(Attach { surface: Some(77), ..FLAT });
        state.attach = prior;
        solids.finish_body(&mut state, 0.065, 0.12, Gates::default());
        assert_eq!(state.attach.unwrap().surface, Some(0), "an attached body re-searches at 2 * GetThingHeight");

        state.attach = None;
        state.phys_flags = 0x40;
        assert!(solids.finish_body(&mut state, 0.065, 0.12, Gates::default()));
        assert_eq!(state.attach.unwrap().surface, Some(0));
    }

    #[test]
    fn a_small_nonzero_folded_crease_still_projects_velocity_and_remainder() {
        let mut state = MoveState {
            pos: [0.0; 3], sector: 0, vel: [-1.0, 0.0, 0.0], remainder: [-1.0, 0.0, 0.0],
            phys_flags: 0x4a4f, kind: 10, move_type: 1, collide: 1, flags: 0, attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        assert!(Solids::collide_hurt(&mut state, [1.0, 1e-7, 0.0], 0.0, &mut Some([1.0, 0.0, 0.0]), false, |_, _| {}));
        assert_eq!(state.vel, [0.0; 3]);
        assert_eq!(state.remainder, [0.0; 3]);
    }

    /// `sithCollision_CollideHurt` (`0x4e9090:24-41`): a fresh face cancels only the into-normal
    /// component. The tangential component of velocity and remainder survives, and the cancelled
    /// component comes back as `(1.0001 - 1) * into` — the engine's slide restitution leaves a hair
    /// of outward push (`0x4e9090:38-41` reads 1.0001 unless physflag `0x20` is set).
    #[test]
    fn a_single_face_slide_preserves_the_tangential_component() {
        let mut state = MoveState {
            pos: [0.0; 3], sector: 0, vel: [-1.0, 0.5, 0.0], remainder: [-1.0, 0.5, 0.0],
            phys_flags: 0x4a4f, kind: 10, move_type: 1, collide: 1, flags: 0, attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        let mut crease = None;
        // A west wall's normal points east, into the room; the body moves north-west into it.
        assert!(Solids::collide_hurt(&mut state, [1.0, 0.0, 0.0], 0.5, &mut crease, false, |_, _| {}));
        assert_eq!(crease, Some([1.0, 0.0, 0.0]), "the fresh face stores the crease normal");
        assert_eq!(state.vel[1], 0.5, "the tangential velocity component is untouched");
        assert_eq!(state.remainder[1], 0.5, "the tangential remainder component is untouched");
        assert!((state.vel[0] - 0.0001).abs() < 1e-6, "{}", state.vel[0]);
        assert!((state.remainder[0] - 0.0001).abs() < 1e-6, "{}", state.remainder[0]);
        assert_eq!(state.vel[2], 0.0);
        assert_eq!(state.remainder[2], 0.0);
    }

    /// The physflag-`0x20` bounce (`0x4e9090:38-41`): the same contact reflects the into-normal
    /// component at restitution 1.9 instead of cancelling it.
    #[test]
    fn physflag_0x20_bounces_at_restitution_1_9() {
        let mut state = MoveState {
            pos: [0.0; 3], sector: 0, vel: [-1.0, 0.5, 0.0], remainder: [-1.0, 0.0, 0.0],
            phys_flags: 0x20, kind: 10, move_type: 1, collide: 1, flags: 0, attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        assert!(Solids::collide_hurt(&mut state, [1.0, 0.0, 0.0], 0.5, &mut None, false, |_, _| {}));
        assert!((state.vel[0] - 0.9).abs() < 1e-6, "1.9 * 1 - 1 reflected: {}", state.vel[0]);
        assert!((state.remainder[0] - 0.9).abs() < 1e-6, "{}", state.remainder[0]);
        assert_eq!(state.vel[1], 0.5, "even the bounce keeps the tangential component");
    }

    /// The two-face crease (`0x4e9090:71-112`): a second face met at distance 0 folds the stored
    /// normal against itself (`rdVector_Normalize3Acc` `0x449b97`) and cancels into that crease.
    /// With `n1 = +x` and `n2 = (x+y)/sqrt(2)` the fold is `normalize(n1 - n2 * (n2 . n1)) =
    /// (x - y)/sqrt(2)`, and a remainder the first face slid to `[0.0001, -1, 0]` ends tangent to
    /// the second face, riding the crease.
    #[test]
    fn a_second_face_at_zero_distance_folds_the_crease() {
        let mut state = MoveState {
            pos: [0.0; 3], sector: 0, vel: [0.0; 3], remainder: [-1.0, -1.0, 0.0],
            phys_flags: 0x4a4f, kind: 10, move_type: 1, collide: 1, flags: 0, attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        let mut crease = None;
        assert!(Solids::collide_hurt(&mut state, [1.0, 0.0, 0.0], 0.25, &mut crease, false, |_, _| {}));
        assert!((state.remainder[0] - 0.0001).abs() < 1e-6, "{}", state.remainder[0]);
        assert_eq!(state.remainder[1], -1.0);

        let diag = std::f32::consts::FRAC_1_SQRT_2;
        assert!(Solids::collide_hurt(&mut state, [diag, diag, 0.0], 0.0, &mut crease, false, |_, _| {}));
        let folded = crease.unwrap();
        assert!((folded[0] - diag).abs() < 1e-6, "the fold is normalize([0.5, -0.5, 0]): {}", folded[0]);
        assert!((folded[1] + diag).abs() < 1e-6, "{}", folded[1]);
        assert_eq!(folded[2], 0.0);
        // The remainder is tangent to the second face and parallel to the crease: the into-n2
        // component `FRAC_1_SQRT_2 * (1 - 0.0001)` is gone, leaving [0.50005, -0.50005, 0].
        let tangent = state.remainder[0] * diag + state.remainder[1] * diag;
        assert!(tangent.abs() < 1e-4, "tangent to the second face: {tangent}");
        assert!((state.remainder[0] - 0.50005).abs() < 1e-4, "{}", state.remainder[0]);
        assert!((state.remainder[1] + 0.50005).abs() < 1e-4, "{}", state.remainder[1]);
        // Its into-crease component is negative (it rides the crease outward), so the final
        // cancellation (`0x4e9090:102-112`) does not fire — the direction stands.
        let along = state.remainder[0] * folded[0] + state.remainder[1] * folded[1];
        assert!(along > 0.0, "parallel to the folded crease: {along}");
    }

    /// `0x4e9090:42` gates the crease on `distance == 0`: a second face met at a nonzero distance
    /// starts a fresh slide instead — the stored crease is replaced and restitution applies.
    #[test]
    fn a_second_face_at_nonzero_distance_starts_a_fresh_crease() {
        let mut state = MoveState {
            pos: [0.0; 3], sector: 0, vel: [0.0; 3], remainder: [-1.0, -1.0, 0.0],
            phys_flags: 0x4a4f, kind: 10, move_type: 1, collide: 1, flags: 0, attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        let mut crease = None;
        assert!(Solids::collide_hurt(&mut state, [1.0, 0.0, 0.0], 0.25, &mut crease, false, |_, _| {}));
        // The first slide left the remainder at [0.0001, -1, 0]; the second face still has an
        // approach of 1 along its normal, so the fresh branch runs with restitution.
        assert!(Solids::collide_hurt(&mut state, [0.0, 1.0, 0.0], 0.5, &mut crease, false, |_, _| {}));
        assert_eq!(crease, Some([0.0, 1.0, 0.0]), "the fresh branch replaced the stored normal");
        assert!((state.remainder[0] - 0.0001).abs() < 1e-6, "{}", state.remainder[0]);
        // The crease branch would cancel exactly (leaving 0); restitution leaves 0.0001.
        assert!((state.remainder[1] - 0.0001).abs() < 1e-6, "{}", state.remainder[1]);
    }

    /// `0x4e9090:28-37`: an approach at or under the engine's 1e-5 epsilon (`0x52300c`) is zeroed
    /// and refused — no slide, no crease. A move away from the face (negative into-normal) and a
    /// non-physics mover are refused the same way (`0x4e9090:21-23`).
    #[test]
    fn an_approach_under_the_engine_epsilon_or_away_is_refused() {
        let mut state = MoveState {
            pos: [0.0; 3], sector: 0, vel: [-1.0, 0.0, 0.0], remainder: [-5e-6, 1.0, 0.0],
            phys_flags: 0x4a4f, kind: 10, move_type: 1, collide: 1, flags: 0, attach: None, attach_flags: 0, touched: Vec::new(), water_surface: None, owner: None,
        };
        let mut crease = None;
        assert!(!Solids::collide_hurt(&mut state, [1.0, 0.0, 0.0], 0.5, &mut crease, false, |_, _| {}));
        assert_eq!(crease, None);
        assert_eq!(state.remainder, [-5e-6, 1.0, 0.0], "the state is untouched");
        assert_eq!(state.vel, [-1.0, 0.0, 0.0]);

        state.remainder = [1.0, 0.0, 0.0];
        assert!(!Solids::collide_hurt(&mut state, [1.0, 0.0, 0.0], 0.5, &mut crease, false, |_, _| {}), "moving away");

        state.remainder = [-1.0, 0.0, 0.0];
        state.move_type = 0;
        assert!(!Solids::collide_hurt(&mut state, [1.0, 0.0, 0.0], 0.5, &mut crease, false, |_, _| {}), "not a physics mover");
    }

    #[test]
    fn overlap_flag_adds_radius_to_existing_plane_output_and_rejects_the_back_side() {
        let mut distance = 2.0;
        assert_eq!(sphere_plane([0.0; 3], [1.0, 0.0, 0.0], 1.0, 0.1, [0.0, 1.0, 0.0], [0.0; 3], 0x400, &mut distance), Some(2.1));
        assert_eq!(sphere_plane([0.0, -0.01, 0.0], [1.0, 0.0, 0.0], 1.0, 0.1, [0.0, 1.0, 0.0], [0.0; 3], 0x400, &mut distance), None);
        assert_eq!(distance, 2.1);
    }
}
