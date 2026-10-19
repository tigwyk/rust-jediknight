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
        self.sequences
            .iter()
            .find(|s| s.file.eq_ignore_ascii_case(file))
    }

    /// The next row after `id`, following `goto_a` — the chain a level's own `jkEndLevel`
    /// walks. `None` at the end of the campaign (`-1`).
    pub fn next_row(&self, id: i32) -> Option<&Sequence> {
        let row = self.sequences.iter().find(|s| s.id == id)?;
        (row.goto_a >= 0)
            .then(|| self.sequences.iter().find(|s| s.id == row.goto_a))
            .flatten()
    }

    /// What finishing `level_file` leads to: the cutscene the graph names (if the very next row
    /// is one) and the level that follows it.
    pub fn flow_after(&self, level_file: &str) -> (Option<&Sequence>, Option<&Sequence>) {
        let Some(row) = self.row_for(level_file) else { return (None, None) };
        let Some(after) = self.next_row(row.id) else { return (None, None) };
        if after.kind == Kind::Cut {
            (Some(after), self.next_row(after.id))
        } else {
            (None, Some(after))
        }
    }
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
        let (film, level) = e.flow_after("01narshadda.jkl");
        assert_eq!(film.map(|s| s.file.as_str()), Some("06a.smk"));
        assert_eq!(level.map(|s| s.file.as_str()), Some("02narshadda.jkl"));
        // A level whose successor is another level has no film in between.
        let (none, straight) = e.flow_after("02narshadda.jkl");
        assert!(none.is_none());
        assert!(straight.is_none(), "the test graph ends there");
    }
}
