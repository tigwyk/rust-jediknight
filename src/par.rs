//! `PAR` particle clouds (`misc\par\*.par`, header `PAR 1.0`).
//!
//! The engine's loader is `rdParticle_LoadEntry` (`0x46c230`): it reads, in order, `PAR <version>`,
//! `SIZE <f>`, `MATERIAL <file>`, `LIGHTINGMODE <d>`, `RADIUS <f>`, `INSERT OFFSET <x> <y> <z>`,
//! `VERTICES <n>` and then `n` lines of `<i>: <x> <y> <z> <cel>`. Each vertex carries a **material
//! cel** index which the loader bounds-checks against the material's own cel count
//! (`if (cels <= cel) fail`), and `sithParticle_Update` (`0x4f1c30`) walks those cels as the cloud
//! ages (`thingFlags & 0x20`), while `thingFlags & 0x1` grows the cloud by scaling its matrix.
//!
//! The shipped install has three files: `sphere.par` and `sphere2.par` (the Force shield's clouds,
//! `MATERIAL 00teleport.mat`) and `test.par` (the editor's sample). The port has no particle renderer
//! yet — this module is the data half, checked by `--audit` against those three files.

/// One `PAR 1.0` cloud.
#[derive(Clone, Debug, Default)]
pub struct Par {
    /// `SIZE`: the world size of one element (the drawn quad's edge).
    pub size: f32,
    /// `MATERIAL`: the `.mat` each element draws (lowercase, as the archive names it).
    pub material: String,
    /// `LIGHTINGMODE`: the engine keeps it at `particle + 0x20`; shipped files all ship 0.
    pub lighting: u32,
    /// `RADIUS`: the cloud's authored radius, used by `sithParticle_Update`'s grow arm.
    pub radius: f32,
    /// `INSERT OFFSET`: where the cloud sits relative to the thing that owns it.
    pub insert: [f32; 3],
    pub vertices: Vec<Vertex>,
}

/// One element of a cloud: its position in the cloud's own frame and the cel it shows.
#[derive(Clone, Copy, Debug, Default)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub cel: i32,
}

impl Par {
    /// What the loader's own checks amount to: a material, a size, a radius and at least one element
    /// whose cel is a valid index. `rdParticle_LoadEntry` fails the whole file when any of these is
    /// missing, so a shipped `.par` that fails this is a parse bug, not a data quirk.
    pub fn is_complete(&self) -> bool {
        !self.material.is_empty()
            && self.size > 0.0
            && self.radius > 0.0
            && !self.vertices.is_empty()
            && self.vertices.iter().all(|v| v.cel >= 0)
    }

    /// The cloud's extent over its own vertices: what a renderer frames and what the `RADIUS` above
    /// should roughly agree with.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for v in &self.vertices {
            for k in 0..3 {
                min[k] = min[k].min(v.pos[k]);
                max[k] = max[k].max(v.pos[k]);
            }
        }
        if self.vertices.is_empty() { ([0.0; 3], [0.0; 3]) } else { (min, max) }
    }
}

/// Parse one `PAR 1.0` cloud. Unknown lines (section banners, comments, `PAR`) are skipped, and a
/// malformed vertex is dropped rather than failing the file — `is_complete` is the gate.
pub fn parse(text: &str) -> Par {
    let mut par = Par::default();
    for line in text.lines() {
        // `#` starts a comment in the shipped files (`# Test particle cloud`, the vertex header).
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("par ") || lower.starts_with("section") {
            continue;
        }
        if let Some(rest) = lower.strip_prefix("size") {
            par.size = rest.trim().parse().unwrap_or(0.0);
        } else if let Some(rest) = lower.strip_prefix("material") {
            par.material = rest.trim().to_ascii_lowercase();
        } else if let Some(rest) = lower.strip_prefix("lightingmode") {
            par.lighting = rest.trim().parse().unwrap_or(0);
        } else if let Some(rest) = lower.strip_prefix("radius") {
            par.radius = rest.trim().parse().unwrap_or(0.0);
        } else if let Some(rest) = lower.strip_prefix("insert offset") {
            let mut it = rest.split_whitespace().filter_map(|t| t.parse().ok());
            if let (Some(x), Some(y), Some(z)) = (it.next(), it.next(), it.next()) {
                par.insert = [x, y, z];
            }
        } else if let Some(rest) = lower.strip_prefix("vertices") {
            // The count is only a hint: the vertex lines that follow are the data, exactly as the
            // loader reads a line at a time and gives up on a short file.
            let _ = rest.trim().parse::<usize>();
        } else if let Some((_, rest)) = line.split_once(':') {
            // `<i>: <x> <y> <z> <cel>`. The engine's `%d` on the last column reads the integer
            // *prefix*, which is how `0.000000` (sphere.par) and `0` (test.par) are both valid.
            let mut it = rest.split_whitespace();
            let (Some(x), Some(y), Some(z), Some(cel)) = (it.next(), it.next(), it.next(), it.next())
            else {
                continue;
            };
            let (Ok(x), Ok(y), Ok(z)) = (x.parse(), y.parse(), z.parse()) else {
                continue;
            };
            let cel = cel
                .parse::<f32>()
                .map(|f| f as i32)
                .or_else(|_| cel.parse::<i32>())
                .unwrap_or(-1);
            par.vertices.push(Vertex { pos: [x, y, z], cel });
        }
    }
    par
}

/// Build the cloud a `type=particle` template's own arguments describe.
///
/// `sithParticle_Initalize` (`0x4f1da0`) never reads a `.par` for these: it calls `rdParticle_New`
/// with the template's `count`, `elementsize` and `material`, then fills `count` elements. Each gets
/// three uniform numbers: two **Euler angles** (degrees) and a radius. The angle spans default to
/// 720° when the template omits `pitchrange=`/`yawrange=` (the shipped templates omit both, so their
/// clouds are spherical), and the radius is `minsize + rand * (range - minsize)`. The engine rotates
/// the unit vector `(0,1,0)` by `(pitch, yaw, 0)` and scales it by that radius; with that input the
/// rotation's second column is exactly `(-sin(yaw)cos(pitch), cos(yaw)cos(pitch), sin(pitch))`.
///
/// `typeflags & 4` gives every element a random material cel instead of `-1` (the engine's "use the
/// material's own current cel"). The port's `-1` is clamped to cel 0 at draw, which is the same
/// visible result for the single-cel effect materials shipped here.
///
/// The per-cloud RNG is local and seeded from the element count, not the shared `cog::pseudo_random`:
/// loading a level must not consume the AI's random stream.
#[allow(clippy::too_many_arguments)]
pub fn generate(
    material: &str,
    count: u32,
    elementsize: f32,
    range: f32,
    minsize: f32,
    pitchrange: f32,
    yawrange: f32,
    typeflags: u32,
    cels: u32,
) -> Par {
    let span = |value: f32| if value == 0.0 { 720.0 } else { value * 2.0 };
    let pitch_span = span(pitchrange);
    let yaw_span = span(yawrange);
    let radial_span = (range - minsize).max(0.0);
    let mut seed = 0x1234_5678u32 ^ count;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / 16_777_216.0
    };
    let mut vertices = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let pitch = (next() - 0.5) * pitch_span;
        let yaw = (next() - 0.5) * yaw_span;
        let radius = minsize + next() * radial_span;
        let (sp, cp) = pitch.to_radians().sin_cos();
        let (sy, cy) = yaw.to_radians().sin_cos();
        let cel = if typeflags & 4 != 0 && cels > 1 {
            ((next() * cels as f32) as u32).min(cels - 1) as i32
        } else {
            -1
        };
        vertices.push(Vertex {
            pos: [
                -sy * cp * radius,
                cy * cp * radius,
                sp * radius,
            ],
            cel,
        });
    }
    Par {
        size: elementsize,
        material: material.to_ascii_lowercase(),
        lighting: 0,
        radius: range,
        insert: [0.0; 3],
        vertices,
    }
}

#[cfg(test)]
mod tests {
    use super::{generate, parse};

    /// The shipped `test.par` shape (`--asset test.par`): the header fields, comments, both vertex
    /// integer styles (`0` here, `0.000000` in `sphere.par`), and the tab-separated last column.
    #[test]
    fn a_par_cloud_parses_its_header_and_elements() {
        let text = "# Test particle cloud\n\
             ###############\n\
             SECTION: HEADER\n\n\
             PAR 1.0\n\n\
             SIZE .002\n\n\
             MATERIAL 00Teleport.mat\n\n\
             LIGHTINGMODE 0\n\n\
             ###############\n\
             SECTION: GEOMETRYDEF\n\n\
             # Object radius\n\
             RADIUS   0.109546\n\n\
             # Insertion offset\n\
             INSERT OFFSET   0.000000   0.000000   0.108216\n\n\
             VERTICES 2\n\n\
             # num:     x:         y:         z:         i:\n\
                 0:  -0.000159  -0.109326  -0.000158   0.000000\n\
                 1:   0.693446  -0.534410  -0.959644\t\t1\n";
        let par = parse(text);
        assert!(par.is_complete());
        assert!((par.size - 0.002).abs() < 1e-6, "size {}", par.size);
        assert_eq!(par.material, "00teleport.mat", "the material name is lowercased");
        assert_eq!(par.lighting, 0);
        assert!((par.radius - 0.109546).abs() < 1e-5, "radius {}", par.radius);
        assert_eq!(par.insert, [0.0, 0.0, 0.108216]);
        assert_eq!(par.vertices.len(), 2);
        assert_eq!(par.vertices[0].pos, [-0.000159, -0.109326, -0.000158]);
        assert_eq!(par.vertices[0].cel, 0, "a float-formatted cel reads as its integer prefix");
        assert_eq!(par.vertices[1].cel, 1, "an integer cel reads as itself");
        let (lo, hi) = par.bounds();
        assert_eq!(lo, [-0.000159, -0.534410, -0.959644]);
        assert_eq!(hi, [0.693446, -0.109326, -0.000158]);
    }

    /// A cloud with no elements is what a bad reader produces, and the engine's loader rejects it.
    #[test]
    fn a_cloud_with_no_elements_is_incomplete() {
        let par = parse("PAR 1.0\nSIZE .01\nMATERIAL test.mat\nRADIUS 1.0\nVERTICES 0\n");
        assert!(!par.is_complete());
    }

    /// `sithParticle_Initalize`'s generation: every element lies inside the authored `range` and
    /// outside `minsize`, the default 720° angle spans reach both hemispheres, and `typeflags & 4`
    /// picks a material cel (the shipped `+rpt_sparks` line: `range=0.04`, `elementsize=0.005`,
    /// `count=24`, `typeflags=0x27`).
    #[test]
    fn a_generated_cloud_fills_its_authored_radius() {
        let par = generate("Sparky.mat", 64, 0.005, 0.04, 0.0, 0.0, 0.0, 0x27, 1);
        assert_eq!(par.vertices.len(), 64);
        assert_eq!(par.material, "sparky.mat");
        assert!((par.size - 0.005).abs() < 1e-6);
        assert!((par.radius - 0.04).abs() < 1e-6);
        let mut high = false;
        let mut low = false;
        for v in &par.vertices {
            let r = (v.pos[0].powi(2) + v.pos[1].powi(2) + v.pos[2].powi(2)).sqrt();
            assert!(r <= 0.04 + 1e-4, "element outside range: {r}");
            high |= v.pos[2] > 0.01;
            low |= v.pos[2] < -0.01;
        }
        assert!(high && low, "the default angle span did not reach both hemispheres");

        let inner = generate("sparky.mat", 32, 0.005, 0.04, 0.02, 0.0, 0.0, 0x27, 1);
        for v in &inner.vertices {
            let r = (v.pos[0].powi(2) + v.pos[1].powi(2) + v.pos[2].powi(2)).sqrt();
            assert!(r >= 0.02 - 1e-4, "element inside minsize: {r}");
        }

        let cels = generate("sparky.mat", 32, 0.005, 0.04, 0.0, 0.0, 0.0, 0x27, 4);
        assert!(cels.vertices.iter().all(|v| (0..4).contains(&v.cel)));
        let single = generate("sparky.mat", 8, 0.005, 0.04, 0.0, 0.0, 0.0, 0x20, 4);
        assert!(single.vertices.iter().all(|v| v.cel == -1), "a cloud without bit 4 picked a cel");
    }
}
