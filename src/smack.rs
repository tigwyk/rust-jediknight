//! Smacker cutscene decoding through the install's own `SMACKW32.DLL`.
//!
//! JK.EXE carries no decoder: it imports nine ordinals from `SMACKW32.DLL` and plays through
//! `FUN_00426730` (see `docs/formats.md` §16.9). A drop-in replacement can do the same — the DLL
//! ships in the install the user already owns, so the cutscene path is byte-identical by
//! construction instead of a reimplementation.
//!
//! **The bitness boundary.** `SMACKW32.DLL` is a 32-bit image (`PE machine 0x014C`), so a
//! 64-bit process cannot load it: `LoadLibrary` fails with `ERROR_BAD_EXE_FORMAT` (193). The
//! decode therefore runs in `tools/smkdec`, a dependency-free 32-bit helper built with
//! `cargo build --release --target i686-pc-windows-msvc --manifest-path tools/smkdec/Cargo.toml`,
//! and this module only spawns it and reads its report. That is the whole reason the helper
//! exists; the game itself stays 64-bit.

use std::path::{Path, PathBuf};

/// Where the helper lands when built with the command in this module's docs.
///
/// `cwd` is the documented case (`cargo run` from the repo, or the built binary started there), but
/// a binary launched anywhere else looked for the helper in *its own* working directory and silently
/// played no films. `BK_SMKDEC` names it explicitly; otherwise the executable's directory and up to
/// three parents are searched, so a build under `target/release/` finds its own repo checkout.
pub fn shim_path() -> Option<PathBuf> {
    let rel = Path::new("tools/smkdec/target/i686-pc-windows-msvc/release/smkdec.exe");
    if let Some(explicit) = std::env::var_os("BK_SMKDEC") {
        let candidate = PathBuf::from(explicit);
        return candidate.exists().then_some(candidate);
    }
    if rel.exists() {
        return Some(rel.to_path_buf());
    }
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    for _ in 0..4 {
        let candidate = dir.join(rel);
        if candidate.exists() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
    None
}

/// One decoded cutscene run.
pub struct Decoded {
    /// The per-frame checksums the helper printed, in order.
    pub hashes: Vec<u64>,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    /// How many of those checksums were distinct — a decoder that repeats one frame would be
    /// broken, and a fades-from-black opening looks like that for the first frames.
    pub distinct: usize,
    /// The first frame that was not entirely zero.
    pub first_ink: Option<usize>,
}

/// Decode `frames` frames of `file` through `dll`, starting at `start`, using the 32-bit helper.
pub fn decode(
    dll: &Path,
    file: &Path,
    frames: usize,
    start: usize,
    out_prefix: &Path,
) -> Option<Decoded> {
    let shim = shim_path()?;
    let output = std::process::Command::new(&shim)
        .arg(dll)
        .arg(file)
        .arg(out_prefix)
        .arg(frames.to_string())
        .arg(start.to_string())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    if std::env::var("BK_SMACK_TRACE").is_ok() {
        eprintln!("smack trace: stdout {} bytes, stderr {} bytes, status {:?}",
            output.stdout.len(), output.stderr.len(), output.status.code());
        for line in text.lines().take(3) {
            eprintln!("  | {line}");
        }
    }
    let mut hashes = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("frame ")
            && let Some((_, hash)) = rest.split_once(": ")
            && let Ok(value) = u64::from_str_radix(hash.trim(), 16)
        {
            hashes.push(value);
        }
    }
    // The helper's summary line: `decoded N frame(s) at WxH, D of them distinct, first non-blank
    // at Some(K), F in the file`.
    let summary = text.lines().find(|l| l.starts_with("decoded "))?;
    let numbers: Vec<&str> = summary.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).collect();
    let width = numbers.get(1).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let height = numbers.get(2).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    // The last number on the summary line is the frame count the *file* declares.
    let frames = numbers.last().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let mut distinct = hashes.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let first_ink = summary
        .split("first non-blank at Some(")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .and_then(|v| v.parse::<usize>().ok());
    Some(Decoded {
        hashes,
        width,
        height,
        frames,
        distinct: distinct.len(),
        first_ink,
    })
}