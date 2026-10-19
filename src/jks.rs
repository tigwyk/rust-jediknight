//! `.jks` savegames: the record container JK.EXE writes.
//!
//! From `JK.EXE` (Ghidra), the writer is `FUN_004da9c0` @ `0x4da9c0`, the reader
//! `FUN_004dab70` @ `0x4dab70`, and the per-object record writer `FUN_004f4210` @ `0x4f4210`.
//! Its tail pins the record framing:
//!
//! ```asm
//! 0x4f4693: SUB ECX, 0x8b4c28          ; payload bytes = end - payload start
//! 0x4f4699: MOV [0x008b4c20], ECX      ; length at header + 0x20
//! 0x4f46ac: MOV [0x008b4c04], 0        ; header field at + 0x04 = 0
//! 0x4f46b6: MOV word [0x008b4c24], 0xc ; record type 0x0C (a thing) at + 0x24
//! 0x4f46bf: CALL 0x004e1910            ; write header, then payload
//! ```
//!
//! so the file is:
//!
//! ```text
//! 0x000  0x62C (1580)  save header; int32 at +0 must be 6
//! 0x62C  0x20  (32)    level name
//! 0x64C  4             one field, DAT_00836c14
//! 0x650  0x18  (24)    player block, DAT_008ee660
//! 0x668  ...           records: 0x28-byte header, then `length` payload bytes
//! ```
//!
//! A record header is **0x28 bytes**: `+0x04` is zero, `+0x20` holds the payload length and
//! `+0x24` the record type (`0x0C` for a thing; `FUN_004e1dd0` @ `0x4e1dd0` dispatches on it
//! through a 65-entry table). Payload field layouts are not modelled here — the bytes are kept
//! verbatim, which is enough to read a real save's record list and to write one back.

use std::io::{Error, ErrorKind};

/// The version the engine demands in the header's first `int32`.
pub const VERSION: u32 = 6;
/// The header block's size (`FUN_00431470(&DAT_00835918, 0x62c)`).
pub const HEADER_LEN: usize = 0x62C;
pub const LEVEL_LEN: usize = 0x20;
pub const PLAYER_LEN: usize = 0x18;
/// A record's header: length at +0x20, type at +0x24, payload after.
pub const RECORD_HEADER: usize = 0x28;
/// Everything before the first record.
pub const PREFIX_LEN: usize = HEADER_LEN + LEVEL_LEN + 4 + PLAYER_LEN;

/// One record: its raw 0x28-byte header and its payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub header: [u8; RECORD_HEADER],
    pub data: Vec<u8>,
}

impl Section {
    /// A record of `kind` with an empty header otherwise, ready to be filled in.
    pub fn new(kind: u16, data: Vec<u8>) -> Section {
        let mut header = [0u8; RECORD_HEADER];
        header[0x24..0x26].copy_from_slice(&kind.to_le_bytes());
        Section { header, data }
    }

    /// The record type at `+0x24`.
    pub fn kind(&self) -> u16 {
        u16::from_le_bytes([self.header[0x24], self.header[0x25]])
    }

    /// The payload length the header claims at `+0x20`.
    pub fn length(&self) -> usize {
        u32::from_le_bytes([
            self.header[0x20],
            self.header[0x21],
            self.header[0x22],
            self.header[0x23],
        ]) as usize
    }
}

/// A savegame: the recovered prefix plus typed records.
#[derive(Debug, Clone, PartialEq)]
pub struct Save {
    /// The 1580-byte block, opaque: only the version word inside it is understood.
    pub header: Vec<u8>,
    pub level: String,
    pub field: u32,
    pub player: [u8; PLAYER_LEN],
    pub sections: Vec<Section>,
}

impl Save {
    /// A fresh save for a level, with the version set and the name padded.
    pub fn new(level: &str) -> Save {
        let mut header = vec![0u8; HEADER_LEN];
        header[0..4].copy_from_slice(&VERSION.to_le_bytes());
        Save {
            header,
            level: level.to_ascii_lowercase(),
            field: 0,
            player: [0u8; PLAYER_LEN],
            sections: Vec::new(),
        }
    }

    /// The version word the file claims.
    pub fn version(&self) -> u32 {
        u32::from_le_bytes([self.header[0], self.header[1], self.header[2], self.header[3]])
    }

    /// Serialise: prefix, then every record with its length and type filled in.
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PREFIX_LEN + 64);
        out.extend_from_slice(&self.header);
        let mut name = [0u8; LEVEL_LEN];
        for (slot, byte) in name.iter_mut().zip(self.level.as_bytes()) {
            *slot = byte.to_ascii_lowercase();
        }
        out.extend_from_slice(&name);
        out.extend_from_slice(&self.field.to_le_bytes());
        out.extend_from_slice(&self.player);
        for section in &self.sections {
            let mut header = section.header;
            header[0x04..0x08].copy_from_slice(&0u32.to_le_bytes());
            header[0x20..0x24].copy_from_slice(&(section.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&header);
            out.extend_from_slice(&section.data);
        }
        out
    }

    /// Parse, checking the prefix the way the engine does: version must be 6, and the file
    /// must at least hold the prefix.
    pub fn read(bytes: &[u8]) -> Result<Save, Error> {
        if bytes.len() < PREFIX_LEN {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("{} bytes is shorter than the {PREFIX_LEN}-byte prefix", bytes.len()),
            ));
        }
        let header = bytes[0..HEADER_LEN].to_vec();
        let version = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        if version != VERSION {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("version {version}, the engine only loads {VERSION}"),
            ));
        }
        let level = String::from_utf8_lossy(&bytes[HEADER_LEN..HEADER_LEN + LEVEL_LEN])
            .trim_end_matches('\0')
            .to_string();
        let at = HEADER_LEN + LEVEL_LEN;
        let field = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        let mut player = [0u8; PLAYER_LEN];
        player.copy_from_slice(&bytes[at + 4..at + 4 + PLAYER_LEN]);
        let mut sections = Vec::new();
        let mut cursor = PREFIX_LEN;
        while cursor + RECORD_HEADER <= bytes.len() {
            let mut record = [0u8; RECORD_HEADER];
            record.copy_from_slice(&bytes[cursor..cursor + RECORD_HEADER]);
            let section = Section { header: record, data: Vec::new() };
            let len = section.length();
            let start = cursor + RECORD_HEADER;
            if start + len > bytes.len() {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "record of type 0x{:02X} claims {len} bytes, {} remain",
                        section.kind(),
                        bytes.len() - start
                    ),
                ));
            }
            let mut section = section;
            section.data = bytes[start..start + len].to_vec();
            sections.push(section);
            cursor = start + len;
        }
        Ok(Save { header, level, field, player, sections })
    }
}

/// A thing record's payload, in the layout `FUN_004f4210` stages.
///
/// The writer fills the record buffer from the thing (Ghidra, `0x4f4210`):
///
/// ```text
/// payload + 0x00  u16  thing + 0x04      template index
/// payload + 0x02  u16  thing + 0x0C      thing type
/// payload + 0x04  u16  *(thing+0x78)+4
/// payload + 0x06  u32  thing + 0x74
/// payload + 0x0A  u32  thing + 0x08
/// payload + 0x0E  f32  thing + 0xB8      position x
/// payload + 0x12  f32  thing + 0xBC      position y
/// payload + 0x16  f32  thing + 0xC0      position z
/// payload + 0x1A  f32  thing + 0x88      pitch
/// payload + 0x1E  f32  thing + 0x8C      yaw
/// payload + 0x22  f32  thing + 0x90      roll
/// ```
///
/// The reader's own field order is not recovered (its entry is a bare `LAB_` block at
/// `0x4f46f0`, not a Ghidra function), so this struct models the writer's fields and zero-fills
/// the rest: enough to store and restore a thing's identity, position and angles, not yet to
/// rebuild one the engine would accept.
pub const THING_PAYLOAD: usize = 0x50;

#[derive(Debug, Clone, PartialEq)]
pub struct ThingRecord {
    pub template: u16,
    pub kind: u16,
    pub pos: [f32; 3],
    pub angles: [f32; 3],
}

impl ThingRecord {
    /// The payload bytes, with the writer's field offsets.
    pub fn to_payload(&self) -> Vec<u8> {
        let mut out = vec![0u8; THING_PAYLOAD];
        out[0x00..0x02].copy_from_slice(&self.template.to_le_bytes());
        out[0x02..0x04].copy_from_slice(&self.kind.to_le_bytes());
        for (i, value) in self.pos.iter().enumerate() {
            let at = 0x0E + i * 4;
            out[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (i, value) in self.angles.iter().enumerate() {
            let at = 0x1A + i * 4;
            out[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        out
    }

    /// Read the same offsets back.
    pub fn from_payload(data: &[u8]) -> Option<ThingRecord> {
        if data.len() < THING_PAYLOAD {
            return None;
        }
        let word = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
        let float = |at: usize| {
            f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
        };
        let mut pos = [0.0; 3];
        let mut angles = [0.0; 3];
        for i in 0..3 {
            pos[i] = float(0x0E + i * 4);
            angles[i] = float(0x1A + i * 4);
        }
        Some(ThingRecord { template: word(0), kind: word(2), pos, angles })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_round_trips_with_the_engine_prefix() {
        let mut save = Save::new("01narshadda.jkl");
        save.player[0] = 0x42;
        save.field = 7;
        // Type 0x0C is a thing record, straight out of `FUN_004f4210`'s tail.
        save.sections.push(Section::new(0x0C, vec![1, 2, 3]));
        let bytes = save.write();
        assert_eq!(bytes.len(), PREFIX_LEN + RECORD_HEADER + 3);
        // The writer's own fields: length at +0x20, type at +0x24, +0x04 zero.
        let record = &bytes[PREFIX_LEN..PREFIX_LEN + RECORD_HEADER];
        assert_eq!(u32::from_le_bytes([record[0x20], record[0x21], record[0x22], record[0x23]]), 3);
        assert_eq!(u16::from_le_bytes([record[0x24], record[0x25]]), 0x0C);
        assert_eq!(&record[0x04..0x08], &[0, 0, 0, 0]);
        let back = Save::read(&bytes).expect("round trip");
        assert_eq!(back.level, "01narshadda.jkl");
        assert_eq!(back.field, 7);
        assert_eq!(back.player[0], 0x42);
        assert_eq!(back.sections[0].kind(), 0x0C);
        assert_eq!(back.sections[0].data, vec![1, 2, 3]);
        assert_eq!(back.version(), 6);
    }

    #[test]
    fn records_are_walked_by_their_own_length() {
        let mut save = Save::new("l");
        save.sections.push(Section::new(0x0C, vec![0xAA; 5]));
        save.sections.push(Section::new(0x0D, vec![0xBB; 2]));
        let back = Save::read(&save.write()).expect("two records");
        assert_eq!(back.sections.len(), 2);
        assert_eq!(back.sections[1].kind(), 0x0D);
        assert_eq!(back.sections[1].data, vec![0xBB; 2]);
    }

    #[test]
    fn a_thing_payload_keeps_its_position_and_angles() {
        let thing = ThingRecord {
            template: 7,
            kind: 2,
            pos: [1.5, -2.25, 0.75],
            angles: [10.0, 20.0, 30.0],
        };
        let payload = thing.to_payload();
        assert_eq!(payload.len(), THING_PAYLOAD);
        // The offsets the writer uses, straight from `0x4f4210`.
        assert_eq!(u16::from_le_bytes([payload[0], payload[1]]), 7);
        assert_eq!(u16::from_le_bytes([payload[2], payload[3]]), 2);
        assert_eq!(
            f32::from_le_bytes([payload[0x0E], payload[0x0F], payload[0x10], payload[0x11]]),
            1.5
        );
        assert_eq!(
            f32::from_le_bytes([payload[0x1A], payload[0x1B], payload[0x1C], payload[0x1D]]),
            10.0
        );
        let back = ThingRecord::from_payload(&payload).expect("payload");
        assert_eq!(back, thing);
    }

    #[test]
    fn a_version_other_than_six_is_refused() {
        let mut bytes = Save::new("01narshadda.jkl").write();
        bytes[0] = 5;
        let error = Save::read(&bytes).expect_err("the engine only loads version 6");
        assert!(error.to_string().contains("only loads 6"), "{error}");
    }

    #[test]
    fn a_truncated_prefix_is_refused() {
        let error = Save::read(&[0u8; 16]).expect_err("too short");
        assert!(error.to_string().contains("shorter"), "{error}");
    }
}