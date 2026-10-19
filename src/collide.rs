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

#[derive(Clone)]
pub struct Solids {
    /// Per sector, the planes of its solid surfaces.
    pub sectors: Vec<Vec<Plane>>,
    /// Per sector, the openings in its hull: a portal surface names the sector on the
    /// other side in its `adjoin` field, which is what makes a doorway a doorway.
    pub portals: Vec<Vec<Portal>>,
    /// Per sector, a point inside its hull and the radius that encloses that hull from
    /// there. Derived from the sector's own vertices, so it is tighter than the file's
    /// bounding radius.
    pub spheres: Vec<([f32; 3], f32)>,
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
    pub verts: Vec<[f32; 3]>,
    pub normal: [f32; 3],
    pub point: [f32; 3],
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

/// Which sector owns each surface: `sector.surfaces` is a `(first, count)` range into the
/// surface table, so this is a flat expansion of it. `usize::MAX` for a surface no sector
/// claims.
pub fn surface_sectors(level: &crate::jkl::Level) -> Vec<usize> {
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
pub fn surface_claims(level: &crate::jkl::Level) -> Vec<Vec<usize>> {
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
    let surface_sector = surface_sectors(level);
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
    // The sector across an adjoin: its mirror's surface, and that surface's sector.
    let across = |adjoin: i32| -> Option<usize> {
        let a = usize::try_from(adjoin).ok()?;
        let mirror = usize::try_from(level.adjoins.get(a)?.mirror).ok()?;
        let surface = *adjoin_surface.get(mirror)?;
        let sector = *surface_sector.get(surface)?;
        (sector != usize::MAX).then_some(sector)
    };

    let mut floors: Vec<Face> = Vec::new();
    let mut walls: Vec<Face> = Vec::new();
    let mut faces: Vec<Face> = Vec::new();
    let mut sectors = Vec::with_capacity(level.sectors.len());
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
            // Winding is not trustworthy, so take the normal from the first two edges.
            let Some(mut normal) = normalize(cross(sub(pts[1], pts[0]), sub(pts[2], pts[0]))) else {
                continue;
            };
            // Orient it by majority vote of the sector's vertices: this face's own
            // vertices lie on the plane and do not vote, and for a convex hull almost
            // everything else is behind.
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
            // An adjoined surface is an opening: the walker goes through it into the
            // sector its *mirror* adjoin belongs to, so it is remembered as a portal
            // rather than a wall.
            match across(surface.adjoin) {
                Some(adjoin) => {
                    let point = pts[0];
                    portals.push(Portal { adjoin, verts: pts, normal, point });
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
                    planes.push(Plane { normal, point: pts[0], surface: *surface_of })
                }
            }
        }
        let mut radius = 0.0f32;
        for p in &hull {
            let d = sub(*p, centre);
            radius = radius.max(dot(d, d).sqrt());
        }
        sectors.push(planes);
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
    Solids { sectors, portals: portal_sets, spheres, floors, walls, wall_of, floor_of, faces, face_of }
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

impl Solids {
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
    ) -> Option<Sweep> {
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
                return closest;
            }
            visited.push(sector);
            let mut wall: Option<Sweep> = None;
            let mut door: Option<(f32, usize)> = None;

            for plane in self.sectors.get(sector)? {
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
                if dot(dir, normal) >= 0.0 {
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
                    });
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
                if dot(dir, normal) >= 0.0 {
                    continue;
                }
                let Some(t) = sphere_plane(from, dir, left, radius, normal, face.point) else {
                    continue;
                };
                let at = [from[0] + dir[0] * t, from[1] + dir[1] * t, from[2] + dir[2] * t];
                if !sphere_face(at, radius, &face) {
                    continue;
                }
                if door.is_none_or(|(dt, _)| t < dt) {
                    door = Some((t, portal.adjoin));
                }
            }

            match (wall, door) {
                (Some(hit), Some((dt, _))) if hit.distance <= dt => return Some(hit),
                (Some(hit), None) => return Some(hit),
                (Some(hit), Some((dt, adjoin))) => {
                    closest.get_or_insert(hit);
                    // A doorway reached *at* the body is still a crossing; step past its plane so the
                    // next sector starts on the far side instead of testing the same plane again.
                    let advance = (dt + 1e-4).min(left);
                    from = [from[0] + dir[0] * advance, from[1] + dir[1] * advance, from[2] + dir[2] * advance];
                    left -= advance;
                    sector = adjoin;
                }
                (None, Some((dt, adjoin))) => {
                    let advance = (dt + 1e-4).min(left);
                    from = [from[0] + dir[0] * advance, from[1] + dir[1] * advance, from[2] + dir[2] * advance];
                    left -= advance;
                    sector = adjoin;
                }
                (None, None) => return closest,
            }
        }
        closest
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
    pub fn move_body(
        &self,
        sector: usize,
        pos: [f32; 3],
        delta: [f32; 3],
        radius: f32,
        step: f32,
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
            match self.sweep(here, at, dir, left, radius) {

                Some(hit) => {
                    blocked = true;
                    let travelled = (hit.point[0] - at[0]) * dir[0] + (hit.point[1] - at[1]) * dir[1];
                    at = hit.point;
                    here = hit.sector;
                    left -= travelled.max(0.0);
                    if left < 1e-4 {
                        break;
                    }
                    // A low obstruction is a step rather than a wall: retry the rest of the move from
                    // one step up. `hit.point` is the centre at contact, so the feet are `radius` below.
                    if hit.point[2] - radius < pos[2] - radius + step {
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
    let clear = [[1.0f32, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0]]
        .iter()
        .all(|d| {
            self.sweep(here, raised, *d, radius * 2.0, radius)
                .is_none_or(|h| h.distance > radius * 0.5)
        });
                        // A step must land *somewhere*: lifting the body at a spot where nothing is below it
                        // is how it walked off the walkway's seam and fell. Eight units is generous, so a
                        // step down into a room still works while a step over a void does not.
                        let lands = self
                            .floor_under(here, raised, 8.0, radius)
                            .is_some_and(|(_, travelled, _)| travelled <= 8.0);
                    
    if clear && lands && self.sweep(here, raised, dir, left, radius).is_none() {
                            at = raised;
                            continue;
                        }
                    }                    // Slide: drop the velocity component that ran into the surface.
                    let into = dir[0] * hit.normal[0] + dir[1] * hit.normal[1];
                    let slide = [dir[0] - hit.normal[0] * into, dir[1] - hit.normal[1] * into];
                    let mag = (slide[0] * slide[0] + slide[1] * slide[1]).sqrt();
                    if mag < 1e-4 || bounce == 2 {
                        break;
                    }
                    dir = [slide[0] / mag, slide[1] / mag, 0.0];
                }
                None => {
                    at = [at[0] + dir[0] * left, at[1] + dir[1] * left, at[2]];
                    break;
                }
            }
        }
        // No hull projection here: `advance` snaps the body onto a sector plane, which can *lower* it
        // through the floor it is standing on. The sweep already walked any doorway, so `here` is the
        // sector the body ended in.
        (at, here, blocked)
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
        // Every face with a polygon, not only the body's sector. `sithPhysics_FindFloor` searches the start
        // sector and the sectors its adjoins lead to, which works because the engine's sectors tile the
        // world; this port's data does not tile — a walkway slab can sit in a sector that nothing below it
        // links to — so a floor search that stays inside one sector drops the body through that seam. The
        // *lateral* answer stays sector-scoped, because a wall in another room must not block a corridor.
        let range = (height * 1.1).max(radius + 0.005);
        let mut best: Option<(f32, f32, usize)> = None;
        for face in &self.faces {
            // A vertical face is not a floor, and a downward one is a ceiling.
            if face.normal[2].abs() < 0.2 {
                continue;
            }
            let normal = if dot(sub(pos, face.point), face.normal) < 0.0 {
                [-face.normal[0], -face.normal[1], -face.normal[2]]
            } else {
                face.normal
            };
            if normal[2] < 0.2 {
                continue;
            }
            let Some(t) = sphere_plane(pos, [0.0, 0.0, -1.0], range, radius, normal, face.point) else {
                continue;
            };
            let at = [pos[0], pos[1], pos[2] - t];
            if !sphere_face(at, radius, face) {
                continue;
            }
            let floor = at[2] - radius;
            let owner = self
                .sectors
                .iter()
                .position(|planes| planes.iter().any(|p| p.surface == face.surface))
                .unwrap_or(sector);
            if best.is_none_or(|(near, _, _)| floor > near) {
                best = Some((floor, pos[2] - at[2], owner));
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
    let Some(normal) = normalize(cross(sub(verts[1], verts[0]), sub(verts[2], verts[0]))) else {
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

impl Solids {
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
            return true;
        }
    }
    false
}
}

impl Solids {
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

    /// How far inside a sector a point is: the smallest distance to any of its planes.
    /// A room gives a large number, a thin trigger sliver almost zero.
    pub fn clearance(&self, sector: usize, pos: [f32; 3]) -> Option<f32> {
        let planes = self.sectors.get(sector)?;
        planes.iter().map(|p| -dot(sub(pos, p.point), p.normal)).reduce(f32::min)
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
    fn sectors_at_exact(&self, pos: [f32; 3]) -> Vec<usize> {
        (0..self.sectors.len())
            .filter(|&i| self.clearance(i, pos).is_some_and(|c| c >= 0.0))
            .collect()
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
    pub fn visible_sectors(&self, seeds: &[usize], sees: &mut dyn FnMut(&[[f32; 3]]) -> bool) -> Vec<usize> {
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
                if seen[portal.adjoin] || !sees(&portal.verts) {
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
    pub fn advance(&self, sector: usize, pos: [f32; 3], radius: f32) -> (usize, [f32; 3]) {
        for portal in self.portals.get(sector).into_iter().flatten() {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An 8 x 4 x 8 room: floor at z = -1, ceiling at z = 1, walls at |x|, |y| = 2.
    /// Normals point out of the room, which is the convention `.jkl` data gives.
    fn room() -> Solids {
        Solids {
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
            faces: Vec::new(),
            face_of: std::collections::HashMap::new(),
        }
    }

    /// Two rooms joined by one opening, and a third room reachable only through the second.
    fn corridor() -> Solids {
        let quad = |z: f32| {
            vec![[-1.0, 0.0, z], [1.0, 0.0, z], [1.0, 0.0, z + 1.0], [-1.0, 0.0, z + 1.0]]
        };
        Solids {
            sectors: vec![Vec::new(), Vec::new(), Vec::new()],
            spheres: vec![([0.0, 0.0, 0.0], 4.0), ([0.0, 8.0, 0.0], 4.0), ([0.0, 16.0, 0.0], 4.0)],
            floors: Vec::new(),
            walls: Vec::new(),
            wall_of: std::collections::HashMap::new(),
            faces: Vec::new(),
            face_of: std::collections::HashMap::new(),
            floor_of: std::collections::HashMap::new(),
            portals: vec![
                vec![Portal { adjoin: 1, verts: quad(0.0), normal: [0.0, 1.0, 0.0], point: [0.0, 0.0, 0.0] }],
                vec![
                    Portal { adjoin: 0, verts: quad(0.0), normal: [0.0, -1.0, 0.0], point: [0.0, 0.0, 0.0] },
                    Portal { adjoin: 2, verts: quad(8.0), normal: [0.0, 1.0, 0.0], point: [0.0, 8.0, 0.0] },
                ],
                vec![Portal { adjoin: 1, verts: quad(8.0), normal: [0.0, -1.0, 0.0], point: [0.0, 8.0, 0.0] }],
            ],
        }
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
            sectors,
            portals: vec![Vec::new()],
            spheres: vec![([0.0, 0.0, 0.0], 4.0)],
            floors,
            walls,
            faces,
            face_of,
            wall_of: [(1usize, 0usize), (2, 1)].into_iter().collect(),
            floor_of: [(0usize, 0usize)].into_iter().collect(),
        }
    }

    #[test]
    fn the_sweep_stops_a_radius_short_of_a_wall() {
        // `sithIntersect_CheckSphereHit`: a body of radius 0.25 moving at the +x wall (x = 2) from the
        // origin travels 2 - 0.25 before it touches, and that is the whole point of the radius.
        let solids = boxed();
        let hit = solids
            .sweep(0, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 10.0, 0.25)
            .expect("the +x wall");
        assert!((hit.distance - 1.75).abs() < 1e-3, "distance {}", hit.distance);
        assert!((hit.normal[0] + 1.0).abs() < 1e-3, "normal {:?} should face the body", hit.normal);
    }

    #[test]
    fn the_sweep_misses_a_wall_the_sphere_never_reaches() {
        // Aiming down the middle of the room must not report either side wall.
        let solids = boxed();
        let hit = solids.sweep(0, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 1.0, 0.25);
        assert!(hit.is_none(), "reported {hit:?} along an open axis");
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
        solids.spheres.push(([0.0, 0.0, -1.5], 4.0));
        solids.sectors[0].retain(|p| p.normal[2] > -0.7);
        // `faces` is what the floor probe scans and it was cloned before the slab moved: keep it in step,
        // or the probe answers with the slab's old position.
        solids.faces = solids.floors.clone();
        solids.faces.extend(solids.walls.iter().cloned());
        solids.portals[0].push(Portal {
            adjoin: 1,
            verts: vec![[-2.0, -2.0, -1.0], [2.0, -2.0, -1.0], [2.0, 2.0, -1.0], [-2.0, 2.0, -1.0]],
            normal: [0.0, 0.0, -1.0],
            point: [0.0, 0.0, -1.0],
        });
        solids.portals.push(vec![Portal {
            adjoin: 0,
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

    #[test]
    fn walks_portals_only_where_the_camera_can_see() {
        let level = corridor();
        // Every portal on screen: the whole corridor, starting from the first room.
        let mut all = |_: &[[f32; 3]]| true;
        assert_eq!(level.visible_sectors(&[0], &mut all), vec![0, 1, 2]);
        // Nothing on screen: just the room the camera is in.
        let mut none = |_: &[[f32; 3]]| false;
        assert_eq!(level.visible_sectors(&[0], &mut none), vec![0]);
        // Only the first opening: the room behind it, but not the one past that.
        let mut first = |verts: &[[f32; 3]]| verts[0][2] < 4.0;
        assert_eq!(level.visible_sectors(&[0], &mut first), vec![0, 1]);
        // The walk is a graph search, so a cycle back to the start adds nothing.
        let mut again = |_: &[[f32; 3]]| true;
        assert_eq!(level.visible_sectors(&[1], &mut again).len(), 3);
        // Two seeds in the same room: the union, without duplicates.
        assert_eq!(level.visible_sectors(&[0, 1], &mut again), vec![0, 1, 2]);
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
        assert!((ceiling - 1.0).abs() < 1e-4, "ceiling = {ceiling}");
        // A point above the ceiling has nothing above it.
        assert!(room.ceiling_above(0, [0.0, 0.0, 2.0]).is_none());
        // Headroom is the difference, which is the number the camera uses.
        let headroom = ceiling - (-0.5);
        assert!((headroom - 1.5).abs() < 1e-4, "headroom = {headroom}");
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
}