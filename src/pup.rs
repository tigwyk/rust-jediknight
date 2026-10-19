//! `.pup` puppets — plain text, one per actor, tying a mesh to its animations.
//!
//! ```text
//! #puppet file for Boc (bo.3do) [Anthony 12/20/96]
//! Mode=0                # Idle
//! stand        bostand.key    0x00    0    0    # standing
//! walk        boleap.key    0x01    2    2    # "walk"
//! ...
//! mode=1, basedon=0    # weapon drawn
//! ...
//! joints
//! 0=5    # 0 = head
//! end
//! ```
//!
//! Each non-`mode=` line names a *state*: the animation name the AI asks for, the
//! `.key` file that plays it, a flags word, and the first/last frame of the loop.
//! `mode=` starts a new mode; its rows inherit the previous mode named by `basedon`.

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// Animation name the AI and scripts ask for (`stand`, `walk`, `fire`).
    pub name: String,
    /// `.key` file that plays it, as the file spells it.
    pub key: String,
    pub flags: u32,
    /// Frame range the animation plays, inclusive.
    pub low: u8,
    pub high: u8,
    pub mode: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pup {
    pub states: Vec<State>,
    /// `(joint, node)` pairs from the `joints` block, in index order.
    pub joints: Vec<(u8, u8)>,
    /// Mode count, from the `mode=` lines.
    pub modes: usize,
    /// Whether a `joints` block existed at all — `ra.pup` ships blank on purpose, so
    /// "no content" and "not a puppet" are different answers.
    pub has_joints_block: bool,
}

impl Pup {
    /// Whether the file described an actor at all. `tur1.pup` is joints only — a
    /// turret whose aim joints the AI drives, with no animation of its own.
    pub fn is_puppet(&self) -> bool {
        !self.states.is_empty() || !self.joints.is_empty() || self.has_joints_block
    }

    /// Every state needs a key file. Frame ranges are data, not a check: `ky.pup`
    /// ships `turnleft kyturnr.key 3..2`, a one-frame range authored backwards.
    pub fn complete(&self) -> bool {
        self.states.iter().all(|s| !s.key.is_empty())
    }
}

/// Parse one `.pup`. Never fails: an unreadable file yields no states.
pub fn parse(text: &str) -> Pup {
    let mut pup = Pup::default();
    let mut mode = 0usize;
    let mut in_joints = false;

    for raw in text.lines() {
        // `#` starts a comment; the shipped files never quote one.
        let line = match raw.split_once('#') {
            Some((code, _)) => code.trim(),
            None => raw.trim(),
        };
        if line.is_empty() {
            continue;
        }
        if in_joints {
            if line.eq_ignore_ascii_case("end") {
                in_joints = false;
                continue;
            }
            if let Some((joint, node)) = line.split_once('=') {
                if let (Ok(j), Ok(n)) = (joint.trim().parse(), node.trim().parse()) {
                    pup.joints.push((j, n));
                }
            }
            continue;
        }
        if line.eq_ignore_ascii_case("joints") {
            in_joints = true;
            pup.has_joints_block = true;
            continue;
        }
        if line.to_ascii_lowercase().starts_with("mode=") {
            // `Mode=0` opens mode 0; `mode=1, basedon=0` opens mode 1.
            mode = line[5..]
                .split(',')
                .next()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(mode + 1);
            pup.modes = pup.modes.max(mode + 1);
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let (Some(name), Some(key)) = (tokens.first(), tokens.get(1)) else { continue };
        let flags = tokens
            .get(2)
            .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        let low = tokens.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let high = tokens.get(4).and_then(|v| v.parse().ok()).unwrap_or(low);
        pup.states.push(State {
            name: name.to_ascii_lowercase(),
            key: key.to_string(),
            flags,
            low,
            high,
            mode,
        });
    }
    // A file with states but no `mode=` line still opens mode 0.
    if pup.modes == 0 && !pup.states.is_empty() {
        pup.modes = 1;
    }
    pup
}

#[cfg(test)]
mod tests {
    use super::parse;

    const SHIPPED: &str = "#puppet file for Boc (bo.3do) [Anthony 12/20/96]\n\
        Mode=0\t\t\t\t# Idle\n\
        stand\t\tbostand.key\t0x00\t0\t0\t# standing\n\
        walk\t\tboleap.key\t0x01\t2\t2\t# \"walk\"\n\
        death\t\tbodie.key\t0x14\t5\t5\t# die\n\
        mode=1, basedon=0\t# weapon drawn\n\
        stand\t\tbostand.key\t0x00\t0\t0\n\
        \n\
        joints\n\
        0=5\t# 0 = head\n\
        1=13\t# 1 = neck\n\
        end\n\
        ";

    #[test]
    fn reads_a_puppet() {
        let pup = parse(SHIPPED);
        assert!(pup.is_puppet());
        assert!(pup.complete());
        assert_eq!(pup.modes, 2);
        assert_eq!(pup.states.len(), 4);
        let walk = pup.states.iter().find(|s| s.name == "walk").unwrap();
        assert_eq!(walk.key, "boleap.key");
        assert_eq!((walk.flags, walk.low, walk.high, walk.mode), (1, 2, 2, 0));
        // The second `stand` belongs to the weapon-drawn mode.
        assert_eq!(pup.states[3].name, "stand");
        assert_eq!(pup.states[3].mode, 1);
        assert_eq!(pup.joints, vec![(0, 5), (1, 13)]);
    }
}