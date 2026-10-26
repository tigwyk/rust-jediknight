//! `episode.jk` — the mission tree.
//!
//! One manifest lives in each episode archive (single player, multiplayer, CTF),
//! and it is what the game walks to sequence cutscenes and levels. Format, from
//! the shipped `Episode\JK1.GOB` copy:
//!
//! ```text
//! "The Force Within"          # title, must be the first line, quoted
//! TYPE  1                     # 1 single player, 2 multiplayer, 8 CTF
//! SEQ   55                    # sequence count hint
//!
//! # <line> <cd>  <level>  <type>   <file>   <lightpow> <darkpow> <gotoA> <gotoB>
//! 11:      1     1        LEVEL    01narshadda.jkl   0  0  12  -1   # Level 1: Narshadda
//! 143:     1     14       DECIDE   38a.smk          0  0  150 600   # Decision Screen
//! ```
//!
//! `gotoA` is the next line, `gotoB` an alternate one; `-1` ends the game. The
//! light/dark split at the `DECIDE` row is expressed entirely through those two
//! edges, so the graph — not a level list — is the real progression.

pub const KIND_SINGLE: i32 = 1;
pub const KIND_MULTI: i32 = 2;
pub const KIND_CTF: i32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A Smacker cutscene. The shipped install carries no `.smk` files.
    Cut,
    Level,
    /// Branch point; `goto_a`/`goto_b` pick the light or dark path.
    Decide,
    Other,
}

#[derive(Debug, Clone)]
pub struct Sequence {
    pub id: i32,
    /// `cd` column — which CD the level was read from in the 1997 retail release.
    pub cd: i32,
    /// Chapter number the levels are labelled with.
    pub number: i32,
    pub kind: Kind,
    pub file: String,
    pub lightpow: i32,
    pub darkpow: i32,
    pub goto_a: i32,
    pub goto_b: i32,
    pub comment: String,
}

#[derive(Debug, Default)]
pub struct Episode {
    pub title: String,
    pub kind: i32,
    pub sequences: Vec<Sequence>,
}

impl Episode {
    pub fn levels(&self) -> impl Iterator<Item = &Sequence> {
        self.sequences.iter().filter(|s| s.kind == Kind::Level)
    }

    /// The first level of the episode: where the game starts.
    pub fn first_level(&self) -> Option<&Sequence> {
        self.levels().next()
    }

    /// The row that names `file`, however the case is spelled (`01narshadda.jkl` is listed as
    /// `01narshadda.jkl` but the level files on disk are `04EscapeHouse.jkl`).
    pub fn row_for(&self, file: &str) -> Option<&Sequence> {
        // The graph names levels by bare file (`01narshadda.jkl`) and the game knows them by archive
        // path (`jkl\01narshadda.jkl`), so a strict compare never matched: finishing level 1 said
        // "end of campaign" instead of loading the film before level 2.
        let name = |path: &str| {
            path.rsplit(['\\', '/']).next().unwrap_or(path).to_ascii_lowercase()
        };
        let wanted = name(file);
        self.sequences
            .iter()
            .find(|s| name(&s.file) == wanted)
    }

    /// The next row after `id`, following `goto_a` — the chain a level's own `jkEndLevel`
    /// walks. `None` at the end of the campaign (`-1`).
    pub fn next_row(&self, id: i32) -> Option<&Sequence> {
        let row = self.sequences.iter().find(|s| s.id == id)?;
        (row.goto_a >= 0)
            .then(|| self.sequences.iter().find(|s| s.id == row.goto_a))
            .flatten()
    }

    /// The cutscenes that play **before** `level_file`: the chain that leads into it.
    ///
    /// The graph's `gotoA` links run forwards, so a level's intro films are the rows nobody in the
    /// level's own chain visits — row 10 of the single-player episode is `03-04a.smk` with
    /// `gotoA 11`, and 11 is level 1. Retail plays that film, then drops the player into the cantina;
    /// a port that starts at `first_level` skips it entirely, which is exactly what a player
    /// reported (the ship and the booth scenes were missing).
    pub fn flow_before(&self, level_file: &str) -> Vec<&Sequence> {
        let Some(level) = self.row_for(level_file) else { return Vec::new() };
        let mut found: Vec<&Sequence> = Vec::new();
        let mut at = level.id;
        // Walk backwards: the film whose `gotoA` is `at`, then the film whose `gotoA` is that
        // film, and so on — a run of CUT rows ends at the previous LEVEL row.
        loop {
            let Some(previous) = self
                .sequences
                .iter()
                .find(|s| s.goto_a == at && s.kind == Kind::Cut)
            else {
                break;
            };
            found.insert(0, previous);
            at = previous.id;
            if found.len() > 8 {
                break;
            }
        }
        found
    }
    /// What finishing `level_file` leads to: the films the graph plays first, in order, and the
    /// level that loads after them. A level can lead through more than one film — the mid-campaign
    /// `DECIDE` row sits between its two — and the decision itself is a row, not a level.
    pub fn flow_after(&self, level_file: &str) -> FlowAfter<'_> {
        let mut flow = FlowAfter::default();
        let Some(row) = self.row_for(level_file) else { return flow };
        let mut at = row.goto_a;
        let mut steps = 0;
        while at >= 0 && steps <= self.sequences.len() {
            steps += 1;
            let Some(next) = self.sequences.iter().find(|s| s.id == at) else { break };
            match next.kind {
                Kind::Cut => {
                    flow.films.push(next);
                    at = next.goto_a;
                }
                // The campaign's own fork after the Maw. This build has no choice screen, so the
                // neutral arm is the light one (`gotoA`); the dark arm is `gotoB`.
                Kind::Decide => at = next.goto_a,
                Kind::Level => {
                    flow.next = Some(next);
                    break;
                }
                Kind::Other => break,
            }
        }
        flow
    }
}

/// The answer to "what does finishing this level lead to": the films in play order, and the level
/// that loads after them (`None` at the end of the campaign). A straight level-to-level step has
/// no films.
#[derive(Debug, Default)]
pub struct FlowAfter<'a> {
    pub films: Vec<&'a Sequence>,
    pub next: Option<&'a Sequence>,
}

pub fn parse(text: &str) -> Episode {
    let mut episode = Episode::default();
    for line in text.lines() {
        let code = line.split('#').next().unwrap_or("").trim();
        if code.is_empty() {
            continue;
        }
        let comment = line.split_once('#').map(|(_, c)| c.trim()).unwrap_or("");
        if episode.title.is_empty() && code.starts_with('"') {
            episode.title = code.trim_matches('"').to_string();
            continue;
        }
        let f: Vec<&str> = code.split_whitespace().collect();
        if f.first() == Some(&"TYPE") {
            episode.kind = f.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            continue;
        }
        // A sequence row is the only row that starts with an `N:` label.
        let Some(id) = f.first().and_then(|v| v.strip_suffix(':')).and_then(|v| v.parse().ok())
        else {
            continue;
        };
        let num = |i: usize| f.get(i).and_then(|v| v.parse().ok()).unwrap_or(-1);
        let kind = match f.get(3).copied() {
            Some("LEVEL") => Kind::Level,
            Some("CUT") => Kind::Cut,
            Some("DECIDE") => Kind::Decide,
            _ => Kind::Other,
        };
        episode.sequences.push(Sequence {
            id,
            cd: num(1),
            number: num(2),
            kind,
            file: f.get(4).map(|v| v.to_string()).unwrap_or_default(),
            lightpow: num(5),
            darkpow: num(6),
            goto_a: num(7),
            goto_b: num(8),
            comment: comment.to_string(),
        });
    }
    episode
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_shipped_shape() {
        let e = parse(
            "\"The Force Within\"\n# Title must be first line.\nTYPE  1\nSEQ   55\n\n\
             11:      1     1        LEVEL    01narshadda.jkl   0  0  12  -1   # Level 1: Narshadda\n\
             143:     1     14       DECIDE   38a.smk          0  0  150 600   # Decision Screen\n\
             end\n",
        );
        assert_eq!(e.title, "The Force Within");
        assert_eq!(e.kind, KIND_SINGLE);
        assert_eq!(e.sequences.len(), 2);

        let level = &e.sequences[0];
        assert_eq!(level.id, 11);
        assert_eq!(level.kind, Kind::Level);
        assert_eq!(level.file, "01narshadda.jkl");
        assert_eq!((level.goto_a, level.goto_b), (12, -1));
        assert_eq!(level.comment, "Level 1: Narshadda");

        let decide = &e.sequences[1];
        assert_eq!(decide.kind, Kind::Decide);
        // The light/dark split is the two goto edges, not a level list.
        assert_eq!((decide.goto_a, decide.goto_b), (150, 600));

        assert_eq!(e.first_level().map(|s| s.file.as_str()), Some("01narshadda.jkl"));
    }

    /// A level's successor is a cutscene, and the cutscene's successor is the next level — the
    /// chain `jkEndLevel` walks.
    /// The single-player episode starts with a film that no forward walk from level 1 reaches.
    #[test]
    fn the_intro_film_is_found_before_the_first_level() {
        let e = parse(
            "\"The Force Within\"\nTYPE  1\nSEQ   3\n\n\
             10:\t1\t1\tCUT\t03-04a.smk\t0\t0\t11\t-1\n\
             11:\t1\t1\tLEVEL\t01narshadda.jkl\t0\t0\t12\t-1\n\
             12:\t1\t1\tCUT\t06a.smk\t0\t0\t20\t-1\n\
             20:\t1\t2\tLEVEL\t02narshadda.jkl\t0\t0\t-1\t-1\n\
             end\n",
        );
        let before: Vec<&str> = e.flow_before("01narshadda.jkl").iter().map(|s| s.file.as_str()).collect();
        assert_eq!(before, ["03-04a.smk"], "level 1 should be preceded by its intro film");
        // The film after a level is a different question, and still answered the same way.
        let flow = e.flow_after("01narshadda.jkl");
        assert_eq!(
            flow.films.iter().map(|s| s.file.as_str()).collect::<Vec<_>>(),
            ["06a.smk"]
        );
        assert_eq!(flow.next.map(|s| s.file.as_str()), Some("02narshadda.jkl"));
        // Level 2 is preceded by `06a.smk` — the film level 1 ends with, which `flow_after` already
        // plays. That overlap is why the app only asks `flow_before` at the campaign start: asking on
        // every load would play each film twice.
        let second: Vec<&str> = e.flow_before("02narshadda.jkl").iter().map(|s| s.file.as_str()).collect();
        assert_eq!(second, ["06a.smk"]);
    }

    #[test]
    fn the_next_row_follows_goto_a() {
        let e = parse(
            "\"Ep\"\nTYPE  1\n\
             11: 1 1 LEVEL 01narshadda.jkl 0 0 12 -1\n\
             12: 1 1 CUT   06a.smk        0 0 20 -1\n\
             20: 1 2 LEVEL 02narshadda.jkl 0 0 -1 -1\n",
        );
        let level = e.row_for("01NARSHADDA.JKL").expect("case-insensitive lookup");
        let cut = e.next_row(level.id).expect("a cutscene follows level 1");
        assert_eq!(cut.file, "06a.smk");
        let next = e.next_row(cut.id).expect("the level follows the cutscene");
        assert_eq!(next.file, "02narshadda.jkl");
        assert!(e.next_row(next.id).is_none(), "the campaign ends at -1");

        // And the same answer through the helper the app uses on completion.
        let flow = e.flow_after("01narshadda.jkl");
        assert_eq!(
            flow.films.iter().map(|s| s.file.as_str()).collect::<Vec<_>>(),
            ["06a.smk"]
        );
        assert_eq!(flow.next.map(|s| s.file.as_str()), Some("02narshadda.jkl"));
        // A level whose successor is another level has no film in between.
        let straight = e.flow_after("02narshadda.jkl");
        assert!(straight.films.is_empty());
        assert!(straight.next.is_none(), "the test graph ends there");
    }

    /// The campaign's mid-game decision: two films and a `DECIDE` row sit between level 15 and the
    /// next level, and the decision's two arms are the light and dark paths. The port has no choice
    /// screen, so `flow_after` takes the neutral arm (`gotoA`, light) — and still keeps both films so
    /// the app plays the whole transition. This is the shipped manifest's own shape (row 143).
    #[test]
    fn a_decision_row_is_crossed_and_its_films_are_kept_in_order() {
        let e = parse(
            "\"Ep\"\nTYPE  1\n\
             141: 1 14 LEVEL  15maw.jkl         0 0 142 -1\n\
             142: 1 14 CUT    38a.smk            0 0 143 -1\n\
             143: 1 14 DECIDE 38a.smk            0 0 150 600\n\
             150: 1 15 CUT    39a.smk            0 0 151 -1\n\
             151: 1 15 LEVEL  16aEscapeShip.jkl 0 0 160 152\n\
             160: 1 16 CUT    41-42a.smk         0 0 161 -1\n\
             161: 1 16 LEVEL  17asarris.jkl      0 0 -1  -1\n\
             600: 1 15 CUT    54a.smk            0 0 601 -1\n\
             601: 1 15 LEVEL  16bEscapeShip.jkl 0 0 610 602\n\
             610: 1 16 CUT    41dsa.smk          0 0 611 -1\n\
             611: 1 16 LEVEL  17bsarris.jkl      0 0 -1  -1\n",
        );
        let flow = e.flow_after("15maw.jkl");
        assert_eq!(
            flow.films.iter().map(|s| s.file.as_str()).collect::<Vec<_>>(),
            ["38a.smk", "39a.smk"],
            "the film before the decision and the chosen arm's film both play"
        );
        assert_eq!(
            flow.next.map(|s| s.file.as_str()),
            Some("16aEscapeShip.jkl"),
            "the neutral arm is the light one"
        );
        // The dark arm is still walkable once its own level is the one being finished.
        let dark = e.flow_after("16bEscapeShip.jkl");
        assert_eq!(dark.next.map(|s| s.file.as_str()), Some("17bsarris.jkl"));
    }
}
