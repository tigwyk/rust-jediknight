//! Playing a decoded cutscene: the frames the 32-bit helper produced, blitted into the same
//! 640x480 overlay canvas the HUD and menus draw on.
//!
//! No new quad, material or camera: the overlay canvas is already a texture on a quad in front
//! of the camera (`setup_overlay`), so a cutscene frame only has to land in `canvas.rgba`.
//!
//! **Streaming, because the films are long.** A cutscene is 992 to 3446 frames at 640x300, so
//! holding them all as RGBA is gigabytes. The helper decodes the whole film to disk *as it plays*
//! (it runs about five times faster than real time), and a frame is read, coloured and blitted
//! when its file appears. Playback holds the last available frame rather than skipping, and stops
//! on the final frame instead of looping.
//!
//! The film is 640x300 (every one of the 25 is, `--smk`), and the canvas is 640x480, so the frame
//! sits centred with 90 rows of black above and below — the same letterbox the engine showed on a
//! 4:3 screen.

use std::path::{Path, PathBuf};

use bevy::prelude::*;

use crate::smack;
use crate::smk;

pub const FRAME_WIDTH: u32 = 640;
pub const FRAME_HEIGHT: u32 = 300;

/// Cutscene playback state. The decoded frames live on disk, not in memory.
#[derive(Resource)]
pub struct Cutscene {
    /// The film's own bytes, kept for palette lookups.
    film: Vec<u8>,
    /// Where the helper writes `<prefix>NNN.raw`.
    prefix: PathBuf,
    /// Frames in the film, from its header.
    pub total: usize,
    pub at: usize,
    /// Time owed to the next frame, in seconds.
    elapsed: f32,
    /// The film's own frame time: 66 ms for the shipped 15.2 fps cutscenes.
    pub frame_seconds: f32,
    /// Set once the last frame has been shown. A transition waits for this.
    pub finished: bool,
    /// The last frame that could be read, kept so a slow decode never blanks the screen.
    last: Vec<u8>,
}

impl Cutscene {
    /// Start the helper on `file` and begin playback at `start`. The helper is a child process:
    /// it decodes ahead while playback reads what is already on disk.
    pub fn load(dll: &Path, file: &Path, start: usize) -> Option<Cutscene> {
        let film = std::fs::read(file).ok()?;
        let layout = smk::Layout::parse(&film)?;
        if !layout.covers(&film) {
            println!("cutscene: {} does not parse as a whole film", file.display());
            return None;
        }
        let total = layout.count;
        let prefix = std::env::temp_dir().join("bk-cutscene");
        // One helper run for the whole film; it writes frames in order as it decodes them.
        let shim = smack::shim_path()?;
        let spawned = std::process::Command::new(shim)
            .arg(dll)
            .arg(file)
            .arg(&prefix)
            .arg(total.to_string())
            .arg("0")
            .stdout(std::process::Stdio::null())
            .spawn();
        match spawned {
            Ok(_) => println!("cutscene: {} ({total} frames) decoding", file.display()),
            Err(e) => {
                println!("cutscene: could not start the decoder: {e}");
                return None;
            }
        }
        Some(Cutscene {
            film,
            prefix,
            total,
            at: start.min(total.saturating_sub(1)),
            elapsed: 0.0,
            // 640x300 at 15.2 fps: 66 ms a frame, from the film's header.
            frame_seconds: 0.066,
            finished: false,
            last: vec![0u8; (FRAME_WIDTH * FRAME_HEIGHT * 4) as usize],
        })
    }

    /// The frame file for an index.
    fn frame_path(&self, index: usize) -> PathBuf {
        PathBuf::from(format!("{}{index:03}.raw", self.prefix.display()))
    }

    /// Read one frame's RGBA from disk, colouring it with the film's own palette.
    fn read_frame(&self, index: usize) -> Option<Vec<u8>> {
        let bytes = std::fs::read(self.frame_path(index)).ok()?;
        if bytes.len() < (FRAME_WIDTH * FRAME_HEIGHT) as usize {
            return None;
        }
        let palette = smk::palette_at(&self.film, index);
        let mut rgba = Vec::with_capacity((FRAME_WIDTH * FRAME_HEIGHT * 4) as usize);
        for value in bytes.iter().take((FRAME_WIDTH * FRAME_HEIGHT) as usize) {
            match palette {
                Some(palette) => {
                    let [r, g, b] = palette[*value as usize];
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
                None => rgba.extend_from_slice(&[*value, *value, *value, 255]),
            }
        }
        Some(rgba)
    }

    /// Advance by `dt`, but only into frames that exist yet: a decode running behind playback
    /// holds the current frame instead of skipping ahead of the picture.
    pub fn advance(&mut self, dt: f32) {
        if self.total == 0 || self.finished {
            return;
        }
        self.elapsed += dt;
        while self.elapsed >= self.frame_seconds {
            if self.at + 1 >= self.total {
                self.finished = true;
                return;
            }
            if !self.frame_path(self.at + 1).exists() {
                // Not decoded yet: keep the frame we have and do not bank the time.
                self.elapsed = self.frame_seconds;
                return;
            }
            self.elapsed -= self.frame_seconds;
            self.at += 1;
        }
    }

    /// The frame to show: the current one if it is decoded, else the last one that was.
    pub fn current(&mut self) -> &[u8] {
        if let Some(rgba) = self.read_frame(self.at) {
            self.last = rgba;
        }
        &self.last
    }

    /// Remove the decoded frames from disk.
    pub fn clean_up(&self) {
        for index in 0..self.total {
            let _ = std::fs::remove_file(self.frame_path(index));
        }
    }
}

impl Drop for Cutscene {
    fn drop(&mut self) {
        self.clean_up();
    }
}

/// Letterbox one frame into the middle of the canvas. Both are RGBA.
pub fn blit(canvas: &mut [u8], width: u32, height: u32, frame: &[u8]) {
    let need = (width * height * 4) as usize;
    if canvas.len() < need || frame.len() < (FRAME_WIDTH * FRAME_HEIGHT * 4) as usize {
        return;
    }
    canvas[..need].fill(0);
    let top = height.saturating_sub(FRAME_HEIGHT) / 2;
    let row_bytes = (FRAME_WIDTH * 4) as usize;
    for row in 0..FRAME_HEIGHT.min(height) {
        let at = ((top + row) * width * 4) as usize;
        let from = (row * FRAME_WIDTH * 4) as usize;
        canvas[at..at + row_bytes].copy_from_slice(&frame[from..from + row_bytes]);
    }
}

/// Blit the current cutscene frame into the overlay canvas and upload it.
pub fn play_cutscene(
    time: Res<Time>,
    cutscene: Option<ResMut<Cutscene>>,
    mut overlay: ResMut<crate::Overlay>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut cutscene) = cutscene else { return };
    cutscene.advance(time.delta_secs());
    let (width, height) = (overlay.canvas.width, overlay.canvas.height);
    let frame = cutscene.current().to_vec();
    blit(&mut overlay.canvas.rgba, width, height, &frame);
    // The overlay image keeps its CPU copy (`RenderAssetUsages::default()`); without that the
    // renderer takes the data, `image.data` is `None`, and this copy goes nowhere (§16.11).
    let id = overlay.image.id();
    let Some(mut image) = images.get_mut(id) else { return };
    let Some(data) = image.data.as_mut() else { return };
    let canvas = &overlay.canvas.rgba;
    if data.len() >= canvas.len() {
        data[..canvas.len()].copy_from_slice(canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic(total: usize) -> Cutscene {
        Cutscene {
            film: Vec::new(),
            prefix: std::env::temp_dir().join("bk-cutscene-test-"),
            total,
            at: 0,
            elapsed: 0.0,
            frame_seconds: 0.066,
            finished: false,
            last: vec![0u8; (FRAME_WIDTH * FRAME_HEIGHT * 4) as usize],
        }
    }

    #[test]
    fn playback_stops_at_the_last_frame() {
        let mut cutscene = synthetic(3);
        // No frame files exist, so playback holds frame 0 and does not skip ahead.
        cutscene.advance(0.5);
        assert_eq!(cutscene.at, 0, "missing frames are not skipped over");
        assert!(!cutscene.finished);
        // At the last frame there is nowhere to advance: the film ends instead of looping.
        cutscene.at = 2;
        cutscene.advance(0.5);
        assert!(cutscene.finished);
        assert_eq!(cutscene.at, 2);
    }

    #[test]
    fn a_frame_lands_letterboxed_in_the_canvas() {
        let mut frame = vec![0u8; (FRAME_WIDTH * FRAME_HEIGHT * 4) as usize];
        for pixel in frame.chunks_exact_mut(4).take(FRAME_WIDTH as usize) {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
        // The overlay canvas the app uses.
        let canvas_height = 480;
        let mut canvas = vec![7u8; (FRAME_WIDTH * canvas_height * 4) as usize];
        blit(&mut canvas, FRAME_WIDTH, canvas_height, &frame);
        let top = (canvas_height - FRAME_HEIGHT) / 2;
        assert_eq!(top, 90);
        assert_eq!(canvas[0], 0, "above the frame is black, not the old contents");
        let at = (top * FRAME_WIDTH * 4) as usize;
        assert_eq!(&canvas[at..at + 4], &[255, 255, 255, 255]);
        assert_eq!(canvas[at - 4], 0);
        let below = ((top + FRAME_HEIGHT) * FRAME_WIDTH * 4) as usize;
        assert_eq!(canvas[below], 0);
    }
}
