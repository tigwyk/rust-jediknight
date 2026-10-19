//! `.key` keyframe animation — plain text, same authoring pipeline as `.3do`.
//!
//! The header carries the frame count and rate; the body is one block per animated
//! node, each naming the mesh it poses and listing its per-frame transform:
//!
//! ```text
//! SECTION: HEADER
//! FLAGS  0x0000
//! TYPE   0xFFFF
//! FRAMES 109
//! FPS    15.000
//! JOINTS 1
//!
//! SECTION: KEYFRAME NODES
//! NODES 1
//! NODE    0
//! MESH NAME tie~bmb
//! ENTRIES 109
//! # num:   frame:   flags:      x:      y:      z:      p:      y:      r:
//!    0:        0   0x0001  0.0005 -0.0008  0.0001  0.0 0.0 0.0
//!                          0.0 0.0 0.0224 0.0 0.0 0.0
//! ```
//!
//! The continuation line is the per-frame delta (dx dy dz dp dy dr); some files omit
//! it, so it is absorbed only when the following line has six bare numbers.

/// One animation entry: the node's transform at one frame, plus its delta.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub frame: u32,
    pub flags: u32,
    pub pos: [f32; 3],
    pub rot: [f32; 3],
    pub delta: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Mesh the node poses, as the file spells it (`tie~bmb`).
    pub mesh: String,
    pub frames: Vec<Frame>,
    /// What the file says this node carries, so a truncated parse is detectable.
    pub declared_entries: usize,
}

impl Node {
    /// The pose at a fractional frame: the last entry at or before it. Posing steps
    /// between entries without interpolating, which is what a 15 fps puppet looks like
    /// when the renderer runs faster than the animation.
    pub fn at(&self, frame: f32) -> Option<&Frame> {
        self.frames.iter().rev().find(|f| f.frame as f32 <= frame).or_else(|| self.frames.first())
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Key {
    pub frames: usize,
    pub fps: f32,
    pub joints: usize,
    /// What the header says the node count is.
    pub declared_nodes: usize,
    pub nodes: Vec<Node>,
}

impl Key {
    /// Whether this keyframe animates anything: a file with nodes and entries.
    pub fn is_drawable(&self) -> bool {
        self.frames > 0 && self.nodes.iter().any(|n| !n.frames.is_empty())
    }

    /// Total entries across all nodes, for the coverage report.
    pub fn entries(&self) -> usize {
        self.nodes.iter().map(|n| n.frames.len()).sum()
    }

    /// Whether every node's entry block came out as long as the file claims. A false
    /// here means the parser lost rows, not that the file is odd.
    pub fn complete(&self) -> bool {
        self.declared_nodes == self.nodes.len()
            && self.nodes.iter().all(|n| n.frames.len() == n.declared_entries)
    }
}

fn numbers(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

fn six(tokens: &[&str]) -> Option<[f32; 3]> {
    let mut out = [0.0f32; 3];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = tokens.get(i)?.parse().ok()?;
    }
    Some(out)
}

/// Parse a `.key` file. Never fails: a malformed file simply yields no nodes, which
/// the coverage audit reports as undecoded rather than crashing a level sweep.
pub fn parse(text: &str) -> Key {
    let mut key = Key::default();
    let mut lines = text.lines().peekable();

    while let Some(raw) = lines.next() {
        let line = raw.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let tokens = numbers(line);

        // Header scalars. `FRAMES`/`FPS`/`JOINTS` only appear in the header, so they
        // need no section tracking.
        match tokens.first().copied() {
            Some("FRAMES") => {
                key.frames = tokens.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                continue;
            }
            Some("FPS") => {
                key.fps = tokens.get(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                continue;
            }
            Some("JOINTS") => {
                key.joints = tokens.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                continue;
            }
            Some("NODES") => {
                key.declared_nodes = tokens.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                continue;
            }
            Some("ENTRIES") => {
                if let Some(node) = key.nodes.last_mut() {
                    node.declared_entries =
                        tokens.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                }
                continue;
            }
            Some("MESH") if tokens.get(1) == Some(&"NAME") => {
                key.nodes.push(Node {
                    mesh: tokens.get(2).copied().unwrap_or_default().to_string(),
                    frames: Vec::new(),
                    declared_entries: 0,
                });
                continue;
            }
            _ => {}
        }

        // An entry line starts with `<index>:`, then frame, flags and six floats.
        let Some(first) = tokens.first() else { continue };
        if !first.ends_with(':') {
            // A continuation line: six bare numbers are the previous entry's delta.
            if tokens.len() == 6 {
                if let (Some(delta), Some(node)) = (six(&tokens), key.nodes.last_mut()) {
                    if let Some(frame) = node.frames.last_mut() {
                        frame.delta = delta;
                    }
                }
            }
            continue;
        }
        let (Some(frame), Some(flags), Some(pos), Some(rot)) = (
            tokens.get(1).and_then(|v| v.parse::<u32>().ok()),
            tokens.get(2).and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok()),
            six(&tokens[3.min(tokens.len())..]),
            six(&tokens[6.min(tokens.len())..]),
        ) else {
            continue;
        };
        let node = match key.nodes.last_mut() {
            Some(n) => n,
            None => continue,
        };
        node.frames.push(Frame { frame, flags, pos, rot, delta: [0.0; 3] });
    }

    // Files that declare the frame count but no per-node block still describe a length.
    key
}

#[cfg(test)]
mod tests {
    use super::parse;

    const SHIPPED: &str = "# KEYFRAME '01VTieB.key' created from 'D:\\Jedi\\...'\n\
        \n\
        ###############\n\
        SECTION: HEADER\n\
        \n\
        FLAGS  0x0000\n\
        TYPE   0xFFFF\n\
        FRAMES 3\n\
        FPS    15.000\n\
        JOINTS 1\n\
        \n\
        SECTION: KEYFRAME NODES\n\
        \n\
        NODES 1\n\
        \n\
        NODE    0\n\
        MESH NAME tie~bmb\n\
        ENTRIES 3\n\
        \n\
        # num:   frame:   flags:           x:\n\
           0:        0   0x0001   0.0005  -0.0008   0.0001   0.0  0.0  0.0\n\
                                 0.0  0.0  0.0224  0.0  0.0  0.0\n\
           1:        1   0x0003   0.0005  -0.0008   0.0225   0.0  0.0  0.0\n\
                                 0.0  0.0  0.0181  -0.04  0.0  0.0\n\
           2:        2   0x0003   0.0005  -0.0008   0.0406  359.9 0.0  0.0\n\
        ";

    #[test]
    fn reads_a_keyframe_file() {
        let key = parse(SHIPPED);
        assert_eq!((key.frames, key.fps as u32, key.joints), (3, 15, 1));
        assert!(key.is_drawable());
        assert_eq!(key.nodes.len(), 1);
        let node = &key.nodes[0];
        assert_eq!(node.mesh, "tie~bmb");
        assert_eq!(node.frames.len(), 3);
        assert_eq!(node.frames[1].pos[2], 0.0225);
        // The continuation line is the delta, when the file carries one.
        assert_eq!(node.frames[0].delta[2], 0.0224);
        assert_eq!(node.frames[2].delta, [0.0; 3]);
        assert_eq!(key.entries(), 3);
        assert!(key.complete());
    }
}
