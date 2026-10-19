//! The retail menu graph: which screen a row opens.
//!
//! Each screen's `_Show` ends in a loop over `jkGuiRend_GetId(&menu)` — the `hoverId` of the
//! element that was clicked — whose `switch` calls the next screen's `_Show`. Both halves
//! are recoverable from the GOG build's plaintext code:
//!
//! - the handler for a screen is the function that draws its menu struct, and that struct
//!   holds the element array at `+0x00` (see `menus::code_backgrounds` and §18.6);
//! - the `switch` cases are the same ids the element records carry, so every edge below is
//!   checked against the data it comes from (`--ui-audit` fails if an id is not on the
//!   screen it claims to be on).
//!
//! `jkGuiMain_Show` (0x4100f0), decompiled:
//!
//! ```c
//! switch (jkGuiRend_GetId(&main_menu)) {          // menu+0x40
//! case 10: jkGuiSingleplayer_Show(); break;        // 0x41a4a0 -> 0x535e58
//! case 11: jkGuiMultiplayer_Show();  break;        // 0x4131f0 -> 0x52c108
//! case 12: /* GUI_QUITCONFIRM dialog */ break;
//! case 13: jkGuiSetup_Show();  break;              // 0x412f40 -> 0x52b588
//! case 14: jkGuiPlayer_ShowNewPlayer(0); break;    // 0x410870 -> 0x526d98
//! case 15: /* cutscenes */ break;                  // 0x410210 -> 0x5269f0
//! case 16: /* credits */ break;
//! }
//! ```
//!
//! The pause menu's (`jkGuiEsc_Show`, 0x411c30) cases are `JKGUIESC_*` 10..17 in the order
//! objectives, map, jedi powers, load, restart, save, setup, abort, which is exactly the id
//! sequence its own rows carry (10, 11, 12, 1, 13, 15, 14, 16, 17).

/// One menu-graph edge.
pub struct Edge {
    /// The element array the edge starts on.
    pub from: u32,
    /// The `hoverId` of the element that opens it.
    pub id: i32,
    /// The element array it opens.
    pub to: u32,
}

/// Edges read out of the `_Show` switches, with the handler each one calls.
pub const EDGES: &[Edge] = &[
    // jkGuiMain_Show (0x4100f0).
    Edge { from: 0x526680, id: 10, to: 0x535e58 }, // jkGuiSingleplayer_Show
    Edge { from: 0x526680, id: 11, to: 0x52c108 }, // jkGuiMultiplayer_Show
    Edge { from: 0x526680, id: 13, to: 0x52b588 }, // jkGuiSetup_Show
    Edge { from: 0x526680, id: 14, to: 0x526d98 }, // jkGuiPlayer_ShowNewPlayer
    Edge { from: 0x526680, id: 15, to: 0x5269f0 }, // jkGuiMain_ShowCutscenes
    // jkGuiEsc_Show (0x411c30): the pause menu's rows.
    Edge { from: 0x529560, id: 10, to: 0x5311e0 }, // jkGuiObjectives_Show
    Edge { from: 0x529560, id: 11, to: 0x52ea68 }, // jkGuiMap_Show
    Edge { from: 0x529560, id: 12, to: 0x52f168 }, // jkGuiForce_Show
    Edge { from: 0x529560, id: 13, to: 0x53b948 }, // jkGuiSaveLoad_Show(0)
    Edge { from: 0x529560, id: 15, to: 0x53b948 }, // jkGuiSaveLoad_Show(1)
    Edge { from: 0x529560, id: 16, to: 0x52b588 }, // jkGuiSetup_Show
    // jkGuiSetup_Show (0x412f40): the tab bar opens each tab's own screen.
    Edge { from: 0x52b588, id: 100, to: 0x535560 }, // jkGuiGeneral_Show
    Edge { from: 0x52b588, id: 101, to: 0x538350 }, // jkGuiGameplay_Show
    Edge { from: 0x52b588, id: 102, to: 0x52d2b0 }, // jkGuiDisplay_Show
    Edge { from: 0x52b588, id: 103, to: 0x527820 }, // jkGuiSound_Show
    // jkGuiMultiplayer_Show (0x4131f0).
    Edge { from: 0x52c108, id: 102, to: 0x539fe8 }, // jkGuiBuildMulti_Show
    // jkGuiBuildMulti_Show (0x41c700): the character list's New opens the editor.
    Edge { from: 0x539fe8, id: 100, to: 0x53a618 }, // jkGuiBuildMulti_ShowNewCharacter
];

/// What activating an element does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Open this screen on top of the current one.
    Open(u32),
    /// Open one of the engine's run-time dialogs on top of the current screen, with these
    /// `jkstrings.uni` keys as its lines. `jkGuiMain_Show` case 12 does exactly this with
    /// `GUI_QUITCONFIRM` and `GUI_QUITCONFIRM_Q`; the dialog's own text elements carry no
    /// key, so the caller fills them.
    Dialog { screen: u32, keys: &'static [&'static str] },
    /// Go back to the screen this one was opened from.
    Close,
    /// Leave the game.
    Quit,
    /// Nothing this build knows how to do.
    Nothing,
}

/// Elements whose action is not simply "open the screen that draws them".
pub const ACTS: &[(u32, i32, Step)] = &[
    // The main menu's Quit row asks first: `jkGuiDialog_Message(GUI_QUITCONFIRM,
    // GUI_QUITCONFIRM_Q)` over the Ok/Cancel dialog, whose Ok then leaves the game.
    (0x526680, 12, Step::Dialog { screen: 0x530068, keys: &["GUI_QUITCONFIRM", "GUI_QUITCONFIRM_Q"] }),
    (0x530068, 1, Step::Quit),
];

/// The screen an element opens, if the graph knows it.
pub fn next(from: u32, id: i32) -> Option<u32> {
    EDGES.iter().find(|e| e.from == from && e.id == id).map(|e| e.to)
}

/// Decide what one element does: an explicit action first, then the graph, then the
/// screen's own close rows.
pub fn step(from: u32, id: i32, key: &str) -> Step {
    // An explicit action first: the main menu's `GUI_QUIT` row is a confirm dialog, not an
    // immediate exit, and the dialog's own Ok is what leaves.
    if let Some((_, _, act)) = ACTS.iter().find(|(base, at, _)| *base == from && *at == id) {
        return *act;
    }
    if quits(key) {
        return Step::Quit;
    }
    match next(from, id) {
        Some(to) => Step::Open(to),
        None if closes(key, id) => Step::Close,
        None => Step::Nothing,
    }
}

/// Whether an element closes its screen.
///
/// The engine's `_Show` loops run until the id is not `-1`, and every screen's cancel/done
/// row carries `hoverId -1`, so that is the screen's own "go back". `GUI_ABORT` and
/// `GUI_RETURN_TO_GAME` are the same thing said in a key.
pub fn closes(key: &str, id: i32) -> bool {
    id == -1 || matches!(key, "GUI_CANCEL" | "GUI_DONE" | "GUI_ABORT" | "GUI_RETURN_TO_GAME" | "GUI_OK")
}

/// Whether an element ends the program rather than closing its screen.
pub fn quits(key: &str) -> bool {
    matches!(key, "GUI_QUIT")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_two_edges_share_an_id() {
        // Two edges from the same screen with the same id would make `next` ambiguous.
        for (i, a) in EDGES.iter().enumerate() {
            for b in &EDGES[i + 1..] {
                assert!(!(a.from == b.from && a.id == b.id), "{:#x} id {} twice", a.from, a.id);
            }
            assert_ne!(a.from, a.to, "self edge on {:#x}", a.from);
        }
    }

    #[test]
    fn a_close_row_is_not_a_screen() {
        assert_eq!(next(0x526680, 10), Some(0x535e58));
        assert_eq!(next(0x526680, 12), None);
        assert!(closes("GUI_CANCEL", 0));
        assert!(closes("GUI_OK", 1));
        assert!(!closes("GUI_NEWGAME", 100));
        assert!(quits("GUI_QUIT"));
    }

    #[test]
    fn navigating_follows_the_recovered_graph() {
        // The main menu's rows, as decompiled at 0x4100f0.
        assert_eq!(step(0x526680, 10, "GUI_SINGLEPLAYER"), Step::Open(0x535e58));
        assert_eq!(step(0x526680, 13, "GUI_SETUP"), Step::Open(0x52b588));
        assert_eq!(step(0x526680, 14, "GUI_PLAYERS"), Step::Open(0x526d98));
        // A row the graph does not name and that is not a close row does nothing.
        // Quit asks first, then its Ok leaves the game.
        assert_eq!(
            step(0x526680, 12, "GUI_QUIT"),
            Step::Dialog { screen: 0x530068, keys: &["GUI_QUITCONFIRM", "GUI_QUITCONFIRM_Q"] }
        );
        assert_eq!(step(0x530068, 1, "GUI_OK"), Step::Quit);
        assert_eq!(step(0x526680, 16, "GUI_CREDITS"), Step::Nothing);
        // A tab, then the tab screen's Ok row goes back.
        assert_eq!(step(0x52b588, 103, "GUI_SOUND"), Step::Open(0x527820));
        assert_eq!(step(0x527820, 1, "GUI_OK"), Step::Close);
    }
}
