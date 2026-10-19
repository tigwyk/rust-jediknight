//! `.spr` sprite sheets.
//!
//! Plain ASCII, one row per face, with a `#` header naming the columns:
//!
//! ```text
//! #material    type    width    height    geo    light    tex    extralight    xoff    yoff    zoff
//! bowx.mat    0    0.05    0.05    4    0    0    0.0        0.0    -0.03    0.0
//! ```
//!
//! Every row is one axis-aligned, camera-facing rectangle: `width`/`height` in
//! world units, offset in model space. The engine draws these as billboards, which
//! is why a sprite is a list of faces rather than a grid of image cells — the
//! frame to draw is chosen by the COG that spawns it, not by the file.
//!
//! All 20 shipped sprites live in `misc\spr\` and are used by effect templates
//! (`sprite=bryx.spr` on `+laserhit` and friends), never by placed objects.

use crate::rows::fields;

pub struct Face {
    pub material: String,
    /// Rectangle size in world units.
    pub width: f32,
    pub height: f32,
    /// Model-space offset of the rectangle's centre.
    pub offset: [f32; 3],
}

pub fn parse(text: &str) -> Vec<Face> {
    let mut faces = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.eq_ignore_ascii_case("end") {
            continue;
        }
        let f = fields(line);
        // Wide enough for material + the numeric columns; the trailing offset
        // columns are the only ones a renderer needs.
        if f.len() < 4 {
            continue;
        }
        let num = |i: usize| f.get(i).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        faces.push(Face {
            material: f[0].to_string(),
            width: num(2),
            height: num(3),
            offset: [num(8), num(9), num(10)],
        });
    }
    faces
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn reads_a_shipped_sprite() {
        let faces = parse(
            "#material\ttype\twidth\theight\tgeo\tlight\ttex\textralight\txoff\tyoff\tzoff\n\
             bowx.mat\t0\t0.05    0.05\t4\t0\t0\t0.0\t\t0.0\t-0.03\t0.0\n",
        );
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].material, "bowx.mat");
        assert_eq!((faces[0].width, faces[0].height), (0.05, 0.05));
        assert_eq!(faces[0].offset[1], -0.03);
    }
}