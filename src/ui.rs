//! Drawing the retail UI: a 32-bit canvas, a menu screen, and the status bar.
//!
//! Everything drawn here comes from the install: a screen's background is its own `.bm`,
//! its text is a `.sft` glyph strip indexed through `ui\jkstrings.uni`, and the status
//! bar's numbers are the level colormap's colours. `menus.rs` recovers the element rects
//! and the engine's font and bitmap tables from the executable; this module only knows
//! how to put pixels on a canvas.
//!
//! Text is drawn exactly as the engine draws it: a glyph's byte is a *palette index*, and
//! the palette is the menu background's own. That is why the fonts need no colour of their
//! own — `large0.sft` is the same grey strip as `large1.sft` except for its brightest
//! index, and the two read as cream and gold only through the background's palette.

use crate::bm;
use crate::menus::{self, CHECKBOX, CUSTOM, PICBUTTON, Screen};
use crate::sft;
use crate::uni;

/// Where a screen's text and artwork come from: the engine's own tables.
pub struct Look<'a> {
    /// The level and the player's pose, for the map panel: `jkGuiMap_DrawMapScreen` draws the
    /// level around `GetLocalPlayerThing`.
    pub map: Option<(&'a crate::jkl::Level, [f32; 3], f32)>,
    /// How many world units the map panel shows across its radius.
    pub map_radius: f32,
    /// Palette index for the map's line work and the player's marker.
    pub map_ink: u8,
    /// The engine's fonts in `text_type` order.
    pub fonts: &'a [sft::Font],
    /// Resolves a picture button's bitmap index to its artwork.
    pub art: &'a dyn Fn(i32) -> Option<bm::Bm>,
    /// The bitmap index of the check box art (`check.bm`), if the install has it.
    pub checkbox: Option<i32>,
    /// The palette a screen's own art carries none of: the engine's `menu->palette` is the
    /// background's, but `bkdialog.bm` has no palette of its own and indexes the level's
    /// colormap instead.
    pub palette: &'a [[u8; 3]; 256],
    /// The force-star art (`foStars.bm`) and how many stars to draw, for the one custom
    /// panel this build can fill without game state.
    pub stars: Option<(&'a bm::Bm, i32)>,
    /// On the force screen, a power's level from the player's inventory. `jkGuiForce_ForceStarsDraw`
    /// reads `GetInvItemAmount(bin)`, then blits the star cell for that level beside the icon: cells
    /// `3..6` for the light-side powers, `7..10` for the dark ones, `0` for a plain star.
    pub force_level: Option<(&'a bm::Bm, &'a dyn Fn(i32) -> i32)>,
    /// Literal text for a run-time dialog's element: `jkGuiDialog_Message` fills the lines
    /// of an already-built Ok/Cancel menu, whose text elements carry no key.
    pub text: &'a dyn Fn(usize) -> Option<String>,
    /// How far a `Panel::LoadBar` has filled, 0..=100.
    pub load_percent: f32,
}

impl<'a> Default for Look<'a> {
    fn default() -> Self {
        // A screen with nothing to resolve: every optional source empty, the map off. Sites
        // that care set the fields they need and take the rest from here.
        struct Empty;
        impl Empty {
            fn art(_: i32) -> Option<bm::Bm> {
                None
            }
            fn text(_: usize) -> Option<String> {
                None
            }
        }
        fn no_palette() -> &'static [[u8; 3]; 256] {
            static ZERO: [[u8; 3]; 256] = [[0; 3]; 256];
            &ZERO
        }
        Self {
            map: None,
            map_radius: 20.0,
            map_ink: 31,
            fonts: &[],
            art: &Empty::art,
            checkbox: None,
            palette: no_palette(),
            stars: None,
            force_level: None,
            text: &Empty::text,
            load_percent: 0.0,
        }
    }
}

/// The art a screen draws over: the full-screen bitmap, and for a dialog the panel the
/// engine copies on top of it. Dialogs compose two layers — the menu art, then
/// `bkdialog.bm` at that bitmap's own `xPos`/`yPos` — which is why `bkdialog.bm` is 414x206
/// rather than a screen.
pub struct Backdrop<'a> {
    pub base: &'a bm::Bm,
    pub panel: Option<&'a bm::Bm>,
}

/// Draw one menu screen: its background, then every element.
///
/// An element's `text_type` is a font index; the engine draws a hovered element with the
/// next font and a pressed one with the next again, so `selected` only shifts the index.
pub fn draw_screen(
    canvas: &mut Canvas,
    screen: &Screen,
    backdrop: &Backdrop,
    strings: &uni::Uni,
    look: &Look,
    selected: Option<usize>,
) {
    // A plate's own palette need not carry the font's ink: `bkForce.bm` maps the glyph pixels
    // (`229..232`, the values the `ui\sft` art uses) to black, which is why the Force Powers labels
    // draw invisibly on their own dark plate while the tally's plate paints them orange. Falling back
    // does not help — the level colormap paints those indices black too — so the real fix is the
    // engine's glyph remap (`stdFont_Draw3`'s remap table), which this port does not carry yet.
    let palette = palette_of(backdrop.base, look.palette);
    if let Some(cell) = backdrop.base.cells.first() {
        canvas.blit_indexed([0, 0], cell.width, cell.height, &cell.pixels, &palette, 1);
    }
    // The panel is drawn with the same palette: the engine copies it into the menu buffer.
    if let Some(panel) = backdrop.panel
        && let Some(cell) = panel.cells.first()
    {
        canvas.blit_indexed([panel.x_pos, panel.y_pos], cell.width, cell.height, &cell.pixels, &palette, 1);
    }
    for (index, item) in screen.items.iter().enumerate() {
        // The engine's draw loop skips an element whose `bIsVisible` its screen cleared.
        if !item.visible {
            continue;
        }
        let lit = selected == Some(index);
        let [x, y, w, h] = item.rect;
        // A custom element is a rect the screen's own code paints, and its own draw
        // callback says which one: `item.panel()`.
        if item.kind == CUSTOM {
            match item.panel() {
                // `jkGuiSingleTally_ForceStarsRender` (0x418000): one bitmap cell repeated
                // `count` times, the run centred in the rect.
                Some(menus::Panel::ForceStars) => {
                    if let Some((art, count)) = look.stars {
                        draw_stars(canvas, &palette, [x, y, w, h], art, count);
                    }
                }
                // `jkGuiObjectives_CustomRender` (0x417b00): with no goal in the inventory it
                // draws the engine's own line, 30px in from the element's left edge.
                // `jkGuiMap_DrawMapScreen` (0x4156a0) with `sithMap_DrawCircle`: the level
                // around the player. `look.map` carries the level and the player's pose.
                Some(menus::Panel::Map) => {
                    if let Some((level, player, yaw)) = look.map {
                        canvas.map([x, y, w, h], level, player, yaw, look.map_radius, &palette, look.map_ink);
                    }
                }
                // `jkGuiTitle_LoadBarDraw` (0x418b90): a frame over the element's own rect, then
                // the fill inset 3px and `height - 6` tall, `percent * (width - 6) / 100` wide, in
                // the element's `extraInt` colour. The two loading screens put their bars at
                // `{220,240,200,20}` (over `bkMain.bm`) and `{330,131,240,20}` (over
                // `bkLoading.bm`), which is the geometry this draws.
                Some(menus::Panel::LoadBar) => {
                    // `0xE1` is the element table's `origExtraInt` (`jkGUITitle_elementsLoad[0]`): the
                    // fill colour the engine passes to `stdDisplay_VBufferFill`.
                    Canvas::load_bar(canvas, &palette, [x, y, w, h], look.load_percent, 0xE1);
                }
                Some(menus::Panel::Objectives) => {
                    if let Some(font) = usize::try_from(item.text_type).ok().and_then(|t| look.fonts.get(t))
                        && let Some(text) = strings.get("GUI_NO_OBJECTIVES")
                    {
                        canvas.draw_text_palette(font, x + 30, y, text, &palette, 1);
                    }
                }
                _ => {}
            }
            continue;
        }
        // A picture button is its artwork, and nothing else. Its cells are the button's
        // states, so a lit one draws the second (`jkGuiRend_PicButtonDraw`'s mip index).
        if item.kind == PICBUTTON {
            if let Some(artwork) = (look.art)(item.bitmap) {
                let [ax, ay] = [if x < 0 { artwork.x_pos } else { x }, if y < 0 { artwork.y_pos } else { y }];
                blit_art(canvas, &palette, [ax, ay], &artwork, [w, h], usize::from(lit));
                // `jkGuiForce_ForceStarsDraw`: the level beside the icon, from the inventory. The
                // cell is `level - 1` biased by three for the light powers (`0x19..0x1c`) and by
                // seven for the dark ones (`0x1e..0x21`); a power with no level draws nothing. Icons
                // left of the panel's middle get their stars to their left, the rest to the right
                // (`x < 0x140`).
                if let Some((star, level_of)) = look.force_level
                    && screen.base == 0x52f168
                {
                    let level = level_of(item.id);
                    if level > 0 {
                        let cell = level - 1
                            + if (0x19..=0x1c).contains(&item.id) {
                                3
                            } else if (0x1e..=0x21).contains(&item.id) {
                                7
                            } else {
                                0
                            };
                        if let Some(star_cell) = star.cells.get(cell as usize) {
                            let width = star_cell.width as i32;
                            let sx = if ax < 0x140 { ax - width - 0x13 } else { ax + w + 0x13 };
                            blit_art(canvas, &palette, [sx, ay + 3], star, [0, 0], cell as usize);
                        }
                    }
                }
            }
            continue;
        }
        // A check box is its box then its text: the box's cells are empty then ticked.
        let mut rect = item.rect;
        if item.kind == CHECKBOX
            && let Some(artwork) = look.checkbox.and_then(|i| (look.art)(i))
            && let Some(piece) = artwork.cells.get(usize::from(item.bitmap != 0))
        {
            blit_art(canvas, &palette, [x, y + (h - piece.height as i32).max(0) / 2], &artwork, [0, 0], 0);
            rect[0] = x + piece.width as i32 + 4;
            rect[2] = (w - piece.width as i32 - 4).max(0);
        }
        // A key the shipped table does not carry draws nothing: the engine's
        // `jkGui_InitMenu` only copies a string when `jkStrings_GetUniString` finds one,
        // leaving the element's text pointer null. A run-time dialog supplies its own.
        let literal = (look.text)(index);
        let text = match &literal {
            Some(text) => text.as_str(),
            None => strings.get(&item.key).unwrap_or_default(),
        };
        let font = usize::try_from(item.text_type)
            .ok()
            .map(|t| t + usize::from(lit))
            .and_then(|t| look.fonts.get(t))
            .or_else(|| look.fonts.get(usize::try_from(item.text_type).unwrap_or(0)))
            .filter(|f| f.height > 0);
        let Some(font) = font else { continue };
        let [x, y, w, h] = rect;
        let tx = x + (w - font.text_width(text) as i32) / 2;
        let ty = y + (h - font.height as i32) / 2;
        canvas.draw_text_palette(font, tx, ty, text, &palette, 1);
    }
}
/// The palette a bitmap is drawn with: its own, or the level colormap's when it carries
/// none (the engine's `menu->palette` is the background's, and `bkdialog.bm` has none).
pub fn palette_of(art: &bm::Bm, fallback: &[[u8; 3]; 256]) -> [[u8; 3]; 256] {
    if let Some(palette) = art.palette_rgb {
        return palette;
    }
    if fallback.iter().any(|slot| *slot != [0, 0, 0]) {
        return *fallback;
    }
    // No colormap loaded either: treat the index as its own grey, so shapes still show.
    let mut grey = [[0u8; 3]; 256];
    for (i, slot) in grey.iter_mut().enumerate() {
        *slot = [i as u8, i as u8, i as u8];
    }
    grey
}

/// Map a window cursor position to a point on the 640x480 UI canvas.
///
/// The canvas is drawn on a quad sized to the camera's frustum height at half a unit out,
/// so it always fills the window's height and is letterboxed horizontally when the window
/// is wider than 4:3 (and the other way round when it is narrower).
pub fn to_canvas(size: [f32; 2], cursor: [f32; 2]) -> Option<[i32; 2]> {
    let [w, h] = size;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let scale = (h / CANVAS[1] as f32).min(w / CANVAS[0] as f32);
    let drawn = [CANVAS[0] as f32 * scale, CANVAS[1] as f32 * scale];
    let left = (w - drawn[0]) / 2.0;
    let top = (h - drawn[1]) / 2.0;
    let x = (cursor[0] - left) / scale;
    let y = (cursor[1] - top) / scale;
    if x < 0.0 || y < 0.0 || x >= CANVAS[0] as f32 || y >= CANVAS[1] as f32 {
        return None;
    }
    Some([x as i32, y as i32])
}

/// The clickable element under a canvas point: the last one wins, as the engine draws its
/// elements in order and the topmost is the one under the cursor.
///
/// `id < 0` is **not** a reason to skip an element: a screen's cancel/done row carries `hoverId -1`
/// (see `nav::closes`), so that test made every Cancel button dead to the mouse — the way out of a
/// sub-screen only worked with the keyboard. Only a text label is unpickable.
pub fn pick(screen: &Screen, at: [i32; 2]) -> Option<usize> {
    screen.items.iter().enumerate().rev().find_map(|(index, item)| {
        if item.kind == 2 || !item.visible {
            return None;
        }
        let [x, y, w, h] = item.rect;
        let inside = at[0] >= x && at[1] >= y && at[0] < x + w && at[1] < y + h;
        inside.then_some(index)
    })
}

/// The canvas' authored size: every element rect and the HUD are in these coordinates.
pub const CANVAS: [i32; 2] = [640, 480];

/// Draw `count` force stars centred in an element's rect, as the tally screen's renderer
/// does: one-cell bitmap copies, `width` apart, starting at
/// `x + (rect.width - count * width) / 2`.
fn draw_stars(canvas: &mut Canvas, palette: &[[u8; 3]; 256], rect: [i32; 4], art: &bm::Bm, count: i32) {
    let Some(cell) = art.cells.first() else { return };
    let width = cell.width as i32;
    if count <= 0 || width <= 0 {
        return;
    }
    let left = rect[0] + (rect[2] - count * width) / 2;
    for i in 0..count {
        blit_art(canvas, palette, [left + i * width, rect[1]], art, [0, 0], 0);
    }
}

/// Blit one element's artwork: the cell at `state` (a button's normal/hover art), clipped
/// to `size` when that is positive.
fn blit_art(canvas: &mut Canvas, palette: &[[u8; 3]; 256], at: [i32; 2], art: &bm::Bm, size: [i32; 2], state: usize) {
    let Some(piece) = art.cells.get(state).or_else(|| art.cells.first()) else { return };
    let [w, h] = size;
    let (w, h) = if w > 0 && h > 0 {
        (w.min(piece.width as i32) as u32, h.min(piece.height as i32) as u32)
    } else {
        (piece.width, piece.height)
    };
    canvas.blit_indexed(at, w, h, &piece.pixels, palette, 1);
}

/// A 32-bit RGBA drawing surface for one screen.
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, rgba: vec![0; (width * height * 4) as usize] }
    }

    /// Blend a screen effect over everything already drawn.
    ///
    /// `stdPalEffects` applies its effect at *scan-out*: the framebuffer is unchanged and the palette
    /// lookup is filtered, tinted and biased. `stdPalEffects_GatherEffects` is precise about what the
    /// three channels mean, and this follows it:
    ///
    /// - `filter[k] == false` divides that channel by **four** (`>> 2`), it does not drop it — force
    ///   seeing's `(0, 1, 1)` dims red and leaves green and blue, which is the blue-grey view.
    /// - an all-zero `tint` means "no tint at all"; otherwise it multiplies.
    /// - `add` (0..255) is added after that.
    ///
    /// `alpha` is how far in the effect has faded.
    pub fn apply_screen_effect(
        &mut self,
        filter: [bool; 3],
        tint: [f32; 3],
        add: [f32; 3],
        alpha: f32,
    ) {
        if alpha <= 0.0 {
            return;
        }
        let alpha = alpha.clamp(0.0, 1.0);
        let tinted = tint != [0.0; 3];
        for pixel in self.rgba.chunks_exact_mut(4) {
            for k in 0..3 {
                let mut wanted = pixel[k] as f32;
                if !filter[k] {
                    wanted /= 4.0;
                }
                if tinted {
                    wanted *= tint[k];
                }
                wanted = (wanted + add[k]).clamp(0.0, 255.0);
                pixel[k] = (pixel[k] as f32 * (1.0 - alpha) + wanted * alpha).round() as u8;
            }
        }
    }

    /// Copy an 8-bit indexed bitmap in, each pixel drawn as `scale` x `scale` block.
    pub fn blit_indexed(
        &mut self,
        at: [i32; 2],
        w: u32,
        h: u32,
        pixels: &[u8],
        palette: &[[u8; 3]; 256],
        scale: i32,
    ) {
        let [x, y] = at;
        let scale = scale.max(1);
        for row in 0..h {
            for col in 0..w {
                let Some(&i) = pixels.get((row * w + col) as usize) else { continue };
                let [r, g, b] = palette[i as usize];
                self.scaled(x + col as i32 * scale, y + row as i32 * scale, scale, [r, g, b]);
            }
        }
    }

    /// One colour, drawn `scale` x `scale` times, ignoring anything outside the canvas.
    fn scaled(&mut self, x: i32, y: i32, scale: i32, [r, g, b]: [u8; 3]) {
        for dy in 0..scale {
            for dx in 0..scale {
                let (px, py) = (x + dx, y + dy);
                if px < 0 || py < 0 || px >= self.width as i32 || py >= self.height as i32 {
                    continue;
                }
                let at = ((py as u32 * self.width + px as u32) * 4) as usize;
                self.rgba[at..at + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
    }

    /// Put one pixel, in palette colour `index`, ignoring anything outside the canvas.
    pub fn put(&mut self, x: i32, y: i32, palette: &[[u8; 3]; 256], index: u8) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let [r, g, b] = palette[index as usize];
        let at = ((y as u32 * self.width + x as u32) * 4) as usize;
        self.rgba[at..at + 4].copy_from_slice(&[r, g, b, 255]);
    }

    /// A straight line, one pixel at a time (Bresenham): the map is drawn as line work.
    pub fn line(&mut self, from: [i32; 2], to: [i32; 2], palette: &[[u8; 3]; 256], index: u8) {
        let (mut x, mut y) = (from[0], from[1]);
        let (dx, dy) = ((to[0] - x).abs(), -(to[1] - y).abs());
        let (sx, sy) = (if x < to[0] { 1 } else { -1 }, if y < to[1] { 1 } else { -1 });
        let mut err = dx + dy;
        loop {
            self.put(x, y, palette, index);
            if x == to[0] && y == to[1] {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Draw the field-log map panel: the level's walls as line work around the player.
    ///
    /// The retail panel is a real render — `jkGuiMap_DrawMapScreen` sets a camera and calls
    /// `sithMap_DrawCircle(jkGuiMap_pCamera, mat)`, which redraws the level into a 520x320
    /// offscreen buffer and blits it. This build has no second 3D camera on the CPU canvas, so
    /// the panel draws what that circular view shows: every vertical face within `radius` world
    /// units of the player, rotated so the player faces up, plus a marker at the centre.
    ///
    /// `ponytail:` retail's zoom and orbit come from `jkGuiMap_viewMat` and the eight transform
    /// buttons; the scale below fits `radius` into the panel and there is no orbit.

    /// `jkGuiTitle_LoadBarDraw`: the loading screen's progress bar.
///
/// The engine draws a rectangle in `menu->fillColor`, then fills
/// `x + 3, y + 3, percent * (width - 6) / 100, height - 6` with the element's `extraInt` colour.
/// Both loading screens in the install's own menu tables use it, at `{220,240,200,20}` over
/// `bkMain.bm` and `{330,131,240,20}` over `bkLoading.bm`.
fn load_bar(canvas: &mut Canvas, palette: &[[u8; 3]; 256], rect: [i32; 4], percent: f32, fill: u8) {
    let [x, y, w, h] = rect;
    if w <= 6 || h <= 6 {
        return;
    }
    // The frame: one pixel around the whole rect.
    for px in x..x + w {
        canvas.put(px, y, palette, 0xff);
        canvas.put(px, y + h - 1, palette, 0xff);
    }
    for py in y..y + h {
        canvas.put(x, py, palette, 0xff);
        canvas.put(x + w - 1, py, palette, 0xff);
    }
    let percent = percent.clamp(0.0, 100.0);
    let filled = (percent / 100.0 * (w - 6) as f32) as i32;
    for px in x + 3..x + 3 + filled {
        for py in y + 3..y + h - 3 {
            canvas.put(px, py, palette, fill);
        }
    }
}

pub fn map(
        &mut self,
        rect: [i32; 4],
        level: &crate::jkl::Level,
        player: [f32; 3],
        yaw: f32,
        radius: f32,
        palette: &[[u8; 3]; 256],
        ink: u8,
    ) {
        let [x, y, w, h] = rect;
        let (cx, cy) = (x + w / 2, y + h / 2);
        let scale = (w.min(h) as f32 / 2.0) / radius.max(1e-3);
        let (sin, cos) = (-yaw.to_radians()).sin_cos();
        // Player-relative, rotated so the facing direction is up the screen, and y flipped:
        // the canvas counts down, the world counts up.
        let project = |v: [f32; 3]| -> [i32; 2] {
            let (dx, dy) = (v[0] - player[0], v[1] - player[1]);
            let (rx, ry) = (dx * cos - dy * sin, dx * sin + dy * cos);
            let r = (rx * rx + ry * ry).sqrt();
            // The retail view is a circle: clip to it.
            let k = if r > radius { radius / r } else { 1.0 };
            [cx + (rx * k * scale) as i32, cy - (ry * k * scale) as i32]
        };
        for surface in &level.surfaces {
            let verts: Vec<[f32; 3]> = surface
                .verts
                .iter()
                .filter_map(|(v, _)| level.vertices.get(*v as usize).copied())
                .collect();
            if verts.len() < 2 {
                continue;
            }
            let u = [verts[1][0] - verts[0][0], verts[1][1] - verts[0][1], verts[1][2] - verts[0][2]];
            let (nz, horizontal) = if verts.len() >= 3 {
                let v = [verts[2][0] - verts[0][0], verts[2][1] - verts[0][1], verts[2][2] - verts[0][2]];
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                ((n[2] / len).abs(), true)
            } else {
                (0.0, false)
            };
            // Walls only: a floor or a ceiling seen from above is a slab, not line work.
            if horizontal && nz > 0.7 {
                continue;
            }
            if verts.iter().all(|v| {
                let (dx, dy) = (v[0] - player[0], v[1] - player[1]);
                dx * dx + dy * dy > radius * radius
            }) {
                continue;
            }
            for pair in verts.windows(2) {
                self.line(project(pair[0]), project(pair[1]), palette, ink);
            }
            if verts.len() > 2 {
                self.line(project(verts[verts.len() - 1]), project(verts[0]), palette, ink);
            }
        }
        // The player: a cross at the centre with a nose pointing up, in the same ink as the
        // line work so it reads against the panel's dark background.
        for d in -4..=4 {
            self.put(cx + d, cy, palette, ink);
            self.put(cx, cy + d, palette, ink);
        }
        self.put(cx, cy - 7, palette, ink);
    }

    /// Draw one line of text with palette-indexed glyph pixels: a glyph's byte is an index
    /// into `palette`, which is how the engine blits menu text (against the menu art's own
    /// palette) and the HUD numbers (against the level colormap).
    ///
    /// A space is advanced but never blitted: these fonts ship junk pixels in the space
    /// slot (`large0.sft` has a 9x43 slot with 72 non-zero pixels in it, which prints as an
    /// apostrophe between words), and the engine's own text loop treats a space as an
    /// advance.
    pub fn draw_text_palette(
        &mut self,
        font: &crate::sft::Font,
        x: i32,
        y: i32,
        text: &str,
        palette: &[[u8; 3]; 256],
        scale: i32,
    ) -> u32 {
        let s = scale.max(1);
        let mut pen = x;
        for ch in text.chars() {
            if let Some((tx, tw)) = font.glyph(ch)
                && ch != ' '
            {
                for row in 0..font.height {
                    for col in 0..tw {
                        let Some(&v) = font.strip.get(((row * font.strip_width) + tx + col) as usize) else {
                            continue;
                        };
                        if v == 0 {
                            continue;
                        }
                        let [r, g, b] = palette[v as usize];
                        self.scaled(pen + col as i32 * s, y + row as i32 * s, s, [r, g, b]);
                    }
                }
            }
            pen += font.advance(ch) * s;
        }
        (pen - x).max(0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `stdPalEffects_GatherEffects`: an unset filter channel is divided by four, not dropped, and an
    /// all-zero tint means no tint at all.
    #[test]
    fn a_filter_dims_a_channel_and_a_zero_tint_does_nothing() {
        let mut canvas = Canvas::new(1, 1);
        canvas.rgba[0..4].copy_from_slice(&[200, 100, 40, 255]);
        // Force seeing: `newColorEffect(0, 1, 1, 0, 0, 0, 0, 0, 0, 1.0)` — red out of the filter.
        canvas.apply_screen_effect([false, true, true], [0.0; 3], [0.0; 3], 1.0);
        assert_eq!(&canvas.rgba[0..3], &[50, 100, 40], "red must be quartered, green and blue kept");
        // A tint scales; a zero tint is skipped entirely, which is the case above.
        canvas.apply_screen_effect([true; 3], [0.5, 1.0, 2.0], [0.0; 3], 1.0);
        assert_eq!(&canvas.rgba[0..3], &[25, 100, 80], "the tint did not scale the channels");
        // `add` biases after the tint, and an alpha of 0 is a no-op.
        canvas.apply_screen_effect([true; 3], [1.0; 3], [10.0, 0.0, 0.0], 1.0);
        assert_eq!(canvas.rgba[0], 35, "the additive bias did not apply");
        let before = canvas.rgba.clone();
        canvas.apply_screen_effect([false; 3], [2.0; 3], [99.0; 3], 0.0);
        assert_eq!(canvas.rgba, before, "an alpha of 0 changed the pixels");
    }

    /// A screen whose two rows sit in a 640x480 canvas, drawn with a 2x2 font and a
    /// two-entry grey palette.
    fn screen() -> (Screen, bm::Bm, Vec<sft::Font>, uni::Uni) {
        let item = |key: &str, id: i32, y: i32| crate::menus::Item {
            key: key.into(),
            kind: 0,
            text_type: 0,
            id,
            bitmap: 0,
            rect: [0, y, 640, 60],
            hint: None,
            visible: true,
            draw: None,
            click: None,
        };
        let screen = Screen {
            base: 0x1000,
            name: "test".into(),
            background: "ui\\bm\\bkmain.bm".into(),
            background_from_code: false,
            panel: None,
            art: Vec::new(),
            items: vec![item("GUI_OK", 1, 0), item("GUI_CANCEL", 2, 60)],
            sound_click: None,
            sound_hover: None,
        };
        // The art: a 2x2 cell of palette entry 7, over a palette whose entry 7 is dark and
        // whose entry 220 (the glyphs) is white.
        let mut art = vec![0u8; bm::HEADER];
        art[0..4].copy_from_slice(b"BM  ");
        art[36..40].copy_from_slice(&8u32.to_le_bytes());
        art.extend_from_slice(&2u32.to_le_bytes());
        art.extend_from_slice(&2u32.to_le_bytes());
        art.extend_from_slice(&[7u8, 7, 7, 7]);
        let mut palette = vec![0u8; 768];
        palette[3 * 3..3 * 3 + 3].copy_from_slice(&[9, 9, 9]);
        palette[7 * 3..7 * 3 + 3].copy_from_slice(&[7, 7, 7]);
        palette[110 * 3..110 * 3 + 3].copy_from_slice(&[128, 128, 128]);
        palette[220 * 3..220 * 3 + 3].copy_from_slice(&[255, 255, 255]);
        art.extend_from_slice(&palette);
        let background = bm::parse(&art).expect("bitmap");
        let mut font = sft::Font { height: 2, strip_width: 2, strip: vec![220; 4], ..Default::default() };
        font.charsets.push(sft::Charset { first: 32, last: 126, entries: vec![(0, 2); 95] });
        let strings = uni::parse("\"GUI_OK\" 0 \"Ok\"\n\"GUI_CANCEL\" 0 \"Cancel\"");
        // Two fonts, so the hovered row can step to the next one.
        let lit = sft::Font { height: 2, strip_width: 2, strip: vec![110; 4], ..Default::default() };
        (screen, background, vec![font, lit], strings)
    }

    fn art(_: i32) -> Option<bm::Bm> {
        None
    }

    /// No colormap: the fixture's art carries its own palette.
    const EMPTY: [[u8; 3]; 256] = [[0u8; 3]; 256];

    /// The drawing look for the fixture: two fonts, no artwork, no colormap.
    fn look<'a>(fonts: &'a [sft::Font], art: &'a dyn Fn(i32) -> Option<bm::Bm>, palette: &'a [[u8; 3]; 256]) -> Look<'a> {
        Look { fonts, art, checkbox: None, palette, stars: None, text: &|_| None, ..Look::default() }
    }

    #[test]
    fn a_screen_draws_over_its_art() {
        let (screen, background, fonts, strings) = screen();
        let mut canvas = Canvas::new(640, 480);
        let (look, backdrop) = (look(&fonts, &art, &EMPTY), Backdrop { base: &background, panel: None });
        draw_screen(&mut canvas, &screen, &backdrop, &strings, &look, None);
        // The 2x2 art fills the top-left corner with palette entry 7.
        assert_eq!(&canvas.rgba[0..4], &[7, 7, 7, 255]);
        // The first row's glyphs are palette entry 220 of that same palette: opaque white.
        assert!(canvas.rgba.chunks(4).skip(1).any(|p| p[3] == 255 && p[0] == 255));
    }

    #[test]
    fn the_cursor_lights_one_row() {
        let (screen, background, fonts, strings) = screen();
        let mut plain = Canvas::new(640, 480);
        let mut lit = Canvas::new(640, 480);
        let look = look(&fonts, &art, &EMPTY);
        let backdrop = Backdrop { base: &background, panel: None };
        draw_screen(&mut plain, &screen, &backdrop, &strings, &look, None);
        draw_screen(&mut lit, &screen, &backdrop, &strings, &look, Some(1));
        assert_ne!(plain.rgba, lit.rgba);
    }

    #[test]
    fn a_picture_button_draws_its_own_art() {
        let (mut screen, background, fonts, strings) = screen();
        screen.items[0].kind = PICBUTTON;
        // A 4x4 button, twice the size of the background art, drawn with palette entry 3.
        let mut piece = vec![0u8; bm::HEADER];
        piece[0..4].copy_from_slice(b"BM  ");
        piece[36..40].copy_from_slice(&2u32.to_le_bytes());
        piece.extend_from_slice(&4u32.to_le_bytes());
        piece.extend_from_slice(&4u32.to_le_bytes());
        piece.extend_from_slice(&[3u8; 16]);
        let piece = bm::parse(&piece).expect("button art");
        let art = |_: i32| Some(piece.clone());
        let mut canvas = Canvas::new(640, 480);
        let (look, backdrop) = (look(&fonts, &art, &EMPTY), Backdrop { base: &background, panel: None });
        draw_screen(&mut canvas, &screen, &backdrop, &strings, &look, None);
        // The button's art lands at its rect's origin, over the background's 2x2 corner,
        // and covers exactly its own 4x4.
        assert_eq!(canvas.rgba[0..4], [9, 9, 9, 255]);
        assert_eq!(canvas.rgba[(3 * 640 + 3) * 4..(3 * 640 + 3) * 4 + 4], [9, 9, 9, 255]);
        assert_eq!(canvas.rgba[(4 * 640 + 4) * 4 + 3], 0);
    }

    #[test]
    fn an_invisible_element_is_neither_drawn_nor_picked() {
        let (mut screen, background, fonts, strings) = screen();
        screen.items[0].kind = PICBUTTON;
        let mut piece = vec![0u8; bm::HEADER];
        piece[0..4].copy_from_slice(b"BM  ");
        piece[36..40].copy_from_slice(&2u32.to_le_bytes());
        piece.extend_from_slice(&4u32.to_le_bytes());
        piece.extend_from_slice(&4u32.to_le_bytes());
        piece.extend_from_slice(&[3u8; 16]);
        let piece = bm::parse(&piece).expect("button art");
        let art = |_: i32| Some(piece.clone());
        let look = look(&fonts, &art, &EMPTY);
        let backdrop = Backdrop { base: &background, panel: None };
        let mut shown = Canvas::new(640, 480);
        draw_screen(&mut shown, &screen, &backdrop, &strings, &look, None);
        // `bIsVisible = 0`: the engine's draw loop skips the element, and so does the picker.
        screen.items[0].visible = false;
        let mut hidden = Canvas::new(640, 480);
        draw_screen(&mut hidden, &screen, &backdrop, &strings, &look, None);
        assert_ne!(shown.rgba, hidden.rgba);
        assert_eq!(pick(&screen, [320, 30]), None);
        assert_eq!(pick(&screen, [320, 90]), Some(1));
    }

    #[test]
    fn the_star_row_draws_its_cells_centred() {
        // The tally screen's star rod: four `foStars.bm` cells across the element's rect.
        let mut piece = vec![0u8; bm::HEADER];
        piece[0..4].copy_from_slice(b"BM  ");
        piece[36..40].copy_from_slice(&1u32.to_le_bytes());
        piece.extend_from_slice(&2u32.to_le_bytes());
        piece.extend_from_slice(&2u32.to_le_bytes());
        piece.extend_from_slice(&[3u8; 4]);
        let stars = bm::parse(&piece).expect("star art");
        let (_, background, fonts, strings) = screen();
        let screen = Screen {
            base: 0x531530,
            name: "tally".into(),
            background: "ui\\bm\\bktally.bm".into(),
            background_from_code: true,
            panel: None,
            art: Vec::new(),
            // The star row is element 4 of the tally array, which is what `menus::panel`
            // keys on; the four before it are hidden so only the stars paint.
            items: (0..5)
                .map(|i| crate::menus::Item {
                    key: String::new(),
                    kind: if i == 4 { CUSTOM } else { 2 },
                    text_type: 0,
                    id: 0,
                    bitmap: 0,
                    rect: [0, 220, 640, 2],
                    hint: None,
                    visible: i == 4,
                    // The tally's star row: its own draw callback is the classification.
                    draw: (i == 4).then_some(0x418000),
                    click: None,
                })
                .collect(),
            sound_click: None,
            sound_hover: None,
        };
        let art = |_: i32| None;
        let mut look = look(&fonts, &art, &EMPTY);
        look.stars = Some((&stars, 3));
        let mut canvas = Canvas::new(640, 480);
        draw_screen(&mut canvas, &screen, &Backdrop { base: &background, panel: None }, &strings, &look, None);
        // Three 2px cells, centred: they start at (640 - 6) / 2 = 317 and cover 317..323.
        let pixel = |x: i32, y: i32| canvas.rgba[((y * 640 + x) * 4) as usize];
        assert_eq!(pixel(317, 220), 9);
        assert_eq!(pixel(322, 220), 9);
        assert_eq!(pixel(316, 220), 0);
        assert_eq!(pixel(323, 220), 0);
    }

    #[test]
    fn the_objectives_page_says_there_are_none() {
        // `jkGuiObjectives_CustomRender`: an empty goal list draws the engine's own line at
        // `rect.x + 30`, `rect.y`, in the element's own font.
        let (mut screen, background, fonts, _) = screen();
        let mut item = screen.items[0].clone();
        item.kind = CUSTOM;
        item.draw = Some(0x417b00);
        item.rect = [40, 110, 520, 290];
        item.key = String::new();
        screen.items = vec![item];
        let strings = uni::parse("\"GUI_NO_OBJECTIVES\" 0 \"No Objectives\"");
        let mut canvas = Canvas::new(640, 480);
        let look = look(&fonts, &art, &EMPTY);
        draw_screen(&mut canvas, &screen, &Backdrop { base: &background, panel: None }, &strings, &look, None);
        // The fixture's glyphs are palette entry 220, white; the first column is 40 + 30.
        let ink = |x: i32| (110..112).any(|y| canvas.rgba[((y * 640 + x) * 4) as usize] == 255);
        assert!(!ink(69));
        assert!(ink(70));
    }

    #[test]
    fn the_pointer_maps_into_the_canvas_letterbox() {
        // A 4:3 window: the canvas fills it, so window pixels are canvas pixels.
        assert_eq!(to_canvas([640.0, 480.0], [0.0, 0.0]), Some([0, 0]));
        assert_eq!(to_canvas([640.0, 480.0], [639.0, 479.0]), Some([639, 479]));
        // Twice the size: still aligned, and the far corner is still the far corner.
        assert_eq!(to_canvas([1280.0, 960.0], [2.0, 2.0]), Some([1, 1]));
        assert_eq!(to_canvas([1280.0, 960.0], [1278.0, 958.0]), Some([639, 479]));
        // A 16:9 window is letterboxed left and right: x=0 is outside the canvas, the
        // canvas' left edge sits at (1280 - 640*720/480... ) = (1280-960)/2 = 160.
        assert_eq!(to_canvas([1280.0, 720.0], [160.0, 360.0]), Some([0, 240]));
        assert_eq!(to_canvas([1280.0, 720.0], [100.0, 360.0]), None);
    }

    #[test]
    fn the_pointer_picks_the_element_under_it() {
        let (screen, _, _, _) = screen();
        // The fixture's two rows are 640x60 at y=0 and y=60.
        assert_eq!(pick(&screen, [320, 30]), Some(0));
        assert_eq!(pick(&screen, [320, 90]), Some(1));
        assert_eq!(pick(&screen, [320, 200]), None);
        // A plain text element is never picked, even where it sits.
        let mut labels = screen.clone();
        labels.items[1].kind = 2;
        assert_eq!(pick(&labels, [320, 90]), None);
        // A *button* with id -1 is picked: that is exactly what a screen's cancel/done row carries
        // (`hoverId -1`, see `nav::closes`), and skipping it made Cancel dead to the mouse.
        let mut cancel = screen.clone();
        cancel.items[0].id = -1;
        assert_eq!(pick(&cancel, [320, 30]), Some(0));
    }
}
