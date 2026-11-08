//! The world-surface draw state: what each surface's cel, light and scroll are at a given
//! runtime time, and the batches those parameters build into.
//!
//! The runtime owns the tables; this module owns the composition. `SurfaceLook` is a snapshot of
//! those tables plus the level's sky header, so the batching pass never has to know which runtime
//! field means what.

use std::collections::HashMap;

use bevy::prelude::{Assets, Handle, Image, Mesh, Resource};

use crate::{jkl, runtime, Batch};

/// `surfflags` bit that marks a sky backdrop face.
///
/// Observed: every face carrying the level's sky material has `0x200` set in its
/// surface flags (`0x204`, `0x224`, `0x304`), while ordinary faces are `0x0`,
/// `0x4` or `0x5`. Sky faces are large vertical backdrop walls far outside the
/// play area, and their baked per-vertex intensity is zero.
pub(crate) const SURFFLAG_SKY: u32 = 0x200;

/// `SITH_SURFACE_CEILINGSKY` — the other half of the engine sky test. The port only knew the horizon bit.
pub(crate) const SURFFLAG_CEILING_SKY: u32 = 0x400;

pub(crate) fn is_sky_surface(surface: &jkl::Surface) -> bool {
    surface.surfflags & (SURFFLAG_SKY | SURFFLAG_CEILING_SKY) != 0
}

/// Whether a surface's **texture** moves without its geometry changing — it has a `SlideWall` rate or
/// the world-wide sky offset applies to it. Only such faces are re-laid by a scroll-only rebuild.
fn surface_scrolls(look: &SurfaceLook, index: usize, surface: &jkl::Surface) -> bool {
    is_sky_surface(surface) || look.scroll(index, surface.surfflags) != [0.0; 2]
}

/// What the scripts have changed about how the world looks: each surface's cel, each surface's extra
/// light and each sector's light. The renderer bakes all three into the mesh the level shader draws,
/// so they travel together — and a change to any of them is what `Runtime::surface_epoch` announces.
#[derive(Default, Clone)]
pub(crate) struct SurfaceLook {
    pub(crate) cels: Vec<f32>,
    /// Each surface's material cel count, so `(own cel + material cel)` wraps where the shader expects.
    pub(crate) counts: Vec<f32>,
    pub(crate) lights: Vec<f32>,
    /// Per sector, `AMBIENT LIGHT + EXTRA LIGHT`: the engine's per-sector camera ambient. It is the
    /// **base** of a world vertex's light product, not an addition on top of full brightness.
    pub(crate) ambient: Vec<f32>,
    pub(crate) sectors: Vec<f32>,
    /// `MaterialAnim`'s per-material cel (`rdMaterial.curCelNum`).
    pub(crate) materials: HashMap<i32, f32>,
    /// `ThingLight`'s light summed per sector: retail lights a radius around the thing, and this
    /// build's baked sector light is the granularity it has, so the sum is added to the sector's own.
    pub(crate) thing_light: Vec<f32>,
    /// `SetFaceLightMode`: a face whose mode is `RD_LIGHTMODE_FULLYLIT` (0) is drawn at full
    /// brightness whatever the bake says — that is what makes the CTF force fields glow.
    pub(crate) light_modes: Vec<i32>,
    /// `SlideWall`'s accumulated texel offset per surface — the engine adds `scrollVector * dt` to
    /// `texVertOffset` and wraps it at 1024, which from the clock is just `rate * time`.
    pub(crate) scrolls: Vec<[f32; 2]>,
    /// `SlideHorizonSky`/`SlideCeilingSky`'s world-wide offset, `[horizon, ceiling]`: every sky face
    /// of that kind moves by it.
    pub(crate) sky: [[f32; 2]; 2],
}

impl SurfaceLook {
    /// Read the live runtime; the vectors are indexed by surface and by sector. `level` supplies the
    /// header's starting sky offsets (`Horizon Sky Offset`/`Ceiling Sky Offset`), which the plane pass
    /// begins at; `SlideHorizonSky`/`SlideCeilingSky` then move it by `rate * time`.
    pub(crate) fn of(rt: &runtime::Runtime, level: &jkl::Level) -> Self {
        SurfaceLook {
            cels: rt.surfaces.iter().map(|s| s.cel).collect(),
            counts: rt.surface_cels.iter().map(|&c| c as f32).collect(),
            lights: rt.surfaces.iter().map(|s| s.light).collect(),
            ambient: rt.sector_light_baked.clone(),
            sectors: rt.sectors.iter().map(|s| s.light).collect(),
            materials: rt.material_cel.iter().map(|(&k, &v)| (k, v)).collect(),
            scrolls: rt
                .surfaces
                .iter()
                .map(|s| {
                    [
                        (s.slide[0] * rt.time).rem_euclid(1024.0),
                        (s.slide[1] * rt.time).rem_euclid(1024.0),
                    ]
                })
                .collect(),
            sky: [
                [
                    (level.horizon_sky_offset[0] + rt.sky_scroll[0][0] * rt.time)
                        .rem_euclid(1024.0),
                    (level.horizon_sky_offset[1] + rt.sky_scroll[0][1] * rt.time)
                        .rem_euclid(1024.0),
                ],
                [
                    (level.ceiling_sky_offset[0] + rt.sky_scroll[1][0] * rt.time)
                        .rem_euclid(1024.0),
                    (level.ceiling_sky_offset[1] + rt.sky_scroll[1][1] * rt.time)
                        .rem_euclid(1024.0),
                ],
            ],
            thing_light: rt.thing_light_by_sector(),
            light_modes: (0..rt.surfaces.len())
                .map(|s| rt.face_light_mode.get(&s).copied().unwrap_or(3))
                .collect(),
        }
    }

    /// The texel offset a scrolling surface's texture is drawn at: its own (`SlideWall`) or, for a
    /// sky face, the world-wide sky offset of that kind.
    pub(crate) fn scroll(&self, surface: usize, flags: u32) -> [f32; 2] {
        let sky = if flags & SURFFLAG_CEILING_SKY != 0 {
            self.sky[1]
        } else if flags & SURFFLAG_SKY != 0 {
            self.sky[0]
        } else {
            [0.0; 2]
        };
        let own = self.scrolls.get(surface).copied().unwrap_or([0.0; 2]);
        [own[0] + sky[0], own[1] + sky[1]]
    }

    /// The cel a surface draws: its own, which is 0 unless a script moved it, plus the animated cel of
    /// its material — `rdActive` uses the material's (`cel == -1`) when the surface has none of its
    /// own, and every shipped surface starts there.
    pub(crate) fn cel(&self, surface: usize, material: i32) -> f32 {
        let own = self.cels.get(surface).copied().unwrap_or(0.0).max(0.0);
        let material = self.materials.get(&material).copied().unwrap_or(0.0);
        let total = own + material;
        let count = self.counts.get(surface).copied().unwrap_or(1.0).max(1.0);
        if count > 1.0 {
            total % count
        } else {
            0.0
        }
    }

    /// `RD_LIGHTMODE_FULLYLIT`: the face is drawn at full brightness whatever the bake says.
    pub(crate) fn fullbright(&self, surface: usize) -> bool {
        self.light_modes.get(surface).copied() == Some(0)
    }

    /// The factor the level's baked vertex light is multiplied by: the engine adds a surface's and a
    /// sector's `extraLight` on top of the baked term, so 0 means "as the level was built".
    pub(crate) fn light(&self, surface: usize, sector: i32) -> f32 {
        let surface = self.lights.get(surface).copied().unwrap_or(0.0);
        let index = usize::try_from(sector).unwrap_or(usize::MAX);
        let sector_own = self.sectors.get(index).copied().unwrap_or(0.0);
        // `ThingLight`: the lamps standing in this sector, summed (see `thing_light_by_sector`).
        let things = self.thing_light.get(index).copied().unwrap_or(0.0);
        // The sector's baked ambient is the base: `rdCamera_SetAmbientLight` takes
        // `ambient + extraLight` per sector, so a dark room is a low base, not `1.0 + deltas`.
        let base = self.ambient.get(index).copied().unwrap_or(1.0);
        (base + surface + sector_own + things).clamp(0.0, 4.0)
    }
}

/// The surface draw state for one frame: a runtime snapshot plus the batching pass that consumes it.
/// Callers hold this value instead of passing a ten-field look snapshot through every build call.
#[derive(Default)]
pub(crate) struct SurfaceScene {
    look: SurfaceLook,
}

impl SurfaceScene {
    /// Snapshot the runtime's surface tables and the level's sky header.
    pub(crate) fn new(rt: &runtime::Runtime, level: &jkl::Level) -> Self {
        Self {
            look: SurfaceLook::of(rt, level),
        }
    }

    /// Build from a caller-provided snapshot. The CLI probes use this to isolate one column; the
    /// normal path is [`Self::new`].
    pub(crate) fn from_look(look: SurfaceLook) -> Self {
        Self { look }
    }

    /// Build the world's batches from this snapshot.
    pub(crate) fn build(
        &self,
        level: &jkl::Level,
        geometry: &crate::collide::LevelGeometry,
        images: &HashMap<i32, Image>,
        hidden: &std::collections::BTreeSet<usize>,
        only_scrolling: bool,
    ) -> Built {
        build_world(level, geometry, images, hidden, &self.look, only_scrolling)
    }
}

/// The light for a drawn surface. `owner` is the single sector the level lists the surface under;
/// duplicate claimant batches reuse it so the same polygon cannot draw at two brightnesses.
pub(crate) fn surface_light(look: &SurfaceLook, surface: usize, owner: Option<usize>) -> f32 {
    owner.map_or_else(
        || look.light(surface, -1),
        |sector| look.light(surface, sector as i32),
    )
}

/// The world's surface batches plus the count of skipped faces and the surface indices behind each
/// key. `SurfaceBatches.0` preserves spawn order; these maps are keyed and unordered, and a rebuild
/// patches every claimant mesh in place.
pub(crate) struct Built {
    pub(crate) batches: HashMap<(usize, i32), Batch>,
    pub(crate) skipped: usize,
    pub(crate) keys: HashMap<(usize, i32), Vec<usize>>,
}

/// The world's surface batches: which mesh asset draws which `(sector, material)`. A script's
/// `SetFaceGeoMode` has to stop drawing polygons, so the rebuild patches these meshes in place
/// rather than respawning the sector's entities.
#[derive(Resource, Default)]
pub(crate) struct SurfaceBatches(
    pub(crate) Vec<((usize, i32), Handle<Mesh>)>,
    pub(crate) HashMap<(usize, i32), Vec<usize>>,
);

impl SurfaceBatches {
    /// Patch every claimant mesh in place. A scroll-only rebuild leaves a mixed batch alone: the
    /// filtered build does not contain its static polygons, so replacing it would drop geometry.
    pub(crate) fn apply(
        &self,
        built: &Built,
        meshes: &mut Assets<Mesh>,
        scroll_only: bool,
    ) -> usize {
        let mut patched = 0usize;
        for (key, handle) in &self.0 {
            if scroll_only
                && built.keys.get(key).map(Vec::len).unwrap_or(0)
                    < self.1.get(key).map(Vec::len).unwrap_or(0)
            {
                continue;
            }
            let Some(batch) = built.batches.get(key).cloned() else {
                continue;
            };
            if let Some(mut mesh) = meshes.get_mut(handle) {
                *mesh = crate::to_mesh(batch);
                patched += 1;
            }
        }
        patched
    }
}

/// Batch every drawable surface by material, so the world is one entity per texture rather than one
/// per polygon.
///
/// `tex_size` is only needed to normalise texture vertices, so a caller that just wants to know
/// whether the geometry builds can pass an empty image map.
///
/// Batches are keyed by `(sector, material)`: one entity per room per texture is what lets the portal
/// walk in `collide::visible_sectors` hide whole rooms with a `Visibility` flip. Keying by material
/// alone drew the level as a handful of all-or-nothing meshes and made sector culling impossible.
fn build_world(
    level: &jkl::Level,
    geometry: &crate::collide::LevelGeometry,
    images: &HashMap<i32, Image>,
    hidden: &std::collections::BTreeSet<usize>,
    look: &SurfaceLook,
    only_scrolling: bool,
) -> Built {
    let claims = &geometry.surface_claims;
    // A surface can be listed by several overlapping sector ranges (a trigger volume inside a
    // room, for instance). Drawing it in each visible claimant keeps culling correct, but the
    // engine has one owning sector per surface; using each claimant's ambient made the same
    // geometry change brightness with whichever list happened to draw last.
    let light_owners = &geometry.surface_sectors;
    let mut batches: HashMap<(usize, i32), Batch> = HashMap::new();
    let mut keys: HashMap<(usize, i32), Vec<usize>> = HashMap::new();
    let mut skipped = 0usize;
    for (index, surface) in level.surfaces.iter().enumerate() {
        if only_scrolling && !surface_scrolls(look, index, surface) {
            continue;
        }
        // Not drawn: no material, not a polygon, or a script set its geomode to `GEOMETRY_NONE`.
        if surface.material < 0 || surface.verts.len() < 3 || hidden.contains(&index) {
            skipped += 1;
            continue;
        }
        // A surface's frame comes from the script (`SetWallCel`, `SurfaceAnim`), and every surface
        // starts at cel 0 — the `.jkl` row carries no cel of its own.
        let cel = look.cel(index, surface.material);
        let scroll = look.scroll(index, surface.surfflags);
        let tex_size = images
            .get(&surface.material)
            .map(|img| {
                (
                    img.texture_descriptor.size.width as f32,
                    img.texture_descriptor.size.height as f32,
                )
            })
            .unwrap_or((1.0, 1.0));

        // A surface drawn in every sector that claims it: hiding any one claimant still
        // leaves it on screen as long as another is visible. Its light comes from the one
        // owning sector, so the duplicate draws cannot disagree about brightness.
        let owners = claims.get(index).map(Vec::as_slice).unwrap_or(&[]);
        let owner = light_owners
            .get(index)
            .copied()
            .filter(|sector| *sector != usize::MAX);
        let light = surface_light(look, index, owner);
        if owners.is_empty() {
            keys.entry((usize::MAX, surface.material))
                .or_default()
                .push(index);
            batches
                .entry((usize::MAX, surface.material))
                .or_default()
                .push_polygon(
                    level,
                    surface,
                    tex_size,
                    cel,
                    light,
                    scroll,
                    look.fullbright(index),
                );
        }
        for &sector in owners {
            keys.entry((sector, surface.material))
                .or_default()
                .push(index);
            batches
                .entry((sector, surface.material))
                .or_default()
                .push_polygon(
                    level,
                    surface,
                    tex_size,
                    cel,
                    light,
                    scroll,
                    look.fullbright(index),
                );
        }
    }
    Built {
        batches,
        skipped,
        keys,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One triangle in sector 0, material 0: enough for batching, no files or app.
    fn triangle_level() -> jkl::Level {
        let mut level = jkl::Level::default();
        level.vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        level.texverts = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        level.materials = vec!["test.mat".into()];
        level.sectors = vec![jkl::Sector {
            surfaces: (0, 1),
            ..Default::default()
        }];
        level.surfaces = vec![jkl::Surface {
            material: 0,
            surfflags: 0,
            geo: 4,
            adjoin: -1,
            verts: vec![(0, 0), (1, 1), (2, 2)],
            intensities: vec![1.0; 3],
            normal: [0.0, 0.0, 1.0],
        }];
        level
    }

    /// The batching pass still keys one triangle batch by the owning sector and material after the
    /// move out of `main.rs`. No files, no Bevy app: a synthetic level and the default look.
    #[test]
    fn build_world_batches_a_surface_under_its_sector() {
        let level = triangle_level();
        let geometry = crate::collide::build(&level);

        let built = SurfaceScene::default().build(
            &level,
            &geometry,
            &Default::default(),
            &Default::default(),
            false,
        );

        assert_eq!(built.skipped, 0);
        assert_eq!(built.batches.len(), 1);
        let batch = built
            .batches
            .get(&(0, 0))
            .expect("surface batch keyed by sector and material");
        assert_eq!(batch.indices.len(), 3, "one triangle");
        assert_eq!(built.keys.get(&(0, 0)).map(Vec::as_slice), Some(&[0][..]));
    }

    /// The batch pass reads the claims it is handed; it does not re-derive them from the level.
    #[test]
    fn scene_batches_the_geometry_claims() {
        let level = triangle_level();
        let mut geometry = crate::collide::build(&level);
        assert_eq!(geometry.surface_claims[0], vec![0]);

        geometry.surface_claims[0].clear();
        let built = SurfaceScene::default().build(
            &level,
            &geometry,
            &Default::default(),
            &Default::default(),
            false,
        );
        assert!(
            built.batches.contains_key(&(usize::MAX, 0)),
            "the unclaimed surface did not take the no-sector fallback"
        );
        assert!(
            !built.batches.contains_key(&(0, 0)),
            "the surface was re-derived into sector 0 after its geometry claim was cleared"
        );
    }

    /// `SurfaceBatches::apply` replaces the mesh behind the batch key; this is the path
    /// `rebuild_surfaces` uses when a script hides or restores a face. A scroll-only rebuild must
    /// skip a key whose filtered build has fewer surfaces than the live batch.
    #[test]
    fn apply_patches_the_batch_mesh_and_skips_a_mixed_scroll_batch() {
        let mut meshes = Assets::<Mesh>::default();
        let mut seeded = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::RENDER_WORLD,
        );
        seeded.insert_indices(bevy::mesh::Indices::U32(vec![0, 1, 2]));
        let handle = meshes.add(seeded);

        let mut batch = Batch::default();
        batch.indices = vec![0, 1, 2, 3, 4, 5];
        let built = Built {
            batches: HashMap::from([((0, 0), batch)]),
            skipped: 0,
            keys: HashMap::from([((0, 0), vec![0])]),
        };
        let batches = SurfaceBatches(
            vec![((0, 0), handle.clone())],
            HashMap::from([((0, 0), vec![0, 1])]),
        );
        let indices = |meshes: &Assets<Mesh>| {
            meshes
                .get(&handle)
                .and_then(|mesh| mesh.indices())
                .map(|indices| indices.len())
        };

        assert_eq!(batches.apply(&built, &mut meshes, true), 0);
        assert_eq!(
            indices(&meshes),
            Some(3),
            "a scroll-only rebuild replaced a mixed batch"
        );

        assert_eq!(batches.apply(&built, &mut meshes, false), 1);
        assert_eq!(indices(&meshes), Some(6), "the batch mesh was not replaced");
    }

    /// The scene is the only normal entry point: it snapshots the runtime once and builds the world
    /// from that snapshot. The synthetic level is one triangle in sector 0, material 0.
    #[test]
    fn scene_builds_from_the_runtime_snapshot() {
        let level = triangle_level();
        let geometry = crate::collide::build(&level);

        let world = runtime::Runtime::handle(&level);
        {
            let mut rt = world.borrow_mut();
            rt.surface_cels = vec![2];
            rt.material_cel.insert(0, 1.0);
        }
        let scene = SurfaceScene::new(&world.borrow(), &level);
        let built = scene.build(&level, &geometry, &Default::default(), &Default::default(), false);

        assert_eq!(built.batches.len(), 1);
        let alpha = built
            .batches
            .get(&(0, 0))
            .and_then(|batch| batch.colors.first())
            .map(|color| color[3]);
        assert_eq!(
            alpha,
            Some(1.0),
            "the scene ignored the runtime's material cel"
        );

        let default = SurfaceScene::default().build(
            &level,
            &geometry,
            &Default::default(),
            &Default::default(),
            false,
        );
        assert_eq!(
            default
                .batches
                .get(&(0, 0))
                .and_then(|batch| batch.colors.first())
                .map(|color| color[3]),
            Some(0.0),
            "the default look should not carry the runtime cel"
        );
    }
}
