//! `.sft` sprite fonts: the glyph strip the UI draws all of its text with.
//!
//! The format is a 40-byte header, one block per character range, and then an embedded
//! `.bm` holding a single glyph strip. Recovered from `stdFont_Load` in the engine (see
//! `docs/formats.md` §18): the header is `SFNT`, version `10`, `marginY`, `marginX`, a
//! `field_10` the engine stores as `field_28`, and a charset count. Each charset is
//! `uint16 first, uint16 last` followed by one `{int32 glyphTexX, int32 glyphWidth}` per
//! character. The strip is the same `.bm` layout as every other bitmap, with `glyphTexX`
//! indexing into it and the strip's height being the line height.
//!
//! The two margins are used the way `stdFont_DrawAsciiGPU` uses them, which is the
//! opposite of what their names suggest: `marginY` is the gap after every glyph on the
//! line, and `marginX` is how wide a space and a tab step are.

use crate::bm;

/// The first four bytes of every `.sft`.
const MAGIC: &[u8; 4] = b"SFNT";
/// Only version 10 is shipped, and the engine refuses anything else.
const VERSION: u32 = 10;
/// `sizeof(stdFontHeader)`.
const HEADER: usize = 40;

/// One contiguous character range and where each glyph sits in the strip.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Charset {
    pub first: u16,
    pub last: u16,
    /// `(glyphTexX, glyphWidth)` per character, in character order.
    pub entries: Vec<(i32, i32)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Font {
    /// Pixels added after every glyph, from the header's `marginY`.
    pub margin_y: i32,
    /// How far a space or a tab steps, from the header's `marginX`.
    pub margin_x: i32,
    /// `field_10` in the header: the engine keeps it in `stdFont::field_28`.
    pub field_10: i32,
    pub charsets: Vec<Charset>,
    pub strip_width: u32,
    /// The strip's height is the font's line height.
    pub height: u32,
    /// 8-bit palette indices into whatever palette the surface being drawn has; `0` is
    /// transparent. The glyphs are authored against the menu art's palette.
    pub strip: Vec<u8>,
    /// Whether every charset and the whole strip were read.
    pub exact: bool,
}

fn i32_at(bytes: &[u8], at: usize) -> Option<i32> {
    let s = bytes.get(at..at + 4)?;
    Some(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let s = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

/// Parse a `.sft`. `None` means "not a font at all" (wrong magic or version).
pub fn parse(bytes: &[u8]) -> Option<Font> {
    if bytes.get(0..4)? != MAGIC {
        return None;
    }
    if i32_at(bytes, 4)? as u32 != VERSION {
        return None;
    }
    let mut font = Font {
        margin_y: i32_at(bytes, 8)?,
        margin_x: i32_at(bytes, 12)?,
        field_10: i32_at(bytes, 16)?,
        ..Default::default()
    };
    let charsets = i32_at(bytes, 20)?.max(0) as usize;
    let mut at = HEADER;
    for _ in 0..charsets {
        let (Some(first), Some(last)) = (u16_at(bytes, at), u16_at(bytes, at + 2)) else {
            return Some(font);
        };
        at += 4;
        if last < first {
            return Some(font);
        }
        let count = (last - first + 1) as usize;
        let Some(block) = bytes.get(at..at + count * 8) else {
            return Some(font);
        };
        let entries = (0..count)
            .map(|i| {
                let e = &block[i * 8..i * 8 + 8];
                (
                    i32::from_le_bytes([e[0], e[1], e[2], e[3]]),
                    i32::from_le_bytes([e[4], e[5], e[6], e[7]]),
                )
            })
            .collect();
        at += count * 8;
        font.charsets.push(Charset { first, last, entries });
    }
    // The rest of the file is a standard `.bm` with exactly one cell: the glyph strip.
    let Some(bitmap) = bm::parse(&bytes[at.min(bytes.len())..]) else {
        return Some(font);
    };
    let Some(cell) = bitmap.cells.first() else {
        return Some(font);
    };
    font.strip_width = cell.width;
    font.height = cell.height;
    font.strip = cell.pixels.clone();
    font.exact = bitmap.exact && font.charsets.len() == charsets;
    Some(font)
}

impl Font {
    /// Where a character's bitmap is, and how wide it is. `None` for a character the
    /// font does not carry.
    pub fn glyph(&self, ch: char) -> Option<(u32, u32)> {
        let c = u32::from(ch);
        let c = u16::try_from(c).ok()?;
        let set = self
            .charsets
            .iter()
            .find(|s| c >= s.first && c <= s.last)?;
        let (x, w) = *set.entries.get((c - set.first) as usize)?;
        (x >= 0 && w > 0).then_some((x as u32, w as u32))
    }

    /// How far the pen moves after drawing (or skipping) one character, the way the
    /// engine's own text loop moves it.
    pub fn advance(&self, ch: char) -> i32 {
        match ch {
            '\t' => self.margin_x.max(0) * 8,
            ' ' => self.margin_x.max(0),
            _ => self.glyph(ch).map_or(0, |(_, w)| w as i32 + self.margin_y.max(0)),
        }
    }

    /// Width of a line of text, margins included.
    pub fn text_width(&self, text: &str) -> u32 {
        text.chars()
            .filter(|c| *c != '\n')
            .map(|c| self.advance(c))
            .sum::<i32>()
            .max(0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    /// A font with one charset over `A`..`B`, two glyphs in an 8x4 strip.
    fn build() -> Vec<u8> {
        let mut bytes = vec![0u8; 40];
        bytes[0..4].copy_from_slice(b"SFNT");
        bytes[4..8].copy_from_slice(&10i32.to_le_bytes());
        bytes[8..12].copy_from_slice(&1i32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2i32.to_le_bytes());
        bytes[20..24].copy_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&65u16.to_le_bytes());
        bytes.extend_from_slice(&66u16.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&3i32.to_le_bytes());
        bytes.extend_from_slice(&3i32.to_le_bytes());
        bytes.extend_from_slice(&5i32.to_le_bytes());
        // The glyph strip: a `.bm` with one 8x4 cell, ink in the first 3x4 block.
        let mut bm = vec![0u8; crate::bm::HEADER];
        bm[0..4].copy_from_slice(b"BM  ");
        bm[4..8].copy_from_slice(&70u32.to_le_bytes());
        bm[36..40].copy_from_slice(&8u32.to_le_bytes());
        bm.extend_from_slice(&8u32.to_le_bytes());
        bm.extend_from_slice(&4u32.to_le_bytes());
        bm.extend(std::iter::repeat_n(0u8, 32));
        bytes.extend_from_slice(&bm);
        bytes
    }

    #[test]
    fn reads_charsets_and_the_glyph_strip() {
        let font = parse(&build()).expect("SFNT");
        assert!(font.exact);
        assert_eq!(font.height, 4);
        assert_eq!(font.strip_width, 8);
        assert_eq!(font.margin_y, 1);
        assert_eq!(font.margin_x, 2);
        assert_eq!(font.charsets.len(), 1);
        assert_eq!(font.glyph('A'), Some((0, 3)));
        assert_eq!(font.glyph('B'), Some((3, 5)));
        assert_eq!(font.glyph('C'), None);
        // Two glyphs of 3 and 5, each followed by the header's 1px gap.
        assert_eq!(font.text_width("AB"), 10);
        // A space steps `marginX`, not a glyph width: 4 + 2 + 6.
        assert_eq!(font.text_width("A B"), 12);
    }

    #[test]
    fn rejects_foreign_bytes() {
        assert!(parse(b"MZ\x90\x00").is_none());
        let mut wrong = build();
        wrong[4] = 9;
        assert!(parse(&wrong).is_none());
    }
}
