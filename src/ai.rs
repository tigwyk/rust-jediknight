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
//! The first number on an instinct line is its *weight* in the engine's instinct
//! scheduler, not a period; this build keeps it as data and does not schedule yet.

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
    /// Whether the parameter line carried every field this build reads. The audit counts a
    /// file as incomplete when one is missing, which is how a truncated `.ai` shows up.
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
        if self.fov <= 0.0 {
            return true;
        }
        let facing = yaw.to_radians();
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
pub fn parse(text: &str) -> Ai {
    let mut ai = Ai::default();
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

    #[test]
    fn sight_uses_distance_and_the_cone() {
        let ai = parse(RYB);
        let at = [0.0, 0.0, 0.0];
        // Yaw 0 faces +X. A target 3 units ahead is seen; 8 units is beyond sightdist=5.
        assert!(ai.sees(at, 0.0, [3.0, 0.0, 0.0]));
        assert!(!ai.sees(at, 0.0, [8.0, 0.0, 0.0]));
        // 200 degrees total means 100 either side: 90 degrees off the facing is seen, 170 is not.
        assert!(ai.sees(at, 0.0, [0.0, 3.0, 0.0]));
        assert!(!ai.sees(at, 0.0, [-3.0, 0.1, 0.0]));
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