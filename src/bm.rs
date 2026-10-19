//! `.bm` bitmaps — the UI's indexed images (menus, HUD, dialog frames).
//!
//! Binary. A 128-byte header (`BM  ` magic, `u32` version, then a table of sizes) is
//! followed by one or more *cells*, each of which is
//!
//! ```text
//! u32 width
//! u32 height
//! u8  pixels[width * height * (bits / 8)]
//! ```
//!
//! The header's `u32` at offset 36 is the colour depth: 8 for the indexed cells,
//! 16 for the `…16.bm` variants that store one `u16` per pixel. Whether a file ends
//! with its own 768-byte palette is read from the length, not a flag: the 640x480
//! dialog backgrounds carry one, the small cursor cells do not and use the level's
//! colormap instead.
//!
//! `arrowleft.bm` is three 24x25 8-bit cells after the header (128 + 3 * 608 = 1952,
//! the file's exact length); `bkdialog.bm` is one 414x206 cell; `forcebrack16.bm` is
//! four 30x30 16-bit cells (128 + 4 * 1808 = 7360).
//!
//! The parse verifies itself: a decode is only called complete when the cells consume
//! the file exactly, so a misread header shows up as a size mismatch rather than as
//! silently shifted pixels.

pub const HEADER: usize = 128;

/// Size of the trailing 256-colour palette, in bytes.
pub const PALETTE: usize = 768;

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub width: u32,
    pub height: u32,
    /// `width * height * (bits / 8)` palette indices (one byte) or `u16` values.
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Bm {
    pub version: u32,
    /// Bits per pixel from the header: 8 for indexed cells, 16 for the `…16` variants.
    pub bits: u32,
    pub cells: Vec<Cell>,
    /// `numMips` from the header. The HUD meters use the mip level as an animation frame
    /// (`mip = (numMips-1) * (max-current)/max`), so this is the frame count.
    pub num_mips: u32,
    /// `xPos` / `yPos` from the header: where the HUD draws the bitmap inside its panel.
    pub x_pos: i32,
    pub y_pos: i32,
    /// Whether the file ends with its own 256-colour palette (768 bytes). Most do; the
    /// small cursor/HUD cells (`arrowleft.bm`) do not and use the level's colormap.
    pub palette: bool,
    /// That palette, as RGB triples, when the file carries one.
    pub palette_rgb: Option<[[u8; 3]; 256]>,
    /// Whether the cells consumed the file exactly.
    pub exact: bool,
}

impl Bm {
    pub fn is_bitmap(&self) -> bool {
        !self.cells.is_empty() && self.cells.iter().all(|c| c.width > 0 && c.height > 0)
    }

    pub fn pixels(&self) -> usize {
        self.cells.iter().map(|c| c.pixels.len()).sum()
    }
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Parse a `.bm`. `None` means "not a bitmap at all" (wrong magic, too short).
pub fn parse(bytes: &[u8]) -> Option<Bm> {
    let magic = bytes.get(0..4)?;
    // The magic is `BM` plus two spaces; `.mat` uses the same four-byte convention.
    if &magic[0..2] != b"BM" {
        return None;
    }
    let version = u32_at(bytes, 4)?;
    let bits = u32_at(bytes, 36).filter(|d| *d == 8 || *d == 16).unwrap_or(8);
    let stride = (bits / 8) as usize;
    let mut bm = Bm {
        version,
        bits,
        cells: Vec::new(),
        num_mips: u32_at(bytes, 16).unwrap_or(0),
        x_pos: u32_at(bytes, 20).unwrap_or(0) as i32,
        y_pos: u32_at(bytes, 24).unwrap_or(0) as i32,
        palette: false,
        palette_rgb: None,
        exact: false,
    };
    let mut at = HEADER;
    loop {
        // A 768-byte tail is the file's own palette, not another cell: the shipped
        // 640x480 dialog backgrounds end that way.
        if bytes.len().saturating_sub(at) == PALETTE {
            bm.palette = true;
            let tail = &bytes[at..];
            let mut rgb = [[0u8; 3]; 256];
            for (i, slot) in rgb.iter_mut().enumerate() {
                *slot = [tail[i * 3], tail[i * 3 + 1], tail[i * 3 + 2]];
            }
            bm.palette_rgb = Some(rgb);
            at = bytes.len();
            break;
        }
        if at + 8 > bytes.len() {
            break;
        }
        let (Some(width), Some(height)) = (u32_at(bytes, at), u32_at(bytes, at + 4)) else {
            break;
        };
        let count = width as usize * height as usize * stride;
        let start = at + 8;
        let Some(pixels) = bytes.get(start..start + count) else { break };
        bm.cells.push(Cell { width, height, pixels: pixels.to_vec() });
        at = start + count;
    }
    bm.exact = at == bytes.len();
    Some(bm)
}

#[cfg(test)]
mod tests {
    use super::{parse, HEADER};

    /// A header the shape the shipped files use, then one 2x2 cell and one 1x3 cell.
    fn build() -> Vec<u8> {
        let mut bytes = vec![0u8; HEADER];
        bytes[0..4].copy_from_slice(b"BM  ");
        bytes[4..8].copy_from_slice(&70u32.to_le_bytes());
        bytes[36..40].copy_from_slice(&8u32.to_le_bytes());
        for (w, h, fill) in [(2u32, 2u32, 7u8), (1, 3, 9)] {
            bytes.extend_from_slice(&w.to_le_bytes());
            bytes.extend_from_slice(&h.to_le_bytes());
            bytes.extend(std::iter::repeat_n(fill, (w * h) as usize));
        }
        bytes
    }

    #[test]
    fn reads_cells_and_checks_the_length() {
        let bm = parse(&build()).expect("magic");
        assert_eq!(bm.version, 70);
        assert!(bm.exact);
        assert!(bm.is_bitmap());
        assert_eq!(bm.cells.len(), 2);
        assert_eq!((bm.cells[0].width, bm.cells[0].height), (2, 2));
        assert_eq!(bm.cells[0].pixels, vec![7; 4]);
        assert_eq!(bm.cells[1].pixels, vec![9; 3]);
        assert_eq!(bm.pixels(), 7);
        assert!(!bm.palette);

        // The shipped dialog backgrounds end with their own 768-byte palette, which
        // must not be mistaken for another cell.
        let mut with_palette = build();
        with_palette.extend(std::iter::repeat_n(0x40u8, super::PALETTE));
        let bm = parse(&with_palette).expect("magic");
        assert!(bm.exact);
        assert!(bm.palette);
        assert_eq!(bm.cells.len(), 2);

        // One byte short: still parses, but no longer claims to be complete.
        let mut short = build();
        short.pop();
        let bm = parse(&short).expect("magic");
        assert!(!bm.exact);
        assert_eq!(bm.cells.len(), 1);
    }

    #[test]
    fn reads_the_16_bit_cells() {
        // `forcebrack16.bm` is four 30x30 cells at two bytes per pixel.
        let mut bytes = vec![0u8; HEADER];
        bytes[0..4].copy_from_slice(b"BM  ");
        bytes[4..8].copy_from_slice(&70u32.to_le_bytes());
        bytes[36..40].copy_from_slice(&16u32.to_le_bytes());
        for _ in 0..4 {
            bytes.extend_from_slice(&30u32.to_le_bytes());
            bytes.extend_from_slice(&30u32.to_le_bytes());
            bytes.extend(std::iter::repeat_n(0u8, 30 * 30 * 2));
        }
        let bm = parse(&bytes).expect("magic");
        assert_eq!(bm.bits, 16);
        assert_eq!(bm.cells.len(), 4);
        assert_eq!(bm.cells[0].pixels.len(), 1800);
        assert!(bm.exact);
    }

    #[test]
    fn rejects_a_foreign_file() {
        assert!(parse(b"MAT \x32\x00\x00\x00").is_none());
        assert!(parse(b"BM").is_none());
    }
}
