//! The in-game status bar: the two corner panels, the four meters and the numbers.
//!
//! Every piece of the layout comes from the data, not from this file:
//!
//! - the panels are `statusLeft.bm` / `statusRight.bm`, which carry their own size;
//! - each meter (`stHealth`, `stShield`, `stFrc`, `stBat`, `stFieldLite`) carries its own
//!   offset inside its panel in the `.bm` header (`xPos`/`yPos`) and its own frame count
//!   (`numMips`), one cell per depletion step;
//! - the number offsets `(13,35)` health, `(23,43) shields`, `(13,19) ammo` and the frame
//!   formulas are the ones the engine's HUD uses (`jkHud_Draw`): health and force deplete
//!   with the value, battery fills with it, shields deplete.
//!
//! `SITHBIN_NUMBINS` is 200, and the force pool is 0..400.

use crate::bm;
use crate::gob;
use crate::sft;
use crate::ui::Canvas;

/// The retail status bar is drawn at `hud_scale` 2 by default, so the 59x60 corner panel
/// covers 118x120 of a 640x480 screen.
pub const HUD_SCALE: i32 = 2;

/// One status value, as the HUD shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hud {
    pub health: i32,
    pub max_health: i32,
    /// 0..=200 (`SITHBIN_NUMBINS`).
    pub shields: i32,
    /// 0..=200.
    pub battery: i32,
    /// 0..=400.
    pub force: i32,
    /// Weapon ammo; `-999` is the engine's "no ammo" marker, drawn as `:::`.
    pub ammo: i32,
    /// The field light is a two-frame toggle, not a meter.
    pub field_light: bool,
    /// Super shields / force surge swap in the alternate fonts and the force frame.
    pub super_shields: bool,
    pub force_surge: bool,
    /// The item in hand, by its `misc\items.dat` name (`bryar_pistol`), and how many of
    /// it the player carries. `None` hides the item panel, as an empty inventory does.
    pub item: Option<String>,
    pub item_count: i32,
    /// The items listed down the right-hand scroll strip, in inventory order.
    pub carried: Vec<(String, i32)>,
}

impl Default for Hud {
    fn default() -> Self {
        Self {
            health: 100,
            max_health: 100,
            shields: 100,
            battery: 100,
            force: 200,
            ammo: 50,
            field_light: false,
            super_shields: false,
            force_surge: false,
            // Weapons have no icon in `ui\bm\` (their readout is the ammo counter); the
            // bottom panel is for inventory items like keys and bacta tanks.
            item: None,
            item_count: 0,
            carried: Vec::new(),
        }
    }
}

/// One record of `misc\items.dat`: the inventory's own list of bins.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDef {
    pub name: String,
    pub id: i32,
    pub max: i32,
    /// The flags column, as the file spells it (`weapon`, `force`, …). `jkHudInv_ItemDatLoad` parses
    /// it through a keyword table into the bit word `sithInventory_RegisterType` stores, so keeping
    /// the words is lossless and needs no transcription of that table.
    pub flags: String,
    /// `jkHudInv_ItemDatLoad`'s `cog <path>` pair: the script a bin runs. The engine loads it and
    /// marks the type with bit 0x40, which is what makes a weapon or force power answer `autoselect:`.
    pub cog: Option<String>,
}

impl ItemDef {
    /// The icon bitmap for this item: `ui\bm\ic<first five characters><8|16>.bm`. The
    /// engine builds that name with `sprintf("ic%.5s%d.bm", name, bpp)`, so the file names
    /// are derived from the item names and not the other way round.
    pub fn icon(&self, bits: u32) -> String {
        let head: String = self.name.chars().take(5).collect();
        format!("ui\\bm\\ic{head}{bits}.bm")
    }
}

/// Parse `misc\items.dat`: `name id min max flags [args]` per line, `#` comments.
pub fn items(text: &str) -> Vec<ItemDef> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let rest: Vec<&str> = line.split_whitespace().collect();
        let (Some(name), Some(id), Some(max)) = (rest.first(), rest.get(1), rest.get(3)) else {
            continue;
        };
        let (Ok(id), Ok(max)) = (id.parse(), max.parse()) else { continue };
        // `jkHudInv_ItemDatLoad`: name id min max flags, then `key=value` words, where
        // `cog=<path>` names that bin's script.
        let flags = rest.get(4).copied().unwrap_or("").to_string();
        let cog = rest.get(5..).unwrap_or(&[]).iter().find_map(|word| {
            let (key, value) = word.split_once('=')?;
            key.eq_ignore_ascii_case("cog").then(|| value.to_string())
        });
        out.push(ItemDef { name: name.to_string(), id, max, flags, cog });
    }
    out
}

/// Which corner a bitmap is drawn in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Left,
    Right,
}

/// One bitmap piece of the bar.
struct Piece {
    name: &'static str,
    panel: Panel,
    /// 0..=400 clamp and the axis the frame count follows.
    kind: Kind,
}

#[derive(Clone, Copy)]
enum Kind {
    Panel,
    /// Health-style: `mip = (numMips-1) * (max-cur)/max`.
    Deplete,
    /// Battery: `mip = cur * (numMips-1) / 200`.
    Fill,
    /// A two-frame toggle (field light).
    Toggle,
}

const PIECES: &[Piece] = &[
    Piece { name: "ui\\bm\\statusLeft.bm", panel: Panel::Left, kind: Kind::Panel },
    Piece { name: "ui\\bm\\statusRight.bm", panel: Panel::Right, kind: Kind::Panel },
    Piece { name: "ui\\bm\\stShield.bm", panel: Panel::Left, kind: Kind::Deplete },
    Piece { name: "ui\\bm\\stHealth.bm", panel: Panel::Left, kind: Kind::Deplete },
    Piece { name: "ui\\bm\\stFrc.bm", panel: Panel::Right, kind: Kind::Deplete },
    Piece { name: "ui\\bm\\stBat.bm", panel: Panel::Right, kind: Kind::Fill },
    Piece { name: "ui\\bm\\stFieldLite.bm", panel: Panel::Right, kind: Kind::Toggle },
];

/// Where the numbers sit inside their panel, and which font draws them.
const NUMBERS: &[(&str, Panel, i32, i32)] = &[
    ("ui\\sft\\helthnum.sft", Panel::Left, 13, 35),
    ("ui\\sft\\armornum.sft", Panel::Left, 23, 43),
    ("ui\\sft\\amonums.sft", Panel::Right, 13, 19),
];

/// Draw a script's message line across the top of the canvas, centred.
///
/// `jkStringOutput` puts a line in the message area for a few seconds; the engine draws it with the
/// medium HUD font over a shadow so it reads against any backdrop. Both are drawn here — the shadow
/// first, one pixel down and right.
pub fn draw_message(canvas: &mut Canvas, font: &sft::Font, palette: &[[u8; 3]; 256], text: &str) {
    if text.is_empty() || font.height == 0 {
        return;
    }
    let width: i32 = text.chars().map(|ch| font.glyph(ch).map(|(_, w)| w as i32).unwrap_or(0)).sum();
    let x = (canvas.width as i32 - width * MESSAGE_SCALE) / 2;
    let y = 12;
    let mut shadow = [[0u8; 3]; 256];
    shadow[0] = [0, 0, 0];
    canvas.draw_text_palette(font, x + 1, y + 1, text, &shadow, MESSAGE_SCALE);
    canvas.draw_text_palette(font, x, y, text, palette, MESSAGE_SCALE);
}

/// The message line's draw scale, so it reads at 640x480 without covering the view.
const MESSAGE_SCALE: i32 = 2;

/// Draw the status bar onto a canvas `width` wide: the two panels at the screen's corners
/// with the meters and numbers inside them.
pub fn draw(canvas: &mut Canvas, gobs: &gob::GobSet, palette: &[[u8; 3]; 256], hud: &Hud) {
    // The scroll strip runs down the right edge underneath the status panel, so it is
    // drawn first.
    draw_carried(canvas, gobs, palette, hud);
    let (mut left_y, mut right_x, mut right_y) = (0, 0, 0);
    // Panels first: the meters are positioned relative to them.
    for piece in PIECES.iter().filter(|p| matches!(p.kind, Kind::Panel)) {
        let Some(file) = load(gobs, piece.name) else { continue };
        let Some(cell) = file.cells.first() else { continue };
        let x = match piece.panel {
            Panel::Left => 0,
            Panel::Right => canvas.width as i32 - cell.width as i32 * HUD_SCALE,
        };
        // Both corners are anchored to the **bottom** of the screen, as the engine does:
        // `jkHud_leftBlitY = Video_format.height - HUD_SCALED(panel height)` and the same for the
        // right panel (`jkHud.c:147`, `jkHud.c:151`). They were drawn at `y = 0`, which put the whole
        // status bar along the top.
        let y = canvas.height as i32 - cell.height as i32 * HUD_SCALE;
        match piece.panel {
            Panel::Left => left_y = y,
            Panel::Right => {
                right_x = x;
                right_y = y;
            }
        }
        canvas.blit_indexed([x, y], cell.width, cell.height, &cell.pixels, palette, HUD_SCALE);
    }
    let origin = |panel: Panel| -> (i32, i32) {
        match panel {
            Panel::Left => (0, left_y),
            Panel::Right => (right_x, right_y),
        }
    };

    for piece in PIECES.iter().filter(|p| !matches!(p.kind, Kind::Panel)) {
        let Some(file) = load(gobs, piece.name) else { continue };
        if file.cells.is_empty() {
            continue;
        }
        let mips = file.cells.len() as i32;
        let mip = match piece.kind {
            Kind::Deplete => {
                let (value, max) = match piece.name {
                    "ui\\bm\\stFrc.bm" => (hud.force.clamp(0, 400), 400),
                    "ui\\bm\\stShield.bm" => (hud.shields.clamp(0, 200), 200),
                    _ => (hud.health.clamp(0, hud.max_health.max(1)), hud.max_health.max(1)),
                };
                (mips - 1) * (max - value) / max
            }
            Kind::Fill => hud.battery.clamp(0, 200) * (mips - 1) / 200,
            Kind::Toggle => {
                if hud.field_light {
                    (mips - 1).min(1)
                } else {
                    0
                }
            }
            Kind::Panel => 0,
        }
        .clamp(0, mips - 1) as usize;
        // The force surge replaces the force meter with its own single frame.
        let surge = piece.name == "ui\\bm\\stFrc.bm" && hud.force_surge;
        let cell = if surge { file.cells.first() } else { file.cells.get(mip) };
        let Some(cell) = cell else { continue };
        let (ox, oy) = origin(piece.panel);
        canvas.blit_indexed(
            [ox + file.x_pos * HUD_SCALE, oy + file.y_pos * HUD_SCALE],
            cell.width,
            cell.height,
            &cell.pixels,
            palette,
            HUD_SCALE,
        );
    }

    // Numbers: three digits, as `stdString_snprintf("%03d")` does in the engine.
    let fonts: Vec<Option<sft::Font>> = NUMBERS
        .iter()
        .map(|(path, ..)| gobs.read(path).and_then(|d| sft::parse(&d)))
        .collect();
    let values = [hud.health, hud.shields, hud.ammo];
    for (i, ((_, panel, x, y), font)) in NUMBERS.iter().zip(fonts.iter()).enumerate() {
        let Some(font) = font else { continue };
        let text = if values[i] == -999 {
            ":::".to_string()
        } else {
            format!("{:03}", values[i].clamp(0, 999))
        };
        // The numbers are colormap-indexed, like every other level asset: the digit
        // font's own bytes pick the colour.
        let (ox, oy) = origin(*panel);
        canvas.draw_text_palette(
            font,
            ox + x * HUD_SCALE,
            oy + y * HUD_SCALE,
            &text,
            palette,
            HUD_SCALE,
        );
    }

    draw_crosshair(canvas, palette);
    let defs = gobs
        .read("misc\\items.dat")
        .map(|d| items(&String::from_utf8_lossy(&d)))
        .unwrap_or_default();
    draw_item(canvas, gobs, palette, hud, &defs);
}

/// The aiming reticle: four bars in colormap entry 0x30, from 6 to 18 units out, which is
/// what `jkHud_Draw` fills (it draws rects, not a bitmap).
fn draw_crosshair(canvas: &mut Canvas, palette: &[[u8; 3]; 256]) {
    let [colour] = [palette[0x30]];
    let (cx, cy) = (canvas.width as i32 / 2, canvas.height as i32 / 2);
    let (inner, outer) = (6 * HUD_SCALE, 18 * HUD_SCALE);
    let arm = outer - inner;
    for [x, y, w, h] in [
        [cx - outer, cy - HUD_SCALE / 2, arm, HUD_SCALE],
        [cx + inner, cy - HUD_SCALE / 2, arm, HUD_SCALE],
        [cx - HUD_SCALE / 2, cy - outer, HUD_SCALE, arm],
        [cx - HUD_SCALE / 2, cy + inner, HUD_SCALE, arm],
    ] {
        for row in 0..h {
            for col in 0..w {
                let (px, py) = (x + col, y + row);
                if px < 0 || py < 0 || px >= canvas.width as i32 || py >= canvas.height as i32 {
                    continue;
                }
                let at = ((py as u32 * canvas.width + px as u32) * 4) as usize;
                canvas.rgba[at..at + 4].copy_from_slice(&[colour[0], colour[1], colour[2], 255]);
            }
        }
    }
}

/// The item in hand: the bracketed icon at the bottom of the screen and its count.
fn draw_item(canvas: &mut Canvas, gobs: &gob::GobSet, palette: &[[u8; 3]; 256], hud: &Hud, defs: &[ItemDef]) {
    let Some(item) = hud.item.as_deref() else { return };
    let width = canvas.width as i32;
    let height = canvas.height as i32;
    // The engine's own anchors (`jkHudInv_Setup`): a 24px slot centred on the screen,
    // 36px up from the bottom, with a 184x24 clearing behind it.
    let slot_x = (width - 24 * HUD_SCALE) >> 1;
    let slot_y = height - 36 * HUD_SCALE;
    let Some(brackets) = load(gobs, "ui\\bm\\icbrack8.bm") else { return };
    let Some(cell) = brackets.cells.first() else { return };
    let bracket_x = (width - cell.width as i32 * HUD_SCALE) >> 1;
    let bracket_y = height - ((cell.height as i32 - 24 * HUD_SCALE) >> 1) - 36 * HUD_SCALE;
    // The bracket frame first, then the icon inside it: the frame's interior is opaque
    // black, so drawing the icon after it is what makes the item visible.
    canvas.blit_indexed([bracket_x, bracket_y], cell.width, cell.height, &cell.pixels, palette, HUD_SCALE);
    if let Some(cell) = load(gobs, &icon_for(defs, item))
        .or_else(|| load(gobs, "ui\\bm\\icdefau8.bm"))
        .and_then(|icon| icon.cells.first().cloned())
    {
        let x = slot_x - (cell.width as i32 - 24 * HUD_SCALE) / 2;
        let y = slot_y - (cell.height as i32 - 24 * HUD_SCALE) / 2;
        canvas.blit_indexed([x, y], cell.width, cell.height, &cell.pixels, palette, HUD_SCALE);
    }
    let Some(font) = load_font(gobs, "ui\\sft\\helthnum.sft") else { return };
    // Left of the brackets, right-aligned count then the item name, as `jkHudInv_Draw`
    // lays them out at `field_0 + 14`.
    let text = format!("{}", hud.item_count.max(0));
    let count_x = slot_x + 14 * HUD_SCALE - font.text_width(&text) as i32;
    canvas.draw_text_palette(&font, count_x, slot_y + 2, &text, palette, HUD_SCALE);
}

/// The scroll strip down the right edge: one icon per carried item, 28px apart.
fn draw_carried(canvas: &mut Canvas, gobs: &gob::GobSet, palette: &[[u8; 3]; 256], hud: &Hud) {
    let defs = gobs
        .read("misc\\items.dat")
        .map(|d| items(&String::from_utf8_lossy(&d)))
        .unwrap_or_default();
    let x = canvas.width as i32 - 32 * HUD_SCALE;
    for (n, (item, count)) in hud.carried.iter().enumerate() {
        let y = 8 + n as i32 * 28 * HUD_SCALE;
        if let Some(cell) = load(gobs, &icon_for(&defs, item)).and_then(|f| f.cells.first().cloned()) {
            canvas.blit_indexed([x, y], cell.width, cell.height, &cell.pixels, palette, HUD_SCALE);
        }
        if *count > 1 && let Some(font) = load_font(gobs, "ui\\sft\\helthnum.sft") {
            let text = format!("{count}");
            let tx = x + 24 * HUD_SCALE - font.text_width(&text) as i32;
            canvas.draw_text_palette(&font, tx, y + 2, &text, palette, HUD_SCALE);
        }
    }
}

/// An item's icon: the `items.dat` record's own name when the item is known, the
/// name-based rule otherwise.
fn icon_for(defs: &[ItemDef], name: &str) -> String {
    match defs.iter().find(|d| d.name == name) {
        Some(def) => def.icon(8),
        None => format!("ui\\bm\\ic{}{}.bm", name.chars().take(5).collect::<String>(), 8),
    }
}

fn load_font(gobs: &gob::GobSet, name: &str) -> Option<sft::Font> {
    sft::parse(&gobs.read(name)?)
}

fn load(gobs: &gob::GobSet, name: &str) -> Option<bm::Bm> {
    bm::parse(&gobs.read(name)?)
}

#[cfg(test)]
mod items_tests {
    use super::*;

    /// `misc\items.dat`'s own shape: a `cog=<path>` word after the flags column. The parser
    /// used to stop at `max`, which dropped every weapon script without saying so.
    #[test]
    fn the_item_table_keeps_the_cog_column() {
        let text = "# bins\nfists 1 0 1 0x024 cog=weap_fists.cog\nbacta_tank 22 0 5 0x004\n";
        let defs = items(text);
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].cog.as_deref(), Some("weap_fists.cog"), "the cog was dropped");
        assert_eq!(defs[0].flags, "0x024");
        assert_eq!(defs[0].max, 1);
        assert_eq!(defs[1].cog, None, "an item with no cog grew one");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depletion_and_fill_frames_follow_the_engine() {
        // 6 frames of health at full, half and empty.
        let frames = 6;
        let mip = |value: i32, max: i32| (frames - 1) * (max - value) / max;
        assert_eq!(mip(100, 100), 0);
        assert_eq!(mip(50, 100), 2);
        assert_eq!(mip(0, 100), 5);
        // Battery fills the other way: empty is frame 0.
        let fill = |value: i32| value * (7 - 1) / 200;
        assert_eq!(fill(0), 0);
        assert_eq!(fill(200), 6);
    }

    #[test]
    fn item_icons_follow_the_engines_name_rule() {
        // `sprintf("ui\\bm\\ic%.5s%d.bm", name, bpp)`: the first five characters of the
        // item name, which is why the files are `icbacta8.bm` and `ickeyre8.bm`.
        let bacta = ItemDef { name: "bactatank".into(), id: 34, max: 5, flags: String::new(), cog: None };
        assert_eq!(bacta.icon(8), "ui\\bm\\icbacta8.bm");
        let key = ItemDef { name: "keyred".into(), id: 39, max: 1, flags: String::new(), cog: None };
        assert_eq!(key.icon(8), "ui\\bm\\ickeyre8.bm");
        let jump = ItemDef { name: "f_jump".into(), id: 20, max: 4, flags: String::new(), cog: None };
        assert_eq!(jump.icon(16), "ui\\bm\\icf_jum16.bm");
    }

    #[test]
    fn items_dat_skips_comments_and_keeps_the_maximum() {
        let text = "# header\n\nbactatank  34  0  5  0x020\nenergy 11 0 500 0x080\nbad line\n";
        let items = items(text);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "bactatank");
        assert_eq!(items[0].max, 5);
        assert_eq!(items[1].id, 11);
        assert_eq!(ItemDef::icon(&items[1], 8), "ui\\bm\\icenerg8.bm");
    }

    #[test]
    fn defaults_are_in_range() {
        let h = Hud::default();
        assert!(h.health <= h.max_health);
        assert!((0..=200).contains(&h.shields));
        assert!((0..=400).contains(&h.force));
    }
}
