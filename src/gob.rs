//! GOB archive reader.
//!
//! Format recovered from the shipped archives and corroborated by the engine's
//! own diagnostics in `JediKnight.EXE` (`stdGob.c`: "GOB " magic, version check,
//! "Bad signature in header of gob file").
//!
//! Layout (little endian):
//!   char   magic[4]   = "GOB "
//!   u32    version    = 20
//!   u32    unknown    = 12 in every shipped archive
//!   u32    count
//!   Entry  entries[count]      (at offset 16)
//!   u8     data[]              (first entry's offset == 16 + count*136)
//!
//! Entry:
//!   u32  offset   absolute offset in the archive
//!   u32  size
//!   char name[128]  NUL-terminated, `dir\subdir\file.ext`

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Entry {
    pub archive: usize,
    pub offset: u64,
    pub size: u32,
}

pub struct GobSet {
    /// The install root, so a name the archives do not hold can come from a loose file:
    /// the engine's own VFS checks the filesystem, and this install keeps 483 MB of
    /// cutscenes there (`Resource\VIDEO\*.SMK`) rather than inside a `.gob`.
    root: PathBuf,
    archives: Vec<PathBuf>,
    /// lowercased archive path -> every entry declaring it, in archive order.
    /// The same name can appear in several archives (`episode.jk` exists once per
    /// episode); lookups use the last, matching the game's later-archive-wins
    /// load order, while `all_named` lets callers see every occurrence.
    index: HashMap<String, Vec<Entry>>,
    /// lowercased file name (no directories) -> first matching entry
    by_name: HashMap<String, Entry>,
}

impl GobSet {
    /// Index every `.gob` under `root` (recursively).
    pub fn open(root: &Path) -> std::io::Result<Self> {
        let mut archives = Vec::new();
        collect_gobs(root, &mut archives)?;
        archives.sort();

        let mut index = HashMap::new();
        let mut by_name = HashMap::new();
        for (i, path) in archives.iter().enumerate() {
            let data = fs::read(path)?;
            if data.len() < 16 || &data[0..4] != b"GOB " {
                continue;
            }
            let count = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
            for e in 0..count {
                let base = 16 + e * 136;
                if base + 136 > data.len() {
                    break;
                }
                let offset = u32::from_le_bytes(data[base..base + 4].try_into().unwrap()) as u64;
                let size = u32::from_le_bytes(data[base + 4..base + 8].try_into().unwrap());
                let raw = &data[base + 8..base + 136];
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                let name = String::from_utf8_lossy(&raw[..end]).to_lowercase();
                if name.is_empty() {
                    continue;
                }
                let entry = Entry { archive: i, offset, size };
                by_name.entry(name.rsplit('\\').next().unwrap().to_string()).or_insert(Entry {
                    archive: i,
                    offset,
                    size,
                });
                index.entry(name).or_insert_with(Vec::new).push(entry);
            }
        }
        Ok(Self { root: root.to_path_buf(), archives, index, by_name })
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Every indexed path, sorted, for exhaustive sweeps such as `--audit`.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.index.keys().cloned().collect();
        names.sort();
        names
    }

    /// Every entry declaring `name`, in archive order.
    pub fn all_named(&self, name: &str) -> &[Entry] {
        let key = name.trim_start_matches(['/', '\\']).replace('/', "\\").to_lowercase();
        self.index.get(&key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn archive_path(&self, archive: usize) -> &std::path::Path {
        &self.archives[archive]
    }

    pub fn archive_count(&self) -> usize {
        self.archives.len()
    }

    /// Look up `name`: as a full `dir\file` path first, then by bare file name.
    /// JK's text files reference materials by bare name (`hdback.mat`) while the
    /// archive stores them under a directory (`3do\mat\hdback.mat`).
    pub fn find(&self, name: &str) -> Option<&Entry> {
        let key = name.trim_start_matches(['/', '\\']).replace('/', "\\").to_lowercase();
        self.index
            .get(&key)
            .and_then(|v| v.last())
            .or_else(|| self.by_name.get(key.rsplit('\\').next().unwrap_or(&key)))
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        if let Some(entry) = self.find(name) {
            return self.read_entry(entry);
        }
        self.read_loose(name)
    }

    /// A loose file under the install root, which is where this build keeps the cutscenes.
    /// The name arrives in the archives' own form (`resource\video\33-34a.smk`), so the
    /// lookup tries it as given and then with Windows' own case-insensitivity applied by
    /// the filesystem — a miss on either is reported as absent, not guessed at.
    fn read_loose(&self, name: &str) -> Option<Vec<u8>> {
        let rel = name.trim_start_matches(['/', '\\']).replace('\\', "/");
        if rel.is_empty() || rel.contains("..") {
            return None;
        }
        let direct = self.root.join(&rel);
        if let Ok(bytes) = fs::read(&direct) {
            return Some(bytes);
        }
        // Archives store lowercased names; the install uses upper-case names, which Windows
        // resolves either way. Other hosts need the case-insensitive walk, per mount.
        for prefix in ["", "Resource", "Resource/VIDEO"] {
            let mut path = if prefix.is_empty() {
                self.root.clone()
            } else {
                self.root.join(prefix)
            };
            let mut walked = true;
            for part in rel.split('/') {
                let Ok(entries) = fs::read_dir(&path) else {
                    walked = false;
                    break;
                };
                let mut found = None;
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy().eq_ignore_ascii_case(part) {
                        found = Some(entry.path());
                        break;
                    }
                }
                match found {
                    Some(next) => path = next,
                    None => {
                        walked = false;
                        break;
                    }
                }
            }
            if walked && let Ok(bytes) = fs::read(&path) {
                return Some(bytes);
            }
        }
        None
    }

    pub fn read_entry(&self, entry: &Entry) -> Option<Vec<u8>> {
        let file = fs::File::open(&self.archives[entry.archive]).ok()?;
        read_at(&file, entry.offset, entry.size as usize)
    }
}

fn read_at(file: &fs::File, offset: u64, size: usize) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file.try_clone().ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; size];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn collect_gobs(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_gobs(&path, out)?;
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gob"))
        {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loose file resolves through the install's mounts, case-insensitively — that is how
    /// the 25 shipped cutscenes are found (`Resource\VIDEO\01-02A.SMK` for a manifest's
    /// `01-02a.smk`).
    #[test]
    fn a_loose_file_resolves_through_the_resource_mounts() {
        let root = std::env::temp_dir().join(format!("bk-vfs-{}", std::process::id()));
        let video = root.join("Resource").join("VIDEO");
        std::fs::create_dir_all(&video).expect("temp tree");
        std::fs::write(video.join("01-02A.SMK"), b"SMK2payload").expect("temp file");
        let gobs = GobSet::open(&root).expect("no archives");
        // The archives' own spelling: lower case, and no directory.
        let bytes = gobs.read("01-02a.smk").expect("loose file");
        assert_eq!(bytes, b"SMK2payload");
        // An unknown name is still absent rather than invented.
        assert!(gobs.read("no-such.smk").is_none());
        std::fs::remove_dir_all(&root).ok();
    }
}