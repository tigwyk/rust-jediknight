//! `.mat` material and `.cmp` colormap decoding.
//!
//! Both formats are binary and were recovered from the shipped assets:
//!
//! `.mat` — `rdMaterial_LoadEntry` (`0x44a260`) is the layout authority: a 76-byte
//! header (`MAT `, version 50, `type`, `num_texinfo`, `num_textures`, 56-byte
//! `rdTexFormat`), a table of 24-byte `rdTexinfo` cels (plus a 16-byte extension each
//! when the cel's `texture_type` has bit 3), then one 24-byte `rdTexture` record per
//! bitmap **immediately followed by that bitmap's mip chain** — the records are
//! interleaved with pixel data, not gathered in a second table. Every shipped `.mat`
//! walks to exactly its own length, less the 768-byte palette when `type & 1`.
//!
//! A sprite `.mat` is therefore a *cel table*, not a strip: `bryx.mat` holds eight
//! separate 32x32 bitmaps and nobody is 8x78 wide.
//!
//! `.cmp` — `CMP ` magic, u32 version (30) and a 64-byte header, then the 256
//! RGB triples of the base palette, then 256-byte tables that remap every colour
//! index for a given view distance (see `Palette::parse`).
//!
//! `.bm` — `BM  ` magic. Not needed for world textures (the world's materials
//! embed their own bitmap) so it is left unimplemented.

pub struct Material {
    /// One cel's width in texels.
    pub width: u32,
    /// The **strip** height: `cels * cel height`. The file's cels are separate
    /// bitmaps; they are packed here into one column so a quad can address cel `k`
    /// with `v = k / cels`, which is what [`crate::sprite_quad`] does.
    pub height: u32,
    /// `num_texinfo` at header offset 12: the number of **cels** (animation frames).
    pub cels: u32,
    /// One palette index per pixel, row-major, top row first, cels stacked.
    pub indices: Vec<u8>,
}

/// The `.mat` header: `MAT `, version, `type`, `num_texinfo`, `num_textures` and the
/// 56-byte `rdTexFormat` — one `fileRead(_, buf, 0x4c)` in `rdMaterial_LoadEntry`.
const MAT_HEADER: usize = 76;

/// One `rdTexinfo` cel, and one `rdTexture` record.
const TEXINFO: usize = 24;
const TEXTURE: usize = 24;
/// The extra bytes a cel carries when `texture_type & 8` is set.
const TEXINFO_EXT: usize = 16;

/// True when the bytes are a MAT container, whether or not it carries a bitmap.
///
/// Some materials are bitmap-less by design: `00red.mat` is 100 bytes — the 76-byte header and one
/// texinfo cel, `num_textures == 0` — and holds no image. The level uses it for effect surfaces
/// (force fields), so it is a valid material that simply has no texture, not a broken file.
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
    let word = |at: usize| -> Option<u32> {
        let b = data.get(at..at + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    // ponytail: 64 cels and 64 bitmaps is far past the shipped maximum (13 cels) and
    // only exists to bound the allocation a corrupt header could ask for.
    let n_texinfo = (word(12)? as usize).min(64);
    let n_tex = (word(16)? as usize).min(64);
    if n_texinfo == 0 || n_tex == 0 {
        // Texture-less by design: `00explosion.mat` is six cels and no bitmap.
        return None;
    }
    // Cel table. A cel whose `texture_type` has bit 3 carries a 16-byte extension whose
    // last word names its bitmap; without it the cel uses the bitmap of its own index.
    let mut cel_tex = Vec::with_capacity(n_texinfo);
    let mut at = MAT_HEADER;
    for i in 0..n_texinfo {
        if word(at)? & 8 != 0 {
            cel_tex.push(word(at + TEXINFO + TEXINFO_EXT - 4)? as usize);
            at += TEXINFO + TEXINFO_EXT;
        } else {
            cel_tex.push(i);
            at += TEXINFO;
        }
    }
    // Bitmap records, each followed by its own mip chain. Every shipped material is
    // 8-bit indexed, so a mip level is `(width >> level) * (height >> level)` bytes.
    let mut bitmaps: Vec<(u32, u32, &[u8])> = Vec::with_capacity(n_tex);
    for _ in 0..n_tex {
        let (w, h, mips) = (word(at)?, word(at + 4)?, word(at + 20)?);
        let start = at + TEXTURE;
        let len = w.checked_mul(h)? as usize;
        if w == 0 || h == 0 || w > 4096 || h > 4096 || start + len > data.len() {
            return None;
        }
        bitmaps.push((w, h, &data[start..start + len]));
        at = start;
        for level in 0..mips {
            at += ((w >> level) as usize) * ((h >> level) as usize);
        }
    }
    // Pack the cels into one column. Shipped multi-cel bitmaps are all the same size,
    // so this is a straight copy; a smaller cel is padded with palette entry 0, which
    // the sprite path keys out.
    let pick = |cel: usize| bitmaps[cel_tex[cel].min(bitmaps.len() - 1)];
    let width = (0..n_texinfo).map(|c| pick(c).0).max()?;
    let cel_h = (0..n_texinfo).map(|c| pick(c).1).max()?;
    // The largest shipped material is 256x256, so a header asking for more than 4 Mi pixels
    // is corrupt: it would allocate a 64 MiB strip if every field were 4096.
    let total = (width as usize)
        .checked_mul(cel_h as usize)?
        .checked_mul(n_texinfo)?;
    if total > 1 << 22 {
        return None;
    }
    let mut indices = vec![0u8; total];
    for cel in 0..n_texinfo {
        let (w, h, pixels) = pick(cel);
        let (w, h) = (w.min(width) as usize, h.min(cel_h) as usize);
        let row = width as usize;
        for y in 0..h {
            let dst = (cel * cel_h as usize + y) * row;
            indices[dst..dst + w].copy_from_slice(&pixels[y * w..y * w + w]);
        }
    }
    Some(Material {
        width,
        height: cel_h * n_texinfo as u32,
        cels: n_texinfo as u32,
        indices,
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

    /// The cel table as `bryx.mat` lays it out: two bit-8 cels whose extension swaps
    /// which bitmap they use, and a first bitmap carrying an extra mip level.
    #[test]
    fn parse_mat_walks_the_cel_table() {
        let mut f = Vec::new();
        f.extend_from_slice(b"MAT ");
        f.extend_from_slice(&50u32.to_le_bytes()); // revision
        f.extend_from_slice(&2u32.to_le_bytes()); // type
        f.extend_from_slice(&2u32.to_le_bytes()); // num_texinfo: cels
        f.extend_from_slice(&2u32.to_le_bytes()); // num_textures: bitmaps
        f.resize(MAT_HEADER, 0); // rdTexFormat, unread
        for index in [1u32, 0] {
            f.extend_from_slice(&8u32.to_le_bytes()); // texture_type: has an extension
            f.resize(f.len() + TEXINFO - 4 + TEXINFO_EXT - 4, 0);
            f.extend_from_slice(&index.to_le_bytes());
        }
        let bitmap = |f: &mut Vec<u8>, value: u8, mips: u32| {
            f.extend_from_slice(&2u32.to_le_bytes()); // width
            f.extend_from_slice(&2u32.to_le_bytes()); // height
            f.resize(f.len() + 20 - 8, 0); // colour key and friends
            f.extend_from_slice(&mips.to_le_bytes()); // mip count, at +20
            f.extend_from_slice(&[value; 4]); // mip 0: 2x2
            for _ in 1..mips {
                f.push(9); // mip 1: 1x1
            }
        };
        bitmap(&mut f, 1, 2);
        bitmap(&mut f, 2, 1);
        let mat = parse_mat(&f).expect("a two-cel material");
        assert_eq!((mat.width, mat.height, mat.cels), (2, 4, 2));
        // Cel 0 is bitmap 1 and cel 1 is bitmap 0: the extension's index wins.
        assert_eq!(mat.indices, [2, 2, 2, 2, 1, 1, 1, 1]);
    }
}
