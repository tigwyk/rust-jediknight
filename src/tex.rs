//! `.mat` material and `.cmp` colormap decoding.
//!
//! Both formats are binary and were recovered from the shipped assets:
//!
//! `.mat` — `MAT ` magic, u32 version (50 in the shipped set), then a 140-byte
//! header carrying image width at +0x74 and height at +0x78, followed by
//! `width * height` bytes of 8-bit palette indices. Verified against the shipped
//! set: `140 + width * height == file length`.
//!
//! `.cmp` — `CMP ` magic, u32 version (30) and a 64-byte header, then the 256
//! RGB triples of the base palette, then 256-byte tables that remap every colour
//! index for a given view distance (see `Palette::parse`).
//!
//! `.bm` — `BM  ` magic. Not needed for world textures (the world's materials
//! embed their own bitmap) so it is left unimplemented.

pub struct Material {
    pub width: u32,
    pub height: u32,
    /// `num_texinfo` at header offset 12: the number of **cels** (animation frames) the bitmap
    /// holds, stacked vertically. A sprite `.mat` is a strip of them — `bryx.mat` carries a spark's
    /// frames — and a world texture is one cel.
    pub cels: u32,
    /// One palette index per pixel, row-major, top row first.
    pub indices: Vec<u8>,
}

const MAT_HEADER: usize = 140;

/// True when the bytes are a MAT container, whether or not it carries a bitmap.
///
/// Some materials are bitmap-less by design — `00RED.mat` is 100 bytes, shorter
/// than the header, and holds only the `MAT ` magic, version 50 and zeroed
/// fields. The level uses it for effect surfaces (force fields), so it is a valid
/// material that simply has no texture, not a broken file.
pub fn is_material(data: &[u8]) -> bool {
    data.len() >= 8 && &data[0..4] == b"MAT "
}

/// The material's `type` word: `rdMaterialHeader.type` at offset 8, stored as `rdMaterial::tex_type`.
///
/// Bit 1 (`type & 2`) is the engine's own test for a surface it must not draw:
/// `sithSurface.c` sets `geometryMode = RD_GEOMETRY_SOLID` only when `(material->tex_type & 2) == 0`, and
/// `sithRender_RenderSectors` skips any surface whose `geometryMode` is unset. Those are the portal
/// materials, so drawing them seals every doorway with a wall the collision system does not have — which
/// reads as a world that is not Euclidean.
pub fn tex_type(data: &[u8]) -> Option<u32> {
    if !is_material(data) || data.len() < 0xc {
        return None;
    }
    Some(u32::from_le_bytes(data[8..0xc].try_into().ok()?))
}

pub fn parse_mat(data: &[u8]) -> Option<Material> {
    if data.len() < MAT_HEADER || &data[0..4] != b"MAT " {
        return None;
    }
    let width = u32::from_le_bytes(data[0x74..0x78].try_into().ok()?);
    let height = u32::from_le_bytes(data[0x78..0x7c].try_into().ok()?);
    // Guard against the small texture-less .mat files, whose header fields are
    // not a bitmap description. ponytail: a length check is enough to reject
    // them; a real header version dispatch would be the upgrade.
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return None;
    }
    let pixels = (width as usize).checked_mul(height as usize)?;
    // Pixel data starts at the fixed 140-byte header and is `width * height`
    // bytes. Many materials carry extra data *after* the bitmap (the length
    // exceeds the header+pixels total by a per-file amount that is not derived
    // from the dimensions), so a trailing tail is tolerated and ignored.
    if MAT_HEADER + pixels > data.len() {
        return None;
    }
    Some(Material {
        width,
        height,
        cels: u32::from_le_bytes(data[0xc..0x10].try_into().ok()?).clamp(1, 64),
        indices: data[MAT_HEADER..MAT_HEADER + pixels].to_vec(),
    })
}

pub struct Palette(pub Vec<[u8; 3]>);

/// Resolution of the light ramp: 32 steps of baked intensity is finer than the levels' own
/// lightmap quantisation.
const LIGHT_STEPS: usize = 32;

/// The installed light ramp; see [`Palette::set_light_ramp`].
static LIGHT_RAMP: std::sync::RwLock<Option<[f32; LIGHT_STEPS + 1]>> = std::sync::RwLock::new(None);

/// The distance fade's colour: the palette seen through the colormap's darkest table, set with the
/// ramp so the level shader walks toward the same value the CPU-side ramp starts from.
static FAR_COLOUR: std::sync::RwLock<Option<[u8; 3]>> = std::sync::RwLock::new(None);


/// `.cmp` layout, verified across all 21 shipped colormaps: every file is
/// `832 + n * 256` bytes.
const CMP_HEADER: usize = 64;
const CMP_PALETTE: usize = 768;

impl Palette {
    /// A neutral palette, used when the level's colormap cannot be read.
    pub fn greyscale() -> Self {
        Palette((0..256).map(|i| [i as u8, i as u8, i as u8]).collect())
    }

    /// Read the 256-colour base palette.
    ///
    /// The palette starts at offset 64, not at the end of the version field: the
    /// 64-byte header is followed by the palette, then by `(len - 832) / 256`
    /// 256-byte tables that remap every colour index for a given view distance
    /// (the engine's distance shading). Reading the palette at offset 30 shifts it
    /// by 34 bytes, which turns a coherent ramp palette into a hue-shifted one:
    /// sky cloud edges came out cyan and whole walls came out magenta and green.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < CMP_HEADER + CMP_PALETTE || &data[0..4] != b"CMP " {
            return None;
        }
        let base = &data[CMP_HEADER..CMP_HEADER + CMP_PALETTE];
        Some(Palette(
            base.chunks_exact(3)
                .map(|c| [c[0], c[1], c[2]])
                .collect(),
        ))
    }

    /// Number of distance-shading tables after the palette.
    pub fn colormap_count(data: &[u8]) -> usize {
        data.len().saturating_sub(CMP_HEADER + CMP_PALETTE) / 256
    }

    /// Install the level's light ramp for [`light_scale`].
    ///
    /// The `.cmp` tables are **brightness ramps**: table 0 is nearly black (mean palette luminance
    /// 14.5 on `01narsh.cmp`) and the last is brighter than the base palette (154.3 against 100.9),
    /// because each table shifts every colour index up or down the palette. The engine picks a
    /// table per surface from its baked light and its distance; this build only had the baked
    /// intensity as a raw multiplier, which ignores the ramp's own shape — a dim surface does not
    /// fade to black linearly, it walks down the palette.
    ///
    /// `ponytail:` one process-global ramp, because this build draws one level at a time. A
    /// second level in the same process would need the ramp passed through the mesh builder.
    pub fn set_light_ramp(data: &[u8]) {
        let Some(palette) = Self::parse(data) else { return };
        let count = Self::colormap_count(data);
        if count < 2 {
            return;
        }
        let lum = |table: &[u8; 256]| -> f32 {
            table
                .iter()
                .map(|i| {
                    let [r, g, b] = palette.0[*i as usize];
                    0.3 * r as f32 + 0.6 * g as f32 + 0.1 * b as f32
                })
                .sum::<f32>()
                / 256.0
        };
        // Unity is the ramp's own midpoint, so a mid-lit surface keeps the brightness it had.
        let Some(mid) = Self::distance_table(data, count / 2) else { return };
        let unity = lum(&mid).max(1.0);
        let mut lut = [1.0f32; LIGHT_STEPS + 1];
        for (k, slot) in lut.iter_mut().enumerate() {
            let level = (k * (count - 1)) / LIGHT_STEPS;
            if let Some(table) = Self::distance_table(data, level) {
                *slot = (lum(&table) / unity).max(0.02);
            }
        }
        if let Ok(mut guard) = LIGHT_RAMP.write() {
            *guard = Some(lut);
        }
    }

    /// Set the distance fade's colour, together with the ramp.
    pub fn set_far_colour(rgb: [u8; 3]) {
        if let Ok(mut guard) = FAR_COLOUR.write() {
            *guard = Some(rgb);
        }
    }

    /// The distance fade's colour, from the installed colormap.
    pub fn installed_far_colour() -> [u8; 3] {
        FAR_COLOUR.read().ok().and_then(|c| *c).unwrap_or([20, 13, 5])
    }

    /// The colour a surface fades toward at maximum distance: the palette as seen through table
    /// 0, which is the ramp's darkest step (mean luminance 14.5 on `01narsh.cmp`).
    pub fn far_colour(data: &[u8]) -> Option<[u8; 3]> {
        let palette = Self::parse(data)?;
        let table = Self::distance_table(data, 0)?;
        let mut sum = [0u32; 3];
        for i in table {
            let [r, g, b] = palette.0[i as usize];
            sum[0] += r as u32;
            sum[1] += g as u32;
            sum[2] += b as u32;
        }
        Some([(sum[0] / 256) as u8, (sum[1] / 256) as u8, (sum[2] / 256) as u8])
    }


    /// The brightness a baked intensity means, from the ramp installed by [`set_light_ramp`].
    /// Falls back to the raw intensity when no ramp was installed.
    pub fn light_scale(intensity: f32) -> f32 {
        let Ok(guard) = LIGHT_RAMP.read() else { return intensity };
        let Some(lut) = guard.as_ref() else { return intensity };
        let at = (intensity.clamp(0.0, 1.0) * LIGHT_STEPS as f32).round() as usize;
        lut[at.min(LIGHT_STEPS)]
    }

    /// One distance table: index → index for a given view distance.
    pub fn distance_table(data: &[u8], step: usize) -> Option<[u8; 256]> {
        let start = CMP_HEADER + CMP_PALETTE + step * 256;
        let bytes = data.get(start..start + 256)?;
        let mut out = [0u8; 256];
        out.copy_from_slice(bytes);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_short_or_foreign_file() {
        assert!(Palette::parse(b"MAT ").is_none());
        assert!(Palette::parse(&[0u8; 831]).is_none());
    }
}
