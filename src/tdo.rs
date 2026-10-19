//! `.3do` model parser.
//!
//! Like `.jkl`, models are plain ASCII with `SECTION:` blocks and `#` comments:
//!
//! ```text
//! SECTION: MODELRESOURCE
//! MATERIALS 3
//!     0:   00crtesd.mat
//!
//! SECTION: GEOMETRYDEF
//! GEOSETS 4 / MESHES 16
//! MESH 0
//! VERTICES 16
//!     0:   0.597179  -0.400029  -0.299695   0.000000
//! TEXTURE VERTICES 48
//!     0: 256.885193  -0.239229
//! FACES 10
//! #  num:  material: type: geo: light: tex: extralight: verts:
//!     0:   0  0x0000  4  3  1  0.0000  4   0,  0   1,  1   5,  5   4,  4
//! ```
//!
//! **A model has one vertex pool per mesh, not per model.** 104 of the 477 shipped
//! models contain more than one `VERTICES` block — the player model has 64 — and
//! each mesh's faces index only that mesh's pool. Concatenating the pools into one
//! silently corrupts every face index after the first mesh.
//!
//! Face vertex pairs are `<vertex>,<texture vertex>`, indexing the mesh's own pool.
//! `material` indexes the model's own `MATERIALS` list, not the level's.

use crate::rows::{fields, floats_in, ints, read_rows};

pub struct Face {
    pub material: i32,
    pub verts: Vec<(u32, i32)>,
}

/// One mesh's private vertex pool and faces.
#[derive(Default)]
pub struct Mesh {
    /// `NAME <mesh name>` — how a `.key` names the mesh it poses.
    pub name: String,
    /// Which level of detail this mesh belongs to. A model's `GEOSETS` are the same
    /// puppet at several detail levels: `ky.3do` has 4 geosets of 16 meshes each, and
    /// all four describe the same body. Drawing them all stacks four copies on top of
    /// each other, so the renderer draws one.
    pub geoset: usize,
    /// Mesh number *within* its geoset, which is what a hierarchy node indexes.
    pub index: usize,
    pub vertices: Vec<[f32; 3]>,
    pub texverts: Vec<[f32; 2]>,
    pub faces: Vec<Face>,
}

/// One row of `SECTION: HIERARCHYDEF`: the rigid segments a puppet is built from.
///
/// `num: flags: type: mesh: parent: child: sibling: numChildren: x y z pitch yaw roll`
#[derive(Default, Clone, Copy)]
pub struct Node {
    /// Index into `Model::meshes`, or `-1` for a node with no geometry of its own.
    pub mesh: i32,
    /// Parent node, or `-1` at the root.
    pub parent: i32,
    /// Pivot, in model space.
    pub pos: [f32; 3],
    /// The node's rotation in its rest pose, in degrees. A `.key` frame at rest repeats
    /// these values exactly, so animation is the *difference* from them.
    pub rot: [f32; 3],
    /// The point the node rotates about, relative to `pos`. Vertices are stored relative
    /// to it, so a pose rotates them about the pivot and only then translates by `pos`.
    pub pivot: [f32; 3],
}

#[derive(Default)]
pub struct Model {
    pub materials: Vec<String>,
    pub meshes: Vec<Mesh>,
    /// `GEOSETS n` — how many levels of detail the mesh list holds.
    pub geosets: usize,
    /// Meshes per geoset, from the `MESHES m` line each geoset repeats.
    pub meshes_per_geoset: usize,
    /// The model's node hierarchy, in file order. `.key` frames index this.
    pub nodes: Vec<Node>,
    /// `RADIUS` from the header: the model's bounding radius **in the file's own units**, which the
    /// engine uses directly as a world quantity (`sithThing.c`: `pThing->moveSize = renderData.model3->radius`
    /// when the template names no `size`). Comparing it with the geometry says how big the model is meant to
    /// be, which no global scale can express.
    pub radius: f32,
    /// `INSERT OFFSET x y z`, in model space. The engine translates the whole
    /// model by this when placing it, and it is not small: even a crate carries
    /// ~0.3 model units (~3 world units), so ignoring it offsets every prop.
    pub insert_offset: [f32; 3],
}

impl Model {
    /// The name of the mesh a node owns, in the geoset that is drawn: how a `.key` names
    /// the node it poses. `None` for a `$$$DUMMY` node, which owns no mesh.
    pub fn node_mesh_name(&self, node: usize) -> Option<&str> {
        let n = self.nodes.get(node)?;
        let m = usize::try_from(n.mesh).ok()?;
        let at = if self.meshes_per_geoset > 0 {
            self.pick_geoset() * self.meshes_per_geoset + m
        } else {
            m
        };
        self.meshes.get(at).map(|x| x.name.as_str()).filter(|n| !n.is_empty())
    }

    pub fn is_drawable(&self) -> bool {
        self.meshes
            .iter()
            .any(|m| !m.vertices.is_empty() && m.faces.iter().any(|f| f.verts.len() >= 3))
    }

    /// Which detail level to draw: the one with the most vertices. `ky.3do` ships four
    /// geosets of the same body, the densest being 202 vertices against 120.
    pub fn pick_geoset(&self) -> usize {
        if self.geosets <= 1 || self.meshes_per_geoset == 0 {
            return 0;
        }
        (0..self.geosets)
            .map(|g| {
                let verts: usize = self
                    .meshes
                    .iter()
                    .filter(|m| m.geoset == g)
                    .map(|m| m.vertices.len())
                    .sum();
                (g, verts)
            })
            .max_by_key(|(_, v)| *v)
            .map(|(g, _)| g)
            .unwrap_or(0)
    }

    /// Which node owns each mesh, or `usize::MAX` when the mesh hangs off no node.
    ///
    /// A hierarchy node indexes a mesh **within its geoset** (`mesh: 12` is mesh 12 of
    /// every geoset), so the mapping uses `Mesh::index` rather than the flat position.
    pub fn mesh_nodes(&self) -> Vec<usize> {
        let mut out = vec![usize::MAX; self.meshes.len()];
        for (i, node) in self.nodes.iter().enumerate() {
            let Ok(want) = usize::try_from(node.mesh) else { continue };
            for (m, mesh) in self.meshes.iter().enumerate() {
                if mesh.index == want {
                    out[m] = i;
                }
            }
        }
        out
    }
}

/// A rigid transform: rotate, then translate. Kept as a 3x3 row-major matrix and a
/// vector so the parser stays independent of the renderer's math types.
#[derive(Clone, Copy)]
pub struct Xform {
    pub rot: [[f32; 3]; 3],
    pub pos: [f32; 3],
}

impl Default for Xform {
    fn default() -> Self {
        Self { rot: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], pos: [0.0; 3] }
    }
}

impl Xform {
    /// `self * other`: apply `other` first, as a child transform does.
    pub fn then(&self, child: &Xform) -> Xform {
        let mut rot = [[0.0f32; 3]; 3];
        for (r, row) in rot.iter_mut().enumerate() {
            for (c, slot) in row.iter_mut().enumerate() {
                *slot = (0..3).map(|k| self.rot[r][k] * child.rot[k][c]).sum();
            }
        }
        let p = child.pos;
        let pos = [
            self.rot[0][0] * p[0] + self.rot[0][1] * p[1] + self.rot[0][2] * p[2] + self.pos[0],
            self.rot[1][0] * p[0] + self.rot[1][1] * p[1] + self.rot[1][2] * p[2] + self.pos[1],
            self.rot[2][0] * p[0] + self.rot[2][1] * p[1] + self.rot[2][2] * p[2] + self.pos[2],
        ];
        Xform { rot, pos }
    }

    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        [
            self.rot[0][0] * p[0] + self.rot[0][1] * p[1] + self.rot[0][2] * p[2] + self.pos[0],
            self.rot[1][0] * p[0] + self.rot[1][1] * p[1] + self.rot[1][2] * p[2] + self.pos[1],
            self.rot[2][0] * p[0] + self.rot[2][1] * p[1] + self.rot[2][2] * p[2] + self.pos[2],
        ]
    }
}

/// The rotation part of a node's matrix, exactly as the engine builds it.
///
/// `rdMatrix_Build34` in Sith does not multiply three axis rotations: it evaluates nine
/// trig expressions into three orientation vectors, which are then the matrix *columns*
/// (`rdMatrix_TransformPoint34` multiplies each by x, y and z respectively). Pitch turns
/// about X, yaw about Z and roll about Y, and the cross terms make the order significant.
/// Transcribed from the reversed `rdMatrix_Build34` / `rdMatrix_TransformPoint34` pair.
fn rotate(pitch: f32, yaw: f32, roll: f32) -> [[f32; 3]; 3] {
    let (sx, cx) = pitch.to_radians().sin_cos();
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sz, cz) = roll.to_radians().sin_cos();
    // The engine's three orientation vectors, in its own row order.
    let rvec = [-(sz * sy) * sx + (cz * cy), (sz * cy) * sx + (cz * sy), -sz * cx];
    let lvec = [-sy * cx, cy * cx, sx];
    let uvec = [(cz * sy) * sx + (sz * cy), -sx * (cy * cz) + (sy * sz), cz * cx];
    // Those vectors are the columns.
    [
        [rvec[0], lvec[0], uvec[0]],
        [rvec[1], lvec[1], uvec[1]],
        [rvec[2], lvec[2], uvec[2]],
    ]
}

/// A node's own matrix: rotate by pitch/yaw/roll, then translate by `pos`.
fn local_matrix(pos: [f32; 3], rot: [f32; 3]) -> Xform {
    Xform { rot: rotate(rot[0], rot[1], rot[2]), pos }
}

/// The world transform of every node, from a keyframe (or from the model's rest pose when
/// `key` is `None`), at a (fractional) frame number.
///
/// A puppet's mesh vertices are **node-local**, so a mesh only lands in place after the
/// whole chain above it is composed. The chain is the engine's, from
/// `rdThing_AccumulateMatrices`:
///
/// ```text
/// joint[n] = joint[parent] * T(-parent.pivot) * [R(n.rot) | n.pos] * T(n.pivot)
/// joint[root] = [R | pos] * T(pivot)
/// ```
///
/// Two consequences the naive composition gets wrong: each node rotates its own mesh
/// about its own pivot but is placed relative to its *parent's* pivot, and a node's
/// parent is not necessarily an earlier row (`3do/actor/ky.3do` puts `k_lcalf` at row 3
/// with `k_lthigh` at row 8), so the chain has to be resolved by name, not by order.
///
/// A keyframe entry is the node's absolute local pose — pitch/yaw/roll in degrees plus
/// `x y z` — and replaces the rest values for that frame (`rdPuppet_BuildJointMatrices`
/// subtracts the rest pose, blends, and adds it back).
pub fn pose_nodes(model: &Model, key: Option<&crate::key::Key>, frame: f32) -> Vec<Xform> {
    // The key lists its nodes **alphabetically by mesh name** (`K_Hip`, `K_Lcalf`, …)
    // while the model lists its meshes in authoring order (`k_torso`, `k_neck`, …), so
    // the names are the mapping. Index order only agrees by luck (`3do/at.3do` happens
    // to match `atstand.key` row for row).
    let by_index = key.map(|k| k.nodes.len() == model.nodes.len()).unwrap_or(false);
    let mut out: Vec<Option<Xform>> = vec![None; model.nodes.len()];

    // Parents can follow their children in the node table, so resolve depth-first.
    fn resolve(
        i: usize,
        model: &Model,
        key: Option<&crate::key::Key>,
        frame: f32,
        by_index: bool,
        out: &mut Vec<Option<Xform>>,
    ) -> Xform {
        if let Some(done) = out[i] {
            return done;
        }
        let node = model.nodes[i];
        let entry = key.and_then(|k| {
            let by_name = model
                .node_mesh_name(i)
                .and_then(|name| k.nodes.iter().find(|n| n.mesh.eq_ignore_ascii_case(name)))
                .and_then(|n| n.at(frame));
            by_name.or_else(|| {
                // No name to match (a `$$$DUMMY` node): fall back to row order, which is
                // right exactly when the two files describe the same node list.
                if by_index {
                    k.nodes.get(i).and_then(|n| n.at(frame))
                } else {
                    None
                }
            })
        });
        // The node's local pose: the keyframe for this frame, or its own rest pose.
        let (pos, rot) = match entry {
            Some(f) => (f.pos, f.rot),
            None => (node.pos, node.rot),
        };
        let local = local_matrix(pos, rot);
        let pivot = node.pivot;
        let rotated_pivot = [
            local.rot[0][0] * pivot[0] + local.rot[0][1] * pivot[1] + local.rot[0][2] * pivot[2],
            local.rot[1][0] * pivot[0] + local.rot[1][1] * pivot[1] + local.rot[1][2] * pivot[2],
            local.rot[2][0] * pivot[0] + local.rot[2][1] * pivot[1] + local.rot[2][2] * pivot[2],
        ];
        let parent = usize::try_from(node.parent)
            .ok()
            .filter(|p| *p < model.nodes.len() && *p != i);
        let mut place = [
            pos[0] + rotated_pivot[0],
            pos[1] + rotated_pivot[1],
            pos[2] + rotated_pivot[2],
        ];
        let world = match parent {
            Some(p) => {
                let pp = model.nodes[p].pivot;
                place = [place[0] - pp[0], place[1] - pp[1], place[2] - pp[2]];
                resolve(p, model, key, frame, by_index, out).then(&Xform { rot: local.rot, pos: place })
            }
            None => Xform { rot: local.rot, pos: place },
        };
        out[i] = Some(world);
        world
    }

    for i in 0..model.nodes.len() {
        resolve(i, model, key, frame, by_index, &mut out);
    }
    out.into_iter().map(Option::unwrap_or_default).collect()
}

pub fn parse(text: &str) -> Model {
    let mut model = Model::default();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0usize;
    // `NAME` sits *above* `VERTICES` in this build's files, so it belongs to a mesh that
    // does not exist yet; holding it until the vertex pool is pushed is what keeps mesh
    // names aligned with mesh indices (they were off by one, which also mispaired every
    // keyframe node).
    let mut pending_name = String::new();

    while i < lines.len() {
        let line = lines[i].trim();
        let lower = line.to_ascii_lowercase();

        if lower.starts_with("radius ") {
            if let Some(v) = floats_in(line, 1, 1).first() {
                model.radius = *v;
            }
            i += 1;
            continue;
        }
        if lower.starts_with("insert offset") {
            let f = floats_in(line, 2, 3);
            if f.len() == 3 {
                model.insert_offset = [f[0], f[1], f[2]];
            }
            i += 1;
            continue;
        }
        if lower.starts_with("materials ") {
            let n = count_after(line);
            i += 1;
            for row in read_rows(&lines, &mut i, n) {
                if let Some(name) = fields(row).get(1) {
                    model.materials.push(name.to_string());
                }
            }
            continue;
        }
        // A new VERTICES block starts a new mesh's pool.
        if lower.starts_with("vertices ") {
            let n = count_after(line);
            i += 1;
            let mut vertices = Vec::with_capacity(n);
            for row in read_rows(&lines, &mut i, n) {
                let f = floats_in(row, 1, 3);
                if f.len() == 3 {
                    vertices.push([f[0], f[1], f[2]]);
                }
            }
            // `MESH n` comes before its `VERTICES`; `NAME` may come either side, so the
            // name is filled in below when it appears.
            let per = model.meshes_per_geoset;
            let index = if per > 0 { model.meshes.len() % per } else { model.meshes.len() };
            let geoset = if per > 0 { model.meshes.len() / per } else { 0 };
            model.meshes.push(Mesh {
                name: std::mem::take(&mut pending_name),
                vertices,
                geoset,
                index,
                ..Default::default()
            });
            continue;
        }
        if lower.starts_with("geosets ") {
            model.geosets = count_after(line);
            i += 1;
            continue;
        }
        // Each geoset repeats its own `MESHES m` line.
        if lower.starts_with("meshes ") && model.meshes.is_empty() {
            model.meshes_per_geoset = count_after(line);
            i += 1;
            continue;
        }
        if lower.starts_with("name ") {
            if let Some(name) = fields(line).get(1) {
                match model.meshes.last_mut() {
                    // `NAME` after a still-unnamed pool belongs to the mesh already pushed.
                    Some(mesh) if mesh.name.is_empty() && !mesh.vertices.is_empty() => {
                        mesh.name = name.to_string()
                    }
                    _ => pending_name = name.to_string(),
                }
            }
            i += 1;
            continue;
        }
        if lower.starts_with("hierarchy nodes") {
            let n = count_after(line);
            i += 1;
            for row in read_rows(&lines, &mut i, n) {
                let f = fields(row);
                // num: flags: type: mesh: parent: child: sibling: numChildren: x y z …
                if f.len() < 11 {
                    continue;
                }
                let mesh: i32 = f[3].parse().unwrap_or(-1);
                let parent: i32 = f[4].parse().unwrap_or(-1);
                let pos: Vec<f32> = (8..11).filter_map(|k| f.get(k)?.parse().ok()).collect();
                let rot: Vec<f32> = (11..14).filter_map(|k| f.get(k)?.parse().ok()).collect();
                let pivot: Vec<f32> = (14..17).filter_map(|k| f.get(k)?.parse().ok()).collect();
                if pos.len() == 3 {
                    model.nodes.push(Node {
                        mesh,
                        parent,
                        pos: [pos[0], pos[1], pos[2]],
                        rot: if rot.len() == 3 { [rot[0], rot[1], rot[2]] } else { [0.0; 3] },
                        pivot: if pivot.len() == 3 {
                            [pivot[0], pivot[1], pivot[2]]
                        } else {
                            [0.0; 3]
                        },
                    });
                }
            }
            continue;
        }
        if lower.starts_with("texture vertices ") {
            let n = count_after(line);
            i += 1;
            let mut texverts = Vec::with_capacity(n);
            for row in read_rows(&lines, &mut i, n) {
                let f = floats_in(row, 1, 2);
                if f.len() == 2 {
                    texverts.push([f[0], f[1]]);
                }
            }
            if let Some(mesh) = model.meshes.last_mut() {
                mesh.texverts = texverts;
            }
            continue;
        }
        if lower.starts_with("faces ") {
            let n = count_after(line);
            i += 1;
            let mut faces = Vec::with_capacity(n);
            for row in read_rows(&lines, &mut i, n) {
                if let Some(face) = parse_face(row) {
                    faces.push(face);
                }
            }
            if let Some(mesh) = model.meshes.last_mut() {
                mesh.faces = faces;
            }
            continue;
        }

        i += 1;
    }

    model
}

/// The integer count at the end of a `KEYWORD <n>` header line.
fn count_after(line: &str) -> usize {
    fields(line).last().and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// `num: material type geo light tex extralight nverts <v,tv>...`
fn parse_face(line: &str) -> Option<Face> {
    let f = fields(line);
    if f.len() < 9 {
        return None;
    }
    let material: i32 = f[1].parse().ok()?;
    let nverts: usize = f[7].parse().ok()?;
    let nums = ints(&f[8..]);
    if nums.len() < nverts * 2 {
        return None;
    }
    let verts = nums[..nverts * 2]
        .chunks_exact(2)
        .map(|p| (p[0].max(0) as u32, p[1]))
        .collect();
    Some(Face { material, verts })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-node puppet whose parent is stored *after* its child, both with pivots —
    /// the case that has to resolve by index and compose the parent's pivot.
    fn puppet() -> Model {
        Model {
            materials: Vec::new(),
            meshes: Vec::new(),
            geosets: 1,
            meshes_per_geoset: 0,
            nodes: vec![
                Node { mesh: -1, parent: 1, pos: [0.0, 0.0, 0.0], rot: [0.0; 3], pivot: [0.0, 0.0, 0.5] },
                Node { mesh: -1, parent: -1, pos: [1.0, 0.0, 0.0], rot: [0.0; 3], pivot: [0.0, 0.0, 2.0] },
            ],
            radius: 0.0,
            insert_offset: [0.0; 3],
        }
    }

    /// `joint[n] = joint[parent] * T(-parent.pivot) * [R|pos] * T(n.pivot)`: a child sits
    /// at its parent's pivot, not at the parent's origin, and the parent may come later
    /// in the node table.
    #[test]
    fn composes_through_a_later_parent_and_its_pivot() {
        let model = puppet();
        let world = pose_nodes(&model, None, 0.0);
        // Root: pos + its own pivot.
        assert_eq!(world[1].pos, [1.0, 0.0, 2.0]);
        // Child: parent's translation, minus the parent's pivot, plus its own pivot.
        assert_eq!(world[0].pos, [1.0, 0.0, 0.5]);
        // A vertex of the child's mesh is that far from the origin.
        let v = world[0].apply([0.0, 0.0, 0.0]);
        assert_eq!(v, [1.0, 0.0, 0.5]);
    }

    /// Rotating the parent swings the child: pitch is about X, and the child swings about
    /// the *parent's* pivot, so a 90 degree pitch maps +Z onto -Y.
    #[test]
    fn parent_rotation_swings_the_child() {
        let mut model = puppet();
        model.nodes[1].rot = [90.0, 0.0, 0.0];
        model.nodes[0].pos = [0.0, 0.0, 0.0];
        model.nodes[1].pivot = [0.0; 3];
        let world = pose_nodes(&model, None, 0.0);
        let v = world[0].apply([0.0, 0.0, 1.0]);
        assert!(v[1] < -0.99, "pitch 90 should send +Z to -Y, got {v:?}");
        assert!(v[2].abs() < 1e-4, "and leave no Z, got {v:?}");
    }
}