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

    /// Whether the actor can see a point: inside `sightdist` and inside the cone.
    ///
    /// The cone here is the engine's own sideways-component gate ([`Self::cone`]); the plain
    /// `sightdist` range is this port's convenience for the callers that only need a rough
    /// "in view" answer (`sithAI_CheckSightThing` itself removes the target's radius and is
    /// ported as `Runtime::check_sight_thing`).
    pub fn sees(&self, from: [f32; 3], yaw: f32, to: [f32; 3]) -> bool {
        let dx = to[0] - from[0];
        let dy = to[1] - from[1];
        let dz = to[2] - from[2];
        let reach = (dx * dx + dy * dy + dz * dz).sqrt();
        if reach <= 0.001 || reach > self.sightdist.max(0.001) {
            return false;
        }
        let (sin, cos) = yaw.to_radians().sin_cos();
        // The engine's frame rows: `lvec = (-sin yaw, cos yaw)` is the facing (`yaw_axes`), and
        // `rvec = (cos yaw, sin yaw)` the row `sithAI_CheckSightThing` takes the sideways
        // component over.
        self.cone([-sin, cos, 0.0], [cos, sin, 0.0], [dx / reach, dy / reach, dz / reach])
    }

    /// The class's `fov=` as the engine stores it: `sithAIClass_LoadEntry` (`0x4f15c0`) writes
    /// `cos(fov · 0.5)` into `class+0x1c`, the multiplier `_DAT_00523128 = 0.5` in the shipped
    /// image. The word is a **cosine**, negative for any class wider than 180 degrees, and that
    /// sign is what picks the cone branch below.
    pub fn fovcos(&self) -> f32 {
        (self.fov * 0.5).to_radians().cos()
    }

    /// The class's horizontal view gate, exactly `sithAI_CheckSightThing`'s cone block
    /// (`0x4eb090`): a **sideways-component** test, not an angle test.
    ///
    /// - `fovcos <= -1` (fov 360 or more): no cone at all.
    /// - `fovcos >= 0`: reject the hemisphere behind the facing (`lvec·d < 0`), then reject
    ///   `1 − fovcos < |rvec·d|`. A 90-degree class accepts only `asin(1 − cos 45°) ≈ 17°` either
    ///   side of its axis — far narrower than the 45° an angle reading suggests.
    /// - `fovcos < 0`: reject only targets **behind** whose `|rvec·d| < fovcos + 1`, which leaves
    ///   a blind wedge dead astern. A 200-degree class does not see all round.
    pub fn cone(&self, lvec: [f32; 3], rvec: [f32; 3], dir: [f32; 3]) -> bool {
        cone_holds(self.fovcos(), lvec, rvec, dir)
    }

}

/// `sithAI_CheckSightThing`'s cone block (`0x4eb090`): a **sideways-component** test, not an
/// angle test. `fovcos` is the class word `cos(fov · 0.5)` (`sithAIClass_LoadEntry` `0x4f15c0`).
///
/// - `fovcos <= -1` (fov 360 or more): no cone at all.
/// - `fovcos >= 0`: reject the hemisphere behind the facing (`lvec·d < 0`), then reject
///   `1 − fovcos < |rvec·d|`. A 90-degree class accepts only `asin(1 − cos 45°) ≈ 17°` either
///   side of its axis — far narrower than the 45° an angle reading suggests.
/// - `fovcos < 0`: reject only targets **behind** whose `|rvec·d| < fovcos + 1`, which leaves
///   a blind wedge dead astern. A 200-degree class does not see all round.
pub fn cone_holds(fovcos: f32, lvec: [f32; 3], rvec: [f32; 3], dir: [f32; 3]) -> bool {
    if fovcos <= -1.0 {
        return true;
    }
    let lateral = (rvec[0] * dir[0] + rvec[1] * dir[1] + rvec[2] * dir[2]).abs();
    let ahead = lvec[0] * dir[0] + lvec[1] * dir[1] + lvec[2] * dir[2];
    if fovcos >= 0.0 {
        if ahead < 0.0 {
            return false;
        }
        if 1.0 - fovcos < lateral {
            return false;
        }
    } else if ahead < 0.0 && lateral < fovcos + 1.0 {
        return false;
    }
    true
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
        // fov 200 has a *negative* fovcos, so the cone rejects only the rear wedge
        // (`|rvec·d| < fovcos + 1 = 0.8264` dead astern): a wide class sees almost all round.
        let wide = parse(RYB);
        let at = [0.0, 0.0, 0.0];
        assert!(wide.sees(at, 0.0, [3.0, 0.0, 0.0]));
        assert!(!wide.sees(at, 0.0, [8.0, 0.0, 0.0]), "beyond sightdist=5");
        assert!(wide.sees(at, 0.0, [-3.0, 0.1, 0.0]), "level with the axis is ahead, seen");

        let coned = parse(&RYB.replace("fov=200", "fov=90"));
        // The engine's facing is `forward = (-sin yaw, cos yaw)`: yaw 0 looks along +Y, which is
        // what retail's own door gunner does when it fires down +Y at yaw 0. The sideways gate is
        // `|rvec·d| <= 1 - cos 45° = 0.2929`, i.e. asin(0.2929) ≈ 17° either side of the axis —
        // not the 45° the fov number suggests.
        let off = |deg: f32, dist: f32| [dist * deg.to_radians().sin(), dist * deg.to_radians().cos(), 0.0];
        assert!(coned.sees(at, 0.0, off(15.0, 3.0)), "15 degrees off axis is inside the sideways gate");
        assert!(!coned.sees(at, 0.0, off(20.0, 3.0)), "20 degrees off axis fails |rvec·d| <= 0.2929");
        assert!(!coned.sees(at, 0.0, [3.0, 0.0, 0.0]), "yaw 0 must not look along +X");
        assert!(!coned.sees(at, 0.0, [0.0, -3.0, 0.0]));
        // Yaw -90 turns the same cone onto +X.
        assert!(coned.sees(at, -90.0, [3.0, 0.0, 0.0]));
    }

    /// `sithAIClass_LoadEntry` (`0x4f15c0`) stores `fov=` as `cos(fov · 0.5)` — the multiplier
    /// `_DAT_00523128 = 0.5` in the shipped image — so the class word is a cosine, negative for
    /// any class wider than 180 degrees.
    #[test]
    fn fovcos_is_the_cosine_of_half_the_total_fov() {
        assert!((parse(RYB).fovcos() - 100.0f32.to_radians().cos()).abs() < 1e-6);
        assert!(parse(&RYB.replace("fov=200", "fov=180")).fovcos().abs() < 1e-6);
        assert!((parse(&RYB.replace("fov=200", "fov=90")).fovcos() - 45.0f32.to_radians().cos()).abs() < 1e-6);
    }

    /// fov > 180 (`fovcos < 0`): `sithAI_CheckSightThing` rejects only targets behind **and**
    /// within `|rvec·d| < fovcos + 1` of the rear axis — a blind wedge dead astern, not the
    /// all-round sight the port's old angle test gave a 200-degree class.
    #[test]
    fn a_wide_class_keeps_a_rear_blind_wedge() {
        let wide = parse(RYB); // fov 200, fovcos = cos 100° ≈ -0.1736
        let at = [0.0, 0.0, 0.0];
        assert!(!wide.sees(at, 0.0, [0.0, -3.0, 0.0]), "dead astern is inside the rear wedge");
        assert!(wide.sees(at, 0.0, [-3.0, -0.4, 0.0]), "far to the side is outside the wedge");
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