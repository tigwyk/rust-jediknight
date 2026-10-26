//! Finding the retail install.
//!
//! The port ships no game data, so every launch needs the folder the user's own *Jedi Knight*
//! install lives in. That folder used to be a hardcoded developer path; this module resolves it
//! instead: an explicit `--game`, the `BK_GAME` environment variable, a saved `game-path.txt`
//! (beside the program for a portable copy, or in `%APPDATA%\bevi-knight\` for an installed one),
//! the GOG registry entries, and the folders GOG puts games in by default.
//!
//! The installer (`packaging\install.ps1`) uses the same order, so a player normally never types a
//! path at all.

use std::path::{Path, PathBuf};

/// The engine executable names the retail releases use. GOG's is the plaintext `JK.EXE` this port
/// needs; the others are accepted so a folder is recognised rather than misreported.
const EXES: [&str; 3] = ["jk.exe", "jediknight.exe", "jediknight2.exe"];

/// Registry locations GOG has used for the games list. 64-bit Windows puts the 32-bit GOG client
/// under `WOW6432Node`; the other two cover native/32-bit and per-user installs.
const GOG_KEYS: [&str; 3] = [
    r"HKLM\SOFTWARE\WOW6432Node\GOG.com\Games",
    r"HKLM\SOFTWARE\GOG.com\Games",
    r"HKCU\SOFTWARE\GOG.com\Games",
];

/// A resolved install and how it was found, so the caller can say so once.
#[derive(Debug)]
pub struct Found {
    pub path: PathBuf,
    pub source: String,
}

/// Does `dir` look like a *Jedi Knight* install: the engine executable and the `Resource` folder?
/// The executable is what the menu tables are read from and `Resource\` is what the archives sit
/// beside, so this rejects a GOG Games folder that merely contains the title in its path.
pub fn is_install(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let exe = std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            EXES.iter().any(|want| name.eq_ignore_ascii_case(want))
        })
    });
    exe && dir.join("Resource").is_dir()
}

/// `game-path.txt` beside the executable, then the per-user config directory. The first valid one
/// wins, so a portable copy overrides a previous install and vice versa.
fn saved_files() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        out.push((
            dir.join("game-path.txt"),
            "game-path.txt beside the program".to_string(),
        ));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        out.push((
            PathBuf::from(appdata).join("bevi-knight").join("game-path.txt"),
            "%APPDATA%\\bevi-knight\\game-path.txt".to_string(),
        ));
    }
    out
}

/// Where a `--set-game`/installer choice is written.
pub fn config_file() -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA")
        .ok_or("APPDATA is not set, so there is nowhere to save the install path")?;
    Ok(PathBuf::from(appdata).join("bevi-knight").join("game-path.txt"))
}

/// Save `dir` as the install after checking it, and return the file written.
pub fn save(dir: &Path) -> Result<PathBuf, String> {
    if !is_install(dir) {
        return Err(format!(
            "{} is not a Jedi Knight install (no JK.EXE or no Resource\\ folder in it)",
            dir.display()
        ));
    }
    let file = config_file()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    std::fs::write(&file, format!("{}\r\n", dir.display()))
        .map_err(|e| format!("writing {}: {e}", file.display()))?;
    Ok(file)
}

/// One `path` value from `reg query ... /s /v path` output, e.g.
/// `    path    REG_SZ    G:\Games\Star Wars Jedi Knight - Dark Forces 2`.
fn registry_line_path(line: &str) -> Option<PathBuf> {
    let mut words = line.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("path") {
        return None;
    }
    if !words.next()?.eq_ignore_ascii_case("REG_SZ") {
        return None;
    }
    let value = line.split_once("REG_SZ")?.1.trim();
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// GOG's registered install paths, in registry order. `reg.exe` exists on every Windows the port
/// runs on, and this runs once at startup before any game data is touched.
fn registry_candidates() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for key in GOG_KEYS {
        let Ok(output) = std::process::Command::new("reg")
            .args(["query", key, "/s", "/v", "path"])
            .output()
        else {
            continue;
        };
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some(path) = registry_line_path(line) {
                out.push((path, "the GOG registry".to_string()));
            }
        }
    }
    out
}

/// The folders GOG's games normally live in: `C:\GOG Games\<title>` for the offline installer and
/// `...\GOG Galaxy\Games\<title>` for Galaxy, on whichever fixed drive has them. Each root's direct
/// children are checked, which is one level deep by construction.
fn common_candidates() -> Vec<(PathBuf, String)> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for drive in b'C'..=b'Z' {
        let base = PathBuf::from(format!("{}:\\", drive as char));
        roots.push(base.join("GOG Games"));
        roots.push(base.join("Games"));
        roots.push(base.join("Program Files (x86)").join("GOG Galaxy").join("Games"));
        roots.push(base.join("Program Files").join("GOG Galaxy").join("Games"));
    }
    let mut out = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let dir = entry.path();
            if dir.is_dir() && is_install(&dir) {
                out.push((dir, "a usual GOG folder".to_string()));
            }
        }
    }
    out
}

/// Resolve the install: explicit `--game`, then `BK_GAME`, saved path, registry, common folders.
///
/// An explicit path is validated and reported as such rather than silently falling through, so a
/// typo is a clear error instead of a puzzling "no archives" message.
pub fn resolve(explicit: Option<&Path>) -> Result<Found, String> {
    if let Some(dir) = explicit {
        if !is_install(dir) {
            return Err(format!(
                "--game {} is not a Jedi Knight install (expected JK.EXE and a Resource\\ folder in it)",
                dir.display()
            ));
        }
        return Ok(Found { path: dir.to_path_buf(), source: "--game".to_string() });
    }
    if let Some(dir) = std::env::var_os("BK_GAME") {
        let dir = PathBuf::from(dir);
        if is_install(&dir) {
            return Ok(Found { path: dir, source: "the BK_GAME environment variable".to_string() });
        }
    }
    for (file, source) in saved_files() {
        if let Ok(text) = std::fs::read_to_string(&file) {
            let dir = PathBuf::from(text.trim());
            if is_install(&dir) {
                return Ok(Found { path: dir, source });
            }
        }
    }
    for (dir, source) in registry_candidates() {
        if is_install(&dir) {
            return Ok(Found { path: dir, source });
        }
    }
    for (dir, source) in common_candidates() {
        if is_install(&dir) {
            return Ok(Found { path: dir, source });
        }
    }
    Err(
        "no Star Wars Jedi Knight: Dark Forces II install found.\n  \
         Looked at: --game, BK_GAME, game-path.txt next to the program, \
         %APPDATA%\\bevi-knight\\game-path.txt, the GOG registry, and the usual GOG folders.\n  \
         Point the port at the folder that holds JK.EXE:\n    \
         bevi-knight.exe --game \"C:\\GOG Games\\Star Wars Jedi Knight - Dark Forces 2\"\n  \
         or save that choice once:\n    \
         bevi-knight.exe --set-game \"C:\\GOG Games\\Star Wars Jedi Knight - Dark Forces 2\"\n  \
         (packaging\\install.ps1 does the search and the save for you.)"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_install_needs_the_engine_and_the_resource_folder() {
        let dir = std::env::temp_dir().join(format!("bk-gamepath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Resource")).unwrap();
        assert!(!is_install(&dir), "an empty Resource folder is not enough");
        std::fs::write(dir.join("JK.EXE"), b"stub").unwrap();
        assert!(is_install(&dir), "JK.EXE + Resource\\ is a retail layout");
        std::fs::remove_dir_all(dir.join("Resource")).unwrap();
        assert!(!is_install(&dir), "the Resource folder is required");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_queries_are_parsed_to_paths() {
        assert_eq!(
            registry_line_path(
                r"    path    REG_SZ    G:\Games\Star Wars Jedi Knight - Dark Forces 2"
            ),
            Some(PathBuf::from(r"G:\Games\Star Wars Jedi Knight - Dark Forces 2"))
        );
        // Header and unrelated values are skipped, not mistaken for a path.
        assert_eq!(registry_line_path("HKEY_LOCAL_MACHINE\\SOFTWARE\\GOG.com\\Games\\1234"), None);
        assert_eq!(registry_line_path(r"    gameName    REG_SZ    STAR WARS"), None);
    }
}
