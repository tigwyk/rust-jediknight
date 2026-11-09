//! The `.ai` files: what an actor's instincts are.
//!
//! Plaintext, one parameter line and one line per instinct:
//!
//! ```text
//! alignment=-1.0, rank=0.5, fov=200, maxstep=0.4, sightdist=5.0, heardist=2.0, accuracy=0.10
//!
//! Jump                2000.0, 0.2, 0.2
//! LookForTarget       500.0, 40000.0
//! PrimaryFire         1250.0, 0.8, 4.0, 0.20, 0.0, 400.0, 0.00, 0.00, 1.0, 500.0
//! ```
//!
//! The first number on an instinct line is the instinct's **own first argument**, whatever its
//! handler reads it as: `LookForTarget`'s scan interval (ms), `listen`'s reaction chance (0-1),
//! `retreat`'s health fraction, `hitandrun`'s hold time. This build keeps it in
//! [`Instinct::weight`] — the name the first parser gave it — and does not run the engine's
//! instinct scheduler.

use std::collections::BTreeMap;

/// One instinct: a name, its weight, and the numbers after it.
#[derive(Debug, Clone, PartialEq)]
pub struct Instinct {
    pub name: String,
    pub weight: f32,
    pub args: Vec<f32>,
}

/// A parsed `.ai`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ai {
    pub alignment: f32,
    pub rank: f32,
    pub fov: f32,
    pub maxstep: f32,
    pub sightdist: f32,
    pub heardist: f32,
    pub accuracy: f32,
    /// The instincts in file order, with their weights.
    pub instincts: Vec<Instinct>,
    /// Every `key=value` the parameter line carried, in case one is worth reading later.
    pub params: BTreeMap<String, f32>,
}

impl Ai {
    /// Whether the parameter line spelled out every field this build reads. It is a
    /// **fully-specified** check, not a validity one: `sithAIClass_LoadEntry` gives every field a
    /// default before it reads the line, and `parse` applies the same defaults, so a class that
    /// omits a key is a normal shipped class. The audit reports the count as "takes the loader's
    /// defaults", which is how a file missing a field is noticed without calling it broken.
    pub fn complete(&self) -> bool {
        for key in ["alignment", "rank", "fov", "maxstep", "sightdist", "heardist", "accuracy"] {
            if !self.params.contains_key(key) {
                return false;
            }
        }
        true
    }

    /// Whether the file looks like an AI class at all: a parameter line plus at least one
    /// instinct. Used by `--audit` to tell a real `.ai` from a stub.
    pub fn is_class(&self) -> bool {
        !self.instincts.is_empty() && !self.params.is_empty()
    }
    /// The instinct with this name, if the file lists it.
    pub fn instinct(&self, name: &str) -> Option<&Instinct> {
        self.instincts.iter().find(|i| i.name.eq_ignore_ascii_case(name))
    }

    /// The distance the chase keeps from its target: `Follow`'s **second** argument, which the
    /// shipped classes set per archetype (`rydefault` 0.75, `grdefault` 2.0). `None` when the class
    /// has no usable `Follow` line.
    pub fn standoff(&self) -> Option<f32> {
        // The parser keeps an instinct line's first number as `weight` (the engine's `fltArg[0]`)
        // and the rest in `args`, so `Follow`'s second argument is `args[0]`.
        self.instinct("Follow")
            .and_then(|follow| follow.args.first())
            .map(|distance| *distance as f32)
            .filter(|distance| *distance > 0.0)
    }

    /// Whether the actor can see a point: inside `sightdist` and inside the cone.
    ///
    /// `fov=` is read as a **total** cone in degrees (`rybdefault.ai` says 200), which is what
    /// the field's name suggests; the engine's own use of it is not recovered, so a value over
    /// 360 is clamped rather than wrapping.
    pub fn sees(&self, from: [f32; 3], yaw: f32, to: [f32; 3]) -> bool {
        let dx = to[0] - from[0];
        let dy = to[1] - from[1];
        let dz = to[2] - from[2];
        let reach = (dx * dx + dy * dy + dz * dz).sqrt();
        if reach <= 0.001 || reach > self.sightdist.max(0.001) {
            return false;
        }
        // `sithAI_CheckSightThing` (`0x4eb090`) applies the cone only when its angle argument is
        // **positive**, and that argument is the cosine of the half angle: a class whose `fov=` is
        // 180 has `cos 90 = 0` and is therefore not coned at all. A point-blank actor facing away
        // (level 1's `reeyeesgun` at 0.34 units, cone `false`, `off 107`) sees the body in retail.
        if self.fov <= 0.0 || self.fov >= 180.0 {
            return true;
        }
        // The engine's facing is `forward = (-sin yaw, cos yaw)` (`sithCamera`'s own frame, held by
        // the port's `yaw_axes`), so the facing direction is yaw turned a quarter turn.
        let facing = yaw.to_radians() + std::f32::consts::FRAC_PI_2;
        let target = dy.atan2(dx);
        let mut delta = (target - facing).to_degrees().abs() % 360.0;
        if delta > 180.0 {
            delta = 360.0 - delta;
        }
        delta <= (self.fov.min(360.0) / 2.0)
    }

}

/// Parse one `.ai` file. Unknown lines are ignored rather than rejected: the shipped files
/// carry comments and blank lines, and an instinct this build cannot use is still data.
///
/// The fields start at the **engine's own defaults**, not zero: `sithAIClass_LoadEntry`
/// (`decomp/004f15c0`) writes 0.5 to `maxstep`, 20.0 to `sightdist`, 10.0 to `heardist` and 0.5 to
/// `accuracy` before it reads the parameter line, and only overwrites the keys a file carries.
/// Ninety-one shipped classes omit at least one key (four omit `accuracy`), so a zero default made
/// them blind or perfectly accurate where the engine gives them its defaults.
pub fn parse(text: &str) -> Ai {
    let mut ai = Ai {
        maxstep: 0.5,
        sightdist: 20.0,
        heardist: 10.0,
        accuracy: 0.5,
        ..Ai::default()
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // The parameter line is the one with `=`.
        if line.contains('=') {
            for pair in line.split(',') {
                let Some((key, value)) = pair.split_once('=') else { continue };
                let key = key.trim().to_ascii_lowercase();
                let Ok(value) = value.trim().parse::<f32>() else { continue };
                ai.params.insert(key.clone(), value);
                match key.as_str() {
                    "alignment" => ai.alignment = value,
                    "rank" => ai.rank = value,
                    "fov" => ai.fov = value,
                    "maxstep" => ai.maxstep = value,
                    "sightdist" => ai.sightdist = value,
                    "heardist" => ai.heardist = value,
                    "accuracy" => ai.accuracy = value,
                    _ => {}
                }
            }
            continue;
        }
        // An instinct line: a name, then numbers.
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let nums: Vec<f32> = parts
            .map(|p| p.trim_end_matches(',').parse::<f32>().ok())
            .take_while(|v| v.is_some())
            .flatten()
            .collect();
        if nums.is_empty() {
            ai.instincts.push(Instinct { name: name.to_string(), weight: 0.0, args: Vec::new() });
        } else {
            ai.instincts.push(Instinct {
                name: name.to_string(),
                weight: nums[0],
                args: nums[1..].to_vec(),
            });
        }
    }
    ai
}

#[cfg(test)]
mod tests {
    use super::*;

    const RYB: &str = "\
# comment
alignment=-1.0, rank=0.5, fov=200, maxstep=0.4, sightdist=5.0, heardist=2.0, accuracy=0.10

Jump\t\t\t\t2000.0, 0.2, 0.2
LookForTarget   \t500.0, 40000.0
Listen          \t0.50, 2.0
PrimaryFire     \t1250.0, 0.8, 4.0, 0.20, 0.0, 400.0, 0.00, 0.00, 1.0, 500.0
OpenDoors
";

    #[test]
    fn parses_the_parameters_and_every_instinct() {
        let ai = parse(RYB);
        assert_eq!(ai.sightdist, 5.0);
        assert_eq!(ai.fov, 200.0);
        assert_eq!(ai.accuracy, 0.10);
        assert_eq!(ai.instincts.len(), 5);
        assert_eq!(ai.instinct("LookForTarget").unwrap().weight, 500.0);
        assert_eq!(ai.instinct("PrimaryFire").unwrap().args.len(), 9);
        // A bare instinct (no numbers) still exists.
        assert_eq!(ai.instinct("OpenDoors").unwrap().weight, 0.0);
    }

    /// `Follow`'s second argument is the chase standoff: 0.75 units for a Ree-Yees, 2.0 for a
    /// Greedo. The port used a fixed 0.25 for every class, so the room's friends walked onto the
    /// player and absorbed the bolts aimed at it.
    #[test]
    fn follow_distance_is_the_chase_standoff() {
        // A `.ai` file is a real text file, so the parser wants real newlines here.
        let reeyees = "alignment=-1.0, rank=0.5\nFollow\t0.0, 0.75\n";
        assert_eq!(parse(reeyees).standoff(), Some(0.75));
        let greedo = "alignment=-1.0, rank=0.5\nFollow\t0.5, 2.0\n";
        assert_eq!(parse(greedo).standoff(), Some(2.0));
        assert_eq!(parse(RYB).standoff(), None, "the fixture has no Follow line");
        // A zero standoff would put the chase on top of the target; it is not used.
        assert_eq!(parse("Follow 0.0, 0.0\n").standoff(), None);
    }

    /// A key a class omits takes the **engine's** default, not zero: `sithAIClass_LoadEntry`
    /// (`decomp/004f15c0`) sets `maxstep` 0.5, `sightdist` 20.0, `heardist` 10.0 and `accuracy` 0.5
    /// before reading the parameter line, and 91 shipped classes omit at least one key (four omit
    /// `accuracy`). A zero default made those actors blind (sightdist 0) and perfectly accurate.
    #[test]
    fn omitted_parameters_take_the_engines_defaults() {
        let sparse = parse("alignment=-1.0, rank=0.5, fov=180\n\nLookForTarget 500.0, 30000.0\n");
        assert_eq!(sparse.sightdist, 20.0, "the loader's sightdist default");
        assert_eq!(sparse.heardist, 10.0, "the loader's heardist default");
        assert_eq!(sparse.maxstep, 0.5, "the loader's maxstep default");
        assert_eq!(sparse.accuracy, 0.5, "the loader's accuracy default");
        // A file that carries the keys still wins over the defaults.
        assert_eq!(parse(RYB).sightdist, 5.0);
        assert_eq!(parse(RYB).accuracy, 0.10);
    }

    #[test]
    fn sight_uses_distance_and_the_cone() {
        // 200 degrees has a *negative* cosine half angle, so the engine's cone gate
        // (`sithAI_CheckSightThing` applies it only when the cosine is positive) never fires:
        // such a class sees all round. A coned class is what exercises the cone.
        let wide = parse(RYB);
        let at = [0.0, 0.0, 0.0];
        assert!(wide.sees(at, 0.0, [3.0, 0.0, 0.0]));
        assert!(!wide.sees(at, 0.0, [8.0, 0.0, 0.0]), "beyond sightdist=5");
        assert!(wide.sees(at, 0.0, [-3.0, 0.1, 0.0]), "200 degrees is all round in the engine");

        let coned = parse(&RYB.replace("fov=200", "fov=90"));
        // The engine's facing is `forward = (-sin yaw, cos yaw)`: yaw 0 looks along +Y, which is
        // what retail's own door gunner does when it fires down +Y at yaw 0. 90 degrees total means
        // 45 either side: 40 degrees off that axis is seen, 50 is not.
        assert!(coned.sees(at, 0.0, [2.5, 3.0, 0.0]));
        assert!(!coned.sees(at, 0.0, [3.0, 0.0, 0.0]), "yaw 0 must not look along +X");
        assert!(!coned.sees(at, 0.0, [0.0, -3.0, 0.0]));
        // Yaw -90 turns the same cone onto +X.
        assert!(coned.sees(at, -90.0, [3.0, 0.0, 0.0]));
    }

    /// `sithAI_CheckSightThing` measures `sightdist` over the **full** vector: it subtracts the two
    /// positions with `rdVector_Sub3`, not their horizontal projection. So a target straight up is
    /// out of sight sooner than one on the floor at the same flat distance — which is why a guard
    /// cannot see a player on a high ledge at 5 units of climb.
    #[test]
    fn sight_distance_is_measured_in_three_dimensions() {
        let ai = parse(RYB);
        let at = [0.0, 0.0, 0.0];
        // 3 flat, 4 up: exactly sightdist=5, and still ahead (fov 200).
        assert!(ai.sees(at, 0.0, [3.0, 0.0, 4.0]), "5.0 units exactly is still in sight");
        // The same flat distance with one more unit of climb is out.
        assert!(!ai.sees(at, 0.0, [3.0, 0.0, 4.5]), "5.4 units is beyond a 5.0 sightdist");
        assert!(ai.sees(at, 0.0, [3.0, 0.0, 0.0]), "3 units on the floor stays in sight");
    }

}