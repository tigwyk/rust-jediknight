//! The retail menus, read out of `JediKnight.EXE` at run time.
//!
//! The engine has no menu description file: each screen is a static array of 100-byte
//! `jkGuiElement` records in `.data`, and the fonts and bitmaps those records index live in
//! two more `.data` tables of `char*` slots. All three are recovered here by reading the
//! shipped executable, so this build transcribes no menu layout, font name or artwork name:
//!
//! ```text
//! +0x00  int   type: 0 = text button, 1 = picture button, 2 = text, 3 = check box, …9 = END
//! +0x04  int   hoverId: the id the screen's handler acts on
//! +0x08  int   textType: an index into the font table
//! +0x0c  char* `ui\jkstrings.uni` key, or the literal text
//! +0x10  int   selectedTextEntry: a picture button's bitmap, a check box's state, or the
//!              text draw flags
//! +0x14  int   x, +0x18 y, +0x1c width, +0x20 height  (640x480 pixels)
//! +0x24  int   1 = visible, +0x28 1 = hoverable
//! +0x2c  char* hint key (loaded by the engine, never drawn in this build of it)
//! +0x30  draw override, +0x34 click handler
//! ```
//!
//! A record is recognised by its key pointer, its type, its text type and a rect that fits
//! a 640x480 screen; consecutive records 100 bytes apart form one screen. The background
//! a screen draws over is inferred from its label (the engine passes it to `jkGui_InitMenu`
//! in code that is not recovered), and `--bm-bg` overrides it.

/// Distance between two element records in a screen's array.
const STRIDE: u32 = 100;
/// How many elements one array can hold, used to stop a walk that never meets `ELEMENT_END`.
const MAX_ELEMENTS: usize = 64;
/// `ELEMENT_END`: the type that terminates a screen's array.
const END: u32 = 9;

/// Offset of the `jkstrings.uni` key pointer inside a record.
const KEY_AT: usize = 0x0c;
/// `ELEMENT_PICBUTTON`: an element whose artwork is `bitmaps[item.bitmap]`.
pub const PICBUTTON: i32 = 1;
/// `ELEMENT_CUSTOM`: an element the screen's own code paints into its rect.
pub const CUSTOM: i32 = 7;
/// `ELEMENT_CHECKBOX`: an element with a check box to the left of its text.
pub const CHECKBOX: i32 = 3;
/// How many screens a 640x480 UI can hold, used to reject nonsense rects.
const SCREEN: [i32; 2] = [640, 480];

/// One element of a screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Key into `ui\jkstrings.uni`, or literal text.
    pub key: String,
    /// `+0x00`: 0 = text button, 1 = picture button, 2 = text, 3 = check box.
    pub kind: i32,
    /// `+0x08`: an index into the engine's font table; the next font is the hover variant.
    pub text_type: i32,
    /// `+0x04`: the id the screen's handler acts on, `-1` for a label.
    pub id: i32,
    /// `+0x10`: a picture button's bitmap index, a check box's state, or text draw flags.
    pub bitmap: i32,
    /// `+0x14..+0x24`: x, y, width, height.
    pub rect: [i32; 4],
    /// `+0x2c`: the hint key, if the element has one.
    pub hint: Option<String>,
    /// `+0x24`: `bIsVisible`. Every shipped element has it set; the engine clears it for
    /// rows a screen's code hides (a difficulty row in the wrong mode, a force power the
    /// player has not found), and the draw loop skips those.
    pub visible: bool,
    /// `+0x30`: the screen's draw callback. A `type 7` element's renderer pointer lives
    /// here, which is how this build knows what a custom panel paints without naming an
    /// address per screen.
    pub draw: Option<u32>,
    /// `+0x34`: the click callback, for the elements that carry one.
    pub click: Option<u32>,
}

impl Item {
    /// What a `type 7` element's own code paints, from its draw callback.
    pub fn panel(&self) -> Option<Panel> {
        self.draw.and_then(panel)
    }
}

/// The force-power screen's buttons, from `jkGUIForce.c`'s own `jkGuiForce_buttons[25]` table.
///
/// The executable cannot supply these: the retail screen fills `jkGuiForce_pElements` on the
/// heap at run time, so no shipped element array carries a power row (§16.11). What the source
/// does give is the table itself — every field below is copied from it:
///
/// ```text
/// /*3*/ { ELEMENT_PICBUTTON, SITHBIN_F_ABSORB, 0, NULL,
///           0, {-1, -1, -1, -1}, 1, 0, "GUI_HINT_ABSORB", 0, jkGuiForce_ButtonClick, 0, {0}, 0},
/// ```
///
/// `(bin, bitmap, hint)`: the bin whose inventory value turns the row on, the `fo*.bm` icon it
/// draws, and the string the panel shows for it. The rect is `{-1, -1, -1, -1}` in the file
/// because `jkGuiForce_Init` lays the icons out; the grid below is that layout, 6 across.
pub const FORCE_POWERS: [(&str, i32, i32, &str); 17] = [
    ("ABSORB", 28, 0, "GUI_HINT_ABSORB"),
    ("BLINDING", 27, 1, "GUI_HINT_BLINDING"),
    ("DESTRUCTION", 33, 3, "GUI_HINT_DESTRUCTION"),
    ("GRIP", 31, 4, "GUI_HINT_GRIP"),
    ("HEALING", 25, 5, "GUI_HINT_HEALING"),
    ("JUMP", 21, 6, "GUI_HINT_JUMP"),
    ("PERSUASION", 26, 7, "GUI_HINT_PERSUASION"),
    ("PULL", 24, 9, "GUI_HINT_PULL"),
    ("SEEING", 23, 10, "GUI_HINT_SEEING"),
    ("SPEED", 22, 11, "GUI_HINT_SPEED"),
    ("THROW", 30, 12, "GUI_HINT_THROW"),
    ("LIGHTNING", 32, 13, "GUI_HINT_LIGHTNING"),
    ("PROTECTION", 29, 8, "GUI_HINT_PROTECTION"),
    ("DEADLYSIGHT", 34, 2, "GUI_HINT_DEADLYSIGHT"),
    // The three powers the retail single-player build does not offer keep their rows: the table has
    // 17 buttons and the screen hides the ones the player has not found. They have no `items.dat`
    // row, so bin 0 — nothing can ever be found there, and a click on one spends nothing.
    ("PROJECT", 0, 0, "GUI_HINT_PROJECT"),
    ("PUSH", 0, 7, "GUI_HINT_PUSH"),
    ("RAGE", 0, 3, "GUI_HINT_RAGE"),
];

/// Build the force-power screen the way `jkGuiForce_Init` does: one picture button per power,
/// laid out six across, with the note and the OK/RESET/QUIT row underneath.
pub fn force_screen() -> Screen {
    let mut items = Vec::new();
    // /*0*/ and /*1*/: the note lines, at the table's own rects.
    for rect in [[160, 320, 320, 30], [10, 15, 620, 30], [10, 46, 620, 30]] {
        items.push(Item {
            key: String::new(),
            kind: 2,
            text_type: 5,
            id: 0,
            bitmap: 0,
            rect,
            hint: None,
            visible: true,
            draw: None,
            click: None,
        });
    }
    const CELL: i32 = 64;
    const LEFT: i32 = 40;
    const TOP: i32 = 110;
    for (index, (name, bin, bitmap, hint)) in FORCE_POWERS.iter().enumerate() {
        let column = index as i32 % 6;
        let row = index as i32 / 6;
        items.push(Item {
            key: name.to_string(),
            kind: 1,
            text_type: 0,
            id: *bin,
            bitmap: *bitmap,
            rect: [LEFT + column * CELL, TOP + row * CELL, -1, -1],
            hint: Some(hint.to_string()),
            // The engine's rule: `jkGuiForce_Init` clears `bIsVisible` for a power whose bin
            // the player does not carry, so the panel shows what you have found (§16.11).
            visible: false,
            draw: None,
            click: None,
        });
    }
    // /*17*/, /*18*/, /*19*/: OK, RESET and QUIT at the table's rects.
    for (key, id, rect) in [
        ("GUI_OK", 1, [550, 420, 80, 40]),
        ("GUI_RESET", 0, [270, 350, 100, 30]),
        ("GUI_QUIT", -1, [0, 420, 100, 40]),
    ] {
        items.push(Item {
            key: key.to_string(),
            kind: 0,
            // `jkGuiForce_pElements`: the three text buttons are `font 2`, like every other screen's
            // (the tally's Ok reads the same field). They still draw nothing — see gaps.md: font 3
            // drew nothing either, so the font index is not the cause.
            text_type: 2,
            id,
            bitmap: 0,
            rect,
            hint: None,
            visible: true,
            draw: None,
            click: None,
        });
    }
    Screen {
        base: 0x52f168,
        name: "force".to_string(),
                // `jkGUIForce.c`: `jkGui_InitMenu(jkGuiForce_pMenu, jkGui_stdBitmaps[JKGUI_BM_BK_FORCE])`,
        // and `jkGui_aBitmaps[9] = "bkForce.bm"`.
        background: String::from("ui\\bm\\bkforce.bm"),
        background_from_code: true,
        panel: None,
        art: FORCE_POWERS.iter().map(|(_, _, b, _)| format!("fo{b}.bm")).collect(),
        items,
        sound_click: None,
        sound_hover: None,
    }
}

/// Show the power rows the player actually holds, and return how many that is.
///
/// `jkGuiForce_Init` clears `bIsVisible` for a power whose bin the player does not carry, so the
/// panel offers what has been found — and the row carries that bin in its `id`, which is what makes
/// this the engine's rule rather than a guess.
pub fn apply_force_rows(screen: &mut Screen, level: impl Fn(i32) -> f32) -> usize {
    let mut shown = 0;
    for item in screen.items.iter_mut().filter(|item| item.kind == 1) {
        item.visible = level(item.id) > 0.0;
        shown += usize::from(item.visible);
    }
    shown
}

/// What a `type 7` element's own screen code paints into its rect.
///
/// The data only reserves the rectangle; each screen's renderer draws it. The mapping below
/// comes from the screen's handler (see `nav` and §18.3b) and the renderer named for it in
/// the symbol list: `jkGuiMap_DrawMapScreen` 0x4156a0 for the map,
/// `jkGuiObjectives_CustomRender` 0x417b00 for the objectives page,
/// `jkGuiSingleTally_ForceStarsRender` 0x418000 for the tally's star row,
/// `jkGuiTitle_LoadBarDraw` for the loading bar. The two character-editor panels are the
/// model viewport and the power grid; the title art panel is the level card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Map,
    Objectives,
    ForceStars,
    LoadBar,
    TitleText,
    Model,
    PowerGrid,
}

impl Panel {
    /// A short name for the `--ui-tables` listing and the audit.
    pub fn name(self) -> &'static str {
        match self {
            Panel::Map => "map",
            Panel::Objectives => "objectives",
            Panel::ForceStars => "stars",
            Panel::LoadBar => "loadbar",
            Panel::TitleText => "title-text",
            Panel::Model => "model",
            Panel::PowerGrid => "power-grid",
        }
    }
}

/// The engine's custom-panel renderers, by the callback address the elements carry. The
/// addresses are the ones each element's `+0x30` field holds in this install; the names come
/// from the symbol list, which is where the `_Show`/`_Startup` names came from too.
pub const PANELS: &[(u32, Panel)] = &[
    (0x4156a0, Panel::Map),          // jkGuiMap_DrawMapScreen
    (0x417b00, Panel::Objectives),   // jkGuiObjectives_CustomRender
    (0x418000, Panel::ForceStars),   // jkGuiSingleTally_ForceStarsRender
    (0x418b90, Panel::LoadBar),      // jkGuiTitle_LoadBarDraw
    (0x418aa0, Panel::TitleText),    // jkGuiTitle_UnkDraw: the loading text block
    (0x419db0, Panel::Model),        // the character viewport
    (0x41a0a0, Panel::PowerGrid),    // the force-power grid
];

/// The panel a draw callback paints.
pub fn panel(draw: u32) -> Option<Panel> {
    PANELS.iter().find(|(address, _)| *address == draw).map(|(_, panel)| *panel)
}

/// One screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    /// Where the element array starts in the executable's image.
    pub base: u32,
    /// A short name: the screen title's key without `GUI_`, or the first row's.
    pub name: String,
    /// `ui\bm\<background>`: the art the screen draws over.
    pub background: String,
    /// Whether [background] came from the screen's own `jkGui_InitMenu` call rather than
    /// from the label heuristic (see [code_backgrounds]).
    pub background_from_code: bool,
    /// The panel a dialog draws over its background, at that bitmap's own `xPos`/`yPos`.
    pub panel: Option<String>,
    /// The `.bm` names this screen's picture buttons index, in order: the shared table for
    /// every screen but the force-power one, which indexes the `fo*.bm` icons.
    pub art: Vec<String>,
    pub items: Vec<Item>,
    /// The menu's own click and hover wav names, from `jkGuiMenu.soundClick` (0x30) and
    /// `soundHover` (0x2C). `jkGuiRend_PlayClickSound` is nothing but `PlayWav(menu->soundClick)`.
    pub sound_click: Option<String>,
    pub sound_hover: Option<String>,
}

impl Screen {
    /// The elements a cursor moves over: the retail screens step through their full-width
    /// rows — buttons drawn with the `large0` font — and treat the rest as click targets.
    ///
    /// Two things this has to get right, both of which were wrong:
    ///
    /// * **A label is not a row.** Several screens draw their *title* with `large0` (Choose Player,
    ///   Cutscenes, New Player, Edit Character), and counting any `large0` element made the cursor
    ///   walk the title alone — so Up/Down/Enter could not reach those screens' Ok or Cancel.
    /// * **A screen made only of ordinary buttons has rows too.** New Game / Load / Cancel on
    ///   `bkSingle.bm` are drawn with `small0`, which left the single-player screen mouse-only.
    ///
    /// Everything Enter can *activate* — a text button, a picture button (the force panel's powers),
    /// a check box — is a row. Text labels, the `type 7` custom panels, list boxes and edit fields are
    /// not: Enter does not run them, and a cursor that stops on a dead element is what made Choose
    /// Player look like it had no Cancel.
    pub fn rows(&self) -> Vec<usize> {
        let navigable = |i: &Item| matches!(i.kind, 0 | 1 | 3);
        let rows: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| navigable(i) && i.text_type == 5)
            .map(|(n, _)| n)
            .collect();
        if !rows.is_empty() {
            return rows;
        }
        self.items
            .iter()
            .enumerate()
            .filter(|(_, i)| navigable(i))
            .map(|(n, _)| n)
            .collect()
    }

    /// The rows the engine would actually let the cursor reach, given which force powers the
    /// player has found.
    ///
    /// Elements carry a `bIsVisible` byte (`+0x24`, parsed as [Item::visible]) and every
    /// shipped element has it set: the engine clears it at run time, per screen. The force
    /// screen's rule is in `jkGUIForce.c` —
    ///
    /// ```c
    /// pElements[i].bIsVisible = !!(playerInfos[player].aItems[id].state & SITHINVENTORY_ITEM_FOUND);
    /// ```
    ///
    /// — so a power this build has no amount for is hidden rather than drawn at rank 0. The
    /// `power-grid` *panel* paints its own cells, which this does not reach.
    pub fn visible_rows(&self, found: &dyn Fn(i32) -> bool) -> Vec<usize> {
        // A row is a force power when its id is one: single-player powers are the bins from
        // `SITHBIN_JEDI_RANK` (20) to `SITHBIN_F_DEADLYSIGHT` (34) in `types_enums.h`, and a
        // screen whose rows mostly name those is the force screen.
        const POWER_FIRST: i32 = 20;
        const POWER_LAST: i32 = 34;
        // A screen with four or more power rows is the force screen.
        let powers = self
            .rows()
            .into_iter()
            .filter(|n| (POWER_FIRST..=POWER_LAST).contains(&self.items[*n].id))
            .count();
        let gated = powers >= 4;
        self.rows()
            .into_iter()
            .filter(|n| {
                let item = &self.items[*n];
                // Elsewhere `bIsVisible` is set on every shipped element and stays set.
                !gated
                    || !(POWER_FIRST..=POWER_LAST).contains(&item.id)
                    || found(item.id)
            })
            .collect()
    }
}

/// Every screen found in the executable, in address order.
#[derive(Debug, Default)]
pub struct Menus {
    pub screens: Vec<Screen>,
    /// `jkGui_aFonts`: the `.sft` names, in the order `text_type` indexes them.
    pub fonts: Vec<String>,
    /// `jkGui_aBitmaps`: the `.bm` names, in the order a picture button's `bitmap`
    /// indexes them. The first 13 are the menu backgrounds.
    pub bitmaps: Vec<String>,
}

impl Menus {
    /// The font a text type names, as a path inside the archives. An element's `text_type`
    /// is an index into the engine's font table, and the engine draws the hovered element
    /// with the next font and the pressed one with the one after that.
    pub fn font(&self, text_type: i32) -> Option<String> {
        let name = self.fonts.get(usize::try_from(text_type).ok()?)?;
        Some(format!("ui\\sft\\{name}"))
    }

    /// The screen that carries a given item key: how a screen with no unique name of its
    /// own is identified (the pause menu is the one with `GUI_RETURN_TO_GAME`).
    pub fn find_by_item(&self, key: &str) -> Option<&Screen> {
        self.screens
            .iter()
            .find(|s| s.items.iter().any(|i| i.key == key))
    }

    /// Find a screen by name (`main`, `esc`, …) or by the hex address of its array.
    pub fn find(&self, name: &str) -> Option<&Screen> {
        let needle = name.trim();
        if let Some(screen) = needle
            .strip_prefix("0x")
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .and_then(|hex| self.screens.iter().find(|s| s.base == hex))
        {
            return Some(screen);
        }
        let lower = needle.to_ascii_lowercase();
        self.screens
            .iter()
            .find(|s| s.name.to_ascii_lowercase() == lower)
            .or_else(|| {
                // An alias by artwork: `esc` finds `bkesc.bm`, `setup` finds `bksetup.bm`.
                self.screens
                    .iter()
                    .find(|s| s.background.to_ascii_lowercase().contains(&format!("bk{lower}")))
            })
            .or_else(|| {
                self.screens
                    .iter()
                    .find(|s| s.name.to_ascii_lowercase().contains(&lower))
            })
    }
}

/// The `.data` section of a PE image, as (file offset, virtual address, size).
struct Section {
    offset: usize,
    address: u32,
    size: usize,
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let s = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// The parts of a PE image this module reads: image base, data sections, and the code.
type Image<'a> = (u32, Vec<Section>, Option<(u32, &'a [u8])>);

/// Read the section table: enough PE to find `.data` and `.text`, no more.
fn sections(bytes: &[u8]) -> Option<Image<'_>> {
    let pe = u32_at(bytes, 0x3c)? as usize;
    if bytes.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let count = u16::from_le_bytes(bytes.get(pe + 6..pe + 8)?.try_into().ok()?) as usize;
    let optional = pe + 24;
    let image_base = u32_at(bytes, optional + 28)?;
    let table = optional + u16::from_le_bytes(bytes.get(pe + 20..pe + 22)?.try_into().ok()?) as usize;
    let mut out = Vec::with_capacity(count);
    let mut code = None;
    for i in 0..count {
        let at = table + i * 40;
        let name = bytes.get(at..at + 8)?;
        let size = u32_at(bytes, at + 8)? as usize;
        let address = u32_at(bytes, at + 12)?;
        let raw_size = u32_at(bytes, at + 16)? as usize;
        let raw = u32_at(bytes, at + 20)? as usize;
        // The data-bearing sections hold the tables; `.text` holds the code that selects
        // them, and is returned on its own because nothing else may scan it for elements.
        if name.starts_with(b".text") {
            code = Some((image_base + address, bytes.get(raw..(raw + raw_size).min(bytes.len()))?));
        } else if name.starts_with(b".data") || name.starts_with(b".rdata") {
            out.push(Section { offset: raw, address: image_base + address, size: size.max(raw_size) });
        }
    }
    Some((image_base, out, code))
}

/// A NUL-terminated printable ASCII string at an address in one of the sections.
fn ascii_at<'a>(bytes: &'a [u8], sections: &[Section], image_base: u32, va: u32) -> Option<&'a str> {
    if va < image_base {
        return None;
    }
    for section in sections {
        let start = section.address;
        if va < start || va >= start + section.size as u32 {
            continue;
        }
        let at = section.offset + (va - start) as usize;
        let tail = bytes.get(at..)?;
        let len = tail.iter().take(64).position(|b| *b == 0)?;
        let text = std::str::from_utf8(&tail[..len]).ok()?;
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
            return None;
        }
        return Some(text);
    }
    None
}

/// An item key: a string that names a `ui\jkstrings.uni` entry.
fn gui_key_at<'a>(bytes: &'a [u8], sections: &[Section], image_base: u32, va: u32) -> Option<&'a str> {
    ascii_at(bytes, sections, image_base, va).filter(|text| {
        text.starts_with("GUI_") && text.bytes().all(|b| b.is_ascii_graphic() || b == b'_')
    })
}

/// The longest runs of adjacent pointers to `<name><ext>` strings: the engine's own name
/// tables (`jkGui_aFonts` lists every `.sft`, `jkGui_aBitmaps` every `.bm` it can load).
/// Taking whole runs picks a table over the short ones that sit beside it, and reading them
/// means this build never transcribes a font or bitmap name. Each run is returned with the
/// address it starts at, which is what the code that fills the run-time handle arrays from
/// it refers to (`see` [handle_array_base]).
fn name_tables(exe: &[u8], sections: &[Section], image_base: u32, ext: &str) -> Vec<(u32, Vec<String>)> {
    let mut tables: Vec<(u32, Vec<String>)> = Vec::new();
    for section in sections {
        let end = (section.offset + section.size).min(exe.len());
        let Some(bytes) = exe.get(section.offset..end) else { continue };
        let mut run: Vec<String> = Vec::new();
        let mut start = 0usize;
        let mut previous = usize::MAX;
        for i in (0..bytes.len().saturating_sub(3)).step_by(4) {
            let name = u32_at(bytes, i)
                .and_then(|va| ascii_at(exe, sections, image_base, va))
                .filter(|text| text.to_ascii_lowercase().ends_with(ext));
            match name {
                Some(text) => {
                    if previous != i.wrapping_sub(4) {
                        run.clear();
                        start = i;
                    }
                    run.push(text.to_string());
                    previous = i;
                }
                None => {
                    if run.len() >= 2 {
                        tables.push((section.address + start as u32, std::mem::take(&mut run)));
                    }
                    run.clear();
                }
            }
        }
        if run.len() >= 2 {
            tables.push((section.address + start as u32, run));
        }
    }
    tables.sort_by_key(|(_, table)| std::cmp::Reverse(table.len()));
    tables
}

/// The longest run, for the single-table cases (the fonts).
fn name_table(exe: &[u8], sections: &[Section], image_base: u32, ext: &str) -> Vec<String> {
    name_tables(exe, sections, image_base, ext)
        .into_iter()
        .next()
        .map(|(_, names)| names)
        .unwrap_or_default()
}

/// The address of the run-time array the engine fills from a name table.
///
/// `jkGui_Startup` loads each name in a loop and stores the loaded handle into that array,
/// so the code reads `mov reg, [esi + <table>] … mov [esi + <array>], eax`. Finding the store
/// a few instructions after the table load gives the array's address without this build
/// knowing it in advance.
fn handle_array_base(code: &[u8], table: u32) -> Option<u32> {
    let needle = table.to_le_bytes();
    for i in 0..code.len().saturating_sub(4) {
        if code[i..i + 4] != needle {
            continue;
        }
        // `89 /r` with mod=10 (disp32) and rm=110 (esi): `mov [esi + disp32], reg`.
        for j in i..(i + 0x40).min(code.len()) {
            if code[j] == 0x89
                && code[j + 1] & 0xc7 == 0x86
                && let Some(base) = u32_at(code, j + 2)
            {
                return Some(base);
            }
        }
    }
    None
}

/// The handle index a screen's `_Startup` passes to `jkGui_InitMenu`.
///
/// The call is `push jkGui_stdBitmaps[i]; push &menu; call`, and the `push` argument is a
/// load from the handle array, so the index is the displacement of a load within the few
/// instructions that set up the call to `menu`'s initializer.
fn init_menu_index(code: &[u8], base: u32, count: usize, menu: u32) -> Option<usize> {
    let tag = menu.to_le_bytes();
    let last = base + 4 * count as u32;
    for i in 0..code.len().saturating_sub(5) {
        // `68 <menu>` (push imm32) followed by `e8` (call).
        if code[i] != 0x68 || code[i + 5] != 0xe8 || &code[i + 1..i + 5] != tag.as_slice() {
            continue;
        }
        // The nearest load before the push, not the first in the window, and a short
        // window: one `_Startup` initializes several menus 16 bytes apart, and a call that
        // passes a literal (the dialogs pass 0) must not inherit its neighbour's bitmap.
        for k in (i.saturating_sub(0x10)..i).rev() {
            if let Some(va) = u32_at(code, k)
                && va >= base
                && va < last
                && (va - base).is_multiple_of(4)
            {
                return Some(((va - base) / 4) as usize);
            }
        }
    }
    None
}

/// Each screen's own background, read out of the code.
///
/// Every screen's `_Startup` fills a menu struct — which holds the element array at `+0x00`
/// — and passes one of the loaded bitmaps to `jkGui_InitMenu`. So a screen's art is the
/// `jkGui_stdBitmaps[i]` argument of the initializer whose struct points at that array.
fn code_backgrounds(
    exe: &[u8],
    sections: &[Section],
    code: &[u8],
    table: u32,
    names: &[String],
    arrays: &[u32],
) -> Vec<(u32, String)> {
    let Some(base) = handle_array_base(code, table) else { return Vec::new() };
    let mut out = Vec::new();
    for section in sections {
        let end = (section.offset + section.size).min(exe.len());
        let Some(bytes) = exe.get(section.offset..end) else { continue };
        for i in (0..bytes.len().saturating_sub(3)).step_by(4) {
            let Some(word) = u32_at(bytes, i) else { continue };
            if !arrays.contains(&word) {
                continue;
            }
            let menu = section.address + i as u32;
            if let Some(index) = init_menu_index(code, base, names.len(), menu)
                && let Some(name) = names.get(index)
            {
                out.push((word, format!("ui\\bm\\{name}")));
            }
        }
    }
    out
}

/// The art a screen draws over, inferred from its label. The engine's own mapping lives in
/// the code that calls `jkGui_InitMenu`, which is not recovered; `--bm-bg` overrides the
/// background. A dialog is a **panel over the menu art**: the engine copies the current
/// menu buffer and then `bkdialog.bm` at that bitmap's own `xPos`/`yPos`.
fn background_for(name: &str, items: &[Item]) -> (&'static str, Option<&'static str>) {
    let mut labels = String::from(name);
    for item in items {
        labels.push(' ');
        labels.push_str(&item.key);
    }
    let has = |needle: &str| labels.contains(needle);
    // The main menu's own label is its first row, "Single Player".
    let background = if name == "singleplayer" {
        "ui\\bm\\bkmain.bm"
    } else if name.starts_with("force") {
        "ui\\bm\\bkforce.bm"
    } else if name.starts_with("loading") {
        "ui\\bm\\bkloading.bm"
    } else if has("EDIT_CHARACTER") || has("NEW_CHARACTER") {
        "ui\\bm\\bkbuildmulti.bm"
    } else if has("LOAD_CHARACTER") {
        "ui\\bm\\bkbuildload.bm"
    } else if name == "multiplayer" || has("GAMENAME") || has("JOINGAME") || has("STARTGAME") {
        "ui\\bm\\bkmulti.bm"
    } else if has("TEAM_SCORE") || has("TALLY") || has("RESET") {
        "ui\\bm\\bktally.bm"
    } else if has("JEDIPOWERS") || has("OBJECTIVES") || has("RETURN_TO_GAME") || has("RESTART") {
        "ui\\bm\\bkesc.bm"
    } else if has("GENERAL") || has("GAMEPLAY") || has("DISPLAY") || has("CONTROLS") || has("ADVANCED") {
        "ui\\bm\\bksetup.bm"
    } else if has("MULTIPLAYER") {
        "ui\\bm\\bkmulti.bm"
    } else if has("SINGLEPLAYER") || has("CHOOSEPLAYER") || has("NEWPLAYER") || has("DIFFICULTY") || has("VIEWCUTSCENES") {
        "ui\\bm\\bksingle.bm"
    } else {
        // Everything left is a dialog: ok/cancel, loading, a name prompt, light-or-dark.
        return ("ui\\bm\\bkmain.bm", Some("ui\\bm\\bkdialog.bm"));
    };
    (background, None)
}

/// One element at `base`, if the record there could be drawn on a 640x480 screen.
///
/// The engine's arrays are terminated by a record whose type is `ELEMENT_END`, which this
/// rejects like any other non-element, so [load] can walk an array by stepping 100 bytes
/// until it sees `None`.
fn element_at(exe: &[u8], sections: &[Section], image_base: u32, base: u32) -> Option<Item> {
    let section = sections
        .iter()
        .find(|s| base >= s.address && base < s.address + s.size as u32)?;
    let at = section.offset + (base - section.address) as usize;
    let kind = u32_at(exe, at)? as i32;
    let id = u32_at(exe, at + 0x04)? as i32;
    let text_type = u32_at(exe, at + 0x08)? as i32;
    let bitmap = u32_at(exe, at + 0x10)? as i32;
    let rect = [
        u32_at(exe, at + 0x14)? as i32,
        u32_at(exe, at + 0x18)? as i32,
        u32_at(exe, at + 0x1c)? as i32,
        u32_at(exe, at + 0x20)? as i32,
    ];
    // A real element has a drawable rect somewhere on the screen, or the placeholder
    // rect the engine fills in from the bitmap: a picture button whose `x/y/width/height`
    // are all -1 (the force icons) takes its position and size from `bitmaps[+0x10]`
    // (`jkGuiRend_PicButtonEventHandler`). The END record has a zero rect, and so does any
    // padding between arrays, so neither passes.
    // Widened to `i64` before adding: these four are arbitrary bytes read out of the executable, so a
    // candidate record can hold values near `i32::MAX` and `rect[0] + rect[2]` overflowed — a panic in
    // debug builds, silently wrapping in release, and the three menu tests that walk the raw table
    // failed on it rather than on a menu.
    let placeholder = rect.iter().all(|v| *v < 0);
    let fits = rect[2] > 0
        && rect[3] > 0
        && rect[0] >= 0
        && rect[1] >= 0
        && i64::from(rect[0]) + i64::from(rect[2]) <= i64::from(SCREEN[0])
        && i64::from(rect[1]) + i64::from(rect[3]) <= i64::from(SCREEN[1]);
    if !(0..=8).contains(&kind) || !(0..=15).contains(&text_type) || !(fits || placeholder) {
        return None;
    }
    // Most elements name a string; a list box, a slider and a picture button have none.
    let key = u32_at(exe, at + KEY_AT)
        .filter(|p| *p >= image_base)
        .and_then(|p| gui_key_at(exe, sections, image_base, p))
        .unwrap_or_default()
        .to_string();
    let hint = u32_at(exe, at + 0x2c)
        .filter(|h| *h >= image_base)
        .and_then(|h| ascii_at(exe, sections, image_base, h).map(|s| s.to_string()));
    // `bIsVisible`; every shipped element sets it, and the engine's draw loop skips the
    // ones a screen clears.
    let visible = u32_at(exe, at + 0x24).is_some_and(|v| v != 0);
    // The screen's own callbacks: a `type 7` element's draw function is the only thing that
    // says what its rectangle paints.
    let callback = |offset: usize| u32_at(exe, at + offset).filter(|v| *v >= image_base);
    Some(Item {
        key,
        kind,
        text_type,
        id,
        bitmap,
        rect,
        hint,
        visible,
        draw: callback(0x30),
        click: callback(0x34),
    })
}

/// Whether `base` is the record the type `ELEMENT_END` (9) occupies.
fn ends_at(exe: &[u8], sections: &[Section], base: u32) -> bool {
    sections
        .iter()
        .find(|s| base >= s.address && base < s.address + s.size as u32)
        .and_then(|s| u32_at(exe, s.offset + (base - s.address) as usize))
        .is_some_and(|kind| kind == END)
}

/// Scan an executable for menu screens: maximal arrays of elements ending at `ELEMENT_END`,
/// plus a run of placeholder picture buttons, which is the one screen stored without an end
/// marker (see `load_force`).
pub fn load(exe: &[u8]) -> Option<Menus> {
    let (image_base, sections, code) = sections(exe)?;
    if sections.is_empty() {
        return None;
    }
    let tables = name_tables(exe, &sections, image_base, ".bm");
    let bitmaps = tables.first().map(|(_, names)| names.clone()).unwrap_or_default();
    // The force icons are their own table: 17 `fo*.bm`/`forceMeter*.bm` names.
    let icons = tables
        .iter()
        .find(|(_, table)| table.iter().any(|name| name.len() > 2 && name[..2].eq_ignore_ascii_case("fo")))
        .map(|(_, names)| names.clone())
        .unwrap_or_else(|| bitmaps.clone());
    let mut screens = Vec::new();
    for section in &sections {
        let end = (section.offset + section.size).min(exe.len());
        let Some(bytes) = exe.get(section.offset..end) else { continue };
        for i in 0..bytes.len() / 4 {
            let base = section.address + i as u32 * 4;
            // An array starts where an element is not preceded by another element; the
            // record 100 bytes back is the previous element of the same array otherwise.
            if element_at(exe, &sections, image_base, base).is_none()
                || element_at(exe, &sections, image_base, base.wrapping_sub(STRIDE)).is_some()
            {
                continue;
            }
            let mut items: Vec<Item> = Vec::new();
            let mut icon_run = 0;
            while items.len() < MAX_ELEMENTS {
                let Some(item) = element_at(exe, &sections, image_base, base + items.len() as u32 * STRIDE) else {
                    break;
                };
                // The force screen's array has no end marker: it stops at the next array's
                // first element, which is where its run of placeholder icons ends.
                if item.rect.iter().all(|v| *v < 0) {
                    icon_run += 1;
                } else if icon_run >= 4 {
                    break;
                }
                items.push(item);
            }
            if items.len() < 2 {
                continue;
            }
            // A screen ends at the end marker, or — for the force screen, whose array has
            // none — at its run of picture buttons that take their rects from their bitmaps.
            let after = base + items.len() as u32 * STRIDE;
            let placeholders = items.iter().filter(|i| i.rect.iter().all(|v| *v < 0)).count();
            let ends = ends_at(exe, &sections, after);
            if !ends && placeholders < 4 {
                continue;
            }
            let title = items
                .iter()
                .find(|i| i.text_type == 6 && !i.key.is_empty())
                .or_else(|| items.iter().find(|i| i.text_type == 5 && !i.key.is_empty()))
                .or_else(|| items.iter().find(|i| !i.key.is_empty()));
            let name = title
                .map(|i| i.key.strip_prefix("GUI_").unwrap_or(&i.key).to_ascii_lowercase())
                .unwrap_or_else(|| format!("{base:#08x}"));

            let (background, panel) = background_for(&name, &items);
            screens.push(Screen {
                base,
                name,
                background: background.to_string(),
                background_from_code: false,
                panel: panel.map(str::to_string),
                art: Vec::new(),
                items,
                sound_click: None,
                sound_hover: None,
            });
        }
    }
    if screens.is_empty() {
        return None;
    }
    // The menu's own wav names. `base` is the element array, which is `jkGuiMenu.paElements` — the
    // struct's first field — so the struct is whatever `.data` word equals `base`. `soundHover` is
    // at +0x2C and `soundClick` at +0x30 (`types.h`), and `jkGuiRend_PlayClickSound` is nothing but
    // `PlayWav(menu->soundClick)`.
    {
        let mut words: Vec<(u32, usize)> = Vec::new();
        for section in &sections {
            let end = (section.offset + section.size).min(exe.len());
            let Some(bytes) = exe.get(section.offset..end) else { continue };
            for i in 0..bytes.len() / 4 {
                words.push((section.address + i as u32 * 4, section.offset + i * 4));
            }
        }
        for screen in &mut screens {
            let mut candidates: Vec<u32> = words
                .iter()
                .filter(|(_, at)| u32_at(exe, *at) == Some(screen.base))
                .map(|(address, _)| *address)
                .collect();
            candidates.sort_unstable();
            for struct_at in candidates {
                let read = |offset: u32| -> Option<String> {
                    let slot = words
                        .iter()
                        .find(|(address, _)| *address == struct_at + offset)
                        .map(|(_, at)| *at)?;
                    let pointer = u32_at(exe, slot)?;
                    if pointer == 0 {
                        return None;
                    }
                    let name = ascii_at(exe, &sections, image_base, pointer)?.to_string();
                    name.to_ascii_lowercase().ends_with(".wav").then_some(name)
                };
                let hover = read(0x2C);
                let click = read(0x30);
                if hover.is_some() || click.is_some() {
                    screen.sound_hover = hover;
                    screen.sound_click = click;
                    break;
                }
            }
        }
    }
    // Replace the label guess with each screen's own art where the code names it.
    if let (Some((table, names)), Some((_, code))) = (tables.first(), code) {
        let arrays: Vec<u32> = screens.iter().map(|s| s.base).collect();
        for (base, background) in code_backgrounds(exe, &sections, code, *table, names, &arrays) {
            if let Some(screen) = screens.iter_mut().find(|s| s.base == base) {
                screen.background = background;
                screen.background_from_code = true;
            }
        }
    }
    // The picture buttons index the shared bitmap table, except on the force screen: its
    // icons are the separate `fo*.bm` table (0x52f118), and it is the screen that draws
    // `bkForce.bm`.
    for screen in &mut screens {
        let force = screen.background.to_ascii_lowercase().ends_with("bkforce.bm");
        screen.art = if force { icons.clone() } else { bitmaps.clone() };
    }
    // A screen with no string of its own takes its name from the art it draws: the map
    // screen, which lists no keys, is the one that draws the field-log bitmap.
    for screen in &mut screens {
        if screen.name.starts_with("0x") && screen.background_from_code {
            let stem = screen.background.trim_start_matches("ui\\bm\\").trim_end_matches(".bm");
            screen.name = stem.trim_start_matches("bk").to_ascii_lowercase();
        }
    }
    Some(Menus {
        screens,
        fonts: name_table(exe, &sections, image_base, ".sft"),
        bitmaps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `bIsVisible` rule, on a screen shaped like the force one: eight power rows whose
    /// ids are the engine's bins, plus a label with no id.
    #[test]
    fn force_rows_hide_until_they_are_found() {
        // The force panel's powers are `type 1` picture buttons (`FORCE_POWERS`), not text.
        let item = |id: i32| Item {
            key: String::new(),
            kind: 1,
            text_type: 5,
            id,
            bitmap: 0,
            rect: [0; 4],
            hint: None,
            visible: true,
            draw: None,
            click: None,
        };
        let mut items: Vec<Item> = (21..=28).map(item).collect();
        // A label: `type 2` is text, so it is not a row whatever font it is drawn with.
        let mut label = item(-1);
        label.kind = 2;
        label.text_type = 1;
        items.push(label);
        let screen = Screen {
            base: 0,
            name: "synthetic force".into(),
            background: String::new(),
            background_from_code: false,
            panel: None,
            art: Vec::new(),
            items,
            sound_click: None,
            sound_hover: None,
        };
        // Nothing found: the eight power rows are hidden, the label is not a row.
        assert_eq!(screen.visible_rows(&|_| false).len(), 0);
        // Everything found: all eight show.
        assert_eq!(screen.visible_rows(&|_| true).len(), 8);
        // One found: exactly that one.
        let one = screen.visible_rows(&|bin| bin == 23);
        assert_eq!(one.len(), 1);
        assert_eq!(screen.items[one[0]].id, 23);
    }

    use super::*;

    /// A minimal PE: one 0x240-byte `.data` section at 0x2000 holding an array of two
    /// elements plus the `ELEMENT_END` marker, their `GUI_` keys and the two name tables.
    fn build() -> Vec<u8> {
        let mut bytes = vec![0u8; 0x500];
        bytes[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
        bytes[0x46..0x48].copy_from_slice(&1u16.to_le_bytes());
        bytes[0x54..0x56].copy_from_slice(&0xe0u16.to_le_bytes());
        // ImageBase is at OptionalHeader + 28 (the header starts at 0x58).
        bytes[0x74..0x78].copy_from_slice(&0x400000u32.to_le_bytes());
        let table = 0x58 + 0xe0;
        bytes[table..table + 8].copy_from_slice(b".data\0\0\0");
        bytes[table + 8..table + 12].copy_from_slice(&0x240u32.to_le_bytes());
        bytes[table + 12..table + 16].copy_from_slice(&0x2000u32.to_le_bytes());
        bytes[table + 16..table + 20].copy_from_slice(&0x240u32.to_le_bytes());
        bytes[table + 20..table + 24].copy_from_slice(&0x200u32.to_le_bytes());

        let data = 0x200; // file offset of `.data`, whose VA is 0x400000 + 0x2000
        let va = |at: usize| 0x400000 + 0x2000 + (at - data) as u32;
        // Past all three 100-byte records: they would otherwise overwrite the keys.
        let keys = data + 0x140;
        bytes[keys..keys + 12].copy_from_slice(b"GUI_ACTIONS\0");
        bytes[keys + 16..keys + 25].copy_from_slice(b"GUI_OK\0\0\0");
        // A `jkGuiElement`: type, hoverId, textType, str, otherData, rect, visible…
        let mut record = |at: usize, id: i32, text_type: i32, key_va: u32, rect: [i32; 4]| {
            bytes[at..at + 4].copy_from_slice(&0i32.to_le_bytes());
            bytes[at + 0x04..at + 0x08].copy_from_slice(&id.to_le_bytes());
            bytes[at + 0x08..at + 0x0c].copy_from_slice(&text_type.to_le_bytes());
            bytes[at + 0x0c..at + 0x10].copy_from_slice(&key_va.to_le_bytes());
            bytes[at + 0x10..at + 0x14].copy_from_slice(&3i32.to_le_bytes());
            for (n, v) in rect.iter().enumerate() {
                bytes[at + 0x14 + n * 4..at + 0x18 + n * 4].copy_from_slice(&v.to_le_bytes());
            }
            bytes[at + 0x24..at + 0x28].copy_from_slice(&1i32.to_le_bytes());
        };
        record(data, 1, 5, va(keys), [0, 100, 400, 40]);
        record(data + 100, 2, 5, va(keys + 16), [0, 150, 400, 40]);
        // The end marker: a record whose type is 9 and whose rect is empty.
        bytes[data + 200..data + 204].copy_from_slice(&9i32.to_le_bytes());
        // The two name tables: three adjacent `.sft` pointers, one `.bm` pointer.
        let (sft, bm) = (data + 0x1a0, data + 0x1c4);
        bytes[sft..sft + 11].copy_from_slice(b"small0.sft\0");
        bytes[sft + 12..sft + 21].copy_from_slice(b"med0.sft\0");
        bytes[sft + 22..sft + 33].copy_from_slice(b"large0.sft\0");
        bytes[bm..bm + 10].copy_from_slice(b"bkMain.bm\0");
        bytes[bm + 12..bm + 25].copy_from_slice(b"arrowLeft.bm\0");
        let mut pointer = |at: usize, to: usize| bytes[at..at + 4].copy_from_slice(&va(to).to_le_bytes());
        pointer(data + 0x180, sft);
        pointer(data + 0x184, sft + 12);
        pointer(data + 0x188, sft + 22);
        pointer(data + 0x190, bm);
        pointer(data + 0x194, bm + 12);
        bytes
    }

    #[test]
    fn finds_a_two_item_screen() {
        let menus = load(&build()).expect("a screen");
        assert_eq!(menus.screens.len(), 1);
        let screen = &menus.screens[0];
        assert_eq!(screen.items.len(), 2);
        assert_eq!(screen.items[0].key, "GUI_ACTIONS");
        assert_eq!(screen.items[1].rect, [0, 150, 400, 40]);
        assert_eq!(screen.rows(), vec![0, 1]);
        assert_eq!(screen.name, "actions");
        assert!(menus.find("0x402000").is_some());
        assert!(menus.find("ACTIONS").is_some());
    }

    #[test]
    fn reads_the_engine_name_tables() {
        let menus = load(&build()).expect("a screen");
        // The adjacent pointers are one table, the lone `.bm` pointer the other.
        assert_eq!(menus.fonts, ["small0.sft", "med0.sft", "large0.sft"]);
        assert_eq!(menus.font(2).as_deref(), Some("ui\\sft\\large0.sft"));
        assert_eq!(menus.font(3), None);
        assert_eq!(menus.bitmaps, ["bkMain.bm", "arrowLeft.bm"]);
        // The screen indexes that table for its picture buttons.
        assert_eq!(menus.screens[0].art, ["bkMain.bm", "arrowLeft.bm"]);
    }

    #[test]
    fn a_custom_element_names_its_renderer() {
        // The address in the element's `+0x30` field is the classification; anything else
        // is an unknown panel rather than a guess.
        assert_eq!(panel(0x418000), Some(Panel::ForceStars));
        assert_eq!(panel(0x4156a0), Some(Panel::Map));
        assert_eq!(panel(0x417b00), Some(Panel::Objectives));
        assert_eq!(panel(0x419db0), Some(Panel::Model));
        assert_eq!(panel(0x410000), None);
        assert_eq!(panel(0), None);
    }

    #[test]
    fn finds_the_handle_array_a_startup_fills() {
        // `mov ecx, [esi + 0x52b208]` … `mov [esi + 0x8568a0], eax`: the store's
        // displacement is the run-time array the loaded handles go into.
        let mut code = vec![0x8b, 0x8e];
        code.extend_from_slice(&0x52b208u32.to_le_bytes());
        code.extend_from_slice(&[0x90; 8]);
        code.extend_from_slice(&[0x89, 0x86]);
        code.extend_from_slice(&0x8568a0u32.to_le_bytes());
        assert_eq!(handle_array_base(&code, 0x52b208), Some(0x8568a0));
        assert_eq!(handle_array_base(&code, 0x52b178), None);
    }

    #[test]
    fn reads_the_bitmap_a_startup_passes() {
        let base = 0x8567e0u32;
        let mut code = Vec::new();
        // `mov eax, [stdBitmaps + 16]; push eax; push 0x400010; call`, then the same for a
        // second menu 25 bytes later, which is how close `jkGuiTitle_Startup`'s calls sit.
        let call = |code: &mut Vec<u8>, bitmap: u32, menu: u32| {
            code.push(0xa1);
            code.extend_from_slice(&bitmap.to_le_bytes());
            code.push(0x50);
            code.push(0x68);
            code.extend_from_slice(&menu.to_le_bytes());
            code.push(0xe8);
            code.extend_from_slice(&0u32.to_le_bytes());
        };
        call(&mut code, base + 16, 0x400010);
        code.push(0x57);
        call(&mut code, base + 20, 0x400020);
        assert_eq!(init_menu_index(&code, base, 35, 0x400010), Some(4));
        // The nearest load wins: the first call's bitmap is not 25 bytes away.
        assert_eq!(init_menu_index(&code, base, 35, 0x400020), Some(5));
        assert_eq!(init_menu_index(&code, base, 35, 0x400030), None);
        // A load past the end of the table is not an index.
        call(&mut code, base + 200, 0x400040);
        assert_eq!(init_menu_index(&code, base, 35, 0x400040), None);
    }

    #[test]
    fn rejects_an_item_off_the_screen() {
        let mut bytes = build();
        // Push the second item's width past the right edge. Its record is no longer an
        // item, which leaves a lone record: too little to be a screen, so none is found.
        let at = 0x200 + 100 + 0x1c;
        bytes[at..at + 4].copy_from_slice(&900i32.to_le_bytes());
        assert!(load(&bytes).is_none());
    }

    /// The built force screen hides a power the player does not hold, and its rows carry the bins
    /// `items.dat` does (`JUMP` 21, `SPEED` 22) rather than the engine's element ids.
    #[test]
    fn the_force_screen_shows_what_the_player_holds() {
        let mut screen = force_screen();
        assert_eq!(apply_force_rows(&mut screen, |_| 0.0), 0, "nothing found, nothing shown");
        let held = [21, 22];
        assert_eq!(apply_force_rows(&mut screen, |bin| f32::from(held.contains(&bin) as u8)), 2);
        let shown: Vec<&str> = screen
            .items
            .iter()
            .filter(|item| item.kind == 1 && item.visible)
            .map(|item| item.key.as_str())
            .collect();
        assert_eq!(shown, ["JUMP", "SPEED"]);
        let bins: Vec<i32> = FORCE_POWERS.iter().map(|(_, bin, _, _)| *bin).collect();
        assert!(bins.contains(&21) && bins.contains(&22), "the table lost a bin");
    }
}
