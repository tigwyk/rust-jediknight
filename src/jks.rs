//! `.jks` savegames: the record container JK.EXE writes.
//!
//! From `JK.EXE` (Ghidra), the writer is `sithGamesave_Process` @ `0x4da9c0`, the reader
//! `sithGamesave_RestoreFile` @ `0x4dab70`, and the per-thing record writer
//! `sithDSSThing_FullDescription` @ `0x4f4210`. The file is a fixed prefix, then a stream of
//! `[u32 type][u32 length][payload]` records — the reader pins that framing:
//!
//! ```c
//! stdConffile_Read(&DAT_008b4c24,4);                  // the record TYPE
//! while (iVar2 != 0) {
//!   stdConffile_Read(&DAT_008b4c20,4);                // the payload LENGTH
//!   stdConffile_Read(&DAT_008b4c28,DAT_008b4c20);     // payload at the message body
//!   sithMessage_Process(&DAT_008b4c00);               // dispatch on the type
//!   stdConffile_Read(&DAT_008b4c24,4);                // next type; a failed read ends the file
//! }
//! ```
//!
//! so the file is:
//!
//! ```text
//! 0x000  0x62C (1580)  save header; int32 at +0 must be 6
//! 0x62C  0x24  (36)    game-side header callback block: the episode tag ("JK1") + one u32
//! 0x650  0x20  (32)    level name ("02narshadda.jkl")
//! 0x670  4             game time (`DAT_00836c14`, passed to `sithTime_SetGameTime`)
//! 0x674  0x18  (24)    player block (`DAT_008ee660`, the game-state block)
//! 0x68C  ...           records: u32 type, u32 length, then `length` payload bytes
//! ```
//!
//! The 0x28-byte message header the writers stage (`+0x04` zero, `+0x20` length, `+0x24`
//! type) never reaches the file: only its type, length and body do, so the length and type
//! are each written once and 8 bytes of the in-memory header are dropped. The prefix was
//! checked against both engine-written autosaves in `tools/parity/`: from `0x68C` the record
//! walk lands exactly on EOF (5856 records for level 1, 7704 for level 2), and level 2's first
//! record is the player at that level's spawn (10.78, -11.98, 3.02), which is the payload
//! layout below.
//!
//! Payload field layouts are only modelled for thing records (`ThingRecord`); every other
//! record's body is kept verbatim, which is enough to read and rewrite a real save.

use std::io::{Error, ErrorKind};

/// The version the engine demands in the header's first `int32`.
pub const VERSION: u32 = 6;
/// The header block's size (`stdConffile_Read(local_62c, 0x62c)`).
pub const HEADER_LEN: usize = 0x62C;
/// The game-side header callback block: episode tag plus one undecoded word.
pub const EPISODE_LEN: usize = 0x24;
/// The level name's field (`stdConffile_Write(level + 4, 0x20)`).
pub const LEVEL_LEN: usize = 0x20;
/// The game time word, `DAT_00836c14`.
pub const TIME_LEN: usize = 4;
pub const PLAYER_LEN: usize = 0x18;
/// A record's on-disk framing: the type, then the payload length.
pub const RECORD_HEADER: usize = 8;
/// Everything before the first record.
pub const PREFIX_LEN: usize = HEADER_LEN + EPISODE_LEN + LEVEL_LEN + TIME_LEN + PLAYER_LEN;

/// One record: its type (the message the engine dispatches on) and its payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// The record type, the value the engine reads into `DAT_008b4c24` (0x0C is a thing).
    pub kind: u32,
    pub data: Vec<u8>,
}

impl Section {
    /// A record of `kind` with an empty payload, ready to be filled in.
    pub fn new(kind: u32, data: Vec<u8>) -> Section {
        Section { kind, data }
    }

    /// The record type.
    pub fn kind(&self) -> u32 {
        self.kind
    }

    /// The payload length the file's length word carries.
    pub fn length(&self) -> usize {
        self.data.len()
    }
}

/// The engine's own bin records: `sithDSS_Inventory` @ `0x4f9900` sends one per inventory bin the
/// player's table knows, with a **fixed 28-byte payload**:
///
/// ```text
/// +0x00  u16  thing + 0x04      template index (the player's)
/// +0x02  u16  the bin index
/// +0x04  u32  bin record +0x00  the bin's value
/// +0x08  u32  bin record +0x08
/// +0x0C  u32  bin record +0x04
/// +0x10  u32  bin record +0x0C
/// +0x14  u32  bin record +0x10
/// +0x18  u32  bin record +0x14
/// ```
///
/// The six words are kept in the order the writer sends them, so two saves can be compared word
/// by word; `value` is the first of them, which is where the engine keeps the bin's amount.
pub const INVENTORY_RECORD: u32 = 0x10;
pub const INVENTORY_PAYLOAD: usize = 0x1C;

#[derive(Debug, Clone, PartialEq)]
pub struct InventoryBin {
    pub template: u16,
    pub bin: u16,
    /// The bin record's six words, in the order the writer sends them (`+0x00` first).
    pub words: [u32; 6],
}

impl InventoryBin {
    pub fn parse(data: &[u8]) -> Option<InventoryBin> {
        if data.len() != INVENTORY_PAYLOAD {
            return None;
        }
        let word16 = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
        let mut words = [0u32; 6];
        for (i, word) in words.iter_mut().enumerate() {
            let at = 4 + i * 4;
            *word = u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
        }
        Some(InventoryBin { template: word16(0), bin: word16(2), words })
    }

    /// The bin's amount, the way the engine keeps it: a float in the first word it sends.
    pub fn value(&self) -> f32 {
        f32::from_bits(self.words[0])
    }
}

/// The engine's whole bin table, in the order `sithGamesave_SaveCurrentWorld` (`0x4da770`) walks
/// `DAT_008bd1c0` and sends one `sithDSS_Inventory` (`0x4f9900`) record per set entry. The ranges
/// are the engine's own registered types (weapons, force powers, items, shields, goals, scores…);
/// both shipped autosaves carry exactly these 82 bins, so this is frozen data, not a guess.
pub const ENGINE_BINS: [u16; 82] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32, 33, 34, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 60, 61, 62,
    63, 64, 65, 66, 70, 71, 72, 73, 74, 75, 80, 81, 82, 83, 84, 99, 100, 101, 102, 103, 104,
    105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 150,
];

/// One `sithDSS_Inventory` record per engine bin, with the player's amount in the value word.
///
/// The payload is fixed at 28 bytes (see [`INVENTORY_PAYLOAD`]). The engine's other five words are
/// its bin-record fields (carries/minimums/flags and a `-1.0` unset sentinel); this writer writes the
/// value and the sentinel, and leaves the four middle words zero, because the fields are not
/// recovered. The **values** are what the level-2 carry is compared on.
pub fn inventory_sections(inv: &std::collections::BTreeMap<i32, f32>, player: u16) -> Vec<Section> {
    ENGINE_BINS
        .iter()
        .map(|&bin| {
            let mut data = vec![0u8; INVENTORY_PAYLOAD];
            data[0..2].copy_from_slice(&player.to_le_bytes());
            data[2..4].copy_from_slice(&bin.to_le_bytes());
            let value = inv.get(&(bin as i32)).copied().unwrap_or(0.0);
            data[4..8].copy_from_slice(&value.to_bits().to_le_bytes());
            // Payload +0x18 is the sixth word, `bin record +0x14`: `-1.0` in every shipped record.
            data[0x18..0x1C].copy_from_slice(&f32::NEG_INFINITY.to_bits().to_le_bytes());
            Section::new(INVENTORY_RECORD, data)
        })
        .collect()
}

/// Every bin-value difference between two savegame bin tables, sorted by bin: a changed value, and
/// a bin either side is missing. Empty means the two carries are the same table.
pub fn bin_value_drift(expected: &Save, actual: &Save) -> Vec<String> {
    let table = |save: &Save| -> std::collections::BTreeMap<u16, f32> {
        save.bins().into_iter().map(|bin| (bin.bin, bin.value())).collect()
    };
    let (want, got) = (table(expected), table(actual));
    let mut drift = Vec::new();
    for (bin, value) in &want {
        match got.get(bin) {
            None => drift.push(format!("bin {bin}: missing (expected {value})")),
            Some(actual) if (actual - value).abs() > 1e-4 => {
                drift.push(format!("bin {bin}: {actual}, expected {value}"))
            }
            Some(_) => {}
        }
    }
    for bin in got.keys().filter(|bin| !want.contains_key(bin)) {
        drift.push(format!("bin {bin}: unexpected"));
    }
    drift
}

/// A savegame: the recovered prefix plus typed records.
#[derive(Debug, Clone, PartialEq)]
pub struct Save {
    /// The 1580-byte block, opaque: only the version word inside it is understood.
    pub header: Vec<u8>,
    /// The game-side header callback block: the first bytes are the episode tag (`JK1` for the
    /// single-player campaign), then a u32 whose meaning is not recovered (1 in level 1's
    /// autosave, 3 in level 2's).
    pub episode: Vec<u8>,
    pub level: String,
    pub time: u32,
    pub player: [u8; PLAYER_LEN],
    pub sections: Vec<Section>,
}

impl Save {
    /// A fresh save for a level, with the version set and the name padded. The engine stores the
    /// bare file name (`02narshadda.jkl`), so a path from the archive index is cut down to it.
    pub fn new(level: &str) -> Save {
        let mut header = vec![0u8; HEADER_LEN];
        header[0..4].copy_from_slice(&VERSION.to_le_bytes());
        let mut episode = vec![0u8; EPISODE_LEN];
        // The engine's own tag for this campaign; a save without it would not restore.
        episode[0..3].copy_from_slice(b"JK1");
        let name = level.rsplit(['/', '\\']).next().unwrap_or(level);
        Save {
            header,
            episode,
            level: name.to_ascii_lowercase(),
            time: 0,
            player: [0u8; PLAYER_LEN],
            sections: Vec::new(),
        }
    }

    /// The version word the file claims.
    pub fn version(&self) -> u32 {
        u32::from_le_bytes([self.header[0], self.header[1], self.header[2], self.header[3]])
    }

    /// The episode tag from the game-side header block.
    pub fn episode_tag(&self) -> String {
        String::from_utf8_lossy(&self.episode[0..EPISODE_LEN.min(0x20)])
            .trim_end_matches('\0')
            .to_string()
    }

    /// The engine's own bin records, in file order (one per bin the player's table knows).
    pub fn bins(&self) -> Vec<InventoryBin> {
        self.sections
            .iter()
            .filter(|s| s.kind == INVENTORY_RECORD)
            .filter_map(|s| InventoryBin::parse(&s.data))
            .collect()
    }

    /// Serialise: prefix, then every record as its type, length and payload.
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PREFIX_LEN + 64);
        out.extend_from_slice(&self.header);
        out.extend_from_slice(&self.episode);
        let mut name = [0u8; LEVEL_LEN];
        for (slot, byte) in name.iter_mut().zip(self.level.as_bytes()) {
            *slot = byte.to_ascii_lowercase();
        }
        out.extend_from_slice(&name);
        out.extend_from_slice(&self.time.to_le_bytes());
        out.extend_from_slice(&self.player);
        for section in &self.sections {
            out.extend_from_slice(&section.kind.to_le_bytes());
            out.extend_from_slice(&(section.data.len() as u32).to_le_bytes());
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
        let episode = bytes[HEADER_LEN..HEADER_LEN + EPISODE_LEN].to_vec();
        let at = HEADER_LEN + EPISODE_LEN;
        let level = String::from_utf8_lossy(&bytes[at..at + LEVEL_LEN])
            .trim_end_matches('\0')
            .to_string();
        let at = at + LEVEL_LEN;
        let time = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        let mut player = [0u8; PLAYER_LEN];
        player.copy_from_slice(&bytes[at + TIME_LEN..at + TIME_LEN + PLAYER_LEN]);
        let mut sections = Vec::new();
        let mut cursor = PREFIX_LEN;
        while cursor + RECORD_HEADER <= bytes.len() {
            let kind = u32::from_le_bytes([
                bytes[cursor],
                bytes[cursor + 1],
                bytes[cursor + 2],
                bytes[cursor + 3],
            ]);
            let len = u32::from_le_bytes([
                bytes[cursor + 4],
                bytes[cursor + 5],
                bytes[cursor + 6],
                bytes[cursor + 7],
            ]) as usize;
            let start = cursor + RECORD_HEADER;
            if start + len > bytes.len() {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "record of type 0x{kind:02X} claims {len} bytes, {} remain",
                        bytes.len() - start
                    ),
                ));
            }
            sections.push(Section { kind, data: bytes[start..start + len].to_vec() });
            cursor = start + len;
        }
        Ok(Save { header, episode, level, time, player, sections })
    }
}

/// A thing record's payload, in the layout `sithDSSThing_FullDescription` stages.
///
/// The writer fills the message body from the thing (Ghidra, `0x4f4210`):
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
/// Checked against a real save: level 2's autosave opens with a type-0x0C record whose type
/// word is 10 (a player) at (10.78, -11.98, 3.02) — that level's spawn pose. The record's own
/// variable-length tail (the type-specific fields the switch in `0x4f4210` packs after the
/// common block) is decoded for the one field the exit comparison needs: the player's health.
/// `0x5014e0` registers `GetThingHealth`/`GetHealth` on `0x502570`, and that handler pushes
/// `*(f32 *)(thing + 0x134)` for types 2 and 10 — the very word the type-specific block writes as
/// `puVar3[1]` (its first word is `thing + 0x130`, the second `thing + 0x134`). The block starts at
/// payload 0x67, plus four bytes when `*(thing + 0x2A8) & 0x400` is set (the shipped player sets it),
/// so the player's health is the f32 at **payload 0x6F**, which is where every shipped save holds
/// `100.0`. The unpadded block plus its AI tail plus the physics tail adds up to the 183-byte
/// payload the player's record carries, which is what confirms the shifted variant.
pub const THING_PAYLOAD: usize = 0x50;

/// One entry of a retail save's `BKSV` thing list (`sections` kind `0x1001`).
///
/// The entry is `u16 kind`, the ASCII template name, then seven floats — position, angles and one
/// more value (100.0 for the player's health) — followed by a 17-byte type-specific tail. Both
/// shipped saves walk on that rule: the tail measured 17 bytes for hundreds of entries across every
/// kind 1..14, and the handful of 14/15/16 readings are names whose last character also parses as a
/// float byte, not a second layout.
#[derive(Debug, Clone, PartialEq)]
pub struct RetailThing {
    pub kind: u16,
    pub template: String,
    pub pos: [f32; 3],
    pub angles: [f32; 3],
    /// The seventh float: the player's health, `1.0` on statics.
    pub value: f32,
}

/// The length of every entry's tail after the seven floats.
pub const RETAIL_TAIL: usize = 17;

/// Decode a retail `0x1001` payload. Returns an empty list when the payload is not a `BKSV` list, and
/// stops at the first entry whose shape does not hold rather than inventing a boundary — the caller can
/// compare the count against the payload length to see how far the walk got.
pub fn retail_things(payload: &[u8]) -> (Vec<RetailThing>, usize) {
    let mut out = Vec::new();
    if !payload.starts_with(b"BKSV") {
        return (out, 0);
    }
    let float = |at: usize| -> Option<f32> {
        let b = payload.get(at..at + 4)?;
        let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        (v.is_finite() && v.abs() < 1e4).then_some(v)
    };
    let mut at = 12;
    while at + 4 <= payload.len() {
        let kind = u16::from_le_bytes([payload[at], payload[at + 1]]);
        if !(1..=14).contains(&kind) {
            break;
        }
        let mut end = at + 2;
        while end < payload.len()
            && (payload[end].is_ascii_alphanumeric() || b"_+-.".contains(&payload[end]))
        {
            end += 1;
        }
        // The name ends where seven plausible floats begin — but a trailing letter can also read as a
        // float byte (`0x59` 'Y' in `walkplayerY`, `0x66` 'f' in `cargoshipf`), which shifts the entry by
        // one and derails the walk. So a candidate is only accepted when the entry it implies is followed
        // by another entry's shape (or by the end of the list).
        let names_end = end;
        let starts_entry = |at: usize| -> bool {
            if at == payload.len() {
                return true;
            }
            let Some(word) = payload.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])) else {
                return false;
            };
            (1..=14).contains(&word)
                && payload
                    .get(at + 2)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b"_+-.".contains(b))
        };
        let mut parsed = None;
        for split in (at + 3..=names_end).rev() {
            let Some(floats) = (0..7)
                .map(|i| float(split + 4 * i))
                .collect::<Option<Vec<f32>>>()
            else {
                continue;
            };
            // The tail is 17 bytes for the overwhelming majority of entries, but a few kinds carry 14,
            // 15 or 16 (measured across both shipped saves). Accept the first tail whose following entry
            // has a plausible shape, 17 first so the common case never pays for the scan.
            if let Some(tail) = [RETAIL_TAIL, 16, 15, 14, 18, 19]
                .into_iter()
                .find(|tail| starts_entry(split + 28 + tail))
            {
                parsed = Some((split, floats, tail));
                break;
            }
        }
        // No tail made the *following* entry plausible: this is either the last entry of the list or the
        // point where the container changes shape. Accept the entry with the canonical tail and stop, so
        // the walk never loses the record it already read.
        let terminal = parsed.is_none();
        let (split, floats, tail) = match parsed {
            Some(found) => found,
            None => {
                let mut fallback = None;
                for split in (at + 3..=names_end).rev() {
                    if let Some(floats) = (0..7)
                        .map(|i| float(split + 4 * i))
                        .collect::<Option<Vec<f32>>>()
                    {
                        fallback = Some((split, floats, RETAIL_TAIL));
                        break;
                    }
                }
                let Some(found) = fallback else { break };
                found
            }
        };
        let next = split + 28 + tail;
        if next > payload.len() {
            break;
        }
        out.push(RetailThing {
            kind,
            template: String::from_utf8_lossy(&payload[at + 2..split]).to_string(),
            pos: [floats[0], floats[1], floats[2]],
            angles: [floats[3], floats[4], floats[5]],
            value: floats[6],
        });
        at = next;
        if terminal {
            break;
        }
    }
    // The bytes the walk consumed: a caller compares it with the payload's length to see whether the
    // list ended cleanly or hit a shape this decoder does not follow.
    (out, at)
}

/// The thing type that means "player" (`GetThingHealth` accepts 2 and 10; the player is 10).
pub const PLAYER_KIND: u16 = 10;
/// Where the player record's health word is (`thing + 0x134`).
pub const PLAYER_HEALTH_OFFSET: usize = 0x6F;

#[derive(Debug, Clone, PartialEq)]
pub struct ThingRecord {
    pub template: u16,
    pub kind: u16,
    pub pos: [f32; 3],
    pub angles: [f32; 3],
    /// The player's health, from the type-specific tail. `None` for anything but a player record
    /// (or for a payload too short to hold it) — the tail is only modelled this far.
    pub health: Option<f32>,
}

impl ThingRecord {
    /// The payload bytes, with the writer's field offsets. The common block is `THING_PAYLOAD`
    /// long; a record that carries a health word is extended to reach it.
    pub fn to_payload(&self) -> Vec<u8> {
        let len = if self.health.is_some() { PLAYER_HEALTH_OFFSET + 4 } else { THING_PAYLOAD };
        let mut out = vec![0u8; len];
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
        if let Some(health) = self.health {
            out[PLAYER_HEALTH_OFFSET..PLAYER_HEALTH_OFFSET + 4]
                .copy_from_slice(&health.to_le_bytes());
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
        let kind = word(2);
        // The player's health word is only read where the writer puts it for a shipped player
        // record, and only when it is a plausible health: a payload that stops short of the
        // type-specific tail, or a value outside the engine's own 0..1000 band, stays unknown
        // rather than reporting a coincidence of the common block.
        let health = (kind == PLAYER_KIND && data.len() >= PLAYER_HEALTH_OFFSET + 4)
            .then(|| float(PLAYER_HEALTH_OFFSET))
            .filter(|health| health.is_finite() && (0.0..=1000.0).contains(health));
        Some(ThingRecord { template: word(0), kind, pos, angles, health })
    }
}

/// The engine-written autosaves kept as parity fixtures, read only if this checkout has them.
/// They are the only ground truth for the container's layout, so the tests below refuse to
/// guess: absent fixtures skip, present ones must walk to EOF.
fn fixture(name: &str) -> Option<Vec<u8>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools")
        .join("parity")
        .join(name);
    std::fs::read(path).ok()
}


#[test]
fn retail_list_decodes_entries_and_stops_on_a_bad_shape() {
    // Two entries with a 17-byte tail each, the second one a player, plus a trailing byte that cannot
    // start an entry: the walk must return both and stop rather than inventing a third.
    let mut payload = Vec::new();
    payload.extend_from_slice(b"BKSV");
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&73u32.to_le_bytes());
    for (kind, name, pos) in [(7u16, "4x4door", [1.0f32, 2.0, 3.0]), (10, "walkplayer", [4.0, 5.0, 6.0])] {
        payload.extend_from_slice(&kind.to_le_bytes());
        payload.extend_from_slice(name.as_bytes());
        for v in pos {
            payload.extend_from_slice(&v.to_le_bytes());
        }
        for v in [0.0f32, 0.0, 0.0, 100.0] {
            payload.extend_from_slice(&v.to_le_bytes());
        }
        // A distinctive tail: an all-zero one makes many split points look like valid entries, which
        // is a property of the fixture rather than of the decoder.
        payload.extend((0..RETAIL_TAIL).map(|i| 0x40 + i as u8));
    }
    // A kind word the walk cannot follow (`0` is not a shipped thing type): four bytes so it is read as
    // an entry header rather than being folded into the previous entry's tail.
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    let (got, consumed) = retail_things(&payload);
    assert_eq!(got.len(), 2);
    assert_eq!(consumed + 4, payload.len(), "the walk must stop before the malformed kind word");
    assert_eq!(got[0].template, "4x4door");
    assert_eq!(got[1].kind, PLAYER_KIND);
    assert_eq!(got[1].pos, [4.0, 5.0, 6.0]);
    assert_eq!(got[1].value, 100.0);
    // A payload without the magic decodes nothing.
    assert!(retail_things(b"nope").0.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_round_trips_with_the_engine_prefix() {
        let mut save = Save::new("01narshadda.jkl");
        save.player[0] = 0x42;
        save.time = 500;
        save.episode[0x20..0x24].copy_from_slice(&7u32.to_le_bytes());
        // Type 0x0C is a thing record, straight out of `sithDSSThing_FullDescription`'s tail.
        save.sections.push(Section::new(0x0C, vec![1, 2, 3]));
        let bytes = save.write();
        assert_eq!(bytes.len(), PREFIX_LEN + RECORD_HEADER + 3);
        // The framing the reader walks: type at +0, length at +4, payload after.
        let record = &bytes[PREFIX_LEN..PREFIX_LEN + RECORD_HEADER];
        assert_eq!(u32::from_le_bytes([record[0], record[1], record[2], record[3]]), 0x0C);
        assert_eq!(u32::from_le_bytes([record[4], record[5], record[6], record[7]]), 3);
        let back = Save::read(&bytes).expect("round trip");
        assert_eq!(back.level, "01narshadda.jkl");
        assert_eq!(back.episode_tag(), "JK1");
        assert_eq!(
            u32::from_le_bytes([
                back.episode[0x20],
                back.episode[0x21],
                back.episode[0x22],
                back.episode[0x23]
            ]),
            7
        );
        assert_eq!(back.time, 500);
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
            health: None,
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

    /// A player record carries its health in the type-specific tail: `0x5014e0` registers
    /// `GetThingHealth` on `0x502570`, which pushes `*(f32 *)(thing + 0x134)`, and
    /// `sithDSSThing_FullDescription` writes that word as the type-specific block's second
    /// entry — payload 0x6F for the shipped player (the block starts at 0x67; a flag bit
    /// shifts it by four). Every shipped save holds 100.0 there, and nothing else in the record
    /// is a health-shaped value, so this pins both the offset and the plausibility guard.
    #[test]
    fn a_player_payload_carries_its_health() {
        let player = ThingRecord {
            template: 0,
            kind: PLAYER_KIND,
            pos: [1.0, 2.0, 3.0],
            angles: [0.0; 3],
            health: Some(80.0),
        };
        let payload = player.to_payload();
        assert_eq!(payload.len(), PLAYER_HEALTH_OFFSET + 4);
        assert_eq!(
            f32::from_le_bytes([
                payload[PLAYER_HEALTH_OFFSET],
                payload[PLAYER_HEALTH_OFFSET + 1],
                payload[PLAYER_HEALTH_OFFSET + 2],
                payload[PLAYER_HEALTH_OFFSET + 3],
            ]),
            80.0
        );
        assert_eq!(ThingRecord::from_payload(&payload).expect("payload"), player);
        // A non-player keeps `None` even when the bytes happen to look like a health, and a
        // payload that stops before the tail keeps it unknown too.
        let mut other = player.clone();
        other.kind = 2;
        assert_eq!(ThingRecord::from_payload(&other.to_payload()).expect("payload").health, None);
        let short = ThingRecord::from_payload(&payload[..THING_PAYLOAD]).expect("common block");
        assert_eq!(short.health, None);
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

    /// The two autosaves JK.EXE wrote itself. This is the check that pins the container: the
    /// walk must consume the file exactly, the level name must be the level, and the first
    /// thing record must be the player standing at that level's spawn.
    #[test]
    fn a_real_autosave_walks_to_eof() {
        for (name, level, spawn) in [
            ("jkauto-01narshadda.jks", "01narshadda.jkl", None),
            (
                "jkauto-02narshadda.jks",
                "02narshadda.jkl",
                Some([10.78f32, -11.98, 3.02]),
            ),
        ] {
            let Some(bytes) = fixture(name) else { continue };
            let save = Save::read(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(save.version(), VERSION);
            assert_eq!(save.episode_tag(), "JK1", "{name}: the episode tag");
            assert_eq!(save.level, level, "{name}: the level name");
            assert!(!save.sections.is_empty(), "{name}: no records");
            let things: Vec<ThingRecord> = save
                .sections
                .iter()
                .filter(|s| s.kind() == 0x0C)
                .filter_map(|s| ThingRecord::from_payload(&s.data))
                .collect();
            assert!(things.len() > 100, "{name}: only {} thing records", things.len());
            let player = things.first().expect("a first record");
            assert_eq!(player.kind, 10, "{name}: the first record is not a player");
            // The player's health, from the type-specific tail (`thing + 0x134`, what
            // `GetThingHealth` reads): the engine's own saves both carry a full bar at spawn.
            assert_eq!(player.health, Some(100.0), "{name}: the player's health");
            if let Some(want) = spawn {
                for (got, want) in player.pos.iter().zip(want.iter()) {
                    assert!((got - want).abs() < 0.05, "{name}: player at {:?}, want {want}", player.pos);
                }
            }
            // Rewriting keeps the record stream: same count, same types and payloads.
            let again = Save::read(&save.write()).expect("the rewritten save parses");
            assert_eq!(again.sections.len(), save.sections.len(), "{name}: rewrite lost records");
            assert_eq!(again.sections[0], save.sections[0], "{name}: rewrite changed record 0");
        }
    }

    /// The carry is the engine's own 82-bin table: one fixed 28-byte record per bin, in the engine's
    /// order, and every word survives the writer and reader unchanged.
    #[test]
    fn a_carry_bin_table_round_trips_through_the_engine_records() {
        let inv: std::collections::BTreeMap<i32, f32> =
            [(1, 1.0), (2, 1.0), (73, 10.5), (75, 20.0)].into_iter().collect();
        let mut save = Save::new("02narshadda.jkl");
        save.sections.extend(inventory_sections(&inv, 0));
        let bins = save.bins();
        assert_eq!(bins.len(), ENGINE_BINS.len(), "the carry is not the engine's bin table");
        assert_eq!(bins.iter().map(|b| b.bin).collect::<Vec<_>>(), ENGINE_BINS);
        let population = bins.iter().find(|b| b.bin == 73).expect("bin 73");
        assert_eq!(population.value(), 10.5);
        let back = Save::read(&save.write()).expect("the written carry reads back");
        assert_eq!(back.bins(), bins, "a bin record changed in the round trip");
        assert_eq!(bin_value_drift(&save, &back), Vec::<String>::new());
        // Drift of any kind is named by its bin: a changed value, a missing bin, an extra one.
        let mut other = back.clone();
        other.sections.extend(inventory_sections(&[(73, 9.0)].into_iter().collect(), 0));
        assert!(bin_value_drift(&save, &other).iter().any(|line| line.starts_with("bin 73:")));
        assert!(bin_value_drift(&save, &save).is_empty());
    }

    /// The level-2 autosave's bin values are the target the port's carry is written and compared to;
    /// that file holds retail's `73 = 10.5`, the route's population credit.
    #[test]
    fn the_retail_level2_autosave_pins_the_carry_bin_values() {
        let Some(bytes) = fixture("jkauto-02narshadda.jks") else { return };
        let retail = Save::read(&bytes).expect("level 2's autosave");
        let bins = retail.bins();
        assert_eq!(bins.len(), ENGINE_BINS.len());
        let value = |bin: u16| bins.iter().find(|b| b.bin == bin).map(|b| b.value()).unwrap_or(0.0);
        assert_eq!(value(73), 10.5, "the population bin");
        assert_eq!(value(75), 20.0, "the tally's stored alignment");
        // Writing those values back matches the same table.
        let inv: std::collections::BTreeMap<i32, f32> =
            bins.iter().map(|b| (b.bin as i32, b.value())).collect();
        let mut carry = Save::new("02narshadda.jkl");
        carry.sections.extend(inventory_sections(&inv, 0));
        assert_eq!(bin_value_drift(&retail, &carry), Vec::<String>::new());
    }

    /// What the engine carries from level 1 to level 2, measured on its own two autosaves. The
    /// inventory records are the engine's whole bin table (82 bins, one fixed-size record each);
    /// exactly five of them differ between the two saves, so those five are the level change's
    /// measurable inventory delta. Bins 71, 73, 75, 99 and 102 are engine bins, not `items.dat`
    /// entries (that table has 1-10, 20-34, 40-51, 60, 83-84), so naming them is still open.
    #[test]
    fn a_real_autosave_carries_five_inventory_bins_between_levels() {
        let (Some(first), Some(second)) = (
            fixture("jkauto-01narshadda.jks"),
            fixture("jkauto-02narshadda.jks"),
        ) else {
            return;
        };
        let first = Save::read(&first).expect("level 1's autosave").bins();
        let second = Save::read(&second).expect("level 2's autosave").bins();
        assert_eq!(first.len(), 82, "the bin table is not the engine's 82 entries");
        assert_eq!(second.len(), 82, "the bin table is not the engine's 82 entries");
        // The engine's own bin table, in the order it sends: the contiguous ranges are its class
        // ranges (weapons, force powers, items, shields, goals, ...) rather than 0..81.
        let mut table: Vec<u16> = Vec::new();
        for range in [0..=18u16, 20..=34, 40..=51, 60..=66, 70..=75, 80..=84, 99..=115] {
            table.extend(range);
        }
        table.push(150);
        assert_eq!(
            first.iter().map(|b| b.bin).collect::<Vec<_>>(),
            table,
            "the records are not the engine's bin table in order"
        );
        let changed: Vec<(u16, f32, f32)> = first
            .iter()
            .zip(second.iter())
            .filter(|(a, b)| a.words != b.words)
            .map(|(a, b)| (a.bin, a.value(), b.value()))
            .collect();
        assert_eq!(
            changed,
            vec![(71, 6.0, 8.0), (73, 0.0, 10.5), (75, 0.0, 20.0), (99, 1000.0, 2000.0), (102, 0.0, 1.0)],
            "the level 1 -> level 2 inventory delta moved"
        );
    }
}
