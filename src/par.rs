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

#[cfg(test)]
mod tests {
    use super::parse;

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
}
