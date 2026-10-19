//! `.uni` string tables — plain text, the game's localised strings.
//!
//! ```text
//! MSGS 456      # <--- DON'T FORGET TO UPDATE THIS COUNT
//!
//! "CREDITS_big_1"         0 "Jedi Knight"
//! "CREDITS_ small_7"     0 "Lead Conceptual Designer"   # note the space in the key
//! END   # !!! DON'T FORGET TO UPDATE THE COUNT AT TOP
//! ```
//!
//! Each row is a quoted key, an unused number, then the quoted text. The `MSGS` count
//! at the top is *not* a reliable check: the shipped files' counts are stale
//! (`jkstrings.uni` declares 900 and carries 787 rows), so completeness is measured by
//! whether any quoted row failed to parse instead. `misc/cogstrings.uni` is the one the
//! scripts' `jkPrintUNIString` refers to.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Uni {
    /// What the header claims, and what was read. The claim is often stale, so it is
    /// reported but not used as a check.
    pub declared: usize,
    pub strings: Vec<(String, String)>,
    /// Quoted rows the parser could not read; zero in every shipped table.
    pub malformed: usize,
}

impl Uni {
    pub fn is_table(&self) -> bool {
        !self.strings.is_empty()
    }

    pub fn complete(&self) -> bool {
        !self.strings.is_empty() && self.malformed == 0
    }

    /// Case-insensitive lookup, which is how the engine keys them (`M2`, `M_BOSS15`).
    /// Nothing calls this yet — the HUD that would show these strings does not exist —
    /// but the parser is only useful if the table can be looked up.
    #[allow(dead_code)]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.strings
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// The text between the first pair of quotes in `line`, and what follows the closing
/// quote.
fn quoted(line: &str) -> Option<(&str, &str)> {
    let start = line.find('"')? + 1;
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some((&rest[..end], &rest[end + 1..]))
}

/// Parse one `.uni`. Never fails: a file with no rows is not a string table.
pub fn parse(text: &str) -> Uni {
    let mut uni = Uni::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.len() >= 4 && line[..4].eq_ignore_ascii_case("MSGS") {
            uni.declared = line[4..]
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            continue;
        }
        if line.eq_ignore_ascii_case("END") || line.starts_with("END ") {
            break;
        }
        let Some((key, after)) = quoted(line) else {
            uni.malformed += 1;
            continue;
        };
        // The number between the key and the text is unused; the text is the next
        // quoted run, which may be empty.
        let Some((value, _)) = quoted(after) else {
            // A key with no text still exists; the engine shows nothing for it.
            uni.strings.push((key.to_string(), String::new()));
            continue;
        };
        uni.strings.push((key.to_string(), value.to_string()));
    }
    uni
}

#[cfg(test)]
mod tests {
    use super::parse;

    const SHIPPED: &str = "MSGS 4\t  # <--- DON'T FORGET TO UPDATE THIS COUNT\n\
        \n\
        #\t\"<key>\"\t\t<unused number>\t\"<string>\"\n\
        \n\
        \t\"M2\"           \t0\t\"Nar Shaddaa Loading Terminal\"\n\
        \t\"M10\"          \t0\t\"Canyon Oasis\"\n\
        \t\"CREDITS_big_3\"\t0\t\"\"\n\
        \t\"CREDITS_ small_7\"\t0\t\"Lead Conceptual Designer and Storyboards \"\n\
        \n\
        END   # !!! DON'T FORGET TO UPDATE THE COUNT AT TOP\n\
        ";

    #[test]
    fn reads_a_string_table() {
        let uni = parse(SHIPPED);
        assert!(uni.is_table());
        assert!(uni.complete());
        assert_eq!(uni.strings.len(), 4);
        assert_eq!(uni.get("m2"), Some("Nar Shaddaa Loading Terminal"));
        // Empty text is a real entry, not a parse failure.
        assert_eq!(uni.get("CREDITS_big_3"), Some(""));
        // Keys may contain spaces; the trailing space in the text is preserved.
        assert_eq!(
            uni.get("CREDITS_ small_7"),
            Some("Lead Conceptual Designer and Storyboards ")
        );
    }
}