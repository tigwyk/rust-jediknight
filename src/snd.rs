//! `.snd` sound classes.
//!
//! Plain ASCII: a table of `<event> <wav> <playflags> <minRadius> <maxRadius>
//! <maxVolume>`, one row per sound the engine may play for an event.
//!
//! ```text
//! # class        filename        playflags    minRadius    maxRadius    maxVolume
//! create        ShotHit02.wav             0x0     0.1     1.5    0.8
//! ```
//!
//! Templates reference these by name (`soundclass=exp_laserhit.snd`), and the first
//! column is the engine event that triggers it (`create`, and further event names
//! in larger tables). Several rows may share an event: the engine picks one at
//! random, which is why a blaster hit has both `ShotHit02.wav` and friends.

use crate::rows::fields;

pub struct Entry {
    /// Engine event that triggers this sound (`create`, …).
    pub event: String,
    pub wav: String,
    /// Raw `playflags` word.
    pub playflags: u32,
    /// Distance over which the sound fades in/out, in world units.
    pub min_radius: f32,
    pub max_radius: f32,
    pub max_volume: f32,
}

/// The mixer's final scale. `sithSoundMixer_UpdateSoundPos` writes
/// `BufferSetVolume(clamp(v, 0, 1) * 0.75)`, so a sound at full volume plays at 0.75.
pub const FINAL_SCALE: f32 = 0.75;
/// How much a sound behind the listener is pulled down: `(1 - -dot * 0.3) * v`.
pub const REAR_PAN: f32 = 0.3;

/// How far into its fade a sound at `distance` is: 0 inside `min_radius`, 1 at
/// `max_radius`, linear between. This is the engine's own ramp
/// (`atan = 1 / (max - min)`, then `clamp((d - min) * atan, 0, 1)`).
pub fn fade(distance: f32, min_radius: f32, max_radius: f32) -> f32 {
    let span = max_radius - min_radius;
    // `minRadius == maxRadius` makes the engine's own ramp multiplier 0, so such a sound
    // never fades at all — it is audible at every distance (475 of the install's 1976 entries).
    if span <= 0.0 {
        return 0.0;
    }
    ((distance - min_radius) / span).clamp(0.0, 1.0)
}

/// A sound's linear volume at `distance`, `forward` being the dot of the listener's facing
/// and the direction to the sound: negative means behind.
pub fn volume(base: f32, distance: f32, min_radius: f32, max_radius: f32, forward: f32) -> f32 {
    let mut v = base;
    if forward < 0.0 {
        v *= 1.0 - -forward * REAR_PAN;
    }
    v *= 1.0 - fade(distance, min_radius, max_radius);
    v.clamp(0.0, 1.0) * FINAL_SCALE
}

impl Entry {
    /// This entry's volume at a distance, using its own shipped radii and `maxVolume`.
    pub fn volume_at(&self, distance: f32, forward: f32) -> f32 {
        volume(self.max_volume, distance, self.min_radius, self.max_radius, forward)
    }
}

pub fn parse(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.eq_ignore_ascii_case("end") {
            continue;
        }
        let f = fields(line);
        if f.len() < 2 || !f[1].to_ascii_lowercase().ends_with(".wav") {
            continue;
        }
        let num = |i: usize, d: f32| f.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
        out.push(Entry {
            event: f[0].to_ascii_lowercase(),
            wav: f[1].to_string(),
            playflags: u32::from_str_radix(f.get(2).unwrap_or(&"0").trim_start_matches("0x"), 16)
                .unwrap_or(0),
            min_radius: num(3, 0.0),
            max_radius: num(4, 0.0),
            max_volume: num(5, 1.0),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{fade, parse, volume, FINAL_SCALE};

    #[test]
    fn the_fade_ramp_is_the_engines() {
        // Inside minRadius nothing is lost; at maxRadius everything is.
        assert_eq!(fade(0.0, 0.1, 1.5), 0.0);
        assert_eq!(fade(1.5, 0.1, 1.5), 1.0);
        // Halfway through the span is halfway through the ramp.
        assert!((fade(0.8, 0.1, 1.5) - 0.5).abs() < 1e-6);
        // Past maxRadius it stays fully faded.
        assert_eq!(fade(99.0, 0.1, 1.5), 1.0);
        // A zero-width radius means no fade at all, which is what the engine's
        // `1 / (max - min)` guard produces.
        assert_eq!(fade(0.0, 1.0, 1.0), 0.0);
        assert_eq!(fade(2.0, 1.0, 1.0), 0.0);
        assert_eq!(fade(999.0, 1.0, 1.0), 0.0);
    }

    #[test]
    fn volume_falls_off_and_a_sound_behind_is_quieter() {
        // `ShotHit02.wav` ships as min 0.1 / max 1.5 / maxVolume 0.8.
        let near = volume(0.8, 0.1, 0.1, 1.5, 1.0);
        let far = volume(0.8, 0.8, 0.1, 1.5, 1.0);
        let gone = volume(0.8, 1.5, 0.1, 1.5, 1.0);
        assert!((near - 0.8 * FINAL_SCALE).abs() < 1e-6, "{near}");
        assert!(far < near && far > gone, "{far} should sit between");
        assert_eq!(gone, 0.0);
        // Behind the listener: `1 - 1 * 0.3` of the same distance.
        let behind = volume(0.8, 0.1, 0.1, 1.5, -1.0);
        assert!((behind - near * 0.7).abs() < 1e-6, "{behind}");
    }

    #[test]
    fn reads_a_shipped_sound_class() {
        let rows = parse(
            "# Foley description for bryar pistol\n#\n\
             # class\t\tfilename\t\tplayflags\tminRadius\tmaxRadius\tmaxVolume\n\
             create\t\tShotHit02.wav             0x0     0.1     1.5\t0.8\n",
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event, "create");
        assert_eq!(rows[0].wav, "ShotHit02.wav");
        assert_eq!(rows[0].playflags, 0);
        assert_eq!((rows[0].min_radius, rows[0].max_radius), (0.1, 1.5));
        assert!((rows[0].max_volume - 0.8).abs() < 1e-6);
    }
}