//! Smacker file structure: the layout of the header and the palette records.
//!
//! `SMACKW32.DLL` decodes pixels but will not hand over the palette: `SmackBufferOpen` refuses a
//! full-size frame (it caps width at 320 and height at 240), and the buffer it does create stays
//! all zero because the DLL only fills it for frames it renders through that buffer. So the
//! palette is read from the file.
//!
//! The header layout is the SDK's, and it is *checked* rather than assumed: with it, the chunk
//! sizes plus the tables and the huffman block add up to the file length exactly
//! (`01-02A.SMK`: 51,556,616 bytes, delta 0).
//!
//! ```text
//!   0  "SMK2"/"SMK4"
//!   4  u32 width, height, frames, frame time
//!  20  u32 video flags        (bit 0 = a ring frame exists, so count = frames + 1)
//!  24  u32 × 7 audio max buffer sizes
//!  52  u32 huffman block size
//!  56  u32 × 4 unpacked huffman tree sizes
//!  72  u32 × 7 audio rate words
//! 100  u32 dummy
//! 104  u32 × count chunk sizes  (bit 0 = keyframe, mask with & !3 for the length)
//!      u8  × count frame types   (bit 0 = the chunk starts with a palette record)
//!      huffman block (`tree_size` from offset 52)
//!      frame chunks, in order
//! ```
//!
//! A palette record is one byte `N` followed by `(N + 1) * 4 - 1` bytes of chunks:
//!
//! ```text
//!   byte & 0x80 -> skip (byte & 0x7F) + 1 entries, keeping the old colours
//!   byte & 0x40 -> colour shift: copy (byte & 0x3F) + 1 entries from a source index
//!   otherwise   -> literal: (byte & 0x3F) + 1 triples of 6-bit RGB
//! ```

/// A 256-entry RGB palette.
pub type Palette = [[u8; 3]; 256];

/// Smacker stores 6-bit colour; this is the SDK's own expansion table.
const PALMAP: [u8; 64] = [
    0x00, 0x04, 0x08, 0x0C, 0x10, 0x14, 0x18, 0x1C, 0x20, 0x24, 0x28, 0x2C, 0x30, 0x34, 0x38, 0x3C,
    0x41, 0x45, 0x49, 0x4D, 0x51, 0x55, 0x59, 0x5D, 0x61, 0x65, 0x69, 0x6D, 0x71, 0x75, 0x79, 0x7D,
    0x82, 0x86, 0x8A, 0x8E, 0x92, 0x96, 0x9A, 0x9E, 0xA2, 0xA6, 0xAA, 0xAE, 0xB2, 0xB6, 0xBA, 0xBE,
    0xC3, 0xC7, 0xCB, 0xCF, 0xD3, 0xD7, 0xDB, 0xDF, 0xE3, 0xE7, 0xEB, 0xEF, 0xF3, 0xF7, 0xFB, 0xFF,
];

/// Where each table starts, derived from the header alone.
pub struct Layout {
    /// The number of chunks: `frames`, plus one when the file has a ring frame.
    pub count: usize,
    sizes_at: usize,
    types_at: usize,
    tree_size: usize,
    data_at: usize,
}

impl Layout {
    /// Parse the header. `None` when the bytes are not a Smacker file.
    pub fn parse(bytes: &[u8]) -> Option<Layout> {
        if bytes.len() < 104 || &bytes[0..3] != b"SMK" {
            return None;
        }
        let word = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
        };
        let frames = word(12)? as usize;
        let ring = word(20)? & 0x01 != 0;
        let tree_size = word(52)? as usize;
        let count = frames + usize::from(ring);
        let sizes_at = 104;
        let types_at = sizes_at + count * 4;
        let tree_at = types_at + count;
        let data_at = tree_at + tree_size;
        // A layout that claims more than the file holds is a misread, not a short file.
        if data_at > bytes.len() {
            return None;
        }
        Some(Layout { count, sizes_at, types_at, tree_size, data_at })
    }

    /// The chunk length and the frame-type byte for one frame.
    fn chunk(&self, bytes: &[u8], index: usize) -> Option<(usize, u8)> {
        if index >= self.count {
            return None;
        }
        let at = self.sizes_at + index * 4;
        let raw = u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?);
        let kind = *bytes.get(self.types_at + index)?;
        Some(((raw & 0xFFFF_FFFC) as usize, kind))
    }

    /// Whether the chunk sizes, the tables and the huffman block fill the file exactly. This is
    /// what makes the layout checkable instead of guessed: it passes on the shipped films.
    pub fn covers(&self, bytes: &[u8]) -> bool {
        let mut at = self.data_at;
        for index in 0..self.count {
            let Some((size, _)) = self.chunk(bytes, index) else { return false };
            at += size;
        }
        at == bytes.len() && self.tree_size > 0
    }
}

/// The palette in force at `frame`: `None` when no palette record has been seen by then.
pub fn palette_at(bytes: &[u8], frame: usize) -> Option<Palette> {
    let layout = Layout::parse(bytes)?;
    let mut at = layout.data_at;
    let mut palette: Palette = [[0u8; 3]; 256];
    let mut seen = false;
    for index in 0..=frame.min(layout.count.saturating_sub(1)) {
        let (size, kind) = layout.chunk(bytes, index)?;
        if kind & 1 != 0
            && let Some(record) = bytes.get(at..at + size)
            && apply_palette(&mut palette, record)
        {
            seen = true;
        }
        at += size;
    }
    seen.then_some(palette)
}

/// Apply one palette record to `palette`. Returns whether anything was written.
///
/// The record's first byte is `N`, and the block covers `4 * N` bytes, so the body is
/// `4 * N - 1` of them (libsmacker's `smk_render`: `size = 4 * (*p)`).
fn apply_palette(palette: &mut Palette, record: &[u8]) -> bool {
    let Some(&first) = record.first() else { return false };
    let size = (first as usize) * 4;
    let Some(body) = record.get(1..size.min(record.len())) else { return false };
    let old = *palette;
    let mut at = 0usize;
    let mut index = 0usize;
    while index < 256 && at < body.len() {
        let chunk = body[at];
        if chunk & 0x80 != 0 {
            // Skip: keep the previous colours for this run.
            index += (chunk as usize & 0x7F) + 1;
            at += 1;
        } else if chunk & 0x40 != 0 {
            // Colour shift: copy entries from a source index in the previous palette.
            let count = (chunk as usize & 0x3F) + 1;
            let Some(&source) = body.get(at + 1) else { return index > 0 };
            for k in 0..count {
                if index + k < 256 && source as usize + k < 256 {
                    palette[index + k] = old[source as usize + k];
                }
            }
            index += count;
            at += 2;
        } else {
            // Set one colour. The chunk byte *is* the red value: there is no selector byte, so
            // the three 6-bit values start here (`smk_render_palette` reads `*p` first).
            let (Some(r), Some(g), Some(b)) = (body.get(at), body.get(at + 1), body.get(at + 2))
            else {
                return index > 0;
            };
            if index < 256 {
                palette[index] = [
                    PALMAP[(r & 0x3F) as usize],
                    PALMAP[(g & 0x3F) as usize],
                    PALMAP[(b & 0x3F) as usize],
                ];
            }
            index += 1;
            at += 3;
        }
    }
    index > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal film: one palette-carrying frame whose record sets two colours.
    fn tiny_smk() -> Vec<u8> {
        let mut bytes = vec![0u8; 104];
        bytes[0..4].copy_from_slice(b"SMK2");
        bytes[4..8].copy_from_slice(&640u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&300u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&1u32.to_le_bytes()); // one frame
        bytes[52..56].copy_from_slice(&4u32.to_le_bytes()); // huffman block
        // An 8-byte record: size byte 2 means the block is 4 * 2 = 8 bytes, so the body is 7 of
        // them — two literal colours (red, green) and one spare byte.
        let mut palette = vec![0u8; 8];
        palette[0] = 2;
        palette[1..4].copy_from_slice(&[63, 0, 0]);
        palette[4..7].copy_from_slice(&[0, 63, 0]);
        bytes.extend_from_slice(&((palette.len() as u32) | 1).to_le_bytes()); // size, keyframe
        bytes.push(1); // frame type: a palette record leads the chunk
        bytes.extend_from_slice(&[0u8; 4]); // huffman block
        bytes.extend_from_slice(&palette);
        bytes
    }

    #[test]
    fn the_layout_covers_the_file_exactly() {
        let bytes = tiny_smk();
        let layout = Layout::parse(&bytes).expect("layout");
        assert_eq!(layout.count, 1);
        assert!(layout.covers(&bytes));
        // A truncated file must not report a covering layout.
        assert!(!layout.covers(&bytes[..bytes.len() - 1]));
    }

    /// A "set colour" chunk is one colour whose three values start at the chunk byte itself.
    #[test]
    fn a_literal_chunk_sets_one_colour() {
        let mut palette: Palette = [[0u8; 3]; 256];
        let mut record = vec![0u8; 8];
        record[0] = 2; // 4 * 2 = 8 bytes, body 7
        record[1..4].copy_from_slice(&[63, 0, 0]); // red
        record[4..7].copy_from_slice(&[0, 63, 0]); // green
        assert!(apply_palette(&mut palette, &record));
        assert_eq!(palette[0], [PALMAP[63], 0, 0]);
        assert_eq!(palette[1], [0, PALMAP[63], 0]);
        assert_eq!(palette[2], [0, 0, 0]);
    }

    #[test]
    fn a_frame_carries_the_palette_it_declares() {
        let bytes = tiny_smk();
        let palette = palette_at(&bytes, 0).expect("the frame declares a palette");
        assert_eq!(palette[0], [PALMAP[63], 0, 0]);
        assert_eq!(palette[1], [0, PALMAP[63], 0]);
    }

    /// A skip chunk leaves the previous palette in place.
    #[test]
    fn a_skip_chunk_keeps_the_old_entries() {
        let mut palette: Palette = [[9u8; 3]; 256];
        let record = [0u8, 0x80 | 4];
        let _ = apply_palette(&mut palette, &record);
        assert_eq!(palette[0], [9, 9, 9]);
    }
}
