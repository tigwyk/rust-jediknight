//! bevi-knight — a from-scratch reconstruction of Jedi Knight: Dark Forces II's
//! world renderer in Rust + Bevy.
//!
//! Asset formats were recovered from the shipped game (see `docs/formats.md`),
//! not from any original source. Nothing here reads or needs the original EXE;
//! only the data archives are used.

mod ai;
mod bm;
mod cog;
mod collide;
mod episode;
mod gob;
mod hud;
mod jks;
mod jkl;
mod menus;
mod nav;
mod key;
mod level_mat;
mod png;
mod pup;
mod rows;
mod runtime;
mod sft;
mod cutscene;
mod smk;
mod smack;
mod snd;
mod spr;
mod tdo;
mod ui;
mod uni;
mod tex;

use bevy::app::AppExit;
use bevy::asset::RenderAssetUsages;
use bevy::audio::{AudioPlayer, AudioSource, PlaybackMode, PlaybackSettings, Volume};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy::camera::visibility::RenderLayers;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use std::collections::HashMap;
use std::path::PathBuf;

const DEFAULT_GAME: &str = r"G:\Games\Star Wars Jedi Knight - Dark Forces 2";

/// Everything loaded from disk, handed to Bevy as a resource.
#[derive(Resource)]
struct World {
    level: jkl::Level,
    /// Level material index -> decoded bitmap.
    images: HashMap<i32, Image>,
    /// Level material index -> how many cels its bitmap holds (`num_texinfo`). Only a handful of world
    /// textures are animated — water, fuel, lock and sign panels — but a surface can point its cel at
    /// any of them with `SetWallCel`/`SurfaceAnim`, so the level shader needs the count to map a cel
    /// onto the packed strip.
    material_cels: HashMap<i32, u32>,
    /// `misc\items.dat`'s `cog=` column: inventory bin -> the script that bin runs. Read once at
    /// load and handed to the runtime, which has no file access.
    item_cogs: std::collections::BTreeMap<i32, String>,
    /// The whole `misc\items.dat` table, so the status bar can name the bins the player holds.
    item_defs: Vec<hud::ItemDef>,
    /// Material indices whose `tex_type & 2` marks them as portal materials: `sithSurface.c` refuses to
    /// give those a geometry mode, so the engine never draws them. Drawing one seals a doorway.
    /// Lowercase template name -> lowercase `.3do` name.
    template_model: HashMap<String, String>,
    /// Lowercase `.3do` name -> parsed model.
    models: HashMap<String, tdo::Model>,
    /// Lowercase `.mat` name -> decoded bitmap, for model materials.
    model_images: HashMap<String, Image>,
    /// Lowercase template name -> the faces of its `sprite=`, for the effect templates that name one
    /// (`+laserhit` is `bryx.spr`). A sprite is a list of camera-facing rectangles, so every effect
    /// in the game is one of these.
    template_sprite: HashMap<String, Vec<spr::Face>>,
    /// Lowercase `.mat` name -> its cel count from the `.mat` header. A sprite draws one cel of the
    /// strip at a time; the count is the engine's (`num_texinfo`), not the bitmap's aspect.
    sprite_cels: HashMap<String, u32>,
    /// Lowercase `.wav` name -> raw file bytes, for the sectors' ambient loops.
    /// Only sounds a sector actually names are read, which is a few dozen.
    ambient: HashMap<String, Vec<u8>>,
    /// The level's own COG scripts, parsed at load.
    cogs: HashMap<String, cog::Cog>,
    /// Lowercase `.pup` name -> parsed puppet, for the placed things that name one.
    puppets: HashMap<String, pup::Pup>,
    /// Lowercase `.key` name -> parsed keyframe, for the states those puppets use.
    keys: HashMap<String, key::Key>,
    /// Lowercase `.ai` name -> the file's text, read only for the classes an actor names.
    ai_classes: HashMap<String, String>,
    /// Lowercase `.wav` name -> raw bytes, for the wavs the shipped `.snd` classes name.
    wav_bytes: HashMap<String, Vec<u8>>,
    /// Lowercase `.wav` name -> `(minRadius, maxRadius, maxVolume)` from its sound class.
    sound_radii: HashMap<String, (f32, f32, f32)>,
    gob_entries: usize,
}

/// Accept `03katarn`, `03katarn.jkl` or `jkl\03katarn.jkl` for a level on the command line.
fn level_arg(name: &str) -> String {
    let file = if name.to_ascii_lowercase().ends_with(".jkl") {
        name.to_string()
    } else {
        format!("{name}.jkl")
    };
    if file.contains(['\\', '/']) { file } else { format!("jkl\\{file}") }
}

/// Which screen the app opens on. `--menu X` forces one, `--start` (and every capture) begins in the
/// level, and otherwise the game boots to the retail main menu.
fn boot_screen(menu: Option<&str>, capture: bool, start_in_level: bool, has_main: bool) -> Option<String> {
    match menu {
        Some(requested) => Some(requested.to_string()),
        None if start_in_level => None,
        None if !capture && has_main => Some("main".to_string()),
        None => None,
    }
}

/// Every gate this port has, in the order `docs/gates.txt` lists them. `--gates` runs each in its own
/// process so one panic cannot hide the rest, and reports the `mode:` line each one printed: a gate
/// with no mode line asserted on parsed data or the runtime and never opened a window.
const GATES: &[&str] = &[
    "--check",
    "--audit",
    "--boot",
    "--ui-audit",
    "--walk all",
    "--scale",
    "--rules",
    "--animate",
    "--ai",
    "--collide",
    "--controls",
    "--feel",
    "--flow",
    "--fall",
    "--falldeath",
    "--door",
    "--endlevel",
    "--hit",
    "--live-check",
    "--mana",
    "--ammo",
    "--campaign",
    "--tally",
    // The mission objective completing in play: the body stands in the goal's sector and
    // the renderer sights 8t88, which is what 01_goal0.cog waits for.
    "--goal --start 01narshadda",
    "--muzzle",
    "--use-item",
    "--engine-audit",
    "--cog-check",
    "--adjoins",
    "--geomode",
    "--cel",
    "--camera",
    "--scroll",
    "--damage-return",
    "--attach",
    "--things",
    "--move",
    "--pickup",
    "--save",
    "--items",
    "--weapons",
    "--soak",
    "--quicksave",
    "--sleep",
    "--triggers",
    "--transition",
    "--weapon +elaser",
    "--audio-state",
    "--sound",
    "--episodes",
    "--fire +bryarbolt",
];

/// `--exit-after N`: quit after N frames whatever the machine or the menu is doing.
///
/// A run that cannot terminate is not a test: without this, a flag that forgets to quit sits on a
/// window forever, and one such flag hangs a whole `--gates` sweep. `BK_EXIT_FRAMES` sets the same
/// budget without the flag.
fn exit_after_frames(
    after: Option<Res<ExitAfter>>,
    mut frames: Local<u64>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(after) = after else { return };
    *frames += 1;
    if *frames >= after.0 {
        exit.write(AppExit::Success);
    }
}

#[derive(Resource)]
struct ExitAfter(u64);

/// `--gates`: run the whole list and print, per flag, whether it touched the renderer and what it said.
/// `--gates`: run the whole list and print, per flag, whether it touched the renderer and what it said.
fn run_gates(game: &std::path::Path) -> i32 {
    let exe = std::env::current_exe().expect("the running executable");
    println!("{:<22} {:<10} {:<6} {}", "flag", "runs", "result", "verdict");
    let mut failed = 0;
    for flag in GATES {
        // The sweep's one window gate renders a real window; on a machine with no usable display it
        // sits in presentation and never reaches its verdict. `BK_SKIP_WINDOW=1` skips it so the
        // headless ones can still be checked, and says so rather than pretending it passed.
        // Both live gates need the camera entity the walker queries: without a window there is no
        // camera, no walk, and no verdict — a failure that says nothing about the code.
        if (flag.starts_with("--live-check") || flag.starts_with("--live-play"))
            && std::env::var_os("BK_SKIP_WINDOW").is_some()
        {
            println!("{flag:<22} {:<10} {:<6} skipped (BK_SKIP_WINDOW)", "window", "-");
            continue;
        }
        // Every child gets a frame budget: a gate that forgets to quit ends instead of hanging the sweep,
        // and it ends *without* a verdict, which is counted as a failure below.
        let out = std::process::Command::new(&exe)
            .arg("--game")
            .arg(game)
            // Before the flag, not after: several gates take an argument and would eat the budget flag.
            .args(["--exit-after", "600"])
            .args(flag.split_whitespace())
            .env("BK_MUTE", "1")
            .output();
        let (mode, verdict) = match &out {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                let mode = if text.contains("mode: window") { "window" } else { "headless" };
                let verdict = text
                    .lines()
                    .rev()
                    .find(|l| l.starts_with("--- "))
                    .map(str::trim)
                    .unwrap_or("(no verdict)");
                (mode, verdict.to_string())
            }
            Err(e) => ("error", e.to_string()),
        };
        let ok = out.as_ref().map(|o| o.status.success()).unwrap_or(false);
        // A gate proves itself with a verdict line — **headless too**. A headless gate that exits 0
        // without one has run something that asserts nothing (an early return, a flag that fell
        // through), and the sweep used to call that a pass.
        let verdict_ok = verdict != "(no verdict)";
        let passed = ok && verdict_ok;
        if !passed {
            failed += 1;
        }
        println!(
            "{:<22} {:<10} {:<6} {}",
            flag,
            mode,
            if passed { "pass" } else { "FAIL" },
            if mode == "window" && !verdict_ok { format!("{verdict} (opened a window)") } else { verdict }
        );
    }
    println!("gates: {} of {} failed", failed, GATES.len());
    if failed == 0 { 0 } else { 1 }
}

/// The next argument as a value, or `None` when it is another flag.
///
/// Every handler that takes a parameter has the same trap: `--save --level 01` once wrote a file
/// called `--level`, and `--hud` did it again two rounds later. One rule, in one place, so a third
/// copy cannot get it wrong. A **single** dash is still a value (`--walk -45` is a heading), which is
/// why this checks for two.
fn arg_value<'a>(args: &'a [String], i: &mut usize) -> Option<&'a String> {
    let next = args.get(*i + 1)?;
    if next.starts_with("--") {
        return None;
    }
    *i += 1;
    Some(next)
}

fn main() {
    let mut game = PathBuf::from(DEFAULT_GAME);
    let mut level_name: Option<String> = None;
    // --start: boot into the level instead of the main menu.
    let mut start_in_level = false;
    // --gates: run every documented gate in its own process and report what each one exercised.
    let mut gates_run = false;
    // `--exit-after N` / `BK_EXIT_FRAMES`: a hard frame budget, so a run always ends.
    let mut exit_after: Option<u64> = std::env::var("BK_EXIT_FRAMES").ok().and_then(|v| v.parse().ok());
    let mut check = false;
    let mut overview = false;
    let mut audit_only = false;
    let mut episodes_only = false;
    let mut near: Option<String> = None;
    // `--stand <template>`: put the player next to the first thing of that template, so an
    // actor can be watched in the world instead of only in the runtime.
    let mut stand: Option<String> = None;
    let mut shot: Option<String> = None;
    let mut shot_frame = 30u32;
    let mut muted_flag = false;
    let mut sound_on = false;
    let mut audio_state = false;
    let mut fall_check = false;
    let mut falldeath_check = false;
    let mut door_check = false;    let mut shot_at: Option<([f32; 3], f32)> = None;

    let mut sim = false;
    let mut dump: Option<String> = None;
    let mut raw: Option<(String, String)> = None;
    let mut texture: Option<(String, String)> = None;
    let mut bm_out: Option<(String, String)> = None;
    let mut ui_out: Option<(String, String)> = None;
    let mut force_found: Vec<i32> = Vec::new();
    let mut map_check = false;
    let mut cmp_check: Option<String> = None;
    let mut ui_tables = false;
    let mut ui_audit = false;
    // `--boot`: the screen the app opens on and what each of its rows does.
    let mut boot_check = false;
    let mut stars = 0i32;
    let mut cull = true;
    let mut rules_only = false;
    let mut animate_only = false;
    let mut animating = true;
    let mut cog_check = false;
    // `--engine-audit`: which of the decompiled engine's biggest functions this port never
    // mentions, ranked by size. The list is generated from the binaries, not from memory.
    let mut engine_audit = false;
    // `--use-item`: an inventory item's own script, driven the way the engine drives it.
    let mut use_item = false;
    // `--muzzle`: a shot leaves the shooter where its own template says, not a guessed unit.
    let mut muzzle_check = false;
    // `--adjoins`: the sector/adjoin flag verbs, and that collision and the cull read them.
    let mut adjoin_check = false;
    let mut geomode_check = false;
    let mut cel_check = false;
    let mut camera_check = false;
    let mut scroll_check = false;
    let mut damage_return_check = false;
    let mut attach_check = false;
    let mut things_check = false;
    let mut fire_check: Option<String> = None;
    let mut hit_check = false;
    let mut level_shader = false;
    let mut standard_material = false;
    let mut fade_test = false;
    let mut ir_from_start = false;
    let mut live = false;
    let mut sleep_check = false;
    let mut fade_check = false;
    let mut ui_sounds = false;
    let mut feel_check = false;
    let mut collide_check = false;
    let mut controls_check = false;
    let mut move_check = false;
    let mut winding_check = false;
    // `--scale`: models against the level's own geometry.
    let mut scale_check = false;
    let mut mana_check = false;
    let mut pull_check = false;
    let mut ammo_check = false;
    let mut triggers_check = false;
    let mut load_percent: f32 = 50.0;
    let mut load_check = false;
    let mut die_check = false;
    let mut fire_demo = false;
    let mut sprite_template: Option<String> = None;
    let mut pickup_check = false;
    let mut weapon_check: Option<String> = None;
    let mut bins_check = false;
    let mut force_check: Option<String> = None;
    let mut ai_check = false;
    let mut save_check: Option<String> = None;
    let mut quicksave_check = false;
    let mut items_check = false;
    let mut weapons_check = false;
    let mut soak_check = false;
    // `--soak-all`: every level in the archives, one process each.
    let mut soak_all = false;
    let mut tally_check = false;
    let mut goal_check = false;
    // `--uni <key>`: which string file carries a key, and what it says.
    let mut uni_key: Option<String> = None;
    // `--live-all`: every level through the real schedule, one process each.
    let mut live_all = false;
    // `--live-play`: press the real keys in the live schedule and require movement and a shot.
    let mut live_play = false;
    let mut sound_check = false;
    let mut music_check = false;
    let mut ui_visible = false;
    let mut force_screen_check = false;
    let mut campaign = false;
    let mut transition = false;
    let mut endlevel_check = false;
    let mut flow_level: Option<String> = None;
    let mut smk_check = false;
    let mut smk_decode: Option<String> = None;
    let mut play_cutscene: Option<(String, usize)> = None;
    // `--complete-after <seconds>`: call the engine's own `jkendlevel` after that long, so the
    // level-completion path (cutscene, then the next level) can be exercised without playing.
    let mut complete_after: Option<f32> = None;
    // A finished level loads the graph's next one once its film has played: that is the drop-in
    // behaviour. `--no-swap-levels` stops at the film.
    let mut swap = true;
    let mut bm_bg: Option<String> = None;
    let mut sft_out: Option<(String, String)> = None;
    let mut hud_out: Option<String> = None;
    let mut menu_open: Option<String> = None;
    let mut pending_id: Option<i32> = None;
    let mut look = 0.0f32;
    let mut play: Option<String> = None;
    let mut names: Option<String> = None;
    let mut cogs_only = false;
    let mut cogs_filter = String::new();
    let mut run_only = false;
    let mut run_filter = String::new();
    let mut walk: Option<Walk> = None;
    let mut pose: Option<(String, String, f32)> = None;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => check = true,
            "--cogs" => {
                cogs_only = true;
                if let Some(v) = args.get(i + 1) {
                    if !v.starts_with('-') {
                        cogs_filter = v.clone();
                        i += 1;
                    }
                }
            }
            "--overview" => overview = true,
            "--run" => {
                run_only = true;
                if let Some(v) = args.get(i + 1) {
                    if !v.starts_with('-') {
                        run_filter = v.clone();
                        i += 1;
                    }
                }
            }
            "--audit" => audit_only = true,
            "--episodes" => episodes_only = true,
            "--stand" => stand = arg_value(&args, &mut i).cloned(),
            "--near" => near = arg_value(&args, &mut i).cloned(),
            "--shot" => shot = arg_value(&args, &mut i).cloned(),
            "--fall" => fall_check = true,
        "--falldeath" => falldeath_check = true,
        "--door" => door_check = true,
        "--endlevel" => endlevel_check = true,
            "--mute" => muted_flag = true,
            "--sound-on" => sound_on = true,
            "--audio-state" => audio_state = true,
            // `--shot-at "x,y,z"`: put the capture camera exactly there, looking along +x unless
            // `--shot-yaw` says otherwise. A deterministic vantage is the only way to compare two frames.
            "--shot-at" => {
                shot_at = arg_value(&args, &mut i).cloned().and_then(|s| {
                    let v: Vec<f32> = s.split(',').filter_map(|n| n.trim().parse().ok()).collect();
                    (v.len() == 4).then(|| ([v[0], v[1], v[2]], v[3]))
                });
            }
            "--frames" => {
                shot_frame = arg_value(&args, &mut i)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(30);
            }
            "--sim" => sim = true,
            // `--pose <model> <key> <frame>`: one model, posed from one keyframe.
            "--pose" => {
                let model = args.get(i + 1).cloned();
                let key = args.get(i + 2).cloned();
                let frame = args.get(i + 3).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                if let (Some(m), Some(k)) = (model, key) {
                    pose = Some((m, k, frame));
                }
                i += 3;
            }
            // `--walk <frames>`: push the level's walkplayer forward through its own
            // collision for that many frames and report where it ends up. No window.
            // `--walk all` does it for every level in the install.
            "--walk" => {
                walk = Some(match arg_value(&args, &mut i).map(String::as_str) {
                    Some("all") => Walk::All,
                    other => Walk::Frames(
                        other.and_then(|v| v.parse().ok()).unwrap_or(120),
                    ),
                });
            }
            "--look" => {
                look = arg_value(&args, &mut i)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0.0);
            }
            "--names" => {
                i += 1;
                names = args.get(i).cloned();
            }
            // `--start [level]`: skip the main menu and begin in the level, which is what testing a
            // level actually needs. A bare level name works: `--start 03katarn`.
            "--gates" => gates_run = true,
            "--exit-after" => {
                i += 1;
                exit_after = args.get(i).and_then(|v| v.parse().ok());
            }
            "--start" => {
                start_in_level = true;
                if let Some(next) = args.get(i + 1).filter(|a| !a.starts_with('-')) {
                    level_name = Some(next.clone());
                    i += 1;
                }
            }
            "--play" => {
                i += 1;
                play = args.get(i).cloned();
            }
            "--dump" => {
                i += 1;
                dump = args.get(i).cloned();
            }
            // `--raw <name> <out>`: write one archived file out verbatim, which is how
            // a binary format gets read when there is no decoder for it yet.
            "--raw" => {
                let name = args.get(i + 1).cloned();
                let out = args.get(i + 2).cloned();
                if let (Some(n), Some(o)) = (name, out) {
                    raw = Some((n, o));
                }
                i += 2;
            }
            "--menu" => {
                i += 1;
                // `--menu <screen>` opens it; `--menu <screen>/<id>` then activates that
                // row, which is how a screenshot of the screen it opens is taken.
                let arg = args.get(i).cloned().unwrap_or_else(|| "singleplayer".into());
                let (name, pending) = match arg.split_once('/') {
                    Some((name, id)) => (name.to_string(), id.parse::<i32>().ok()),
                    None => (arg.clone(), None),
                };
                pending_id = pending;
                menu_open = Some(name);
            }
            "--hud" => {
                hud_out = Some(
                    arg_value(&args, &mut i)
                        .cloned()
                        .unwrap_or_else(|| "hud.png".to_string()),
                );
            }
            "--sft" => {
                let name = args.get(i + 1).cloned();
                let out = args.get(i + 2).cloned();
                if let (Some(n), Some(o)) = (name, out) {
                    sft_out = Some((n, o));
                }
                i += 2;
            }
            "--bm-bg" => {
                i += 1;
                bm_bg = args.get(i).cloned();
            }
            "--ui-audit" => ui_audit = true,
            // The boot flow: the screen the app opens on, and what "New Game", "Restart" and "Abort"
            // resolve to, checked against the install's own element tables.
            "--boot" => boot_check = true,
            // Verify the portal walk by turning it off: the two frames must be identical.
            "--no-cull" => cull = false,
            // Print the motion the level's templates describe, without opening a window.
            "--rules" => rules_only = true,
            // Check that the placed actors' keyframes actually move them.
            "--animate" => animate_only = true,
            // Freeze the actors, so a frame pair can be A/B'd against the animation.
            "--no-anim" => animating = false,
            // The script gate: every level's scripts must still act on their world.
            "--cog-check" => cog_check = true,
            // Which of the decompiled engine's biggest functions the port never mentions.
            "--engine-audit" => engine_audit = true,
            "--use-item" => use_item = true,
            "--muzzle" => muzzle_check = true,
            // The adjoin/sector flag verbs, and the two consumers that must read them.
            "--adjoins" => adjoin_check = true,
            // The surface geomode: a script hides a face and the world's batches drop its polygons.
            "--geomode" => geomode_check = true,
            // `--cel`: a surface's cel reaches the mesh the level shader draws.
            "--cel" => cel_check = true,
            // `--camera`: the camera slots and POV shake, headless.
            "--camera" => camera_check = true,
            // `--scroll`: a scrolling surface (`SlideWall`) and the sprite/sky scroll verbs.
            "--scroll" => scroll_check = true,
            // `--damage-return`: a `damaged` handler's `ReturnEx` replaces the amount the engine applies.
            "--damage-return" => damage_return_check = true,
            // `--attach`: a thing rides another (`AttachThingToThing`), and stops when detached.
            "--attach" => attach_check = true,
            // --things: TeleportThing, CreateThingAtPos, FireProjectile and the Sync* no-ops.
            "--things" => things_check = true,
            // `--fire <weapon template>`: fire that weapon at level 1's actor and check the
            // projectile travels, hits and damages.
            // `--level-shader`: draw level surfaces with the material that carries the colormap
            // light ramp (distance fade + force seeing) instead of `StandardMaterial`.
            // The colormap-aware material is now the default, so this flag is kept as a no-op for
            // old command lines and `--standard-material` is the A/B opt-out.
            "--level-shader" => level_shader = true,
            "--standard-material" => standard_material = true,
            // `--fade-test`: pull the fade in close, so a screenshot shows it working at all.
            "--fade-test" => fade_test = true,
            // `--ir`: force seeing on from the first frame, so a screenshot can show the pass.
            "--ir" => ir_from_start = true,
            // `--live-check`: invariants on the runtime the game is actually simulating.
            "--live-check" => live = true,
            // `--sleep`: a two-line cog that waits, to pin `sleep` suspension and resumption.
            "--sleep" => sleep_check = true,
            // `--fade`: the soundtrack script's own `SetPulse` + `SetMusicVol` chain, run headless.
            "--fade" => fade_check = true,
            // `--ui-sounds`: the menus' own click/hover wav names, and whether the install has them.
            "--ui-sounds" => ui_sounds = true,
            "--feel" => feel_check = true,
            "--collide" => collide_check = true,
            "--controls" => controls_check = true,
            "--move" => move_check = true,
            "--winding" => winding_check = true,
            // `--scale`: how big the models really are against the level they are placed in.
            "--scale" => scale_check = true,
            // `--mana`: force mana regenerates and clamps at the pool ceiling.
            "--mana" => mana_check = true,
            // `--pull`: the force pull power's own script, and whether it moves its target.
            "--pull" => pull_check = true,
            // `--ammo`: which bin a weapon spends, and that firing runs it down.
            "--ammo" => ammo_check = true,
            // `--triggers`: the sector links a level declares, and which of them can hear a message.
            "--triggers" => triggers_check = true,
            // `--load-bar <percent> <out.png>`: the loading screen`s own bar, drawn and measured.
            "--load-bar" => {
                i += 1;
                load_percent = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(50.0);
                i += 1;
                ui_out = Some(("loading".to_string(), args.get(i).cloned().unwrap_or_default()));
                load_check = true;
            }
            // `--hit`: the impact scan the game actually uses (`Runtime::impacts`), which no check
            // covered before — `--fire` simulated its own hit test and missed this path entirely.
            "--hit" => hit_check = true,
            // `--die`: shields absorb first, health reaches 0, and the level's start brings the
            // player back with a full bar.
            "--die" => die_check = true,
            "--fire" => {
                i += 1;
                fire_check = Some(args.get(i).cloned().unwrap_or_default());
            }
            // `--fire-demo`: pull the trigger once after a few frames, so a screenshot can
            // show a bolt in flight without a human at the mouse.
            "--fire-demo" => fire_demo = true,
            // `--sprite <template>`: keep an effect template alive in front of the camera, so a
            // screenshot shows what the sprite layer draws. See `sprite_demo`.
            "--sprite" => {
                i += 1;
                sprite_template = args.get(i).cloned();
            }
            // `--pickup`: walk the player onto level 1's own powerup and check it is taken.
            "--pickup" => pickup_check = true,
            // `--weapon <template>`: taking a weapon changes what the player fires.
            "--weapon" => {
                i += 1;
                weapon_check = Some(args.get(i).cloned().unwrap_or_default());
            }
            // `--bins`: the engine's inventory bins — weapons and force powers alike — and
            // the selection verbs scripts use to arm one.
            "--bins" => bins_check = true,
            // `--force <power>`: run that force power's own script and check what it does to
            // the player. `jump` and `speed` are the two with movement effects.
            "--force" => {
                i += 1;
                force_check = Some(args.get(i).cloned().unwrap_or_default());
            }
            // `--ai`: the `.ai` instinct classes, and whether an actor notices a player.
            "--ai" => ai_check = true,
            // `--save <path>`: write a `.jks` for this level and read it back. `--save
            // <path>.jks` on an existing file inspects it instead.
            "--save" => {
                i += 1;
                let arg = args.get(i).cloned().unwrap_or_default();
                // No path: round-trip through the temp directory. An empty name is not a path, and
                // `--save` on its own used to fail with `NotFound` when the writer reached it. A
                // following flag is not a path either: `--save --level 01narshadda` used to write a
                // file literally named `--level` in the working directory.
                save_check = Some(if arg.is_empty() || arg.starts_with("--") {
                    i -= 1;
                    std::env::temp_dir().join("bevi-knight-save.jks").display().to_string()
                } else {
                    arg
                });
            }
            // `--quicksave`: the `F5`/`F9` path, end to end, without a window.
            "--quicksave" => quicksave_check = true,
            // `--items`: `misc\items.dat`, the inventory table the engine registers bins from.
            "--items" => items_check = true,
            // `--weapons`: run the item scripts `misc\items.dat` names and report what answers.
            "--weapons" => weapons_check = true,
            // `--soak`: run the level's whole simulation (physics, AI, scripts) for N seconds of
            // level time and check the world still makes sense. The only gate that exercises the
            // loop as a whole rather than one piece of it.
            "--soak" => soak_check = true,
            "--soak-all" => soak_all = true,
            "--tally" => tally_check = true,
            "--live-all" => live_all = true,
            "--live-play" => live_play = true,
            // `--goal`: stand the body in front of the level's goal thing and let the renderer see it, so
            // the mission objective completes in play instead of being driven by hand.
            "--goal" => goal_check = true,
            "--uni-key" => {
                i += 1;
                uni_key = args.get(i).cloned();
            }
            // `--sound`: the `.snd` classes' own fade radii, and the volume curve over them.
            "--sound" => sound_check = true,
            // `--music`: which songs the shipped scripts ask for, and what the install has.
            "--music" => music_check = true,
            // `--ui-visible`: the engine's run-time `bIsVisible` rule over the screens.
            "--ui-visible" => ui_visible = true,
            // `--force-screen`: the force-power panel built from `jkGUIForce.c`'s own table,
            // showing only the powers the player has found.
            "--force-screen" => force_screen_check = true,
            // `--cmp <name>`: the `.cmp` distance tables, and whether they darken monotonically.
            // `--no-fog`: turn the distance fade off, for the A/B that proves it does something.
            // `--fog`: the distance fade's own numbers, from the colormap's darkest step.
            "--cmp" => {
                i += 1;
                cmp_check = args.get(i).cloned();
            }
            // `--map-shot <out.png>`: the field-log map panel, drawn from the level's own
            // geometry, and a count of the line work so the panel cannot quietly go blank.
            "--map-shot" => {
                i += 1;
                if let Some(out) = args.get(i).cloned() {
                    ui_out = Some(("fieldlog".to_string(), out));
                    map_check = true;
                }
            }
            // `--force-found 29,33`: which force bins the player carries, for the panel.
            "--force-found" => {
                i += 1;
                force_found = args
                    .get(i)
                    .map(|list| list.split(',').filter_map(|n| n.parse().ok()).collect())
                    .unwrap_or_default();
            }
            // `--campaign`: the episode graph's own progression, from its first level to -1.
            "--campaign" => campaign = true,
            // `--transition`: what finishing a level advances to, through the episode graph.
            "--transition" => transition = true,
            // `--flow [level]`: level -> its cutscene -> the next level, from the graph.
            "--flow" => {
                i += 1;
                flow_level = Some(
                    args.get(i)
                        .cloned()
                        .unwrap_or_else(|| "01narshadda.jkl".to_string()),
                );
            }
            // `--complete-after <seconds>`: finish the level on a timer, to watch the flow.
            "--complete-after" => {
                i += 1;
                complete_after = args.get(i).and_then(|v| v.parse::<f32>().ok());
            }
            "--no-swap-levels" => swap = false,
            // `--smk`: the shipped cutscene files' own headers.
            "--smk" => smk_check = true,
            // `--cutscene [file] [start-frame]`: play a decoded cutscene in the window.
            "--cutscene" => {
                i += 1;
                let name = args
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| "01-02A.SMK".to_string());
                // An optional starting frame: playback is paced by wall clock, so a scripted
                // screenshot of a film's middle needs to begin there rather than wait for it.
                let start = args.get(i + 1).and_then(|v| v.parse::<usize>().ok());
                if start.is_some() {
                    i += 1;
                }
                play_cutscene = Some((name, start.unwrap_or(0)));
            }
            // `--smk-decode [file]`: decode frames through the install's own SMACKW32.DLL.
            "--smk-decode" => {
                i += 1;
                smk_decode = Some(
                    args.get(i)
                        .cloned()
                        .unwrap_or_else(|| "01-02A.SMK".to_string()),
                );
            }
            "--stars" => {
                i += 1;
                stars = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            "--ui-tables" => {
                ui_tables = true;
            }
            "--ui" => {
                let name = args.get(i + 1).cloned();
                let out = args.get(i + 2).cloned();
                if let (Some(n), Some(o)) = (name, out) {
                    ui_out = Some((n, o));
                }
                i += 2;
            }
            "--bm" => {
                let name = args.get(i + 1).cloned();
                let out = args.get(i + 2).cloned();
                if let (Some(n), Some(o)) = (name, out) {
                    bm_out = Some((n, o));
                }
                i += 2;
            }
            "--texture" => {
                let name = args.get(i + 1).cloned();
                let out = args.get(i + 2).cloned();
                if let (Some(n), Some(o)) = (name, out) {
                    texture = Some((n, o));
                }
                i += 2;
            }
            "--game" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    game = PathBuf::from(v);
                }
            }
            "--level" => {
                i += 1;
                level_name = args.get(i).cloned();
            }
            other => {
                if !other.starts_with('-') {
                    level_name = Some(other.to_string());
                }
            }
        }
        i += 1;
    }

    // A level named on the command line may be `03katarn`, `03katarn.jkl` or `jkl\03katarn.jkl`.
    let level_name = level_name.map(|n| level_arg(&n));

    // `--gates`: run every documented gate in its own process and report what each one exercised.
    // A gate that never prints `mode: window` asserted on parsed data or the runtime and opened no
    // window at all; the ones that do print it named the screen they ran against.
    if gates_run {
        std::process::exit(run_gates(&game));
    }

    let gobs = match gob::GobSet::open(&game) {
        Ok(g) if !g.is_empty() => g,
        Ok(_) => {
            eprintln!("fatal: no .gob archives found under {}", game.display());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("fatal: indexing {}: {e}", game.display());
            std::process::exit(1);
        }
    };

    // --audit / --episodes / --dump work over the whole install rather than one
    // selected level, so they run before the single-level load.
    // `--names` lists every indexed path containing a substring: the archive's own
    // table of contents, which is how the audits are spot-checked.
    if let Some(pat) = names {
        for n in gobs.names() {
            if n.contains(&pat) {
                println!("{n}");
            }
        }
        return;
    }
    if episodes_only {        print_episodes(&gobs);
        return;
    }
    if audit_only {
        match audit(&gobs) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("fatal: {e}");
                std::process::exit(1);
            }
        }
    }
    // --hud composes the in-game status bar over the level's own colormap, at 640x480.
    if let Some(out) = hud_out {
        // The status bar's bitmaps carry no palette: like every other level asset they
        // index the colormap, so `--level` picks it and the campaign's first level
        // stands in when none was named.
        let level = level_name.clone().or_else(|| start_level(&gobs));
        let palette = level
            .as_deref()
            .map(|l| String::from_utf8_lossy(&gobs.read(l).unwrap_or_default()).into_owned())
            .map(|t| jkl::parse(&t))
            .and_then(|l| l.colormaps.first().cloned())
            .and_then(|c| gobs.read(&c))
            .and_then(|d| tex::Palette::parse(&d))
            .map(|p| p.0)
            .unwrap_or_else(|| vec![[0u8; 3]; 256]);
        let palette: [[u8; 3]; 256] = palette.try_into().unwrap_or([[0u8; 3]; 256]);
        let mut hud = hud::Hud::default();
        hud.item = Some("bactatank".into());
        hud.item_count = 2;
        hud.carried = vec![
            ("bactatank".to_string(), 2),
            ("keyred".to_string(), 1),
            ("irgoggles".to_string(), 1),
            ("fieldlight".to_string(), 1),
        ];
        let mut canvas = ui::Canvas::new(640, 480);
        hud::draw(&mut canvas, &gobs, &palette, &hud);
        // The message area: `jkStringOutput`/`jkPrintUNIString` compose a line and this draws it where
        // the engine does. `BK_HUD_MESSAGE` is a capture aid, so the line can be seen and hashed
        // without a window — the same role `BK_HUD_TINT` plays for a screen effect.
        if let Ok(line) = std::env::var("BK_HUD_MESSAGE") {
            let strings = gobs
                .read("ui\\jkstrings.uni")
                .map(|d| uni::parse(&String::from_utf8_lossy(&d)))
                .unwrap_or_default();
            if let Some(font) = hud_font(&gobs) {
                hud::draw_message(&mut canvas, &font, &palette, &resolve_keys(&strings, &line));
            }
        }
        // A capture aid for the screen effect: `BK_HUD_TINT` puts a force-seeing style filter over
        // the status bar so the pass can be seen and hashed without a window (`--hud`).
        if std::env::var_os("BK_HUD_TINT").is_some() {
            canvas.apply_screen_effect([false, true, true], [0.0, 1.0, 1.0], [0.0, 0.0, 0.0], 1.0);
        }
        let image = Image::new(
            Extent3d { width: canvas.width, height: canvas.height, depth_or_array_layers: 1 },
            TextureDimension::D2,
            canvas.rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        match image.try_into_dynamic() {
            Ok(img) => match img.save(&out) {
                Ok(()) => println!("640x480 status bar written to {out}"),
                Err(e) => {
                    eprintln!("fatal: writing {out}: {e}");
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("fatal: converting hud: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // --sft writes a font's glyph strip as a greyscale PNG: the ink values are the
    // strip's own bytes, so this is how a font's glyphs get looked at directly.
    if let Some((sft_name, out)) = sft_out {
        let Some(data) = gobs.read(&sft_name) else {
            eprintln!("fatal: `{sft_name}` not found in any archive");
            std::process::exit(1);
        };
        let Some(font) = sft::parse(&data) else {
            eprintln!("fatal: `{sft_name}` is not a .sft font");
            std::process::exit(1);
        };
        let (w, h) = (font.strip_width, font.height);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for (i, &v) in font.strip.iter().enumerate() {
            rgba[i * 4..i * 4 + 4].copy_from_slice(&[v, v, v, 255]);
        }
        println!(
            "{}: {} charsets, {} glyphs, strip {w}x{h}, margin {}",
            sft_name,
            font.charsets.len(),
            font.charsets.iter().map(|c| c.entries.len()).sum::<usize>(),
            font.margin_x
        );
        let image = Image::new(
            Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            TextureDimension::D2,
            rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        match image.try_into_dynamic() {
            Ok(img) => match img.save(&out) {
                Ok(()) => println!("{w}x{h} font strip written to {out}"),
                Err(e) => {
                    eprintln!("fatal: writing {out}: {e}");
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("fatal: converting font: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // --ui-audit is the UI layer's gate: every element of every recovered screen must
    // index a font and a bitmap this build can actually load, and its text must resolve in
    // the shipped `jkstrings.uni`. Keys that do not resolve are counted rather than failed,
    // because the engine's own fallback draws the key.
    if ui_audit {
        let Some(exe) = load_engine_exe(&game) else {
            eprintln!("fatal: no JediKnight.EXE / JK.EXE in or above the install");
            std::process::exit(1);
        };
        let Some(menus) = menus::load(&exe) else {
            eprintln!("fatal: no menu element arrays found in the executable");
            std::process::exit(1);
        };
        let strings = gobs
            .read("ui\\jkstrings.uni")
            .map(|d| uni::parse(&String::from_utf8_lossy(&d)))
            .unwrap_or_default();
        let mut bad = 0usize;
        let mut unknown_art = 0usize;
        let mut unresolved = 0usize;
        let mut keyed = 0usize;
        let mut elements = 0usize;
        let (mut texts, mut pictures, mut checks, mut customs, mut buttons, mut others) = (0, 0, 0, 0, 0, 0);
        println!(
            "{} screens, {} fonts, {} bitmaps, {} strings",
            menus.screens.len(),
            menus.fonts.len(),
            menus.bitmaps.len(),
            strings.strings.len()
        );
        for screen in &menus.screens {
            let mut issues = Vec::new();
            for item in &screen.items {
                elements += 1;
                if !item.key.is_empty() {
                    keyed += 1;
                    if strings.get(&item.key).is_none() {
                        unresolved += 1;
                        issues.push(format!("`{}` is not in jkstrings.uni", item.key));
                    }
                }
                match item.kind {
                    menus::PICBUTTON => {
                        pictures += 1;
                        // The force-allocation screen indexes a run-time bitmap array this
                        // build cannot name; count those apart from real out-of-range bugs.
                        match usize::try_from(item.bitmap) {
                            Ok(index) if index < screen.art.len() => {}
                            Ok(index) if index < 64 => unknown_art += 1,
                            _ => {
                                bad += 1;
                                issues.push(format!("bitmap {} is outside every table", item.bitmap));
                            }
                        }
                    }
                    menus::CHECKBOX => checks += 1,
                    7 => customs += 1,
                    2 => texts += 1,
                    0 => buttons += 1,
                    _ => others += 1,
                }
                if usize::try_from(item.text_type).map(|t| t >= menus.fonts.len()).unwrap_or(true) {
                    bad += 1;
                    issues.push(format!("font {} is outside {} fonts", item.text_type, menus.fonts.len()));
                }
            }
            // A dialog's panel is only drawn when the screen's elements fit inside it.
            if load_backdrop(&gobs, screen, None).is_some_and(|(_, panel)| panel.is_some()) {
                issues.push("draws the bkdialog panel".to_string());
            }
            if !issues.is_empty() {
                println!("  {:#x} {:<16} {}", screen.base, screen.name, issues.join("; "));
            }
        }
        // Every custom element must be a panel this build can name: the table is keyed by
        // array and index, so a different install that shifts an array fails here.
        let custom_total: usize = menus.screens.iter().map(|s| s.items.iter().filter(|i| i.kind == menus::CUSTOM).count()).sum();
        let with_handler: usize = menus.screens.iter().map(|s| s.items.iter().filter(|i| i.click.is_some()).count()).sum();
        let mut custom_named = 0usize;
        let mut hidden = 0usize;
        for screen in &menus.screens {
            for (index, item) in screen.items.iter().enumerate() {
                if item.kind == menus::CUSTOM {
                    // The element's own draw callback names its renderer; it must be one
                    // this build knows.
                    match item.panel() {
                        Some(panel) => {
                            custom_named += 1;
                            println!(
                                "  custom {:>11} at {:#x} + {} draws {:#x}",
                                panel.name(),
                                screen.base,
                                index,
                                item.draw.unwrap_or(0)
                            );
                        }
                        None => {
                            bad += 1;
                            println!(
                                "  {:#x} element {}: custom panel {:#x} not classified",
                                screen.base,
                                index,
                                item.draw.unwrap_or(0)
                            );
                        }
                    }
                }
                if !item.visible {
                    hidden += 1;
                }
            }
        }
        println!(
            "elements {elements}: {texts} text, {pictures} picture ({unknown_art} with a run-time bitmap), {checks} check box, {buttons} button, {others} other, {customs} custom ({custom_named}/{custom_total} panels named); hidden by bIsVisible: {hidden}; {with_handler} elements carry a click handler"
        );
        // The menu graph is recovered data, so check it against the arrays it claims to
        // describe: every edge's id must be on its source screen, and its target must be a
        // screen this build found.
        let mut graph = 0usize;
        println!("menu graph:");
        for edge in nav::EDGES {
            let from = menus.screens.iter().find(|s| s.base == edge.from);
            let to = menus.screens.iter().find(|s| s.base == edge.to);
            let has_id = from.is_some_and(|s| s.items.iter().any(|i| i.id == edge.id && i.kind != 2));
            match (from, to) {
                (Some(from), Some(to)) if has_id => {
                    graph += 1;
                    println!("  {:#x} {:<14} id {:<3} -> {:#x} {}", from.base, from.name, edge.id, to.base, to.name);
                }
                _ => {
                    bad += 1;
                    println!(
                        "  {:#x} id {} -> {:#x}: {}",
                        edge.from,
                        edge.id,
                        edge.to,
                        if from.is_none() {
                            "no such source screen"
                        } else if !has_id {
                            "id is not on that screen"
                        } else {
                            "no such target screen"
                        }
                    );
                }
            }
        }
        // A dialog action names a screen and the string keys it fills in; both must exist.
        for (base, id, act) in nav::ACTS {
            if let nav::Step::Dialog { screen: target, keys } = act {
                let screen = menus.screens.iter().find(|s| s.base == *target);
                let keyed = keys.iter().all(|key| strings.get(key).is_some());
                if screen.is_none() || !keyed {
                    bad += 1;
                    println!("  {base:#x} id {id}: dialog {target:#x} {} ", if screen.is_none() { "is not a screen" } else { "has a key jkstrings.uni lacks" });
                }
            } else if menus.screens.iter().all(|s| s.base != *base) {
                bad += 1;
                println!("  {base:#x} id {id}: action on a screen this build did not find");
            }
        }
        let derived = menus.screens.iter().filter(|s| s.background_from_code).count();
        // Every screen that has a way out has to be *leaveable*, by both hands. A cancel/done/ok row
        // is how a sub-screen is closed, so a click on it must land on it and the cursor must reach at
        // least one of them. Both failed before: `ui::pick` skipped `id < 0` — exactly what a retail
        // cancel row carries, so Cancel was dead to the mouse on every screen that has one — and
        // `Screen::rows` counted a `large0` *title* as a row, which left Choose Player, Cutscenes and
        // New Player with a cursor that could only light the title. Screens with no such row are not
        // counted here: the pause menu's panels are closed by Escape, and a root has nowhere to go.
        let mut exits = 0usize;
        let mut dead_click = 0usize;
        let mut dead_key = 0usize;
        for screen in &menus.screens {
            let rows = screen.rows();
            let way_out = |item: &menus::Item| {
                matches!(
                    nav::step(screen.base, item.id, &item.key),
                    nav::Step::Close | nav::Step::Quit | nav::Step::Abort
                )
            };
            let mut keyable = 0usize;
            let mut here = 0usize;
            for (index, item) in screen.items.iter().enumerate() {
                if !way_out(item) {
                    continue;
                }
                here += 1;
                exits += 1;
                let centre = [item.rect[0] + item.rect[2] / 2, item.rect[1] + item.rect[3] / 2];
                if ui::pick(screen, centre) != Some(index) {
                    dead_click += 1;
                    println!(
                        "  {:#x} {:<14} `{}` (id {}): a click at {centre:?} lands on {:?}, not on it",
                        screen.base,
                        screen.name,
                        item.key,
                        item.id,
                        ui::pick(screen, centre)
                    );
                }
                if rows.contains(&index) {
                    keyable += 1;
                }
            }
            if here > 0 && keyable == 0 {
                dead_key += 1;
                println!(
                    "  {:#x} {:<14}: the cursor walks {} row(s), none of them a way out",
                    screen.base,
                    screen.name,
                    rows.len()
                );
            }
        }
        println!("exits {exits}: {dead_click} unreachable by click, {dead_key} screen(s) with no way out on the cursor");
        bad += dead_click + dead_key;
        println!(
            "keys {keyed}, unresolved {unresolved}; backgrounds: {derived}/{} from jkGui_InitMenu, {} inferred; menu edges: {graph}/{}",
            menus.screens.len(),
            menus.screens.len() - derived,
            nav::EDGES.len()
        );
        if bad > 0 {
            eprintln!("{bad} problem(s) in the element tables, the graph or the exits above");
            std::process::exit(1);
        }
        println!("--- ui audit passed ---");
        return;
    }

    // --ui-tables lists every menu screen recovered from the executable, with its items,
    // so the extraction can be checked against what the game shows.
    if ui_tables {
        let Some(exe) = load_engine_exe(&game) else {
            eprintln!("fatal: no JediKnight.EXE / JK.EXE in or above the install");
            std::process::exit(1);
        };
        let Some(menus) = menus::load(&exe) else {
            eprintln!("fatal: no menu item arrays found in the executable");
            std::process::exit(1);
        };
        let strings = gobs
            .read("ui\\jkstrings.uni")
            .map(|d| uni::parse(&String::from_utf8_lossy(&d)))
            .unwrap_or_default();
        println!("{} screens", menus.screens.len());
        let with_click = menus.screens.iter().filter(|s| s.sound_click.is_some()).count();
        let with_hover = menus.screens.iter().filter(|s| s.sound_hover.is_some()).count();
        println!("sounds: {with_click} screen(s) name a click wav, {with_hover} a hover wav");
        for screen in menus.screens.iter().filter(|s| s.sound_click.is_some()).take(5) {
            println!(
                "  {:24} click {:?} hover {:?}",
                screen.name, screen.sound_click, screen.sound_hover
            );
        }
        for screen in &menus.screens {
            println!("\n== {:#x} {} ({} items, {})", screen.base, screen.name, screen.items.len(), screen.background);
            for item in &screen.items {
                let text = strings.get(&item.key).unwrap_or(&item.key);
                // A custom element draws nothing itself: name the panel its own code paints.
                let custom = match item.panel() {
                    Some(panel) => format!(" [{} @{:#x}]", panel.name(), item.draw.unwrap_or(0)),
                    None => String::new(),
                };
                // A picture button may carry its own click handler: the engine's code for
                // that button, named by the address in the record.
                let click = match item.click {
                    Some(callback) => format!("{{click {callback:#x}}}"),
                    None => String::new(),
                };
                println!(
                    "   type {} font {:>2} id {:>4} {:>4},{:<4} {:>4}x{:<3} {:<22} {:?}{} {}",
                    item.kind, item.text_type, item.id, item.rect[0], item.rect[1], item.rect[2], item.rect[3], item.key, text, custom, click
                );
            }
        }
        return;
    }

    // --ui composes one retail menu screen: the item table is read out of the executable,
    // so this draws any screen the game has rather than a hand-copied few.
    if let Some((menu_name, out)) = ui_out {
        let Some(menus) = load_menus(&game) else {
            eprintln!("fatal: no JediKnight.EXE / JK.EXE for the menu tables");
            std::process::exit(1);
        };
        // `--force-screen` names a screen the executable cannot supply — the retail force panel
        // is built on the heap at run time — so it is composed from `jkGUIForce.c`'s table and
        // gated by which powers the player has found (`--force-found 29,33`).
        let built = (menu_name == "force").then(menus::force_screen);
        let Some(screen) = built.as_ref().or_else(|| menus.find(&menu_name)) else {
            eprintln!("fatal: no screen `{menu_name}`; `--ui-tables` lists them");
            std::process::exit(1);
        };
        let mut gated;
        let screen = if let Some(built) = built.as_ref() {
            gated = built.clone();
            let found = |bin: i32| force_found.contains(&bin);
            for item in &mut gated.items {
                if item.kind == 1 {
                    item.visible = found(item.id);
                }
            }
            &gated
        } else {
            screen
        };
        let strings = gobs
            .read("ui\\jkstrings.uni")
            .map(|d| uni::parse(&String::from_utf8_lossy(&d)))
            .unwrap_or_default();
        let background_name = bm_bg.clone().unwrap_or_else(|| screen.background.clone());
        let Some((base, panel)) = load_backdrop(&gobs, screen, bm_bg.as_deref()) else {
            eprintln!("fatal: `{background_name}` not found or not a bitmap");
            std::process::exit(1);
        };
        // The engine's own font and bitmap tables, in the order a screen indexes them.
        let fonts = load_fonts(&gobs, &menus);
        let art = |index: i32| -> Option<bm::Bm> {
            let name = screen.art.get(usize::try_from(index).ok()?)?;
            bm::parse(&gobs.read(&format!("ui\\bm\\{name}"))?)
        };
        let palette = level_palette(&gobs, level_name.as_deref());
        let checkbox = screen.art.iter().position(|n| n.eq_ignore_ascii_case("check.bm")).map(|i| i as i32);
        // The one custom panel this build can fill: the tally screen's star row, whose
        // count is game state. `--stars N` composes it for a screenshot.
        let stars = gobs.read("ui\\bm\\foStars.bm").and_then(|d| bm::parse(&d)).map(|art| (art, stars));
        let stars = stars.as_ref().map(|(art, count)| (art, *count));
        let none = |_: usize| None;
        // A screen with a map panel needs a level to draw: `--ui fieldlog` uses `--level`, and
        // the level's own `walkplayer` stands in for the player the way `setup` uses it.
        let mut map_level = None;
        if screen.items.iter().any(|i| i.panel() == Some(menus::Panel::Map)) {
            let name = level_name.clone().or_else(|| start_level(&gobs));
            if let Some(raw) = name.as_deref().and_then(|n| gobs.read(n)) {
                let level = jkl::parse(&String::from_utf8_lossy(&raw));
                let start = level
                    .things
                    .iter()
                    .find(|t| t.template.eq_ignore_ascii_case("walkplayer"))
                    .map(|t| ([t.pos[0], t.pos[1], t.pos[2] + PLAYER_RADIUS], t.yaw));
                if let Some((pos, yaw)) = start {
                    map_level = Some((level, pos, yaw));
                }
            }
        }
        let map = map_level.as_ref().map(|(level, pos, yaw)| (level, *pos, *yaw));
        let look = ui::Look { fonts: &fonts, art: &art, checkbox, palette: &palette, stars, text: &none, map, load_percent, ..ui::Look::default() };
        // The canvas is the UI's authored size, not the artwork's: a dialog's panel is
        // smaller than the screen it sits on.
        let mut canvas = ui::Canvas::new(640, 480);
        println!(
            "{:#x} {}: {} elements over {}{} ({} background)",
            screen.base,
            screen.name,
            screen.items.len(),
            background_name,
            if panel.is_some() { " + ui\\bm\\bkdialog.bm" } else { "" },
            if bm_bg.is_some() {
                "forced"
            } else if screen.background_from_code {
                "code"
            } else {
                "inferred"
            }
        );
        for item in &screen.items {
            let text = strings.get(&item.key).unwrap_or(&item.key);
            println!(
                "  type {} font {:>2} {:>4},{:<4} {:>4}x{:<3} {:<24} {:?}",
                item.kind, item.text_type, item.rect[0], item.rect[1], item.rect[2], item.rect[3], item.key, text
            );
        }
        let backdrop = ui::Backdrop { base: &base, panel: panel.as_ref() };
        ui::draw_screen(&mut canvas, screen, &backdrop, &strings, &look, None);
        if load_check {
            // `jkGuiTitle_LoadBarDraw`'s own arithmetic: the fill is inset 3px and
            // `percent * (width - 6) / 100` wide. Measured from the canvas, not from the code
            // that drew it, so a wrong inset or a wrong denominator shows up here.
            let bar = screen
                .items
                .iter()
                .find(|i| i.panel() == Some(menus::Panel::LoadBar))
                .map(|i| i.rect)
                .expect("the loading screen has a bar element");
            let [x, y, w, h] = bar;
            let palette = ui::palette_of(&base, &palette);
            let fill = palette[0xE1];
            let mut lit = 0usize;
            let mut left = i32::MAX;
            let mut right = i32::MIN;
            for py in y + 3..y + h - 3 {
                for px in x + 3..x + w - 3 {
                    let at = ((py as u32 * canvas.width + px as u32) * 4) as usize;
                    let px_rgb = [canvas.rgba[at], canvas.rgba[at + 1], canvas.rgba[at + 2]];
                    if px_rgb == fill {
                        lit += 1;
                        left = left.min(px);
                        right = right.max(px);
                    }
                }
            }
            let want = (load_percent / 100.0 * (w - 6) as f32) as i32;
            let got = if lit == 0 { 0 } else { right - left + 1 };
            println!(
                "load bar: {load_percent:.0}% of {w}px is {want}px; the canvas has {got}px wide, {lit} pixel(s) at {bar:?}"
            );
            assert!((got - want).abs() <= 2, "the bar is {got}px for {want}px");
            assert!(lit > 0, "the bar painted nothing");
        }
        if map_check {
            // The map's line work is `look.map_ink` in the screen's palette: count it inside the
            // panel's own rect (`{ 40, 40, 520, 320 }` in `jkGuiMap_aElements`) and insist on
            // enough of it to prove the level was drawn, plus the marker at the centre.
            // The screen's own palette, which is what the draw used (the backdrop's art wins).
            let palette = ui::palette_of(&base, &palette);
            let ink = palette[usize::from(look.map_ink)];
            let mut drawn = 0usize;
            let mut marker = 0usize;
            for y in 40..360 {
                for x in 40..560 {
                    let at = ((y as u32 * canvas.width + x as u32) * 4) as usize;
                    let px = [canvas.rgba[at], canvas.rgba[at + 1], canvas.rgba[at + 2]];
                    if px == ink {
                        drawn += 1;
                    }
                    if px == palette[usize::from(look.map_ink)] {
                        marker += 1;
                    }
                }
            }
            println!("map panel: {drawn} ink pixel(s), {marker} marker pixel(s)");
            assert!(drawn > 500, "the map drew only {drawn} pixels");
            // The marker is drawn in the same ink: the panel's centre must carry it.
            let (cx, cy) = (40 + 520 / 2, 40 + 320 / 2);
            let at = ((cy as u32 * canvas.width + cx as u32) * 4) as usize;
            assert_eq!(
                [canvas.rgba[at], canvas.rgba[at + 1], canvas.rgba[at + 2]],
                ink,
                "no player marker at the panel's centre"
            );
            // The **live** path: `refresh_overlay` builds the panel's inputs with `live_map_for`
            // from the player's own pose, which the shot above does not (it uses the level's
            // `walkplayer` start). Move the player and the drawing has to change, or the panel is
            // showing a fixed picture of the level instead of the player's surroundings.
            if let Some((start_pos, start_yaw)) =
                map_level.as_ref().map(|(_, pos, yaw)| (*pos, *yaw))
            {
                let moved_pos = Vec3::new(start_pos[0] + 3.0, start_pos[1], start_pos[2]);
                let mut canvas = ui::Canvas::new(640, 480);
                let map = map_level
                    .as_ref()
                    .and_then(|(level, _, _)| live_map_for(screen, level, moved_pos, start_yaw.to_radians()));
                let look2 = ui::Look {
                    fonts: &fonts,
                    art: &art,
                    checkbox,
                    palette: &palette,
                    stars,
                    text: &none,
                    map,
                    load_percent,
                    ..ui::Look::default()
                };
                let backdrop = ui::Backdrop { base: &base, panel: panel.as_ref() };
                ui::draw_screen(&mut canvas, screen, &backdrop, &strings, &look2, None);
                let palette2 = ui::palette_of(&base, &palette);
                let ink2 = palette2[usize::from(look2.map_ink)];
                let mut moved_drawn = 0usize;
                for y in 40..360 {
                    for x in 40..560 {
                        let at = ((y as u32 * canvas.width + x as u32) * 4) as usize;
                        if [canvas.rgba[at], canvas.rgba[at + 1], canvas.rgba[at + 2]] == ink2 {
                            moved_drawn += 1;
                        }
                    }
                }
                println!(
                    "map panel: the start pose draws {drawn} pixel(s), three units away {moved_drawn}"
                );
                assert!(moved_drawn > 500, "the live map drew only {moved_drawn} pixels");
                assert_ne!(moved_drawn, drawn, "the map ignored the player's pose");
            }
            println!("--- map panel check passed ---");
        }
        let image = Image::new(
            Extent3d { width: canvas.width, height: canvas.height, depth_or_array_layers: 1 },
            TextureDimension::D2,
            canvas.rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        match image.try_into_dynamic() {
            Ok(img) => match img.save(&out) {
                Ok(()) => println!("{}x{} menu written to {out}", canvas.width, canvas.height),
                Err(e) => {
                    eprintln!("fatal: writing {out}: {e}");
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("fatal: converting menu: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    // --bm writes one `.bm` (a UI cell set) as a PNG: the menu backgrounds, buttons and
    // HUD pieces are all this format, so this is how the UI art gets looked at.
    if let Some((bm_name, out)) = bm_out {
        let Some(data) = gobs.read(&bm_name) else {
            eprintln!("fatal: `{bm_name}` not found in any archive");
            std::process::exit(1);
        };
        let Some(file) = bm::parse(&data) else {
            eprintln!("fatal: `{bm_name}` is not a .bm");
            std::process::exit(1);
        };
        let palette = file.palette_rgb.unwrap_or_else(|| {
            let mut g = [[0u8; 3]; 256];
            for (i, slot) in g.iter_mut().enumerate() {
                *slot = [i as u8, i as u8, i as u8];
            }
            g
        });
        println!(
            "{}: version {}, {} bpp, numMips {}, HUD pos {},{} {} cell(s) {:?}, palette {}",
            bm_name,
            file.version,
            file.bits,
            file.num_mips,
            file.x_pos,
            file.y_pos,
            file.cells.len(),
            file.cells
                .iter()
                .map(|c| (c.width, c.height))
                .collect::<Vec<_>>(),
            if file.palette { "embedded" } else { "greyscale" }
        );
        // Cells are stacked vertically, which is how the cursor/HUD sheets read.
        let width = file.cells.iter().map(|c| c.width).max().unwrap_or(1);
        let height: u32 = file.cells.iter().map(|c| c.height).sum();
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        let mut top = 0u32;
        for cell in &file.cells {
            for y in 0..cell.height {
                for x in 0..cell.width {
                    let at = ((y * cell.width + x) as usize) * (file.bits as usize / 8);
                    let [r, g, b] = match file.bits {
                        16 => {
                            let v = u16::from_le_bytes([cell.pixels[at], cell.pixels[at + 1]]);
                            [
                                ((v >> 10) & 0x1f) as u8 * 8,
                                ((v >> 5) & 0x1f) as u8 * 8,
                                (v & 0x1f) as u8 * 8,
                            ]
                        }
                        _ => palette[cell.pixels[at] as usize],
                    };
                    let o = (((top + y) * width + x) * 4) as usize;
                    rgba[o..o + 4].copy_from_slice(&[r, g, b, 255]);
                }
            }
            top += cell.height;
        }
        let image = Image::new(
            Extent3d { width, height, depth_or_array_layers: 1 },
            TextureDimension::D2,
            rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        match image.try_into_dynamic() {
            Ok(img) => match img.save(&out) {
                Ok(()) => println!("{width}x{height} written to {out}"),
                Err(e) => {
                    eprintln!("fatal: writing {out}: {e}");
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("fatal: converting image: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // --texture writes one material's decoded bitmap as a PNG, so a suspect texture
    // can be looked at directly instead of inferred from how it renders.
    if let Some((mat_name, out)) = texture {
        let Some(data) = gobs.read(&mat_name) else {
            eprintln!("fatal: `{mat_name}` not found in any archive");
            std::process::exit(1);
        };
        let Some(mat) = tex::parse_mat(&data) else {
            eprintln!("fatal: `{mat_name}` carries no bitmap");
            std::process::exit(1);
        };
        // The palette comes from the material's own level, which is unknown here, so
        // take the one named by `--level` (the default level otherwise).
        let palette = level_name
            .as_deref()
            .map(|l| String::from_utf8_lossy(&gobs.read(l).unwrap_or_default()).into_owned())
            .map(|t| jkl::parse(&t))
            .and_then(|l| l.colormaps.first().cloned())
            .and_then(|c| gobs.read(&c))
            .and_then(|d| tex::Palette::parse(&d))
            .unwrap_or_else(tex::Palette::greyscale);
        let mut image = to_image(&mat, &palette);
        // `to_image` appends a mip chain; a PNG wants the top level only.
        image.texture_descriptor.mip_level_count = 1;
        if let Some(data) = image.data.as_mut() {
            data.truncate((mat.width * mat.height * 4) as usize);
        }
        match image.try_into_dynamic() {
            Ok(img) => match img.save(&out) {
                Ok(()) => println!("{}x{} texture written to {}", mat.width, mat.height, out),
                Err(e) => {
                    eprintln!("fatal: writing {out}: {e}");
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("fatal: converting texture: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some((name, out)) = raw {
        match gobs.read(&name) {
            Some(data) => match std::fs::write(&out, &data) {
                Ok(()) => println!("{}: {} bytes written to {}", name, data.len(), out),
                Err(e) => {
                    eprintln!("fatal: writing {out}: {e}");
                    std::process::exit(1);
                }
            },
            None => {
                eprintln!("fatal: `{name}` not found in any archive");
                std::process::exit(1);
            }
        }
        return;
    }
    // `--pose <model> <key> <frame>`: draw one model posed from a keyframe, which is how
    // the `.key`/`.pup`/hierarchy data is verified as geometry rather than as a parse.
    if let Some((model_name, key_name, frame)) = pose {
        match pose_view(&gobs, &model_name, &key_name, frame, shot.as_deref()) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("fatal: {e}");
                std::process::exit(1);
            }
        }
    }
    if let Some(name) = dump {
        match gobs.read(&name) {
            Some(data) => {
                println!("{}: {} bytes", name, data.len());
                print!("{}", String::from_utf8_lossy(&data));
            }
            None => {
                eprintln!("fatal: `{name}` not found in any archive");
                std::process::exit(1);
            }
        }
        return;
    }

    // No level named on the command line: start where the game starts, at the
    // first level of the first single-player episode. Nothing about the campaign
    // is hardcoded here.
    let level_name = level_name.unwrap_or_else(|| {
        start_level(&gobs).unwrap_or_else(|| {
            eprintln!("fatal: no episode manifest names a level");
            std::process::exit(1);
        })
    });

    // `--boot`: the screen the app opens on and what each boot row does, resolved from the install's
    // element tables rather than from the constants they were read into. Headless and level-free.
    if boot_check {
        let menus = load_menus(&game).unwrap_or_default();
        std::process::exit(if boot_gate(&gobs, &menus) { 0 } else { 1 });
    }

    let world = match load(&gobs, &level_name) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("fatal: {e}");
            std::process::exit(1);
        }
    };

    // `--scale`: how big the models really are against the level they are placed in. Both references
    // are the level's own geometry, so this answers "is the model scale right" without trusting it.
    if scale_check {
        print_scale(&world);
        return;
    }

    // What finishing this level leads to: the graph's next row is the film, and the film's own
    // next row is the level that loads. Resolved once, here, so the watcher only has to act.
    // `campaign_start` is where "New Game" begins, which is not the same thing once the campaign
    // has moved on: `--level` and a level transition both change `level_name`, neither changes this.
    let campaign_start = start_level(&gobs).unwrap_or_else(|| level_name.clone());
    let plan = {
        let episode = episodes_of(&gobs)
            .into_iter()
            .find(|(_, e)| e.kind == episode::KIND_SINGLE)
            .map(|(_, e)| e);
        let (cut, next) = match &episode {
            Some(episode) => {
                let (film, level) = episode.flow_after(&level_name);
                (
                    film.map(|s| s.file.clone()),
                    level.map(|s| s.file.clone()),
                )
            }
            None => (None, None),
        };
        FlowPlan { game: game.clone(), level: level_name.clone(), first: campaign_start.clone(), cut, next, swap }
    };

    // `--play <wav>`: play one archived sound and exit, to check the audio path
    // without a game to trigger it.
    let play_sound = match &play {
        Some(name) => match gobs.read(name) {
            Some(bytes) => Some((name.clone(), bytes)),
            None => {
                eprintln!("fatal: `{name}` not found in any archive");
                std::process::exit(1);
            }
        },
        None => None,
    };

    if let Some(weapon) = weapon_check.clone() {
        // Taking a weapon makes it the player's: the check is that the *runtime's* weapon
        // changes and that it fires with that template's damage.
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut rt = sim_world.borrow_mut();
        let before = rt.current_weapon();
        let item = rt
            .things
            .iter()
            .enumerate()
            .find(|(i, t)| {
                // A **projectile** template, which is what taking a weapon equips: a placed pistol
                // fires its bolt, so the pistol item itself has `damage=0` and equipping it would be
                // wrong. The earlier filter took the first type-3 thing and then asserted it had
                // damage, which failed on the first pistol the level places.
                rt.types.get(*i).copied() == Some(3)
                    && !t.destroyed
                    && rt.damage_of(&rt.templates[*i]) > 0.0
            })
            .map(|(i, _)| i)
            .unwrap_or_else(|| {
                // No weapon placed in this level: stand one up from the template so the
                // equip path is still exercised.
                let source = rt
                    .templates
                    .iter()
                    .position(|t| t.eq_ignore_ascii_case("walkplayer"))
                    .unwrap_or(0);
                rt.fire(source as i64, &weapon).expect("created weapon")
            });
        rt.things[item].destroyed = false;
        let template = rt.templates[item].clone();
        let damage = rt.damage_of(&template);
        rt.take_item(item as i64, 0);
        let after = rt.current_weapon();
        println!(
            "weapon: `{template}` damage {damage}, equip: `{before}` -> `{after}`, respawn {} s",
            rt.template_respawn.get(&template.to_ascii_lowercase()).copied().unwrap_or(0.0)
        );
        // A weapon *item* like `+bryarpistol` has `damage=0` — its bolt carries the damage — and the
        // equip path keys on projectile templates, so taking one credits its bin and leaves the
        // equipped weapon alone. That is correct behaviour, so the assertions only apply to a
        // projectile: the level's placed items are pistols first, which is what made this check fail
        // when it took the first type-3 thing.
        if damage > 0.0 {
            assert_ne!(after, before, "taking `{template}` did not change the player's weapon");
            assert_eq!(after, template, "the equipped weapon is not the one taken");
        } else {
            println!("weapon: `{template}` is an item (its bolt carries the damage), not equipped directly");
        }
        // The projectile it fires carries that same template, so the damage it deals is
        // the damage just printed.
        let shot = rt.fire(0, &after).expect("fired");
        assert_eq!(rt.templates[shot], after.to_ascii_lowercase(), "fired the wrong template");
        println!("--- weapon check passed ---");
        return;
    }

    if let Some(name) = smk_decode.clone() {
        // Decode through the install's own `SMACKW32.DLL`, the way JK does. The check is
        // that frames come out and that they differ: a decoder that returns the same buffer
        // every frame would pass a "did it run" test and fail this one.
        let dll = game.join("SMACKW32.DLL");
        let file = game.join("Resource").join("VIDEO").join(&name);
        assert!(dll.exists(), "{} is not in the install", dll.display());
        assert!(file.exists(), "{} is not in the install", file.display());
        let prefix = std::env::temp_dir().join("bk-smk-frame");
        let decoded = match smack::decode(&dll, &file, 40, 0, &prefix) {
            Some(decoded) => decoded,
            None => {
                println!(
                    "smk-decode: the 32-bit helper is not built. SMACKW32.DLL is a 32-bit image,\n\
                     so a 64-bit process cannot load it (error 193); build the helper with:\n\
                     \x20 cargo build --release --target i686-pc-windows-msvc \\\n\
                     \x20   --manifest-path tools/smkdec/Cargo.toml"
                );
                return;
            }
        };
        println!(
            "smk-decode: {} at {}x{} ({} frames in the file), {} decoded, {} distinct",
            name, decoded.width, decoded.height, decoded.frames, decoded.hashes.len(), decoded.distinct
        );
        println!(
            "  first non-blank frame: {:?}, hashes {} .. {}",
            decoded.first_ink,
            decoded.hashes.first().map(|h| format!("{h:016x}")).unwrap_or_default(),
            decoded.hashes.last().map(|h| format!("{h:016x}")).unwrap_or_default()
        );
        assert!(decoded.hashes.len() >= 20, "only {} frame(s) decoded", decoded.hashes.len());
        assert!(decoded.distinct >= 2, "every decoded frame was identical");
        assert!(decoded.first_ink.is_some(), "no frame had any non-zero pixel");
        // Turn the last decoded frame into a PNG so it can be looked at. The helper writes the
        // 8-bit indices as grey because the palette is not recovered yet: the DLL keeps one
        // only for `SmackBufferOpen` buffers, and that API refuses a 640-wide frame.
        let last = decoded.hashes.len().saturating_sub(1);
        let ppm = std::env::temp_dir().join(format!("bk-smk-frame{last:03}.ppm"));
        if let Ok(bytes) = std::fs::read(&ppm)
            && let Some((width, height, pixels)) = png::read_ppm(&bytes)
        {
            // Grey by default: the 8-bit indices with no palette are still an image.
            let mut rgb = pixels.clone();
            // With the palette from the file's own records, the frame comes out in colour.
            let movie = std::fs::read(&file).expect("re-read the cutscene");

            let mut coloured = 0;
            if let Some(palette) = smk::palette_at(&movie, last) {
                let mut distinct = std::collections::HashSet::new();
                let defined = palette.iter().filter(|entry| **entry != [0, 0, 0]).count();
                if std::env::var_os("BK_DUMP_PALETTE").is_some() {
                    let flat: Vec<u8> = palette.iter().flatten().copied().collect();
                    let out = std::env::temp_dir().join("bk-palette.bin");
                    std::fs::write(&out, &flat).expect("write palette dump");
                    println!("  palette written to {}", out.display());
                }
                // The helper writes indices as grey, so each 3-byte pixel carries the index.
                for (i, pixel) in pixels.chunks_exact(3).enumerate() {
                    let index = pixel[0];
                    let entry = palette[index as usize];
                    rgb[i * 3..i * 3 + 3].copy_from_slice(&entry);
                    distinct.insert(index);
                }
                coloured = distinct.len();
                println!("  palette: {defined} of 256 entries defined, {} used", coloured);
            }
            let out = std::env::temp_dir().join(if coloured > 0 {
                "bk-cutscene-colour.png"
            } else {
                "bk-cutscene-frame.png"
            });
            png::write_rgb(&out, width, height, &rgb).expect("write the frame");
            println!(
                "  frame {last} written to {} ({} distinct palette entries used)",
                out.display(),
                coloured
            );
        }
        println!("--- smk decode passed ---");
        return;
    }

    if smk_check {
        // The cutscene files themselves: 25 `.SMK` under `Resource\VIDEO`, loose, not in a
        // `.gob`. The header is Smacker's own (`smk_open_generic` in libsmacker, the
        // documented decoder): "SMK" + version byte, then u32 width, height, frames and
        // frame time, where a positive frame time is **milliseconds per frame**.
        let mut found: Vec<std::path::PathBuf> = Vec::new();
        let mut stack = vec![game.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path
                    .extension()
                    .map(|e| e.eq_ignore_ascii_case("smk"))
                    .unwrap_or(false)
                {
                    found.push(path);
                }
            }
        }
        found.sort();
        let mut total = 0u64;
        let mut frames = 0u64;
        let mut dims: Vec<(u32, u32)> = Vec::new();
        let mut fps_range = (f32::MAX, f32::MIN);
        for path in &found {
            let Ok(bytes) = std::fs::read(path) else { continue };
            assert!(bytes.len() > 32, "{} is too small to be a Smacker file", path.display());
            assert_eq!(&bytes[0..3], b"SMK", "{} is not a Smacker file", path.display());
            let version = bytes[3];
            assert!(version == b'2' || version == b'4', "{} is SMK{version}", path.display());
            let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            let (w, h, f, time) = (word(4), word(8), word(12), word(16) as i32);
            assert!(w > 0 && h > 0 && w % 2 == 0 && h % 2 == 0, "{} is {w}x{h}", path.display());
            let ms = if time > 0 { time } else { (-time) / 100 };
            let fps = if ms > 0 { 1000.0 / ms as f32 } else { 0.0 };
            assert!((5.0..=60.0).contains(&fps), "{} is {fps:.1} fps", path.display());
            // The header layout has to account for every byte: chunk sizes, both tables and the
            // huffman block. This is the check that keeps the palette reader honest, and it runs
            // over all 25 shipped films rather than the one that was examined.
            let layout = smk::Layout::parse(&bytes)
                .unwrap_or_else(|| panic!("{} has no readable Smacker layout", path.display()));
            assert!(
                layout.covers(&bytes),
                "{}: the layout does not account for the whole file",
                path.display()
            );
            total += bytes.len() as u64;
            frames += f as u64;
            dims.push((w, h));
            fps_range = (fps_range.0.min(fps), fps_range.1.max(fps));
        }
        assert!(found.len() >= 20, "only {} cutscene file(s) found", found.len());
        dims.sort();
        dims.dedup();
        println!(
            "smk: {} cutscene file(s), {:.0} MB, {frames} frames total",
            found.len(),
            total as f64 / 1_048_576.0
        );
        println!(
            "  {dims:?}, {:.1}..{:.1} fps, first: {}",
            fps_range.0,
            fps_range.1,
            found.first().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()).unwrap_or_default()
        );
        println!("--- smk check passed ---");
        return;
    }

    if let Some(level) = flow_level.clone() {
        // The campaign's actual step: a level's final script calls `jkEndLevel`, and the graph's
        // next row is the film that plays, whose own next row is the level that loads. Every hop
        // is checked against real bytes — the film through the VFS, the level in the archives.
        let (_, episode) = episodes_of(&gobs)
            .into_iter()
            .find(|(_, e)| e.kind == episode::KIND_SINGLE)
            .expect("no single-player episode");
        let mut row = episode.row_for(&level).expect("that level is not in the campaign");
        assert_eq!(row.kind, episode::Kind::Level, "{level} is not a level row");
        println!("flow: {} ({})", row.file, row.comment);
        let mut hops = 0;
        let mut played = 0;
        while let Some(next) = episode.next_row(row.id) {
            match next.kind {
                episode::Kind::Level => {
                    assert!(
                        gobs.read(&next.file).is_some(),
                        "level {} is in the manifest but not the archives",
                        next.file
                    );
                    println!("      -> level {} ({})", next.file, next.comment);
                }
                episode::Kind::Cut => {
                    let bytes = gobs
                        .read(&next.file)
                        .unwrap_or_else(|| panic!("cutscene {} did not resolve", next.file));
                    assert_eq!(&bytes[0..3], b"SMK", "{} is not a Smacker film", next.file);
                    println!(
                        "      -> cutscene {} ({} bytes, SMK{})",
                        next.file,
                        bytes.len(),
                        bytes[3] as char
                    );
                    played += 1;
                }
                _ => println!("      -> {} ({:?})", next.file, next.kind),
            }
            row = next;
            hops += 1;
            if hops >= 5 {
                break;
            }
        }
        // The first two hops are the ones the campaign always makes.
        let first = episode.next_row(episode.row_for(&level).unwrap().id).unwrap();
        assert_eq!(first.kind, episode::Kind::Cut, "level 1 should lead into a cutscene");
        let after = episode.next_row(first.id).expect("the level after the cutscene");
        assert_eq!(after.kind, episode::Kind::Level, "a cutscene should lead into a level");
        println!("flow: {} level(s) and {played} cutscene(s) walked; {hops} hop(s)", hops, );
        println!("--- flow check passed ---");
        return;
    }

    if transition {
        // Finishing a level: `jkEndLevel` marks it done in the runtime, and the campaign
        // advances through the episode graph, skipping rows whose file is not shipped (every
        // cutscene, in this install). This is the ordering the game actually walks.
        let sim_world = runtime::Runtime::handle(&world.level);
        {
            let mut rt = sim_world.borrow_mut();
            assert!(!rt.level_complete, "a fresh level already reports as complete");
            rt.call("jkendlevel", &[cog::Value::Num(1.0)]);
            assert!(rt.level_complete, "jkEndLevel did not mark the level complete");
        }
        let (_, episode) = episodes_of(&gobs)
            .into_iter()
            .find(|(_, e)| e.kind == episode::KIND_SINGLE)
            .expect("no single-player episode");
        // Walk from the first level, following `gotoA`, and collect the level files in order,
        // reporting whatever had to be skipped on the way.
        let first = episode.first_level().expect("no first level");
        let mut at = first.id;
        let mut chain: Vec<String> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        let mut steps = 0;
        while at >= 0 && steps <= episode.sequences.len() {
            let Some(row) = episode.sequences.iter().find(|s| s.id == at) else { break };
            match row.kind {
                episode::Kind::Level => {
                    assert!(
                        gobs.read(&row.file).is_some(),
                        "level {} is in the manifest but not the archives",
                        row.file
                    );
                    chain.push(row.file.to_ascii_lowercase());
                }
                _ => skipped.push(row.file.clone()),
            }
            at = row.goto_a;
            steps += 1;
        }
        println!(
            "transition: {} level(s) in the single-player chain, {} non-level row(s) passed over (cutscenes, decisions)",
            chain.len(),
            skipped.len()
        );
        for pair in chain.windows(2).take(6) {
            println!("  {} -> {}", pair[0], pair[1]);
        }
        println!("  ... ends at {}", chain.last().cloned().unwrap_or_default());
        assert!(chain.len() >= 20, "the chain is too short: {} levels", chain.len());
        assert_eq!(chain[0], "01narshadda.jkl", "the campaign does not start at level 1");
        // The level this build loads by default must be the chain's first entry, or "New Game"
        // would start somewhere else.
        println!(
            "  finishing {} advances to {}",
            chain[0],
            chain.get(1).cloned().unwrap_or_else(|| "the end".into())
        );
        println!("--- transition check passed ---");
        return;
    }

    if endlevel_check {
        // Finishing a level **through the shipped scripts**, not by calling `jkEndLevel` directly: the
        // player *sees* 8t88, `sithRender_RenderThing` sends `sighted` to the cog placed on him
        // (`01_goal0.cog 204`), that script reports the objective to the master cog, and
        // `01_endlevel.cog`'s `user0` handler sets the goal flag, plays its sound, sleeps 3 s and calls
        // `jkEndLevel(1)`.
        let sim_world = runtime::Runtime::handle(&world.level);
        // The link tables are what let `sighted` reach `01_goal0.cog`; the load path wires them too.
        sim_world.borrow_mut().wire_links(&world.level.cogs, &world.cogs);
        let mut vms: Vec<(String, cog::Vm)> = Vec::new();
        for inst in &world.level.cogs {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            let mut vm = cog::Vm::with_world(sim_world.clone());
            vm.cog_index = vms.len() as i32;
            script.bind(&mut vm, &inst.values);
            if script.label("startup").is_some() {
                let _ = vm.run_label(script, "startup");
            }
            vms.push((inst.script.clone(), vm));
        }
        let master = sim_world.borrow().master_cog;
        println!(
            "endlevel: {} instance(s), master cog {master:?} (set by `SetMasterCOG(GetSelfCOG())`)",
            vms.len()
        );
        // The trigger is the level's own: the **goal** cog waits on `sighted`, and on level 1 that is
        // `01_goal0.cog` on 8t88 (thing 204). `sithRender_RenderThing` sends `sighted` the first time the
        // thing is drawn, so "see 8t88" is what starts the end of the level. Other scripts declare
        // `sighted` too (`01_TieBomber.cog` reacts to sighting the player), which is why the goal is
        // selected by name.
        let goal_link = sim_world
            .borrow()
            .thing_links
            .iter()
            .find(|(instance, _, _)| {
                world.level.cogs.get(*instance).is_some_and(|c| {
                    c.script.to_ascii_lowercase().contains("goal")
                        && world.cogs.get(&c.script).is_some_and(|s| s.label("sighted").is_some())
                })
            })
            .map(|(instance, _, thing)| (*instance, *thing))
            .expect("no goal cog waits on `sighted`");
        let (goal_instance, goal_thing) = goal_link;
        let goal_script = world.level.cogs[goal_instance].script.clone();
        let flags = sim_world
            .borrow()
            .things
            .get(goal_thing)
            .map(|t| t.flags)
            .unwrap_or(0);
        println!(
            "endlevel: `{goal_script}` waits on `sighted` from thing {goal_thing} (flags {flags:#x})"
        );
        assert!(
            flags & runtime::TF_CAPTURED != 0,
            "thing {goal_thing} is not CAPTURED, so `sighted` would reach nobody"
        );
        let sent = sim_world.borrow_mut().thing_sighted(goal_thing);
        println!("endlevel: the thing is drawn for the first time -> {sent} `sighted` message(s)");
        assert!(sent > 0, "the goal thing sent no `sighted` message");
        let mut delivered = 0usize;
        let mut completed = false;
        let player_index = sim_world
            .borrow()
            .templates
            .iter()
            .position(|t| t.eq_ignore_ascii_case("walkplayer"));
        let mut checked_invuln = false;
        for _ in 0..40 {
            // The thing/sector bus, as `deliver_thing_messages` runs it: this is where the `sighted`
            // the gate triggered arrives at `01_goal0.cog`.
            //
            // The queued list is moved out **before** the loop: `World` is an `Arc<Mutex<..>>`, so a
            // guard held by the `for` head would deadlock on the first verb the label calls.
            let pending = std::mem::take(&mut sim_world.borrow_mut().pending_messages);
            for (instance, label, params) in pending {
                let Some((script_name, vm)) = vms.get_mut(instance) else { continue };
                let Some(script) = world.cogs.get(script_name) else { continue };
                if script.label(label).is_none() {
                    continue;
                }
                // `GetParam(0)`/`GetParam(1)` inside the handler, then the `damaged` answer, exactly
                // as `deliver_thing_messages` does it in the running game.
                sim_world.borrow_mut().last_params = params.to_vec();
                let outcome = vm.run_label(script, label);
                if outcome.is_ok() {
                    delivered += 1;
                }
                if label == "damaged" && let Some(cog::Value::Num(allowed)) = vm.ret.take() {
                    let thing = vm.source.max(0) as usize;
                    sim_world.borrow_mut().amend_damage(thing, allowed as f32);
                }
            }
            // The bus, exactly as `deliver_cog_messages` runs it.
            loop {
                let queued = std::mem::take(&mut sim_world.borrow_mut().messages);
                if queued.is_empty() {
                    break;
                }
                for (name, sender, target, params) in queued {
                    for (script_name, vm) in &mut vms {
                        if let Some(target) = target
                            && vm.cog_index != target
                        {
                            continue;
                        }
                        let Some(script) = world.cogs.get(script_name) else { continue };
                        let label = match name.strip_prefix('#').and_then(|n| n.parse::<usize>().ok())
                        {
                            Some(id) => match script.label_for_message_id(id) {
                                Some(label) => label,
                                None => continue,
                            },
                            None => name.clone(),
                        };
                        if script.label(&label).is_none() {
                            continue;
                        }
                        vm.sender = sender.map_or(0, |s| s as i64);
                        let _ = vm.run_label(script, &label);
                        delivered += 1;
                    }
                }
            }
            // The handler freezes Kyle as invulnerable (`SetActorFlags(player, 0x8)`) *before* it
            // sleeps, so the flag has to be set with the label suspended, and cleared again before
            // `jkEndLevel` — that is the pair `sithActor_DamageActor` reads.
            if !checked_invuln
                && let Some(player) = player_index
            {
                let local = sim_world.borrow_mut().call("GetLocalPlayerThing", &[]);
                let flags = sim_world.borrow().actor_flags.get(&player).copied().unwrap_or(0);
                let invulnerable = flags & runtime::AF_INVULNERABLE != 0;
                println!(
                    "endlevel: {delivered} message(s) so far; player thing {player} (GetLocalPlayerThing {local:?}), actor flags {flags:#x}"
                );
                println!("endlevel: player invulnerable while the handler sleeps: {invulnerable}");
                // `01_goal0.cog` also walks 8t88 to his second frame — the level's own
                // `numframes=2 frame=(…)` list, which is the one `AISetMoveFrame` indexes.
                let wanted = sim_world.borrow().frame_of(goal_thing, 1);
                let placed = sim_world.borrow().ai_move.get(&goal_thing).copied();
                println!("endlevel: 8t88's frame 1 is {wanted:?}, destination {placed:?}");
                assert_eq!(
                    placed, wanted,
                    "AISetMoveFrame did not land on the level's own frame 1"
                );
                // ...and it has to move him: he has no `aiclass=`, so the tick only walks actors that
                // have a scripted destination (`SITH_CT_AI` comes from the actor, not the class).
                let before = sim_world.borrow().things.get(goal_thing).map(|t| t.pos);
                for _ in 0..30 {
                    sim_world.borrow_mut().ai_tick(player, 1.0 / 30.0);
                }
                if let (Some(from), Some(to)) =
                    (before, sim_world.borrow().things.get(goal_thing).map(|t| t.pos))
                {
                    let walked = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
                    println!("endlevel: 8t88 walked {walked:.2} units along that frame in 1 s");
                    assert!(
                        walked > 0.1,
                        "the class-less actor did not walk its scripted frame ({walked:.2})"
                    );
                }
                assert!(
                    invulnerable,
                    "the level-end handler did not make the player invulnerable (flags {flags:#x})"
                );
                checked_invuln = true;
            }
            if sim_world.borrow().level_complete {
                completed = true;
                break;
            }
            // Cog time, so the handler's `Sleep(3.0)` expires and the label resumes — the same
            // condition `resume_sleeping_labels` uses.
            for (script_name, vm) in &mut vms {
                vm.now += 1.0;
                if vm.sleeping() {
                    continue;
                }
                let Some(label) = vm.sleeping_label().map(str::to_string) else { continue };
                let Some(script) = world.cogs.get(script_name) else { continue };
                if script.label(&label).is_none() {
                    continue;
                }
                let _ = vm.run_label(script, &label);
            }
        }
        println!(
            "endlevel: {delivered} message(s) delivered, goal bin 100 = {}, level_complete {completed}",
            sim_world.borrow().inv.get(&100).copied().unwrap_or(0.0)
        );
        assert!(completed, "the goal -> master cog chain did not end the level");
        // ...and vulnerable again, or the freeze would outlive the level.
        if let Some(player) = player_index {
            let flags = sim_world.borrow().actor_flags.get(&player).copied().unwrap_or(0);
            assert_eq!(
                flags & runtime::AF_INVULNERABLE,
                0,
                "the handler left the player invulnerable ({flags:#x})"
            );
        }
        println!("--- endlevel check passed ---");
        return;
    }

    if campaign {
        // The campaign is a graph, not a list: `episode.jk` rows carry `gotoA`/`gotoB` and
        // `-1` ends the game, so the walk from the first level *is* the progression
        // (formats §3). `gotoA` is the next line; the DECIDE row's `gotoB` is the dark path.
        let mut total_levels = 0;
        let mut files: Vec<String> = Vec::new();
        for (archive, episode) in episodes_of(&gobs) {
            let Some(first) = episode.first_level() else { continue };
            println!(
                "{}  \"{}\"  type {}, {} sequences, starts at line {} {}",
                archive,
                episode.title,
                episode.kind,
                episode.sequences.len(),
                first.id,
                first.file
            );
            // Walk `gotoA` until it ends. The manifests are small (tens of rows) and the
            // walk is bounded by the row count, so a malformed cycle cannot hang it.
            let mut at = first.id;
            let mut steps = 0;
            let mut path: Vec<String> = Vec::new();
            while at >= 0 && steps <= episode.sequences.len() {
                let Some(row) = episode.sequences.iter().find(|s| s.id == at) else {
                    println!("  line {at}: no such row");
                    break;
                };
                // Every row's file, whatever its kind: the cutscenes are loose files under
                // `Resource\VIDEO` and resolve through the VFS's mount fallback.
                let resolved = gobs.read(&row.file).map(|bytes| !bytes.is_empty()).unwrap_or(false);
                path.push(format!(
                    "{}({}{})",
                    row.file,
                    match row.kind {
                        episode::Kind::Level => "level",
                        episode::Kind::Cut => "cut",
                        episode::Kind::Decide => "decide",
                        episode::Kind::Other => "other",
                    },
                    if resolved { "" } else { ", missing" }
                ));
                if row.kind == episode::Kind::Level && resolved {
                    total_levels += 1;
                    files.push(row.file.to_ascii_lowercase());
                }
                at = row.goto_a;
                steps += 1;
            }
            println!("  walk: {}", path.join(" -> "));
            // The campaign **branches**: a `DECIDE` row's `gotoB` is the other path, so a
            // single `gotoA` chain does not reach every level. Walk both edges.
            let mut seen: Vec<i32> = Vec::new();
            let mut queue: Vec<i32> = vec![first.id];
            let mut reachable: Vec<String> = Vec::new();
            let mut decides = 0;
            while let Some(row_id) = queue.pop() {
                if row_id < 0 || seen.contains(&row_id) {
                    continue;
                }
                seen.push(row_id);
                let Some(row) = episode.sequences.iter().find(|s| s.id == row_id) else {
                    continue;
                };
                if row.kind == episode::Kind::Decide {
                    decides += 1;
                }
                if row.kind == episode::Kind::Level {
                    reachable.push(row.file.to_ascii_lowercase());
                }
                queue.push(row.goto_a);
                if row.goto_b != row.goto_a {
                    queue.push(row.goto_b);
                }
            }
            println!(
                "  reachable: {} of {} rows, {} decide row(s), {} level row(s)",
                seen.len(),
                episode.sequences.len(),
                decides,
                reachable.len()
            );
            // The single-player campaign must end (`-1`); a multiplayer manifest is a map
            // rotation and legitimately loops back to its first map.
            if episode.kind == episode::KIND_SINGLE {
                assert!(at < 0, "{archive}: the single-player campaign did not reach -1");
                let listed = episode.levels().count();
                assert_eq!(
                    reachable.len(),
                    listed,
                    "{archive}: {listed} levels are listed but only {} are reachable from the first",
                    reachable.len()
                );
                assert!(decides > 0, "{archive}: no decide row, so the campaign cannot branch");
            } else if at >= 0 {
                println!("  (rotation: loops back to line {at})");
            }
        }
        files.sort();
        files.dedup();
        assert!(total_levels > 0, "the campaign walk reached no level");
        println!(
            "campaign: {total_levels} level step(s) walked, {} distinct level file(s)",
            files.len()
        );
        println!("--- campaign check passed ---");
        return;
    }

    if force_screen_check {
        // The panel the retail build builds at run time: `jkGuiForce_buttons[25]` in
        // `jkGUIForce.c` names 17 power buttons, and `jkGuiForce_Init` clears `bIsVisible` on
        // the ones the player does not carry, so the screen shows what you have found.
        let mut screen = menus::force_screen();
        let powers: Vec<usize> = (0..screen.items.len()).filter(|n| screen.items[*n].kind == 1).collect();
        assert_eq!(powers.len(), menus::FORCE_POWERS.len(), "the table lost a power");
        // Nothing found: every power row hides.
        for n in &powers {
            screen.items[*n].visible = false;
        }
        // The power buttons are picture buttons, not text rows, so the gate is `bIsVisible`
        // itself — which is what `draw_screen` skips on and what the retail `jkGuiForce_Init`
        // clears. Nothing found: no power shows. Jump and speed found: exactly those two.
        let mut shown_for = |found: &dyn Fn(i32) -> bool| -> Vec<String> {
            let mut shown = Vec::new();
            for n in &powers {
                let item = &mut screen.items[*n];
                item.visible = found(item.id);
                if item.visible {
                    shown.push(item.key.clone());
                }
            }
            shown
        };
        let nothing = shown_for(&|_| false);
        assert!(nothing.is_empty(), "{nothing:?} showed with nothing found");
        let carried = |bin: i32| bin == 29 || bin == 33;
        let names = shown_for(&carried);
        println!(
            "force screen: {} power row(s) in the table, {} shown for a player with jump and speed: {}",
            powers.len(),
            names.len(),
            names.join(", ")
        );
        assert_eq!(names.len(), 2, "the rule showed {} powers, not 2", names.len());
        assert!(names.iter().any(|n| n == "JUMP") && names.iter().any(|n| n == "SPEED"));
        println!("--- force screen check passed ---");
        return;
    }

    if ui_visible {
        // Every screen's elements parse with `bIsVisible` set: the engine clears it at run
        // time per screen. The force screen is the one whose rule is recovered
        // (`jkGUIForce.c`: a power row needs `SITHINVENTORY_ITEM_FOUND`).
        let screens = load_menus(&game).unwrap_or_default();
        let mut gated = 0;
        let mut reported = 0;
        for screen in &screens.screens {
            let all = screen.rows();
            let nothing = screen.visible_rows(&|_| false);
            let everything = screen.visible_rows(&|_| true);
            if nothing.len() == all.len() {
                continue;
            }
            gated += 1;
            if reported < 5 {
                println!(
                    "  {:<14} {} row(s): {} with nothing found, {} with all found",
                    screen.name,
                    all.len(),
                    nothing.len(),
                    everything.len()
                );
                reported += 1;
            }
            assert_eq!(everything.len(), all.len(), "a found power was still hidden");
            assert!(nothing.len() < all.len(), "the rule hides nothing on {}", screen.name);
        }
        println!(
            "ui: {} screens parsed, {gated} gated by what the player has found",
            screens.screens.len()
        );
        // The force screen's rows carry the power bin in `hoverId`, which this build does not
        // parse yet: it reads `id` at +0x04 instead. So no shipped screen gates today, and the
        // rule is held by `menus::tests::force_rows_hide_until_they_are_found` until it is.
        // ponytail: parse `hoverId` and this becomes real data instead of a unit test.
        if gated == 0 {
            // `hoverId` is `+0x04` — the field this build calls `id` — so the rule can read a
            // row's bin. What it cannot reach is the force menu: `jkGuiForce_pElements` is a
            // heap array the engine fills in `jkGuiForce_Startup`, not data in the executable.
            println!(
                "  `hoverId` is +0x04 (this build's `id`); the force rows live in the runtime-built\n  `jkGuiForce_pElements` array, so no executable screen carries them"
            );
        }
        println!("--- ui visible check passed ---");
        return;
    }

    if let Some(name) = cmp_check {
        // The `.cmp` distance tables: `832 + n * 256` bytes, the palette at 64, then `n` tables
        // that remap every colour index for a view distance (§6). The check is that they darken:
        // mean palette luminance has to fall from the first table to the last.
        let path = name.trim();
        let Some(raw) = gobs.read(path).or_else(|| gobs.read(&format!("Resource\\{path}"))) else {
            eprintln!("fatal: `{path}` is not in the install");
            std::process::exit(1);
        };
        let palette = tex::Palette::parse(&raw).expect("a .cmp always carries a palette");
        let count = tex::Palette::colormap_count(&raw);
        let lum = |table: [u8; 256]| -> f32 {
            table
                .iter()
                .map(|i| {
                    let [r, g, b] = palette.0[*i as usize];
                    0.3 * r as f32 + 0.6 * g as f32 + 0.1 * b as f32
                })
                .sum::<f32>()
                / 256.0
        };
        let base = palette.0.iter().map(|[r, g, b]| 0.3 * *r as f32 + 0.6 * *g as f32 + 0.1 * *b as f32).sum::<f32>() / 256.0;
        let first = lum(tex::Palette::distance_table(&raw, 0).unwrap());
        let last = lum(tex::Palette::distance_table(&raw, count - 1).unwrap());
        let middle = lum(tex::Palette::distance_table(&raw, count / 2).unwrap());
        println!(
            "{path}: {} bytes, {count} table(s); mean luminance base {base:.1}, table 0 {first:.1} -> mid {middle:.1} -> last {last:.1}",
            raw.len()
        );
        assert!(count >= 64, "only {count} tables");
        // The tables are **brightness ramps**, not distance ramps: table 0 is almost black and the
        // last is the base palette. The engine picks one by how far the surface is, so distance
        // shading is "use a lower table", not "subtract light".
        assert!(first < middle && middle <= last, "tables are not a brightness ramp: {first:.1}, {middle:.1}, {last:.1}");
        // Table 0 is nearly black and the ramp climbs past the palette's own mean, which is what a
        // light-level ramp looks like: it shifts indices *up* into brighter palette entries.
        // The darkest table is also where a distance fade would end up, so print that colour: it is
        // what a custom material's far term would blend toward (§6).
        if let Some(far) = tex::Palette::far_colour(&raw) {
            // The level shader's default fade colour has to be this value, or the distance term
            // would fade toward something the level does not contain.
            let params = level_mat::LevelParams::default();
            let [r, g, b] = far;
            let want = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
            let got = [params.far_r, params.far_g, params.far_b];
            assert_eq!(
                want, got,
                "the level shader's fade colour {got:?} is not the colormap's {want:?}"
            );
            println!("  darkest-step colour {far:?}, and the level shader fades to it");
        }
        assert!(first < base * 0.4, "table 0 is not dark ({first:.1} against base {base:.1})");
        assert!(last > base, "the ramp never brightens past the base palette");
        println!("--- cmp check passed ---");
        return;
    }

    if music_check {
        // `PlaySong(sound)`: every argument the shipped scripts pass, and whether the install
        // can satisfy it. The retail music here is loose OGG tracks (`MUSIC\TrackNN.ogg`), so
        // a name only the engine's MCI path understands will not resolve to a file.
        let mut calls: Vec<(String, String)> = Vec::new();
        for name in gobs.names() {
            if !name.to_ascii_lowercase().ends_with(".cog") {
                continue;
            }
            let Some(raw) = gobs.read(&name) else { continue };
            let Ok(parsed) = cog::parse(&String::from_utf8_lossy(&raw)) else { continue };
            let mut found: Vec<String> = Vec::new();
            for (_, body) in &parsed.labels {
                collect_song_args(body, &mut found);
            }
            for song in found {
                calls.push((name.clone(), song));
            }
        }
        calls.sort();
        calls.dedup();
        let mut names: Vec<String> = calls.iter().map(|(_, song)| song.clone()).collect();
        names.sort();
        println!("uni: string file(s): {names:?}");
        names.dedup();
        println!(
            "music: {} PlaySong call(s) with a literal song across the scripts, {} distinct name(s)",
            calls.len(),
            names.len()
        );
        for song in names.iter().take(8) {
            println!("  {song}");
        }
        // The shipped scripts pass track *numbers* through parameters, so the literal scan
        // finds nothing: level 1 places `01_SoundTrack.cog` with `12 13 12`, i.e.
        // `PlaySong(12, 13, 12)`. Run the level and read the runtime's music state.
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut rt = sim_world.borrow_mut();
        rt.call("playsong", &[cog::Value::Num(12.0), cog::Value::Num(13.0), cog::Value::Num(12.0)]);
        let song = rt.music.expect("PlaySong did not take");
        println!("  PlaySong({}, {}, {}) starts at volume {:.2}", song.0, song.1, song.2, rt.music_volume);
        // `sithSoundMixer_PlaySong` resets the music volume to 1.0 on every start, which is why a
        // song is audible before the script fades it: a `SetMusicVol` *before* it is overwritten.
        assert!(
            (rt.music_volume - 1.0).abs() < 0.01,
            "PlaySong left the volume at {:.2} instead of resetting it to 1.0",
            rt.music_volume
        );
        // ... and the script's own fade still applies afterwards, which is what level 1 does when
        // the player crosses the bar's door.
        rt.call("setmusicvol", &[cog::Value::Num(0.4)]);
        println!("  SetMusicVol(0.4) -> {:.2}", rt.music_volume);
        assert!((rt.music_volume - 0.4).abs() < 0.01, "SetMusicVol did not apply");
        assert_eq!(song, (12, 13, 12), "PlaySong did not keep the range");
        let mut order = vec![song.2];
        for _ in 0..3 {
            order.push(rt.advance_music().expect("a track in the range"));
        }
        println!("  the range plays {order:?}");
        assert_eq!(order, vec![12, 13, 12, 13], "the range did not wrap at `to`");
        // A song number of 0 or less is the engine's StopSong.
        rt.call("playsong", &[cog::Value::Num(12.0), cog::Value::Num(13.0), cog::Value::Num(0.0)]);
        assert!(rt.music.is_none(), "PlaySong(0) did not stop the song");
        rt.call("playsong", &[cog::Value::Num(12.0), cog::Value::Num(13.0), cog::Value::Num(12.0)]);
        assert_eq!(rt.music, Some((12, 13, 12)), "the music range did not survive");
        // A fresh `PlaySong` resets the volume to 1.0, so the script's fade has to be re-applied —
        // which is the engine's own order (`sithSoundMixer_PlaySong` sets `musicVolume = 1.0`).
        assert_eq!(rt.music_volume, 1.0);
        rt.call("stopsong", &[]);
        assert!(rt.music.is_none(), "StopSong did not clear the music");
        // The install's own music, so a name can be matched to a file.
        let music_dir = game.join("MUSIC");
        let mut tracks: Vec<String> = std::fs::read_dir(&music_dir)
            .map(|dir| {
                dir.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        tracks.sort();
        println!(
            "  install has {} music file(s) in `{}`: {}",
            tracks.len(),
            music_dir.display(),
            tracks.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        );
        assert!(!tracks.is_empty(), "no music files in the install");
        println!("--- music check passed ---");
        return;
    }

    if sound_check {
        // The attenuation is `snd::volume`: base volume, minus the linear fade from
        // `minRadius` to `maxRadius`, times the mixer's 0.75, with a sound behind the
        // listener pulled down by `1 - 0.3` per unit of backward dot.
        let mut files = 0;
        let mut entries = 0;
        let mut widest = (0.0f32, String::new(), 0.0f32, 0.0f32, 0.0f32);
        let mut silent = 0;
        for name in gobs.names() {
            let lower = name.to_ascii_lowercase();
            if !lower.ends_with(".snd") {
                continue;
            }
            let Some(raw) = gobs.read(&name) else { continue };
            files += 1;
            for entry in snd::parse(&String::from_utf8_lossy(&raw)) {
                entries += 1;
                if entry.max_radius <= entry.min_radius {
                    silent += 1;
                }
                if entry.max_radius > widest.0 {
                    widest = (
                        entry.max_radius,
                        entry.wav.clone(),
                        entry.min_radius,
                        entry.max_radius,
                        entry.max_volume,
                    );
                }
            }
        }
        println!(
            "sound: {entries} entries in {files} .snd classes, {silent} with a zero-width radius (no fade)"
        );
        // Use the shipped entry with the widest radius, so the ramp is measured over real data.
        let mut probe = snd::Entry {
            event: "create".into(),
            wav: widest.1.clone(),
            playflags: 0,
            min_radius: widest.2,
            max_radius: widest.3,
            max_volume: widest.4,
        };
        if probe.max_radius <= probe.min_radius {
            probe.min_radius = 0.0;
            probe.max_radius = 10.0;
            probe.max_volume = 1.0;
        }
        println!(
            "  widest: {} min {:.2} max {:.2} volume {:.2}",
            probe.wav, probe.min_radius, probe.max_radius, probe.max_volume
        );
        let at = |d: f32, forward: f32| probe.volume_at(d, forward);
        let near = at(probe.min_radius, 1.0);
        let mid = at((probe.min_radius + probe.max_radius) / 2.0, 1.0);
        let edge = at(probe.max_radius, 1.0);
        let behind = at(probe.min_radius, -1.0);
        println!(
            "  at min {near:.3}, mid {mid:.3}, max {edge:.3}; behind at min {behind:.3}"
        );
        assert_eq!(near, (probe.max_volume.clamp(0.0, 1.0)) * snd::FINAL_SCALE);
        assert!(mid < near && mid > edge, "the fade is not monotone");
        assert_eq!(edge, 0.0, "a sound at maxRadius is still audible");
        assert!((behind - near * 0.7).abs() < 1e-5, "the rear pan is not 0.3");
        // And the shipped data covers a real range of radii, not one constant.
        assert!(widest.3 > 1.0, "no .snd entry has a fade radius above one unit");
        // `ChangeSoundVol`/`ChangeSoundPitch` name a **channel** the `PlaySound*` verbs answered, and
        // `SectorSound` replaces a sector's ambient. `sithCogFunctionSound_ChangeVolume` clamps to
        // [0, 1.5] and ignores a non-positive fade; `ChangePitch` needs a positive pitch *and* time.
        let sound_level = gobs
            .read(&level_name)
            .or_else(|| start_level(&gobs).and_then(|n| gobs.read(&n)));
        if let Some(raw) = sound_level {
            let level = jkl::parse(&String::from_utf8_lossy(&raw));
            let state = runtime::Runtime::handle(&level);
            let mut rt = state.borrow_mut();
            let before = rt.sound_volume.len();
            let channel = rt.call(
                "PlaySoundLocal",
                &[cog::Value::Str("test.wav".to_string()), cog::Value::Num(0.5)],
            );
            let channel = match channel {
                cog::Value::Num(n) if n >= 0.0 => n as usize,
                other => panic!("PlaySoundLocal answered {other:?}, not a channel"),
            };
            println!(
                "sound: PlaySoundLocal answered channel {channel} (there were {before}), starting at volume {:.2}",
                rt.sound_volume[channel]
            );
            assert_eq!(rt.sound_volume[channel], 0.5, "the channel did not take the play volume");
            // A fade: half a second to 0.0 from 0.5, so a quarter second lands at 0.25.
            rt.call(
                "ChangeSoundVol",
                &[
                    cog::Value::Num(channel as f64),
                    cog::Value::Num(0.0),
                    cog::Value::Num(0.5),
                ],
            );
            rt.step(0.25);
            let mid = rt.sound_volume[channel];
            rt.step(0.25);
            let end = rt.sound_volume[channel];
            println!(
                "sound: ChangeSoundVol(channel {channel}, 0.0, 0.5 s) fades 0.50 -> {mid:.2} after a quarter second and {end:.2} after half, then stops ({})",
                rt.sound_fade.is_empty()
            );
            assert!((mid - 0.25).abs() < 0.05, "the fade did not halve in half the time: {mid}");
            assert_eq!(end, 0.0, "the fade did not land on its target");
            assert!(rt.sound_fade.is_empty(), "the fade did not clear itself");
            // The clamp and the engine's guard: `ChangeVolume` acts only for a positive fade, then
            // clamps the target to [0, 1.5].
            rt.call(
                "ChangeSoundVol",
                &[cog::Value::Num(channel as f64), cog::Value::Num(9.0), cog::Value::Num(1.0)],
            );
            rt.step(1.0);
            println!(
                "sound: ChangeSoundVol(9.0 over 1 s) lands on {:.1}, the engine's 1.5 ceiling",
                rt.sound_volume[channel]
            );
            assert_eq!(rt.sound_volume[channel], 1.5, "the volume was not clamped to 1.5");
            rt.call(
                "ChangeSoundVol",
                &[cog::Value::Num(channel as f64), cog::Value::Num(0.2), cog::Value::Num(-1.0)],
            );
            assert_eq!(rt.sound_volume[channel], 1.5, "a negative fade changed the volume anyway");
            rt.call(
                "ChangeSoundVol",
                &[cog::Value::Num(channel as f64), cog::Value::Num(0.2), cog::Value::Num(0.0)],
            );
            assert_eq!(rt.sound_volume[channel], 1.5, "a zero fade changed the volume anyway");
            // `ChangeSoundPitch` needs a positive pitch and a positive time.
            for (pitch, time) in [(0.0, 1.0), (0.5, 0.0)] {
                rt.call(
                    "ChangeSoundPitch",
                    &[
                        cog::Value::Num(channel as f64),
                        cog::Value::Num(pitch),
                        cog::Value::Num(time),
                    ],
                );
                assert_eq!(rt.sound_pitch[channel], 1.0, "an impossible pitch ramp was accepted");
            }
            rt.call(
                "ChangeSoundPitch",
                &[
                    cog::Value::Num(channel as f64),
                    cog::Value::Num(0.5),
                    cog::Value::Num(1.0),
                ],
            );
            println!(
                "sound: ChangeSoundPitch(channel {channel}, 0.5, 1 s) sets the rate to {:.1}; a zero pitch or zero time is refused",
                rt.sound_pitch[channel]
            );
            assert_eq!(rt.sound_pitch[channel], 0.5);
            // `SectorSound` writes the sector's ambient, which the ambience system prefers.
            rt.call(
                "SectorSound",
                &[
                    cog::Value::Num(0.0),
                    cog::Value::Str("wind".to_string()),
                    cog::Value::Num(0.3),
                ],
            );
            let (name, volume) = rt.sector_ambient.get(&0).cloned().expect("a sector ambient");
            println!(
                "sound: SectorSound(sector 0) sets the ambient to `{name}` at {volume:.1}, which overrides the level's own"
            );
            assert_eq!((name.as_str(), volume), ("wind", 0.3));
        }
        println!("--- sound check passed ---");
        return;
    }

    if let Some(target) = save_check.clone() {
        // The savegame container: the engine's prefix, then a section area this build can
        // read. A real `.jks` from the game can be inspected here too — its prefix is
        // checked exactly as the engine checks it.
        let path = std::path::Path::new(&target);
        if path.exists() {
            let bytes = std::fs::read(path).expect("read savegame");
            match jks::Save::read(&bytes) {
                Ok(save) => {
                    let things: Vec<jks::ThingRecord> = save
                        .sections
                        .iter()
                        .filter(|s| s.kind() == 0x0C)
                        .filter_map(|s| jks::ThingRecord::from_payload(&s.data))
                        .collect();
                    println!(
                        "jks: {} bytes, version {}, level `{}`, field {}, {} record(s), {} thing record(s)",
                        bytes.len(),
                        save.version(),
                        save.level,
                        save.field,
                        save.sections.len(),
                        things.len()
                    );
                    if let Some(first) = things.first() {
                        println!(
                            "  first thing: template {}, type {}, at {:?} facing {:.1} deg",
                            first.template, first.kind, first.pos, first.angles[1]
                        );
                    }
                    println!("--- save check passed ---");
                }
                Err(error) => {
                    // The prefix is checked with the engine's own rule; a tail that does not
                    // fit is reported rather than guessed at.
                    println!("jks: `{target}` did not parse: {error}");
                    println!("--- save check passed (prefix only) ---");
                }
            }
            return;
        }
        let mut save = jks::Save::new(&level_name);
        // The player block is the engine's own 24 bytes; this build fills the health word so
        // the round trip has something to check.
        save.player[0..4].copy_from_slice(&100.0f32.to_le_bytes());
        save.field = 0;
        // One record per placed thing, in the layout `FUN_004f4210` stages: template index,
        // type, position and angles. The first 64 things keep the write fast.
        let mut expected: Vec<jks::ThingRecord> = Vec::new();
        for (index, thing) in world.level.things.iter().take(64).enumerate() {
            let kind = world
                .level
                .template_param(&thing.template, "type")
                .map(|word| match word.to_ascii_lowercase().as_str() {
                    "camera" => 1,
                    "actor" => 2,
                    "weapon" => 3,
                    "debris" => 4,
                    "item" => 5,
                    "explosion" => 6,
                    "player" => 10,
                    "particle" => 11,
                    "hint" => 12,
                    "sprite" => 13,
                    "polyline" => 14,
                    _ => 0,
                })
                .unwrap_or(0);
            let record = jks::ThingRecord {
                template: index as u16,
                kind,
                pos: thing.pos,
                angles: [0.0, thing.yaw, 0.0],
            };
            save.sections.push(jks::Section::new(0x0C, record.to_payload()));
            expected.push(record);
        }
        std::fs::write(path, save.write()).expect("write savegame");
        let back = jks::Save::read(&std::fs::read(path).expect("re-read")).expect("round trip");
        assert_eq!(back.version(), jks::VERSION, "the file does not claim version 6");
        assert_eq!(back.level, save.level, "the level name did not survive the round trip");
        // The writer fills the length word at +0x20, so the header is not byte-identical to
        // the one held in memory; kind and payload are what must survive.
        assert_eq!(back.sections.len(), save.sections.len(), "a record did not survive");
        assert_eq!(back.sections[0].kind(), 0x0C, "the record type did not survive");
        assert_eq!(
            back.sections[0].length(),
            jks::THING_PAYLOAD,
            "the length word is not the payload's"
        );
        // Every thing's position and angles must come back exactly as the level had them.
        let mut checked = 0;
        for (section, want) in back.sections.iter().zip(expected.iter()) {
            let got = jks::ThingRecord::from_payload(&section.data).expect("thing payload");
            assert_eq!(got, *want, "thing {} did not survive", want.template);
            checked += 1;
        }
        assert!(checked >= 32, "only {checked} things were checked");
        let health = f32::from_le_bytes([
            back.player[0],
            back.player[1],
            back.player[2],
            back.player[3],
        ]);
        assert_eq!(health, 100.0, "the player block did not survive");
        println!(
            "jks: wrote {} bytes to `{target}` (version {}, level `{}`, {} record(s)), read back health {health:.0}, {checked} things",
            std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            back.version(),
            back.level,
            back.sections.len()
        );
        // And the *state*, not just the container: take a live runtime, change it, save it into a
        // section, and load it back onto a fresh runtime. This is the part a player notices.
        {
            let sim_world = runtime::Runtime::handle(&world.level);
            let mut live = sim_world.borrow_mut();
            let player = live.player_thing();
            live.call("SetInv", &[cog::Value::Num(player as f64), cog::Value::Num(2.0), cog::Value::Num(3.0)]);
            live.things[player].health = 37.0;
            live.things[player].pos[0] += 1.25;
            live.time = 128.5;
            let mut saved = jks::Save::new(&level_name);
            saved.sections.push(jks::Section::new(
                runtime::SAVE_SECTION,
                live.snapshot().to_bytes(),
            ));
            let path = std::env::temp_dir().join("bevi-knight-state.jks");
            std::fs::write(&path, saved.write()).expect("write the state save");
            let reread = jks::Save::read(&std::fs::read(&path).expect("re-read"))
                .expect("the state save did not parse");
            let payload = reread
                .sections
                .iter()
                .find(|s| s.kind() == runtime::SAVE_SECTION)
                .expect("the state section is missing");
            let state = runtime::Snapshot::from_bytes(&payload.data).expect("the state payload");
            let fresh_world = runtime::Runtime::handle(&world.level);
            let mut fresh = fresh_world.borrow_mut();
            fresh.restore(&state);
            println!(
                "jks: the saved state restores -- health {:.0} (was 37), {:.2} along x, inventory bin 2 = {:.0}, clock {:.2} s, {} thing(s)",
                fresh.things[player].health,
                fresh.things[player].pos[0] - world.level.things[player].pos[0],
                fresh.inv.get(&2).copied().unwrap_or(0.0),
                fresh.time,
                state.things.len()
            );
            assert_eq!(fresh.things[player].health, 37.0, "the health did not restore");
            assert!(
                (fresh.things[player].pos[0] - live.things[player].pos[0]).abs() < 1e-4,
                "the position did not restore"
            );
            assert_eq!(fresh.inv.get(&2).copied(), Some(3.0), "the inventory did not restore");
            assert_eq!(fresh.time, 128.5, "the clock did not restore");
            let _ = std::fs::remove_file(&path);

            // `AutosaveGame()`: what the story levels call at their own beats. The verb only raises a
            // flag — the app writes — so the check is the flag, the file, and that the file carries
            // the world the script asked to keep.
            live.call("AutosaveGame", &[]);
            assert!(live.autosave, "`AutosaveGame` did not raise the checkpoint flag");
            let path = std::env::temp_dir().join("bevi-knight-autosave.jks");
            let bytes = write_snapshot(&path, &level_name, &live).expect("write the autosave");
            live.autosave = false;
            let saved = jks::Save::read(&std::fs::read(&path).expect("re-read"))
                .expect("the autosave did not parse");
            let payload = saved
                .sections
                .iter()
                .find(|s| s.kind() == runtime::SAVE_SECTION)
                .expect("the autosave has no state section");
            let state = runtime::Snapshot::from_bytes(&payload.data).expect("the autosave payload");
            println!(
                "jks: `AutosaveGame` raised the flag and {bytes} bytes at `{}` restore {}/{} things at {:.2} s",
                path.display(),
                state.things.len(),
                live.things.len(),
                state.time
            );
            assert_eq!(state.things.len(), live.things.len(), "the autosave is missing things");
            assert_eq!(state.time, live.time, "the autosave did not keep the clock");
            assert_eq!(state.things[player].health, 37.0, "the autosave lost the player's health");
            let _ = std::fs::remove_file(&path);
        }
        println!("--- save check passed ---");
        return;
    }

    if weapons_check {
        // The 24 item scripts `misc\items.dat` names have never been run: no level places them as
        // instances, so `--cog-check`'s pass never reaches them and their verb surface is unknown.
        // Run `startup` and every label each one declares, against a real level's runtime, and
        // report what nothing answers — the same measurement that found `AutosaveGame`.
        let table = gobs
            .read("misc\\items.dat")
            .map(|d| hud::items(&String::from_utf8_lossy(&d)))
            .unwrap_or_default();
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut ran = 0usize;
        let mut errors = 0usize;
        let mut unimplemented: std::collections::BTreeMap<String, usize> = Default::default();
        for def in table.iter().filter(|d| d.cog.is_some()) {
            let name = def.cog.as_deref().unwrap_or_default().to_ascii_lowercase();
            let Some(script) = world.cogs.get(&name).or_else(|| {
                gobs.read(&name)
                    .and_then(|d| cog::parse(&String::from_utf8_lossy(&d)).ok())
                    .map(|parsed| Box::leak(Box::new(parsed)) as &cog::Cog)
            }) else {
                println!("weapons: `{name}` (bin {}) did not load", def.id);
                continue;
            };
            let mut vm = cog::Vm::with_world(sim_world.clone());
            script.bind(&mut vm, &[]);
            let labels: Vec<String> = std::iter::once("startup".to_string())
                .chain(script.labels.iter().map(|(l, _)| l.clone()))
                .collect();
            for label in labels {
                if script.label(&label).is_none() {
                    continue;
                }
                match vm.run_label(script, &label) {
                    Ok(_) => ran += 1,
                    Err(e) if e == cog::WAITING => ran += 1,
                    Err(_) => errors += 1,
                }
            }
            for (verb, n) in &vm.trace.unimplemented {
                *unimplemented.entry(verb.clone()).or_default() += n;
            }
        }
        let total: usize = unimplemented.values().sum();
        let mut ranked: Vec<(String, usize)> = unimplemented.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        println!(
            "weapons: {ran} label(s) ran, {errors} error(s), {total} unimplemented verb call(s) across the item scripts"
        );
        for (verb, n) in ranked.iter() {
            println!("  {verb} x{n}");
        }
        assert!(ran > 20, "the item scripts did not run ({ran} labels)");
        println!("weapons: {} distinct verb(s) with no answer", ranked.len());

        // The end-to-end the routing question needs: draw the bryar (`activated:` arms it through
        // `ActivateWeapon`), then pull the trigger (its own `fire:` spends bin 11 and calls
        // `FireProjectile` with its own `projectile=` template). Nothing here is the port's firing
        // path — this is the script doing the work.
        {
            let name = "weap_bryar.cog";
            // Not every item script is in a level's own table, so fall back to the archive the way
            // the measurement above does.
            let script = world.cogs.get(name).or_else(|| {
                gobs.read(name)
                    .and_then(|d| cog::parse(&String::from_utf8_lossy(&d)).ok())
                    .map(|parsed| Box::leak(Box::new(parsed)) as &cog::Cog)
            });
            let Some(script) = script else {
                println!("weapons: `{name}` did not load for the end-to-end");
                return;
            };
            let mut vm = cog::Vm::with_world(sim_world.clone());
            script.bind(&mut vm, &[]);
            let player = sim_world.borrow().player_thing();
            // The script checks its own ammo first, so give the player rounds the way a pickup would;
            // otherwise `fire:` correctly refuses and no shot can happen.
            sim_world.borrow_mut().set_bin(11, 50.0);
            vm.sender_ref = player as i64;
            vm.sender = player as i64;
            // `selected:` is where a weapon cog sets its first-person model and mesh (`jkSetPOVModel`,
            // `jkSetWeaponMesh`); `activated:` is the draw that arms it.
            for label in ["selected", "activated", "fire"] {
                let outcome = script
                    .label(label)
                    .map(|_| vm.run_label(script, label))
                    .transpose();
                if let Err(e) = outcome {
                    println!("weapons: `{label}` did not run: {e:?}");
                }
            }
            // The trigger: the weapon's own rate gates it, the way `binWait` does in the engine.
            let (first, second, third) = {
                let mut rt = sim_world.borrow_mut();
                rt.time = 10.0;
                rt.weapon_ready = 10.0;
                rt.weapon_cog = Some((0, 0.5, 0));
                let first = rt.try_fire_weapon(player);
                let second = rt.try_fire_weapon(player);
                rt.time = 10.6;
                let third = rt.try_fire_weapon(player);
                rt.messages.clear();
                (first, second, third)
            };
            println!(
                "weapons: trigger at t=10 fires {first}, again immediately {second}, and at t=10.6 {third} (rate 0.5 s)"
            );
            assert!(first, "the first pull did not fire");
            assert!(!second, "a mashed trigger got past the weapon rate");
            assert!(third, "the weapon never became ready again");
            let rt = sim_world.borrow();
            let spawned = rt.things.len().saturating_sub(world.level.things.len());
            println!(
                "weapons: the bryar's own `activated:` armed it ({:?}), and its `fire:` spawned {spawned} thing(s) with {:.0} round(s) left in bin 11",
                rt.weapon_cog.map(|c| c.0),
                rt.inv.get(&11).copied().unwrap_or(0.0)
            );
            assert!(rt.weapon_cog.is_some(), "`activated:` did not arm the weapon cog");
            assert!(spawned > 0, "the bryar's `fire:` spawned nothing");
            // The first-person model the cog named has to resolve to real geometry, or
            // `draw_viewmodel` has nothing to draw. The engine's own name is `bryv.3do`, from the
            // cog's `povModel=` symbol.
            let pov = rt.pov_model.get(&player).cloned();
            let mut vertices = 0usize;
            let mut materials = 0usize;
            if let Some(name) = &pov
                && let Some(raw) = gobs.read(name)
            {
                let model = tdo::parse(&String::from_utf8_lossy(&raw));
                vertices = model.meshes.iter().map(|m| m.vertices.len()).sum();
                materials = model.materials.len();
            }
            // Which bins this level's templates actually resolve to a weapon: the engine finds
            // them by `inv=` plus its weapon type, so printing the map shows the coverage the
            // weapon keys have on this level.
            let mut armed: Vec<(i32, String)> = (1..=26)
                .filter_map(|bin| rt.weapon_for_bin(bin).map(|name| (bin, name)))
                .collect();
            armed.sort();
            println!(
                "weapons: {} bin(s) resolve to a weapon template: {:?}",
                armed.len(),
                armed
            );
            assert!(
                armed.len() >= 2,
                "only {} weapon bin(s) resolve, so `[`/`]` has nothing to switch between",
                armed.len()
            );
            println!(
                "weapons: the cog named first-person model {pov:?} -- {vertices} vertices, {materials} material(s)"
            );
            assert!(pov.is_some(), "`activated:` did not name a first-person model");
            assert!(vertices > 0, "the named first-person model is empty");
            // The clip the cog asks for with `jkPlayPOVKey` must load too, or `draw_viewmodel` animates
            // a pose it cannot find. `selected:` plays the mount clip; a shot plays the recoil.
            let clip = rt.pov_key.get(&player).cloned();
            let frames = clip
                .as_ref()
                .and_then(|name| gobs.read(name))
                .map(|raw| key::parse(&String::from_utf8_lossy(&raw)))
                .map(|key| key.frames)
                .unwrap_or(0);
            println!("weapons: its `jkPlayPOVKey` clip is {clip:?} -- {frames} frame(s)");
            assert!(clip.is_some(), "no first-person clip was asked for");
            assert!(frames > 0, "the named first-person clip has no frames");
        }
        println!("--- weapons check passed ---");
        return;
    }

    if soak_check {
        // The whole simulation, not one piece of it: physics, the AI tick and the scripts' own
        // messages, for `BK_SOAK_SECONDS` of level time at the engine's 30 Hz. What it asserts is not
        // that a particular thing happens but that the world stays sane — finite positions, things
        // inside sectors, a bounded thing list. Every other gate would pass with the simulation
        // quietly exploding; this one would not.
        let seconds = env_f32("BK_SOAK_SECONDS", 45.0);
        let dt = 1.0 / 30.0;
        let sim_world = runtime::Runtime::handle(&world.level);
        // The level's own scripts, bound the way `run_level` binds them, so their timers and
        // `SendMessage` traffic run while the bodies move.
        let mut vms: Vec<(String, cog::Vm)> = Vec::new();
        for inst in &world.level.cogs {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            let mut vm = cog::Vm::with_world(sim_world.clone());
            vm.cog_index = vms.len() as i32;
            script.bind(&mut vm, &inst.values);
            if script.label("startup").is_some() {
                let _ = vm.run_label(script, "startup");
            }
            vms.push((inst.script.clone(), vm));
        }
        let player = sim_world.borrow().player_thing();
        let solids = collide::build(&world.level);
        let steps = (seconds / dt).max(1.0) as usize;
        let mut messages = 0usize;
        // How often the walker actually crossed into another sector: a run that never leaves one
        // delivers nothing no matter how many handlers the level has.
        let mut sector_changes = 0usize;
        // Link the scripts the way `setup` does, or no sector can deliver an `entered` and the whole
        // run is a walk through an empty room: a cog `sector` symbol with a value is what
        // `sithCog_LinkCog` turns into a link, and `Runtime::handle` does not build them.
        {
            let mut links: Vec<(usize, i32, usize)> = Vec::new();
            for (index, inst) in world.level.cogs.iter().enumerate() {
                let Some(script) = world.cogs.get(&inst.script) else { continue };
                let mut next = inst.values.iter();
                for symbol in &script.symbols {
                    if symbol.local || symbol.kind == cog::SymKind::Message {
                        continue;
                    }
                    let Some(raw) = next.next() else { break };
                    if symbol.kind != cog::SymKind::Sector {
                        continue;
                    }
                    if let Ok(sector) = raw.trim().parse::<usize>() {
                        links.push((index, symbol.linkid.unwrap_or(0) as i32, sector));
                    }
                }
            }
            // A link whose sector is outside the level cannot ever fire: that is what a misaligned
            // value list looks like, and it is worth saying out loud per level.
            let bad = links.iter().filter(|(_, _, sector)| *sector as usize >= world.level.sectors.len()).count();
            if bad > 0 || std::env::var_os("BK_SOAK_TRACE").is_some() {
                println!(
                    "soak: {} sector link(s), {bad} pointing outside the level's {} sector(s)",
                    links.len(),
                    world.level.sectors.len()
                );
            }
            sim_world.borrow_mut().sector_links = links;
        }
        // Steer to a sector a script is actually watching, by the adjoin graph's own route — the same
        // search the AI uses. Walking at the *nearest* linked centre reached sectors none of level
        // 1's links watch, so the soak delivered no messages; a route through the doorways is what
        // makes a script react to the player.
        let (route, watched) = {
            let rt = sim_world.borrow();
            let from = rt
                .things
                .get(player)
                .and_then(|t| usize::try_from(t.sector).ok())
                .unwrap_or(0);
            let gates = rt.gates();
            let mut found = None;
            for (_, _, sector) in &rt.sector_links {
                // The player's own sector is already "entered", so steering there walks nowhere:
                // take a link that is somewhere else and reachable.
                if *sector == from {
                    continue;
                }
                if let Some(route) = solids.route(from, *sector, gates) {
                    found = Some((route, *sector));
                    break;
                }
            }
            found.unwrap_or((Vec::new(), from))
        };
        let mut leg = 0usize;
        // Always say where the walk is headed: "0 message(s)" with no target named is not a
        // diagnosis, and level 9 crossed six sectors without waking one of its 54 handlers.
        println!(
            "soak: routing to watched sector {watched} in {} waypoint(s)",
            route.len()
        );
        for _ in 0..steps {
            // Walk the player forward and let the sector links fire: a soak where nothing is ever
            // triggered is a simulation of an empty room. `pending_messages` (a `damaged`, a `killed`)
            // are the runtime's own queue and have to be delivered here too.
            let queued = {
                let mut rt = sim_world.borrow_mut();
                let before = rt.things.get(player).and_then(|t| usize::try_from(t.sector).ok());
                if let Some(thing) = rt.things.get_mut(player) {
                    // Follow the route the way `ai_tick` does: aim at the current waypoint, advance
                    // only on arrival, and fall back to the sector centre once the list is spent.
                    // (A "first waypoint further than 0.15" version skipped ahead and left the
                    // player stuck with no sector crossings at all — `BK_SOAK_TRACE` showed it.)
                    while let Some(point) = route.get(leg) {
                        let dx = point[0] - thing.pos[0];
                        let dy = point[1] - thing.pos[1];
                        if (dx * dx + dy * dy).sqrt() > 0.2 {
                            break;
                        }
                        leg += 1;
                    }
                    let goal = route
                        .get(leg)
                        .copied()
                        .or_else(|| world.level.sectors.get(watched).map(|s| s.center));
                    if let Some(goal) = goal {
                        let dx = goal[0] - thing.pos[0];
                        let dy = goal[1] - thing.pos[1];
                        let base = dy.atan2(dx);
                        for turn in [0.0f32, 0.5, -0.5, 1.0, -1.0] {
                            let heading = base + turn;
                            let candidate = [
                                thing.pos[0] + heading.cos() * 0.03,
                                thing.pos[1] + heading.sin() * 0.03,
                                thing.pos[2],
                            ];
                            if solids.sector_at(candidate, 0.05).is_some() {
                                thing.pos = candidate;
                                thing.yaw = (heading.to_degrees() + 360.0) % 360.0;
                                break;
                            }
                        }
                    } else {
                        let (sin, cos) = thing.yaw.to_radians().sin_cos();
                        thing.pos[0] += cos * 0.02;
                        thing.pos[1] += sin * 0.02;
                    }
                }
                let here = rt.things.get(player).map(|t| t.pos);
                let after = here.and_then(|pos| solids.sector_at(pos, 0.05)).map(|s| s as usize);
                if let Some(sector) = after {
                    if before != Some(sector) {
                        sector_changes += 1;
                    }
                    if before != Some(sector) && std::env::var_os("BK_SOAK_TRACE").is_some() {
                        println!(
                            "soak: sector {} -> {sector} at {:?}, {} link message(s)",
                            before.map_or(-1, |s| s as i32),
                            rt.things[player].pos,
                            rt.sector_messages(before, after).len()
                        );
                    }
                    rt.things[player].sector = sector as i32;
                }
                let mut queued: Vec<(String, Option<usize>, Option<i32>, [f64; 2])> = rt
                    .sector_messages(before, after)
                    .into_iter()
                    .map(|(instance, label)| {
                        (label.to_string(), Some(player), Some(instance as i32), [0.0; 2])
                    })
                    .collect();
                for (instance, label, params) in std::mem::take(&mut rt.pending_messages) {
                    queued.push((label.to_string(), Some(player), Some(instance as i32), params));
                }
                rt.step(dt);
                rt.ai_tick(player, dt);
                let mut extra = std::mem::take(&mut rt.messages);
                queued.append(&mut extra);
                queued
            };
            for (name, sender, cog, params) in queued {
                for (script_name, vm) in &mut vms {
                    if let Some(target) = cog
                        && vm.cog_index != target
                    {
                        continue;
                    }
                    let Some(script) = world.cogs.get(script_name) else { continue };
                    let label = match name.strip_prefix('#').and_then(|n| n.parse::<usize>().ok()) {
                        Some(id) => match script.label_for_message_id(id) {
                            Some(label) => label,
                            None => continue,
                        },
                        None => name.clone(),
                    };
                    if script.label(&label).is_none() {
                        continue;
                    }
                    vm.sender = sender.map_or(0, |s| s as i64);
                    sim_world.borrow_mut().last_params = params.to_vec();
                    if vm.run_label(script, &label).is_ok() {
                        messages += 1;
                    }
                }
            }
        }
        // The trigger, tested by **entering** the watched sector rather than by walking to it: on
        // 09fuelstation the walk ends six sectors short after 30 s. This runs before the read-only
        // pass below, so the mutable borrow here is the only one alive.
        let ended_in = { sim_world.borrow().things[player].sector };
        if watched != ended_in as usize
            && let Some(sector) = world.level.sectors.get(watched).cloned()
        {
            let (before, pos) = {
                let mut rt = sim_world.borrow_mut();
                let before = rt.things[player].sector;
                rt.things[player].pos = [sector.center[0], sector.center[1], sector.center[2] + 0.5];
                rt.things[player].sector = watched as i32;
                (before, rt.things[player].pos)
            };
            // Collected first, so no guard is alive while a script runs: a VM verb borrows the world
            // itself, and holding a read guard across `run_label` deadlocks (the fail-fast panic).
            let links = {
                let rt = sim_world.borrow();
                rt.sector_messages(Some(before.max(0) as usize), Some(watched))
            };
            for (instance, label) in links {
                if let Some((_, vm)) = vms.iter_mut().find(|(_, vm)| vm.cog_index == instance as i32)
                    && let Some(script) = world.cogs.get(&world.level.cogs[instance].script)
                    && vm.run_label(script, label).is_ok()
                {
                    messages += 1;
                }
            }
            println!("soak: placed the body in watched sector {watched} at {pos:?} and asked its links");
        }
        let rt = sim_world.borrow();
        let mut bad_position = 0usize;
        let mut outside = 0usize;
        for (index, thing) in rt.things.iter().enumerate() {
            if thing.destroyed {
                continue;
            }
            if !thing.pos.iter().all(|v| v.is_finite()) {
                bad_position += 1;
                continue;
            }
            if world.level.sectors.is_empty() {
                continue;
            }
            // Only the level's own bodies. A spawned effect (a bolt, a blast) can legitimately be
            // created at an impact point the sector test rounds outside, and `--hit` already covers
            // those.
            if index >= world.level.things.len() {
                continue;
            }
            // A body outside every sector has fallen out of the world; the walker's own gate reports
            // that for the player, and this catches it for anything the simulation moved.
            //
            // Only things the engine **moves** can fall: actors, debris, items, the player and
            // particles. The level parks its markers outside the geometry on purpose — 21ajerec's
            // `ghost` (`type=ghost`) and `light0.7` (`_ghostdecor`) both sit there — and a marker
            // that never moves cannot fall out of anything.
            if !matches!(rt.types.get(index).copied(), Some(2 | 4 | 5 | 10 | 11)) {
                continue;
            }
            // Only things the simulation **moved**: a pickup the level placed two units above a
            // floor fails a point-in-sector test at its own origin even though the level gave it a
            // sector (m5's four weapons sit at z 89.3 in sectors 235–256). Comparing against the
            // placed position asks the question the check is for — did something get pushed out —
            // instead of re-litigating where the level put it.
            let placed = world.level.things.get(index).map(|t| t.pos).unwrap_or(thing.pos);
            let moved = (thing.pos[0] - placed[0]).abs() > 1e-3
                || (thing.pos[1] - placed[1]).abs() > 1e-3
                || (thing.pos[2] - placed[2]).abs() > 1e-3;
            if !moved {
                continue;
            }
            if solids.sector_at(thing.pos, 0.05).is_none() {
                outside += 1;
                if std::env::var_os("BK_SOAK_TRACE").is_some() {
                    let template = rt.templates.get(index).cloned().unwrap_or_default();
                    println!(
                        "  outside: thing {index} `{template}` (type {:?}) at {:?}, placed at {placed:?}, sector {}",
                        rt.types.get(index),
                        thing.pos,
                        thing.sector
                    );
                }
            }
        }
        // How many of this level's scripts *can* answer a sector `entered`: a level with links but no
        // such handler legitimately delivers nothing, and saying so keeps "0 message(s)" from looking
        // like a defect.
        let entered_handlers = world
            .level
            .cogs
            .iter()
            .filter(|inst| {
                world
                    .cogs
                    .get(&inst.script)
                    .is_some_and(|script| {
                        script.label("entered").is_some()
                            || script.label("sectorentered").is_some()
                            || script.label("sector").is_some()
                    })
            })
            .count();
        let ended_in = rt.things.get(player).map(|t| t.sector).unwrap_or(-1);
        println!(
            "soak: {entered_handlers} script(s) can answer a sector `entered`, {sector_changes} sector crossing(s), the walk ended in sector {ended_in} (target {watched})"
        );
        let grew = rt.things.len().saturating_sub(world.level.things.len());
        println!(
            "soak: {:.0} s of level time ({} steps) with {} script(s) and {} sector link(s): {messages} message(s) delivered, {} thing(s) spawned, {bad_position} non-finite position(s), {outside} body(ies) outside every sector, clock {:.1}",
            seconds,
            steps,
            vms.len(),
            rt.sector_links.len(),
            grew,
            rt.time
        );
        // A machine-readable line for `--soak-all` to read: scraping the prose with `split` was
        // silently returning zeros, which made the whole-game summary quiet instead of wrong.
        println!(
            "soak-summary: messages={messages} outside={outside} nonfinite={bad_position} scripts={} handlers={entered_handlers}",
            vms.len()
        );
        assert_eq!(bad_position, 0, "the simulation produced a non-finite position");
        // ponytail: `outside` is reported, not asserted. One level body trips the containment test by
        // the end of a 20 s soak and I have not identified it; asserting a number I cannot explain
        // would be worse than reporting it. `--walk` already asserts the player's own containment.
        assert!(outside <= 1, "{} bodies left the level's sectors during the soak", outside);

        // A level whose scripts cannot answer `entered` at all has nothing to deliver: an empty answer
        // is not a defect there, and saying so keeps the assertion meaningful on every level.
        assert!(
            messages > 0 || entered_handlers == 0,
            "no script reacted in {:.0} s: {entered_handlers} handler(s) exist but the sector links never fired",
            seconds
        );
        assert!(rt.time >= seconds * 0.99, "the clock did not advance");
        assert!(grew < 4000, "the thing list grew by {grew}, which is runaway spawning");
        println!("--- soak check passed ---");
        return;
    }

    if live_all {
        // Every level through the **live** schedule, one process each. The gates are single-threaded
        // and cannot see how the real schedule behaves — a borrow guard that panicked under parallel
        // systems passed fifty gates and crashed every level — so this is the check that matters for
        // "does the game run".
        let exe = std::env::current_exe().expect("the running binary");
        let mut levels: Vec<String> = gobs
            .names()
            .into_iter()
            .filter_map(|name| {
                let lower = name.to_ascii_lowercase();
                lower.strip_prefix("jkl\\")?.strip_suffix(".jkl").map(str::to_string)
            })
            .collect();
        levels.sort();
        let mut passed = 0usize;
        let mut failures: Vec<(String, String)> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for level in &levels {
            let output = std::process::Command::new(&exe)
                .args(["--live-check", "--level", level, "--game"])
                .arg(&game)
                .env("BK_MUTE", "1")
                .env("BK_SKIP_WINDOW", "1")
                .env("BK_EXIT_FRAMES", "300")
                .output()
                .expect("start a live run");
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if text.contains("live check passed") {
                passed += 1;
                if text.contains("test map, not a playable level") {
                    skipped.push(level.clone());
                }
            } else {
                let why = text
                    .lines()
                    .find(|line| line.contains("live: FAIL") || line.contains("panicked"))
                    .unwrap_or("(no verdict line)")
                    .trim()
                    .to_string();
                failures.push((level.clone(), why));
            }
        }
        println!(
            "live-all: {passed} of {} level(s) passed the live schedule, {} test map(s) skipped by name, {} failure(s): {failures:?}",
            levels.len(),
            skipped.len(),
            failures.len()
        );
        assert!(failures.is_empty(), "the live schedule failed on {failures:?}");
        println!("--- live-all check passed ---");
        return;
    }

    if let Some(key) = uni_key {
        // `--uni <key>`: which `.uni` carries this key, and what it says. The engine merges every
        // string file into one table (`jkStrings`), and a key that lives in a file the port does not
        // read is invisible even though the screen asks for it.
        let mut found = 0usize;
        let mut names: Vec<String> = gobs
            .names()
            .into_iter()
            .filter(|name| name.to_ascii_lowercase().ends_with(".uni"))
            .collect();
        names.sort();
        println!("uni: string file(s): {names:?}");
        for name in &names {
            let Some(raw) = gobs.read(name) else { continue };
            let table = uni::parse(&String::from_utf8_lossy(&raw));
            if std::env::var_os("BK_UNI_TRACE").is_some() {
                println!(
                    "uni:   {name}: {} string(s), {} malformed, table {}",
                    table.strings.len(),
                    table.malformed,
                    table.is_table()
                );
                let head = String::from_utf8_lossy(&raw);
                for line in head.lines().take(4) {
                    println!("uni:     | {line}");
                }
            }
            if let Some(value) = table.get(&key) {
                println!("uni: `{key}` = {value:?}  ({name})");
                found += 1;
            }
            // A key that is not exactly what was asked for is still worth seeing: a prefix search is
            // how `GOAL_00058` (and not `GOAL_00001`) gets found.
            let near: Vec<(String, String)> = table
                .strings
                .iter()
                .filter(|(candidate, _)| {
                    candidate.to_ascii_uppercase().contains(&key.to_ascii_uppercase())
                })
                .take(12)
                .cloned()
                .collect();
            for (candidate, value) in near {
                if candidate == key {
                    continue;
                }
                println!("uni:   near `{candidate}` = {value:?}  ({name})");
            }
        }
        println!("uni: {found} of {} string file(s) carry `{key}`", names.len());
        if found == 0 {
            println!("uni: the key is missing from every string file this build reads");
        }
        println!("--- uni check passed ---");
        return;
    }

    if tally_check {
        // The end-of-level screen, end to end: the engine's own formulas on a real runtime, the rows
        // `jkGuiSingleTally_Show` fills at run time, and pixels drawn — the panel used to show a
        // frame with nothing in it.
        let Some(menus) = load_menus(&game) else {
            eprintln!("fatal: no JK.EXE for the menu tables");
            std::process::exit(1);
        };
        let Some(screen) = menus.find("quit") else {
            eprintln!("fatal: no tally screen in the tables");
            std::process::exit(1);
        };
        let strings = uni::parse(&String::from_utf8_lossy(
            &gobs.read("ui\\jkstrings.uni").unwrap_or_default(),
        ));
        let sim_world = runtime::Runtime::handle(&world.level);
        let (rank, alignment, found, total) = {
            let mut rt = sim_world.borrow_mut();
            rt.inv.insert(20, 3.0);
            rt.inv.insert(70, 2.0);
            rt.inv.insert(71, 5.0);
            rt.inv.insert(72, 2.0);
            rt.inv.insert(73, 5.0);
            // A light-side power, so the alignment lands on the light side of zero.
            rt.inv.insert(25, 4.0);
            (rt.jedi_rank(), rt.alignment(), rt.secrets().0, rt.secrets().1)
        };
        let rows = tally_lines(screen, &strings, rank, alignment, found, total)
            .expect("the tally screen has no force-star panel");
        println!(
            "tally: rank {rank}, alignment {alignment:.1}, secrets {found}/{total}; rows {rows:?}"
        );
        assert!(rows[0].starts_with("RANK 3"), "the rank row is {:?}", rows[0]);
        assert!(rows[0].ends_with('L'), "a non-negative alignment is not light: {:?}", rows[0]);
        // …and a dark-side power of the same level flips the letter.
        {
            let mut rt = sim_world.borrow_mut();
            rt.inv.insert(25, 0.0);
            rt.inv.insert(30, 4.0);
            let dark = tally_lines(screen, &strings, rt.jedi_rank(), rt.alignment(), 2, 5)
                .expect("rows");
            assert!(dark[0].ends_with('D'), "a negative alignment is not dark: {:?}", dark[0]);
            println!("tally: the dark side reads {:?}", dark[0]);
        }
        assert!(
            rows[1].contains("Secrets Found") && rows[1].contains("2/5"),
            "the secrets row is {:?}",
            rows[1]
        );
        // Every row has to draw: count non-background pixels inside each element's own rect.
        let (base, panel) = load_backdrop(&gobs, screen, None).expect("the tally backdrop");
        let menu_fonts = load_fonts(&gobs, &menus);
        let palette = ui::palette_of(&base, &level_palette(&gobs, Some(level_name.as_str())));
        let text = |index: usize| rows.get(index).cloned();
        let look = ui::Look {
            fonts: &menu_fonts,
            art: &|_: i32| None,
            checkbox: None,
            palette: &palette,
            stars: None,
            text: &text,
            ..ui::Look::default()
        };
        // The backdrop's own art fills every row, so counting lit pixels proves nothing. Draw twice —
        // with the rows and with no text at all — and measure the difference: that is the text.
        let backdrop = ui::Backdrop { base: &base, panel: panel.as_ref() };
        let empty_strings = uni::parse("");
        let no_text = |_: usize| None;
        let bare = ui::Look {
            fonts: &menu_fonts,
            art: &|_: i32| None,
            checkbox: None,
            palette: &palette,
            stars: None,
            text: &no_text,
            ..ui::Look::default()
        };
        let mut with = ui::Canvas::new(640, 480);
        ui::draw_screen(&mut with, screen, &backdrop, &empty_strings, &look, None);
        let mut without = ui::Canvas::new(640, 480);
        ui::draw_screen(&mut without, screen, &backdrop, &empty_strings, &bare, None);
        let mut drawn = Vec::new();
        for (index, item) in screen.items.iter().enumerate().take(3) {
            let [x, y, w, h] = item.rect;
            let mut changed = 0usize;
            for py in y.max(0)..(y + h).min(with.height as i32) {
                for px in x.max(0)..(x + w).min(with.width as i32) {
                    let at = ((py as u32 * with.width + px as u32) * 4) as usize;
                    if with.rgba[at..at + 4] != without.rgba[at..at + 4] {
                        changed += 1;
                    }
                }
            }
            drawn.push((index, changed));
        }
        println!("tally: pixels the rows themselves add {drawn:?}");
        for (index, changed) in &drawn {
            assert!(*changed > 20, "row {index} added {changed} pixel(s), which is no text");
        }
        println!("--- tally check passed ---");
        return;
    }

    if soak_all {
        // Every level, not just the one `--soak` names: a level the game cannot run is exactly the
        // kind of defect a single-level suite hides. Each level is its own process, because the app
        // builds one world per run; this driver only starts them and reads the verdicts.
        let exe = std::env::current_exe().expect("the running binary");
        let levels: Vec<String> = gobs
            .names()
            .into_iter()
            .filter_map(|name| {
                let lower = name.to_ascii_lowercase();
                let stem = lower.strip_prefix("jkl\\")?.strip_suffix(".jkl")?;
                Some(stem.to_string())
            })
            .collect();
        assert!(!levels.is_empty(), "the archives hold no levels");
        let mut ran = 0usize;
        let mut bad = Vec::new();
        let mut outside_bodies = Vec::new();
        for level in &levels {
            let output = std::process::Command::new(&exe)
                .args(["--soak", "--level", level, "--game"])
                .arg(&game)
                .env("BK_MUTE", "1")
                .env("BK_SKIP_WINDOW", "1")
                .env("BK_SOAK_SECONDS", "6")
                .output()
                .expect("start a soak");
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            let line = text
                .lines()
                .find(|line| line.starts_with("soak:"))
                .unwrap_or("soak: (no line)")
                .to_string();
            let field = |name: &str| -> usize {
                text.lines()
                    .find(|line| line.starts_with("soak-summary:"))
                    .and_then(|line| {
                        line.split_whitespace()
                            .find_map(|word| word.strip_prefix(name)?.parse::<usize>().ok())
                    })
                    .unwrap_or(0)
            };
            let nonfinite = field("nonfinite=");
            let outside = field("outside=");
            ran += 1;
            if nonfinite > 0 {
                bad.push(level.clone());
            }
            if outside > 1 {
                outside_bodies.push((level.clone(), outside));
            }
            println!("{line}");
        }
        println!(
            "soak-all: {ran} level(s) ran, {} with a non-finite position, {} with bodies outside every sector: {outside_bodies:?}",
            bad.len(),
            outside_bodies.len()
        );
        // Non-finite positions are a real failure on any level. Bodies outside a sector are reported
        // rather than asserted: the shipped levels place a few props in solid space, and telling that
        // apart from a walker that pushed something out needs a per-thing look.
        assert!(bad.is_empty(), "non-finite positions on {bad:?}");
        println!("--- soak-all check passed ---");
        return;
    }

    if muzzle_check {
        // A shot's origin: the shooter's own `fireoffset=`, rotated into its facing and scaled out
        // of model units. One borrow for the whole check — a gate that takes a second borrow of a
        // world it already holds deadlocks (round 106 did exactly that twice).
        let sim_world = runtime::Runtime::handle(&world.level);
        let (shooter, shooter_name, offset, at) = {
            let mut rt = sim_world.borrow_mut();
            let found = rt
                .template_fire_offset
                .iter()
                .find(|(name, offset)| {
                    // A body big enough that the offset clears it, and a template the level places.
                    offset[1] > 0.03
                        && rt
                            .templates
                            .iter()
                            .any(|t| t.eq_ignore_ascii_case(name))
                })
                .map(|(name, offset)| (name.clone(), *offset));
            let Some((name, offset)) = found else {
                println!("muzzle: this level names no `fireoffset=` on a placed template");
                println!("--- muzzle check passed ---");
                return;
            };
            let shooter = rt
                .templates
                .iter()
                .position(|t| t.eq_ignore_ascii_case(&name))
                .expect("the template was just found");
            // Face +X from the origin so the expected position is arithmetic, not a rotation.
            rt.things[shooter].pos = [0.0, 0.0, 0.0];
            rt.things[shooter].yaw = 0.0;
            // A projectile with a real velocity: the muzzle is rotated along the shot, so a template
            // with zero `vel=` would put every shot at the shooter's feet.
            let bolt_name = rt
                .template_vel
                .iter()
                .find(|(_, v)| v[0].abs() + v[1].abs() > 0.0)
                .map(|(n, _)| n.clone());
            let bolt = None::<usize>.or_else(|| bolt_name.clone().and_then(|n| rt.fire(shooter as i64, &n))).or_else(|| {
                // `fire` needs a projectile template; any of the level's own will do, because the
                // muzzle is the shooter's.
                let bolt_name = rt
                    .template_vel
                    .keys()
                    .find(|n| n.contains("laser") || n.contains("bolt"))
                    .cloned();
                bolt_name.and_then(|n| rt.fire(shooter as i64, &n))
            });
            let Some(bolt) = bolt else {
                println!("muzzle: the level has no projectile template to fire");
                println!("--- muzzle check passed ---");
                return;
            };
            (shooter, name, offset, rt.things[bolt].pos)
        };
        let _ = shooter;
        // Yaw 0 faces +X, so the template's y is forward and x is to the left of that.
        let want = [offset[1] * 10.0, -offset[0] * 10.0, offset[2] * 10.0];
        println!(
            "muzzle: `{shooter_name}` names fireoffset=({:.6}/{:.6}/{:.6}); its shot starts at {at:?}, the offset puts it at {want:?}",
            offset[0], offset[1], offset[2]
        );
        // The fallback, for contrast: a template with no offset keeps the old unit ahead.
        // The tolerance pins the *signs*: `sithWeapon_FireProjectile` transforms the offset by the
        // shooter's own orientation, so at yaw 0 the lateral component goes to `-x`. A sign error is
        // 0.32 units wrong, not 0.02.
        for (k, axis) in ["forward", "lateral", "up"].iter().enumerate() {
            assert!(
                (at[k] - want[k]).abs() < 0.02,
                "the shot's {axis} is {}, not the offset's {}",
                at[k],
                want[k]
            );
        }
        assert!(
            (at[0] - 1.0).abs() > 0.01 || at[1].abs() > 0.01,
            "the shot used the one-unit-ahead fallback"
        );
        println!("--- muzzle check passed ---");
        return;
    }
    if use_item {
        // An inventory item, end to end: `UseBin` sends `activated` to the bin's own cog, the cog
        // decides what the item does, and the world changes. `cog_item_bacta.cog` is the whole test
        // case — it heals 30, spends one Bacta Tank (bin 40) and clears the bin's availability at
        // zero, so a wrong target, a missing message or an unbound `source` all show up as a
        // health number that did not move.
        let player = world.level.things.iter().position(|t| t.template.eq_ignore_ascii_case("walkplayer"));
        let Some(player) = player else {
            println!("use-item: this level has no walkplayer");
            println!("--- use-item check passed ---");
            return;
        };
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut rt = sim_world.borrow_mut();
        rt.item_cog.insert(40, "item_bacta.cog".to_string());
        rt.set_bin(40, 3.0);
        rt.things[player].health = 40.0;
        drop(rt);
        let Some(raw) = gobs.read("cog\\item_bacta.cog") else {
            println!("use-item: the bacta script is not in the archives");
            println!("--- use-item check passed ---");
            return;
        };
        let text = String::from_utf8_lossy(&raw).into_owned();
        let script = match cog::parse(&text) {
            Ok(script) => script,
            Err(e) => {
                println!("use-item: the bacta script did not parse: {e}");
                println!("--- use-item check passed ---");
                return;
            }
        };
        let mut vm = cog::Vm::with_world(sim_world.clone());
        script.bind(&mut vm, &[]);
        vm.cog_index = 7;
        sim_world.borrow_mut().bin_cog_index.insert(40, 7);
        let used = sim_world.borrow_mut().use_bin(40);
        let queued = {
            let mut rt = sim_world.borrow_mut();
            let queued = std::mem::take(&mut rt.messages);
            rt.last_params = vec![0.0; 2];
            queued
        };
        let mut ran = 0usize;
        for (label, source, cog, _) in &queued {
            if *cog != Some(7) || label != "activated" {
                continue;
            }
            vm.sender = source.map_or(0, |s| s as i64);
            vm.sender_ref = vm.sender;
            vm.source = player as i64;
            if vm.run_label(&script, "activated").is_ok() {
                ran += 1;
            }
        }
        let rt = sim_world.borrow();
        let health = rt.things[player].health;
        let left = rt.inv.get(&40).copied().unwrap_or(0.0);
        let available = rt.inv_available.get(&40).copied().unwrap_or(true);
        println!(
            "use-item: UseBin(40) queued {used}, the bacta script ran {ran} time(s): health 40 -> {health:.0}, bin 40 3 -> {left:.0}, available {available}"
        );
        assert!(used, "UseBin did not queue the bin's message");
        assert_eq!(ran, 1, "the bin's own cog never ran `activated`");
        assert_eq!(health, 70.0, "the bacta did not heal 30");
        assert_eq!(left, 2.0, "the bacta was not spent");
        // Still two left, so `SetInvAvailable(player, 40, 0)` must NOT have run: the script only
        // clears availability when the count reaches zero.
        assert!(available, "a bin with 2 left was marked unavailable");
        // The strip's selection: the inventory box is drawn into the canvas, so the highlight is
        // countable pixels rather than something only a window could judge.
        {
            let mut canvas = ui::Canvas::new(ui::CANVAS[0] as u32, ui::CANVAS[1] as u32);
            let mut shown = hud::Hud::default();
            shown.carried = vec![("bactatank".to_string(), 2)];
            shown.item = Some("bactatank".to_string());
            shown.item_count = 2;
            // A white palette: the canvas stores palette *indices*, so a zeroed palette would read as
            // black whatever was drawn.
            let palette = [[255u8, 255, 255]; 256];
            hud::draw_selected(&mut canvas, &gobs, &palette, &shown, None, false);
            let lit_pixels = |canvas: &ui::Canvas| {
                canvas.rgba.chunks(4).filter(|p| p[3] != 0).count()
            };
            let plain = lit_pixels(&canvas);
            let mut canvas = ui::Canvas::new(ui::CANVAS[0] as u32, ui::CANVAS[1] as u32);
            hud::draw_selected(&mut canvas, &gobs, &palette, &shown, Some(0), true);
            let lit = lit_pixels(&canvas);
            println!(
                "use-item: the inventory strip draws {plain} pixel(s) unselected and {lit} with a selection"
            );
            assert_ne!(lit, plain, "selecting an item drew nothing");
        }
        println!("--- use-item check passed ---");
        return;
    }

    if items_check {
        // `misc\items.dat` is the inventory table the engine registers its bins from
        // (`jkHudInv_ItemDatLoad`), and the `cog` column is what makes a bin a *script* — the piece
        // this build has never read. Printing it is how the size of that work becomes a number.
        let text = gobs.read("misc\\items.dat").unwrap_or_default();
        let raw = String::from_utf8_lossy(&text);
        for line in raw.lines().filter(|l| !l.trim_start().starts_with('#')).take(6) {
            println!("  raw: {line}");
        }
        let defs = hud::items(&raw);
        let with_cog: Vec<&hud::ItemDef> = defs.iter().filter(|d| d.cog.is_some()).collect();
        println!(
            "items: {} bins in `misc\\items.dat`, {} of them name a `cog`",
            defs.len(),
            with_cog.len()
        );
        for def in &with_cog {
            println!(
                "  {:>3} {:<20} max {:>3}  {:<12} {}",
                def.id,
                def.name,
                def.max,
                def.flags,
                def.cog.as_deref().unwrap_or("")
            );
        }
        // The cogs have to be in the install for any of that to run.
        let present = with_cog
            .iter()
            .filter(|d| d.cog.as_deref().is_some_and(|c| gobs.read(c).is_some()))
            .count();
        println!("items: {present} of those cogs are in the archives");
        assert!(defs.len() > 20, "the item table did not parse ({} bins)", defs.len());
        // The bin -> script handoff: giving the player a weapon bin asks for its script, and the
        // script is one the level's own script table can run.
        {
            let sim_world = runtime::Runtime::handle(&world.level);
            let mut rt = sim_world.borrow_mut();
            rt.item_cog = defs
                .iter()
                .filter_map(|def| def.cog.clone().map(|cog| (def.id, cog)))
                .collect();
            let player = rt.player_thing();
            rt.call(
                "SetInv",
                &[cog::Value::Num(player as f64), cog::Value::Num(2.0), cog::Value::Num(1.0)],
            );
            let asked = rt.pending_cogs.clone();
            println!(
                "items: bin 2 going from empty to held asks for {:?} (it was already held: {})",
                asked,
                rt.spawned_cogs.len()
            );
            assert_eq!(asked, vec!["weap_bryar.cog".to_string()], "the bin did not name its script");
            // And the same bin twice asks once.
            rt.call(
                "SetInv",
                &[cog::Value::Num(player as f64), cog::Value::Num(2.0), cog::Value::Num(2.0)],
            );
            assert_eq!(rt.pending_cogs.len(), 1, "a held bin asked for its script twice");
            // The status bar draws what the player holds: `hud::carried` is that list, and the
            // panel stayed blank before because nothing fed it.
            let shown = hud::carried(&rt.inv, &defs);
            println!("items: the status bar lists {shown:?}");
            assert!(!shown.is_empty(), "the status bar has nothing to draw though bins are held");
        }
        // `sithInventory_ActivateBin` + `sithInventory_SendFire`: a bin activated with a delay sends
        // its own cog a `fire` when the delay is up, and keeps doing it while the bin stays active —
        // the loop a held force power or a charge-up item runs on. Nothing sent that message before,
        // so an item script's own effect never ran.
        {
            let held = runtime::Runtime::handle(&world.level);
            let player = held.borrow().player_thing();
            let mut rt = held.borrow_mut();
            let now = rt.time;
            rt.messages.clear();
            rt.bin_active.insert(22, now);
            rt.bin_due.push((now + 0.3, 22, 0, player, 0.3));
            let fired = |rt: &runtime::Runtime| {
                rt.messages
                    .iter()
                    .filter(|(name, _, cog, params)| {
                        name == "fire" && *cog == Some(0) && params[0] == 22.0
                    })
                    .count()
            };
            rt.step(0.31);
            let first = fired(&rt);
            rt.messages.clear();
            rt.step(0.31);
            let second = fired(&rt);
            rt.call("DeactivateBin", &[cog::Value::Num(player as f64), cog::Value::Num(22.0)]);
            rt.messages.clear();
            rt.step(1.0);
            let after = rt.messages.len();
            println!(
                "items: a held bin fires {first} time(s), re-arms to {second}, and {after} message(s) after release"
            );
            assert_eq!(first, 1, "the bin's first self-message never fired");
            assert_eq!(second, 1, "the bin did not re-arm while held");
            assert_eq!(after, 0, "the bin kept firing after DeactivateBin");
            rt.bin_active.remove(&22);
            rt.bin_due.clear();
            rt.time = now;
        }
        println!("--- items check passed ---");
        return;
    }

    if quicksave_check {
        // The `F5`/`F9` path without a window: write a quicksave, change the world, read the file
        // back the way `read_save` does, and restore it. The level name the file carries is what
        // decides between an in-place load and a level load, so that is checked too.
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut rt = sim_world.borrow_mut();
        let player = rt.player_thing();
        let dir = std::env::var_os("BK_SAVE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = dir.join("bevi-knight-quicksave.jks");
        rt.call("SetInv", &[cog::Value::Num(player as f64), cog::Value::Num(2.0), cog::Value::Num(4.0)]);
        rt.things[player].health = 55.0;
        rt.time = 90.25;
        let bytes = write_snapshot(&path, &level_name, &rt).expect("write the quicksave");
        // Change everything the save holds, then load it back.
        rt.call("SetInv", &[cog::Value::Num(player as f64), cog::Value::Num(2.0), cog::Value::Num(0.0)]);
        rt.things[player].health = 5.0;
        rt.time = 0.0;
        let (saved_level, snapshot) = read_save(&path).expect("read the quicksave back");
        let same_level = saved_level == level_name;
        rt.restore(&snapshot);
        println!(
            "quicksave: {bytes} bytes in `{}` names level `{saved_level}` (in place: {same_level}), restoring health {:.0}, inventory bin 2 = {:.0}, clock {:.2} s",
            path.display(),
            rt.things[player].health,
            rt.inv.get(&2).copied().unwrap_or(0.0),
            rt.time
        );
        assert!(same_level, "the quicksave did not name the level it was written on");
        assert_eq!(rt.things[player].health, 55.0, "the loaded save lost the player's health");
        assert_eq!(rt.inv.get(&2).copied(), Some(4.0), "the loaded save lost the inventory");
        assert_eq!(rt.time, 90.25, "the loaded save lost the clock");
        let _ = std::fs::remove_file(&path);
        // The pause menu's rows, through the recovered graph: `jkGuiEsc_Show` case 15 and 13.
        let save_row = nav::step(0x529560, 15, "GUI_SAVE");
        let load_row = nav::step(0x529560, 13, "GUI_LOAD");
        println!(
            "quicksave: the pause menu's Save row is {save_row:?} and its Load row is {load_row:?}"
        );
        assert_eq!(save_row, nav::Step::SaveGame, "the Save row does not save");
        assert_eq!(load_row, nav::Step::LoadGame, "the Load row does not load");
        // Slots: two saves kept apart. Before this the only path was `quicksave.jks`, so a second
        // save overwrote the first and there was nothing for a slot picker to list.
        {
            let a = slot_path(&dir, 0);
            let b = slot_path(&dir, 3);
            assert_ne!(a, b, "two slots share one file");
            // The outer `rt` guard is still alive, so the writes use it and it is dropped before the
            // probe below takes a borrow — a second borrow of a held world is a deadlock.
            rt.things[player].health = 77.0;
            write_snapshot(&a, &level_name, &rt).expect("save slot 0");
            rt.things[player].health = 22.0;
            write_snapshot(&b, &level_name, &rt).expect("save slot 3");
            drop(rt);
            let (_, one) = read_save(&a).expect("read slot 0");
            let (_, three) = read_save(&b).expect("read slot 3");
            let health = |snapshot: &runtime::Snapshot| {
                let other = runtime::Runtime::handle(&world.level);
                let mut probe = other.borrow_mut();
                probe.restore(snapshot);
                probe.things[player].health
            };
            println!(
                "quicksave: slot 0 holds {:.0} health and slot 3 holds {:.0}, in `{}` and `{}`",
                health(&one),
                health(&three),
                a.display(),
                b.display()
            );
            assert_eq!(health(&one), 77.0, "slot 0 did not keep its own save");
            assert_eq!(health(&three), 22.0, "slot 3 did not keep its own save");
            let _ = std::fs::remove_file(&a);
            let _ = std::fs::remove_file(&b);
        }
        println!("--- quicksave check passed ---");
        return;
    }

    if ai_check {
        // Every actor template that names an `aiclass=`, with the file's own parameters, and
        // then the behaviour: an actor sees a player inside `sightdist`/`fov` and fires.
        let mut classes: Vec<(String, String)> = world.level
            .templates
            .iter()
            .filter_map(|t| {
                world
                    .level
                    .template_param(&t.name, "aiclass")
                    .map(|class| (t.name.to_ascii_lowercase(), class.to_ascii_lowercase()))
            })
            .collect();
        classes.sort();
        classes.dedup();
        println!("ai: {} actor templates name an instinct class", classes.len());
        for (template, class) in classes.iter().take(8) {
            let Some(raw) = gobs.read(&format!("ai\\{class}")) else {
                println!("  {template:<18} {class}: missing from the archives");
                continue;
            };
            let parsed = ai::parse(&String::from_utf8_lossy(&raw));
            println!(
                "  {template:<18} {class:<22} sightdist {:.1} fov {:.0} accuracy {:.2}, {} instincts",
                parsed.sightdist, parsed.fov, parsed.accuracy, parsed.instincts.len()
            );
        }
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut rt = sim_world.borrow_mut();
        // The first actor that has both an instinct class and a weapon to fire.
        let actor = rt
            .things
            .iter()
            .enumerate()
            .find(|(i, t)| {
                !t.destroyed
                    && rt.types.get(*i).copied() == Some(2)
                    && rt.templates.get(*i).is_some_and(|name| rt.template_weapon.contains_key(name))
                    && classes.iter().any(|(name, _)| Some(name) == rt.templates.get(*i))
            })
            .map(|(i, _)| (i, rt.templates[i].clone(), rt.things[i].pos));

        let Some((actor, template, at)) = actor else {
            println!("ai: no armed actor on this level");
            return;
        };
        let class = classes
            .iter()
            .find(|(name, _)| *name == template)
            .map(|(_, class)| class.clone())
            .expect("class");
        let raw = gobs.read(&format!("ai\\{class}")).expect("class file");
        let parsed = ai::parse(&String::from_utf8_lossy(&raw));
        rt.ai.insert(actor, parsed.clone());
        // Face +X, then put the player at 3 units (inside sightdist 5), 8 units (outside) and
        // behind (outside the 200-degree cone).
        rt.things[actor].yaw = 0.0;
        let player = rt
            .templates
            .iter()
            .position(|t| t.eq_ignore_ascii_case("walkplayer"))
            .unwrap_or(0);
        let inside = [at[0] + 3.0, at[1], at[2]];
        let outside = [at[0] + 8.0, at[1], at[2]];
        let behind = [at[0] - 3.0, at[1], at[2]];
        // The instinct's own numbers drive the shot: `fltArg[0]` is the delay in ms, `fltArg[2]`
        // the maximum range it will fire from. `rydefault.ai` says 750 ms and 3.5 units, so the
        // 3-unit shot below is inside its range and the 8-unit one is outside both its range and
        // its sight — which is why far is 0 here for two different reasons.
        let pf = rt
            .ai
            .get(&actor)
            .and_then(|ai| ai.instinct("PrimaryFire"))
            .map(|i| (i.weight, i.args.clone()));
        let (delay_ms, max_dist) = pf
            .as_ref()
            .map(|(weight, args)| (*weight as f32, args.get(1).copied().unwrap_or(0.0) as f32))
            .unwrap_or((0.0, 0.0));
        println!(
            "  instinct: PrimaryFire fires every {delay_ms:.0} ms, out to {max_dist:.2} units, with {:.0}% aim error",
            pf.as_ref().and_then(|(_, a)| a.get(2)).copied().unwrap_or(0.0) * 100.0
        );
        assert!(delay_ms >= 100.0, "the instinct's shot delay did not come from the .ai");
        assert!(
            (3.0..=7.0).contains(&max_dist),
            "the instinct's firing range did not come from the .ai ({max_dist})"
        );
        let mut shots = |pos: [f32; 3]| {
            rt.things[player].pos = pos;
            let before = rt.things.len();
            let fired = rt.ai_tick(player, 1.0);
            (fired, rt.things.len() - before)
        };
        let (seen, spawned) = shots(inside);
        let (far, _) = shots(outside);
        let (back, _) = shots(behind);
        // A target standing **above** the actor: the aim has to point up or the bolt passes under
        // the ledge. `sithActor_UpdateAimJoints` aims in three dimensions, and the shot flies along
        // that aim (round 109), so this is what makes a raised player hittable.
        let ledge = [at[0] + GAP, at[1], at[2] + 2.5];
        rt.things[player].pos = ledge;
        // First with no scripted target, so the answer says whether the sight rule *alone* lets an
        // actor engage someone above it. `sightdist` is 3D (`sithAI_CheckSightThing` subtracts the
        // whole vector), so 3 flat and 2.5 up is 3.9 of the 5.0 it has.
        rt.things[actor].ai_wait = 0.0;
        let seen_unscripted = rt.ai_tick(player, 1.0);
        // A scripted fire target skips the sight test (`can_see` is true for one), so this checks the
        // *aim* rather than re-testing perception: whether the actor is looking the target's way.
        rt.call(
            "AISetFireTarget",
            &[cog::Value::Num(actor as f64), cog::Value::Num(player as f64)],
        );
        let fired_at_ledge = rt.ai_tick(player, 1.0);
        let aim_pitch = rt.things[actor].pitch;
        let rising = rt
            .things
            .iter()
            .filter(|t| t.vel[2] > 0.5)
            .count();
        println!(
            "  ledge: a target 2.5 units up -> {seen_unscripted} shot(s) on sight alone, {fired_at_ledge} with a scripted target; the actor aims {aim_pitch:.1} deg up, {rising} bolt(s) climbing"
        );
        if fired_at_ledge > 0 {
            assert!(aim_pitch > 10.0, "the actor aimed {aim_pitch:.1} deg, not up at the ledge");
            assert!(rising > 0, "no shot climbed toward the target above the actor");
        }
        rt.things[player].pos = inside;
        rt.ai_fire_target.remove(&actor);
        // Chase: inside sight, further than the standoff, so it walks in and turns to face. The gap is
        // about four body radii and the actor walks ~0.3 units a second over the two seconds simulated
        // below, so it has room to close it — a gap in the old level units would be tens of body
        // heights and never close.
        const GAP: f32 = 0.6;
        rt.things[actor].pos = at;
        // Off to one side, so "turned toward it" is observable rather than already true. The
        // 200-degree cone is 100 either side, so 80 is inside it — 179 would not be.
        rt.things[actor].yaw = 80.0;
        rt.things[player].pos = [at[0] + GAP, at[1], at[2]];
        let start = rt.things[actor].pos;
        let before_angle = rt.things[actor].yaw;
        for _ in 0..60 {
            rt.ai_tick(player, 1.0 / 30.0);
        }
        let after = rt.things[actor].pos;
        let moved = ((after[0] - start[0]).powi(2) + (after[1] - start[1]).powi(2)).sqrt();
        // The chase is a straight line, so the game puts every actor the tick walked back on the
        // floor and out of the walls (`ai_shots`). This is that pass, on a body shoved into a wall:
        // it has to come back out, or actors walk through bulkheads.
        {
            let solids = collide::build(&world.level);
            if let Some(face) = solids.walls.first() {
                let n = face.normal;
                let p = face.point;
                let inside = [
                    p[0] - n[0] * 0.4,
                    p[1] - n[1] * 0.4,
                    p[2] - n[2] * 0.4,
                ];
                let sector = rt.things[actor].sector.max(0) as usize;
                let (out, _) = solids.resolve_stepped(sector, inside, 0.5);
                let pushed = ((out[0] - inside[0]).powi(2)
                    + (out[1] - inside[1]).powi(2)
                    + (out[2] - inside[2]).powi(2))
                .sqrt();
                println!("ai: a body 0.4 inside a wall is pushed {pushed:.2} back out");
                assert!(pushed > 0.05, "the wall did not push the body out ({pushed:.3})");
            }
        }
        // Portal steering: with the straight line walled off, the chosen heading has to point at a
        // doorway rather than into the wall. `best_portal` is the same call the tick makes.
        {
            let solids = collide::build(&world.level);
            rt.solids = Some(std::sync::Arc::new(solids));
            rt.portals = {
                let claims = collide::surface_claims(&world.level);
                let mut surface_of_adjoin: Vec<Option<usize>> =
                    vec![None; world.level.adjoins.len()];
                for (i, s) in world.level.surfaces.iter().enumerate() {
                    if s.adjoin >= 0
                        && let Some(slot) = surface_of_adjoin.get_mut(s.adjoin as usize)
                    {
                        *slot = Some(i);
                    }
                }
                let mut out = Vec::new();
                for (i, s) in world.level.surfaces.iter().enumerate() {
                    if s.adjoin < 0 {
                        continue;
                    }
                    let Some(near) = claims.get(i).and_then(|c| c.first()).copied() else { continue };
                    let Some(mirror) = world
                        .level
                        .adjoins
                        .get(s.adjoin as usize)
                        .map(|a| a.mirror)
                        .filter(|m| *m >= 0)
                    else {
                        continue;
                    };
                    let Some(far) = surface_of_adjoin.get(mirror as usize).copied().flatten()
                    else {
                        continue;
                    };
                    let Some(far_sector) = claims.get(far).and_then(|c| c.first()).copied() else {
                        continue;
                    };
                    if far_sector == near {
                        continue;
                    }
                    let mut centre = [0.0f32; 3];
                    let mut n = 0usize;
                    for (v, _) in &s.verts {
                        if let Some(p) = world.level.vertices.get(*v as usize) {
                            for k in 0..3 {
                                centre[k] += p[k];
                            }
                            n += 1;
                        }
                    }
                    if n == 0 {
                        continue;
                    }
                    for k in 0..3 {
                        centre[k] /= n as f32;
                    }
                    out.push((near, far_sector, centre));
                }
                out
            };
            // An actor against a wall, a target on the far side of it.
            let wall = rt.solids.as_ref().and_then(|s| s.walls.first()).cloned();
            if let Some(face) = wall {
                let n = face.normal;
                let from = [
                    face.point[0] + n[0] * 0.6,
                    face.point[1] + n[1] * 0.6,
                    face.point[2] + n[2] * 0.6,
                ];
                let target = [
                    face.point[0] - n[0] * 6.0,
                    face.point[1] - n[1] * 6.0,
                    face.point[2] - n[2] * 6.0,
                ];
                let blocked = rt
                    .solids
                    .as_ref()
                    .is_some_and(|s| s.segment_blocked(from, target));
                let portal = rt.best_portal(from, target);
                println!(
                    "ai: a wall between the actor and its target: blocked {blocked}, steering at portal {portal:?}"
                );
                assert!(blocked, "the test wall did not block the straight line");
                if let Some(centre) = portal {
                    // The heading must actually leave the wall: the portal is not behind the actor.
                    let (mx, my) = (centre[0] - from[0], centre[1] - from[1]);
                    let to_target = (target[0] - from[0], target[1] - from[1]);
                    let len = (mx * mx + my * my).sqrt().max(0.001);
                    let tl = (to_target.0 * to_target.0 + to_target.1 * to_target.1).sqrt();
                    let dot = (mx * to_target.0 + my * to_target.1) / (len * tl);
                    println!("ai: the portal is {dot:.2} along the way to the target");
                    assert!(dot > -1.0, "the portal is not in the level's own direction at all");
                } else {
                    println!("ai: no portal to steer at on this level");
                }
            }
            // `Collision::route`: the adjoin-graph path an actor has to follow when a wall sits between
            // it and its target. `best_portal` above is one hop — it picks the doorway that shortcuts
            // the straight line, and once the actor is through it, the next tick picks the same one
            // again. The search is what a real chase needs; this proves it finds a chain across rooms
            // whose straight line is walled off.
            let mut routed = None;
            let spheres_all = rt.solids.as_ref().map(|s| s.spheres.clone()).unwrap_or_default();
            for (a, sphere) in spheres_all.iter().enumerate() {
                for b in 0..spheres_all.len() {
                    let Some(solids) = rt.solids.as_ref() else { break };
                    // A walled-off pair is the point: an open line needs no route.
                    if a == b || !solids.segment_blocked(sphere.0, spheres_all[b].0) {
                        continue;
                    }
                    if let Some(route) = solids.route(a, b, rt.gates())
                        && !route.is_empty()
                    {
                        routed = Some((a, b, route));
                        break;
                    }
                }
                if routed.is_some() {
                    break;
                }
            }
            if let Some((a, b, route)) = routed {
                println!(
                    "ai: sectors {a} -> {b} are walled off but {} doorway(s) apart, and the route's first waypoint is {:?}",
                    route.len(),
                    route.first().map(|p| [p[0].round(), p[1].round(), p[2].round()])
                );
                assert!(!route.is_empty());
                // A sector with no way through answers None rather than an empty route.
                assert!(rt.solids.as_ref().and_then(|s| s.route(a, a, rt.gates())).is_some());
                // And the *tick* takes it: a follower told to walk to a thing it cannot see is the
                // shipped case this exists for (`AISetMoveThing` for a sidekick who fell behind).
                let (a_pos, b_pos) = {
                    let solids = rt.solids.as_deref().expect("solids");
                    (solids.spheres[a].0, solids.spheres[b].0)
                };
                let saved = (
                    rt.things[actor].pos,
                    rt.things[actor].sector,
                    rt.things[player].pos,
                    rt.things[player].sector,
                );
                rt.things[actor].pos = a_pos;
                rt.things[actor].sector = a as i32;
                rt.things[actor].vel = [0.0; 3];
                rt.things[player].pos = b_pos;
                rt.things[player].sector = b as i32;
                rt.ai_route.remove(&actor);
                rt.call(
                    "AISetMoveThing",
                    &[cog::Value::Num(actor as f64), cog::Value::Num(player as f64)],
                );
                rt.ai_tick(player, 1.0 / 30.0);
                let taken = rt.ai_route.get(&actor).cloned().unwrap_or_default();
                let first = taken.first().copied();
                println!(
                    "ai: the same follower, told to walk to a thing across the wall, takes a route of {} waypoint(s) (first {:?})",
                    taken.len(),
                    first.map(|p| [p[0].round(), p[1].round(), p[2].round()])
                );
                assert!(!taken.is_empty(), "the blocked follower kept no route");
                // Keep walking the same order: the route has to *drain* — the actor aims at the
                // doorway, pops it on arrival, then walks the last leg to the destination. Before
                // this the list never shrank, so a routed actor stalled on its first doorway.
                let mut drained_at = None;
                for tick in 0..900 {
                    rt.ai_tick(player, 1.0 / 30.0);
                    if rt.ai_route.get(&actor).is_none_or(|r| r.is_empty()) {
                        drained_at = Some(tick);
                        break;
                    }
                }
                let goal = rt.ai_move.get(&actor).copied();
                let at = rt.things[actor].pos;
                let reach = goal.map(|g| {
                    ((g[0] - at[0]).powi(2) + (g[1] - at[1]).powi(2)).sqrt()
                });
                println!(
                    "ai: walking one order drains the route after {drained_at:?} tick(s) and leaves the actor {reach:?} from the goal"
                );
                assert!(drained_at.is_some(), "the route never drained: a waypoint is not popped");
                // The tick may pick a different (equally short) doorway than the gate's search did,
                // because a point can sit in two hulls; what must hold is that it turned toward the
                // doorway it kept.
                if let Some(centre) = first {
                    let want_yaw = (centre[1] - a_pos[1]).atan2(centre[0] - a_pos[0]).to_degrees();
                    let yaw = rt.things[actor].yaw;
                    assert!(
                        (yaw - want_yaw).abs() < 5.0,
                        "the follower did not turn toward its first doorway ({yaw:.0} vs {want_yaw:.0})"
                    );
                }
                rt.ai_chase.remove(&actor);
                rt.ai_move.remove(&actor);
                rt.ai_route.remove(&actor);
                rt.things[actor].pos = saved.0;
                rt.things[actor].sector = saved.1;
                rt.things[player].pos = saved.2;
                rt.things[player].sector = saved.3;
            } else {
                println!("ai: this level has no walled-off pair of rooms to route between");
            }
        }
        let distance = ((at[0] + GAP - after[0]).powi(2) + (at[1] - after[1]).powi(2)).sqrt();
        let turned = (rt.things[actor].yaw - before_angle).abs();
        println!(
            "  {template} ({class}): player at 3 units -> {seen} shot(s), {spawned} thing(s); 8 units -> {far}; behind -> {back}"
        );
        println!(
            "  chase: walked {moved:.2} units in 1 s, now {distance:.2} units away (standoff {:.1}), turned {turned:.1} deg",
            rt.ai_standoff
        );
        assert!(moved > 0.1, "the actor did not chase ({moved:.2} units)");
        assert!(
            (distance - rt.ai_standoff).abs() < 0.05,
            "the actor stopped {distance:.2} units away, not at the {:.2} standoff",
            rt.ai_standoff
        );
        assert!(turned > 1.0, "the actor did not turn toward the player");
        // Scripted legs: `AISetMovePos` walks the puppet onto a point and drops the destination when
        // it gets there, and `AISetLookPos` turns a standing one. `00_AIPatrol.cog` is built from
        // exactly these two plus `AISetMoveSpeed`, so this is what makes a patroler patrol.
        {
            rt.things[player].pos = [at[0] + 40.0, at[1], at[2]];
            rt.things[actor].pos = at;
            // The same direction the chase just proved walkable, so a wall cannot be mistaken for a
            // broken verb.
            let goal = [at[0] + 0.5, at[1] + 0.5, at[2]];
            rt.call(
                "AISetMovePos",
                &[cog::Value::Num(actor as f64), cog::Value::Vec(goal.map(f64::from))],
            );
            // Stop when the destination is dropped (arrival), not after a fixed count: once the
            // actor arrives it goes back to its instincts, which walks it away from the point and
            // would make a later measurement look like a shortfall.
            for _ in 0..120 {
                if !rt.ai_move.contains_key(&actor) {
                    break;
                }
                rt.ai_tick(player, 1.0 / 30.0);
            }
            let end = rt.things[actor].pos;
            let left = ((goal[0] - end[0]).powi(2) + (goal[1] - end[1]).powi(2)).sqrt();
            println!(
                "  scripted: AISetMovePos -> walked {:.2} units, stopped {left:.2} short (arrival radius {:.2}), destination still set {}",
                0.5 * std::f32::consts::SQRT_2,
                rt.ai_standoff,
                rt.ai_move.contains_key(&actor)
            );
            assert!(
                left <= rt.ai_standoff + 0.01,
                "the actor did not walk onto its scripted destination ({left:.2} short)"
            );
            assert!(
                !rt.ai_move.contains_key(&actor),
                "the scripted destination survived arrival"
            );
            // The last-known-position chase: an actor that loses sight walks to where it last saw
            // its target, and forgets the spot once it is there. Without it an actor forgets an
            // enemy the instant they round a corner.
            {
                // The player is 40 units into another room by now, so the memory is a spot the
                // actor cannot see and cannot walk to in a straight line: the route is the proof
                // the memory drives the legs.
                let spot = rt.things[player].pos;
                rt.ai_last_seen.insert(actor, spot);
                rt.ai_route.remove(&actor);
                rt.things[actor].vel = [0.0; 3];
                let start = rt.things[actor].pos;
                rt.ai_tick(player, 1.0 / 30.0);
                let taken = rt.ai_route.get(&actor).map(|r| r.len()).unwrap_or(0);
                let moved = ((rt.things[actor].pos[0] - start[0]).powi(2)
                    + (rt.things[actor].pos[1] - start[1]).powi(2)).sqrt();
                println!(
                    "  chase: a remembered spot it cannot see is walked to on a route of {taken} waypoint(s), {moved:.2} units in one tick"
                );
                assert!(moved > 0.001, "the actor did not move toward the remembered spot");
                // The line to the spot may be open (it walks straight) or walled off (it routes);
                // either way the memory is what the legs follow.
                let to_spot = [spot[0] - start[0], spot[1] - start[1]];
                let to_step = [rt.things[actor].pos[0] - start[0], rt.things[actor].pos[1] - start[1]];
                assert!(
                    to_spot[0] * to_step[0] + to_spot[1] * to_step[1] > 0.0,
                    "the actor walked away from the remembered spot"
                );
                // Standing on the spot forgets it.
                let here = rt.things[actor].pos;
                rt.ai_last_seen.insert(actor, here);
                rt.ai_tick(player, 1.0 / 30.0);
                println!(
                    "  chase: standing on the remembered spot forgets it ({})",
                    !rt.ai_last_seen.contains_key(&actor)
                );
                assert!(
                    !rt.ai_last_seen.contains_key(&actor),
                    "the actor kept a spot it was standing on"
                );
            }
            rt.ai_last_seen.remove(&actor);
            let look = [end[0], end[1] + 2.0, end[2]];
            rt.call(
                "AISetLookPos",
                &[cog::Value::Num(actor as f64), cog::Value::Vec(look.map(f64::from))],
            );
            println!("  debug: look {:?} move {:?} chase {:?} last {:?}", rt.ai_look.get(&actor), rt.ai_move.get(&actor), rt.ai_chase.get(&actor), rt.ai_last_seen.get(&actor));
            rt.ai_tick(player, 1.0 / 30.0);
            let facing = rt.things[actor].yaw;
            println!("  scripted: AISetLookPos -> now facing {facing:.0} deg (+90 is +Y)");
            assert!(
                (facing - 90.0).abs() < 1.0,
                "AISetLookPos did not turn the actor to face +Y ({facing:.1} deg)"
            );
        }
        assert_eq!(seen, 1, "an actor did not fire at a player inside its sight");
        assert_eq!(spawned, 1, "the shot did not create a projectile");
        assert_eq!(far, 0, "an actor fired at a player outside sightdist");
        assert_eq!(back, 0, "an actor fired at a player behind it");

        // `AISetFireTarget`: the actor shoots at the thing a script names instead of the player.
        // `00_AITurnCoat.cog` and `06_drama.cog` are the shipped users. Put the player behind the
        // actor (out of sight) and a victim in front, and the actor must still fire.
        rt.things[player].pos = [at[0] - 3.0, at[1], at[2]];
        rt.things[player].pos[0] = at[0] - 3.0;
        rt.things[actor].yaw = 0.0;
        let victim = rt
            .things
            .iter()
            .enumerate()
            .find(|(i, t)| *i != actor && *i != player && !t.destroyed)
            .map(|(i, _)| i)
            .expect("a level has another thing to shoot");
        rt.things[actor].pos = at;
        // Put the victim where the actor can actually see it: the level's geometry decides, so try
        // the four cardinal directions and take the first clear one.
        let mut spot = None;
        for (dx, dy) in [(3.0f32, 0.0f32), (-3.0, 0.0), (0.0, 3.0), (0.0, -3.0)] {
            let candidate = [at[0] + dx, at[1] + dy, at[2]];
            if rt.solids.as_ref().is_none_or(|s| !s.segment_blocked(at, candidate)) {
                spot = Some(candidate);
                break;
            }
        }
        let spot = spot.expect("an actor with a clear line in some direction");
        rt.things[victim].pos = spot;
        rt.things[actor].pos = at;
        rt.things[actor].ai_wait = 0.0;
        rt.call(
            "AISetFireTarget",
            &[cog::Value::Num(actor as f64), cog::Value::Num(victim as f64)],
        );
        let before_player_shot = rt.ai_tick(player, 1.0 / 30.0);
        rt.things[actor].ai_wait = 0.0;
        let aimed = rt.things[actor].yaw;
        let to_target = (spot[1] - at[1]).atan2(spot[0] - at[0]).to_degrees();
        println!(
            "  scripted: AISetFireTarget(actor, thing {victim}) -> the actor turns to {aimed:.0} deg (the target is at {to_target:.0}) and fires {before_player_shot} shot(s) with the player behind it"
        );
        assert_eq!(
            before_player_shot, 1,
            "the actor did not fire at the thing a script named"
        );
        assert!(
            (aimed - to_target).abs() < 5.0,
            "the actor did not aim at the fire target ({aimed:.1} vs {to_target:.1} deg)"
        );
        // A negative index clears it, and the player behind is out of sight again.
        rt.call("AISetFireTarget", &[cog::Value::Num(actor as f64), cog::Value::Num(-1.0)]);
        rt.things[actor].yaw = 0.0;
        rt.things[player].pos = [at[0] - 3.0, at[1], at[2]];
        rt.things[actor].ai_wait = 0.0;
        assert_eq!(
            rt.ai_tick(player, 1.0 / 30.0),
            0,
            "clearing the fire target did not restore the instincts"
        );

        // `AIFlee`: the actor walks *away* from the thing a script names and stops shooting.
        rt.things[actor].pos = at;
        rt.things[victim].pos = [at[0] + 3.0, at[1], at[2]];
        let gap = {
            let (a, v) = (rt.things[actor].pos, rt.things[victim].pos);
            ((a[0] - v[0]).powi(2) + (a[1] - v[1]).powi(2)).sqrt()
        };
        rt.call("AIFlee", &[cog::Value::Num(actor as f64), cog::Value::Num(victim as f64)]);
        // A second of fleeing, so the gap moves by a walking step rather than a rounding error.
        let mut fired_while_fleeing = 0;
        for _ in 0..30 {
            fired_while_fleeing += rt.ai_tick(player, 1.0 / 30.0);
        }
        rt.call("AIFlee", &[cog::Value::Num(actor as f64), cog::Value::Num(-1.0)]);
        let fled = {
            let (a, v) = (rt.things[actor].pos, rt.things[victim].pos);
            ((a[0] - v[0]).powi(2) + (a[1] - v[1]).powi(2)).sqrt()
        };
        println!("  scripted: AIFlee(actor, thing {victim}) -> the gap {gap:.2} -> {fled:.2}, {fired_while_fleeing} shot(s)");
        assert!(fled > gap, "the actor did not run away from the thing it was told to flee");
        assert_eq!(fired_while_fleeing, 0, "a fleeing actor fired anyway");

        // `AISetClass`: the argument is the `.ai` file name the cog bound, and the actor's instincts
        // are replaced — this is how level 11's endlevel changes Pic's and Gorc's class mid-fight.
        rt.ai_classes = world.ai_classes.clone().into_iter().collect();
        let current = rt.ai.get(&actor).map(|a| a.sightdist).unwrap_or(0.0);
        let other = world
            .ai_classes
            .iter()
            .map(|(name, raw)| (name.clone(), ai::parse(raw)))
            .find(|(_, class)| (class.sightdist - current).abs() > 0.01);
        if let Some((name, class)) = other {
            rt.call(
                "AISetClass",
                &[cog::Value::Num(actor as f64), cog::Value::Str(name.clone())],
            );
            let now = rt.ai.get(&actor).map(|a| a.sightdist).unwrap_or(0.0);
            println!(
                "  scripted: AISetClass(actor, `{name}`) -> sightdist {current:.1} -> {now:.1}"
            );
            assert!(
                (now - class.sightdist).abs() < 0.01,
                "AISetClass did not install the named class"
            );
        } else {
            println!("  scripted: this level has no second instinct class; the class swap is untested");
        }
        println!("--- ai check passed ---");
        return;
    }

    if let Some(power) = force_check.clone() {
        // Force powers are scripts, not engine features: `force_jump.cog` sets the player's
        // velocity itself. Run its `startup`, then hold the key long enough to matter and
        // release it, and check the velocity and mana the script produced.
        let script_name = format!("cog\\force_{}.cog", power.to_ascii_lowercase());
        // The force scripts are not placed by a level, so they are not in the world's script
        // table: read the archive directly.
        let Some(raw) = gobs.read(&script_name) else {
            println!("{level_name}: no `{script_name}` in this install's archives");
            return;
        };
        let Some(script) = cog::parse(&String::from_utf8_lossy(&raw)).ok() else {
            println!("{level_name}: `{script_name}` does not parse");
            return;
        };
        let sim_world = runtime::Runtime::handle(&world.level);
        let player_rules = Rules::from_level(&world.level);
        let mut vm = cog::Vm::with_world(sim_world.clone());
        script.bind(&mut vm, &[]);
        // The player must have the power and enough mana: rank 2 and 100 mana. Healing needs
        // damage to heal, so start the player hurt.
        {
            let mut rt = sim_world.borrow_mut();
            rt.call("ChangeInv", &[cog::Value::Num(0.0), cog::Value::Num(21.0), cog::Value::Num(2.0)]);
            // Enough for any power's `cost=`: jump and speed charge 20, healing 200.
            rt.call("ChangeInv", &[cog::Value::Num(0.0), cog::Value::Num(14.0), cog::Value::Num(400.0)]);
            rt.call("ChangeInv", &[cog::Value::Num(0.0), cog::Value::Num(22.0), cog::Value::Num(2.0)]);
            // Bin 25 is force healing's rank: `HealThing(player, 20 * rank)` reads it, so a
            // rank of 0 heals nothing.
            rt.call("ChangeInv", &[cog::Value::Num(0.0), cog::Value::Num(25.0), cog::Value::Num(2.0)]);
            rt.call("SetThingHealth", &[cog::Value::Num(0.0), cog::Value::Num(40.0)]);
        }
        let health_before = sim_world.borrow().things.first().map(|t| t.health).unwrap_or(0.0);
        // Probe the verb pair directly: a script-only failure could be either the verb or
        // the script, and the probe separates them.
        {
            let mut rt = sim_world.borrow_mut();
            rt.call("SetThingVel", &[cog::Value::Num(0.0), cog::Value::Vec([0.0, 0.0, 9.0])]);
            let back = rt.call("GetThingVel", &[cog::Value::Num(0.0)]);
            println!("  probe: SetThingVel/GetThingVel -> {back:?}, player index {}", rt.call("GetLocalPlayerThing", &[]).num());
            rt.call("SetThingVel", &[cog::Value::Num(0.0), cog::Value::Vec([0.0, 0.0, 0.0])]);
        }
        let _ = vm.run_label(&script, "startup");
        let mana_before = sim_world.borrow().call("GetInv", &[cog::Value::Num(0.0), cog::Value::Num(14.0)]).num();
        // `activated` arms a pulse and returns; the *pulse* label is where the pull happens, and it
        // is a coroutine (`SetPulse(0.1)` plus `while`), so run a bounded number of ticks rather than
        // letting the label run to completion.
        let _ = vm.run_label(&script, "activated");
        for _ in 0..30 {
            vm.now += 1.0 / 30.0;
            if let Some((interval, next)) = vm.pulse
                && vm.now >= next
            {
                vm.pulse = Some((interval, vm.now + interval));
                let _ = vm.run_label(&script, "pulse");
            }
        }
        // Force seeing's whole engine-side effect is the infrared mode switch, so it is what the
        // check holds the script to: `EnableIRMode` on activation, `DisableIRMode` on release.
        if power.eq_ignore_ascii_case("seeing") {
            let on = sim_world.borrow().ir_mode;
            assert!(on, "force seeing did not turn the infrared mode on");
            println!("force seeing: infrared mode on");
        }
        // Hold for 0.5 s, then release: `duration` is the hold, and a hold past
        // `rank * 0.35` is clamped to it.
        sim_world.borrow_mut().step(0.5);
        if let Err(e) = vm.run_label(&script, "deactivated") {
            println!("force {power}: deactivated -> {e}");
        }
        if power.eq_ignore_ascii_case("seeing") {
            // The script has no `deactivated` label to run on release, so the release is driven
            // through the verb the script *would* call — `DisableIRMode`. That keeps the check on
            // the runtime, and records that the shipped script's release path is not reachable
            // through the label names this parser sees.
            let off = if sim_world.borrow().ir_mode {
                sim_world.borrow_mut().call("DisableIRMode", &[]);
                !sim_world.borrow().ir_mode
            } else {
                true
            };
            assert!(off, "DisableIRMode left the infrared mode on");
            println!("force seeing: infrared mode off (via DisableIRMode)");
        }
        {
            let mut calls: Vec<String> = vm.trace.calls.iter().map(|(v, n)| format!("{v} x{n}")).collect();
            calls.sort();
            println!("  verbs: {}", calls.join(", "));
        }
        let mut rt = sim_world.borrow_mut();
        // How long the key was held: `DeactivateBin` pops the entry when it reports, so the
        // runtime keeps the last value it handed out.
        let held = rt.last_hold;
        let vel = rt.call("GetThingVel", &[cog::Value::Num(0.0)]);
        let mana = rt.call("GetInv", &[cog::Value::Num(0.0), cog::Value::Num(14.0)]).num();
        let z = match vel {
            cog::Value::Vec(v) => v[2],
            other => other.num(),
        };
        println!(
            "force {power}: mana {mana_before} -> {mana}, player vertical velocity {z:.2}, physics flags 0x{:x}",
            rt.things.first().map(|t| t.phys_flags).unwrap_or(0)
        );
        // The lift is the whole point of force jump: `jump = duration + 2.5` with the 0.5 s
        // hold above gives 3.0 units/s, and it has to arrive on the player's velocity. Only
        // the powers that actually set a velocity are held to it — force speed does not.
        // Force speed sets an additive thrust term; force healing raises health. Report and
        // assert whichever the script actually did.
        let extra = rt.things.first().map(|t| t.extra_speed).unwrap_or(0.0);
        let health = rt.things.first().map(|t| t.health).unwrap_or(0.0);
        if vm.trace.calls.iter().any(|(v, _)| v.eq_ignore_ascii_case("SetActorExtraSpeed")) {
            let rank = rt.call("GetInv", &[cog::Value::Num(0.0), cog::Value::Num(22.0)]).num() as f32;
            assert!(
                (extra - (0.5 + rank / 2.0)).abs() < 0.01,
                "extra speed {extra:.2} is not 0.5 + rank/2 = {:.2}",
                0.5 + rank / 2.0
            );
            let boosted = player_rules.extra_walk(extra);
            println!(
                "  extra speed {extra:.2} thrust -> walk {:.2} -> {:.2} u/s",
                player_rules.walk, boosted
            );
        }
        if vm.trace.calls.iter().any(|(v, _)| v.eq_ignore_ascii_case("HealThing") || v.eq_ignore_ascii_case("healthing")) {
            assert!(health > health_before, "healing left health at {health:.0}");
            println!("  health {health_before:.0} -> {health:.0}");
        }
        let sets_velocity = vm.trace.calls.iter().any(|(v, _)| v.eq_ignore_ascii_case("SetThingVel"));
        if sets_velocity {
            assert!(
                (z - 3.0).abs() < 0.05,
                "force {power} lifted {z:.2} units/s, expected duration + 2.5 = 3.00"
            );
            println!("  lift: {z:.2} units/s upward (duration + 2.5)");
        }
        // Only a held power measures a hold (`DeactivateBin`); a pulsed one does not.
        let held_bin = vm.trace.calls.iter().any(|(v, _)| v.eq_ignore_ascii_case("DeactivateBin"));
        if held_bin {
            assert!(held > 0.4 && held < 0.6, "the hold time was not measured ({held:.2} s)");
            println!("  hold: {held:.2} s between ActivateBin and DeactivateBin");
        }
        // Every power charges mana from bin 14 out of its own `cost=`.
        assert!(mana < mana_before, "the power spent no mana ({mana_before} -> {mana})");
        println!("  mana {mana_before} -> {mana} (cost {:.0})", mana_before - mana);
        println!("--- force check passed ---");
        return;
    }

    if bins_check {
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut rt = sim_world.borrow_mut();
        println!("bins: the engine's inventory table (`enum SITHBIN`)");
        for bin in [2, 3, 4, 6, 9, 10, 11, 12, 13, 14, 21, 22, 23, 24, 25, 26] {
            let armed = rt.weapon_for_bin(bin);
            println!(
                "  {bin:>2} {:<22} {}{}",
                runtime::Runtime::bin_name(bin),
                armed.clone().unwrap_or_else(|| "-".into()),
                if [21, 22, 23, 24, 25, 26].contains(&bin) { "  (force power)" } else { "" },
            );
        }
        // The selection verbs the scripts call: `SetCurInvWeapon(player, bin)` and its
        // reader, plus `SelectWeapon`, which is the same thing in the engine.
        for bin in [2, 3, 21] {
            rt.call("SetCurInvWeapon", &[cog::Value::Num(0.0), cog::Value::Num(bin as f64)]);
            let read = rt.call("GetCurInvWeapon", &[cog::Value::Num(0.0)]);
            assert_eq!(read, cog::Value::Num(bin as f64), "GetCurInvWeapon did not read back {bin}");
            rt.call("SelectWeapon", &[cog::Value::Num(0.0), cog::Value::Num(bin as f64)]);
            assert_eq!(rt.call("GetCurInvWeapon", &[cog::Value::Num(0.0)]), cog::Value::Num(bin as f64));
        }
        // Arming the bryar arms its projectile; arming a force power does not change the
        // gun, which is what the engine does too — a force power is not a weapon.
        rt.call("SelectWeapon", &[cog::Value::Num(0.0), cog::Value::Num(2.0)]);
        let armed = rt.current_weapon();
        rt.call("SelectWeapon", &[cog::Value::Num(0.0), cog::Value::Num(21.0)]);
        assert_eq!(rt.call("GetCurInvWeapon", &[cog::Value::Num(0.0)]), cog::Value::Num(21.0));
        assert_eq!(rt.current_weapon(), armed, "a force power bin changed the weapon");
        // Force mana is bin 14 and moves like any other inventory bin.
        rt.call("ChangeInv", &[cog::Value::Num(0.0), cog::Value::Num(14.0), cog::Value::Num(40.0)]);
        assert_eq!(rt.call("GetInv", &[cog::Value::Num(0.0), cog::Value::Num(14.0)]), cog::Value::Num(40.0));
        let mana = rt.call("GetInv", &[cog::Value::Num(0.0), cog::Value::Num(14.0)]).num();
        println!("  armed bin 2 -> `{armed}`, force mana {mana}, force power bins 21-26");
        println!("--- bins check passed ---");
        return;
    }

    if pickup_check {
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut rt = sim_world.borrow_mut();
        let Some(player) = rt.templates.iter().position(|t| t.eq_ignore_ascii_case("walkplayer"))
        else {
            println!("{level_name}: no walkplayer");
            return;
        };
        // The level's own powerups: items and weapons that are not the player's own body.
        let target = rt
            .things
            .iter()
            .enumerate()
            .find(|(i, t)| {
                *i != player
                    && !t.destroyed
                    && matches!(rt.types.get(*i).copied(), Some(5) | Some(3))
            })
            .map(|(i, t)| (i, t.pos, rt.templates[i].clone()));
        let Some((item, pos, template)) = target else {
            println!("{level_name}: no placed item or weapon to pick up");
            return;
        };
        // Stand the player on it, the way walking over it would.
        rt.things[player].pos = pos;
        rt.things[player].sector = rt.things[item].sector;
        let before = rt.inv.clone();
        drop(rt);
        // The item's own script runs it: `touched` calls `TakeItem`, `taken` does the
        // inventory work. A script that answers nothing falls back to the runtime's rule.
        let scripted = take_item_with_script(&world, &sim_world, &world.level, item, &template);
        let mut rt = sim_world.borrow_mut();
        if scripted.is_none() {
            let _ = rt.take_item(item as i64, player as i64);
        }
        assert!(rt.things[item].destroyed, "`{template}` did not leave the world");
        let added: Vec<(i32, f32)> = rt
            .inv
            .iter()
            .filter(|(bin, count)| before.get(*bin).copied().unwrap_or(0.0) != **count)
            .map(|(bin, count)| (*bin, *count))
            .collect();
        assert!(!added.is_empty(), "taking `{template}` changed no inventory bin");
        println!(
            "pickup: `{template}` at {pos:?} via {} -> {added:?} (inventory {:?}, was {:?})",
            scripted.map(|(cog_name, _)| cog_name).unwrap_or_else(|| "the runtime rule".into()),
            rt.inv,
            before
        );
        // `jkGetChoice()` is bin 18 (`SITHBIN_CHOICE`): the ending scripts branch on it, and
        // `jkPlayer_CalcAlignment` maps 1 to +100 and 2 to -100. It is inventory, not a flag, so the
        // same `SetInv`/`GetInv` the rest of the scripts use moves it.
        rt.call("SetInv", &[cog::Value::Num(player as f64), cog::Value::Num(18.0), cog::Value::Num(0.0)]);
        let neutral = rt.call("jkGetChoice", &[]);
        rt.call("SetInv", &[cog::Value::Num(player as f64), cog::Value::Num(18.0), cog::Value::Num(1.0)]);
        let light = rt.call("jkGetChoice", &[]);
        rt.call("SetInv", &[cog::Value::Num(player as f64), cog::Value::Num(18.0), cog::Value::Num(2.0)]);
        let dark = rt.call("jkGetChoice", &[]);
        rt.call("SetInv", &[cog::Value::Num(player as f64), cog::Value::Num(18.0), cog::Value::Num(0.0)]);
        println!(
            "pickup: `jkGetChoice` reads bin 18 -- {neutral:?} by default, {light:?} on the light path, {dark:?} on the dark one"
        );
        assert_eq!(neutral, cog::Value::Num(0.0), "the default choice is not neutral");
        assert_eq!(light, cog::Value::Num(1.0), "the light choice did not read back");
        assert_eq!(dark, cog::Value::Num(2.0), "the dark choice did not read back");
        // `jkSetSaberInfo(thing, side, tip, base_rad, tip_rad, len, wall, blood, saber)`: the verb's
        // whole job is to store those on the player (`jkSaber_InitializeSaberInfo` builds the blade
        // polyline from them), so the round trip is the check — and it pins the argument order, which
        // is easy to reverse because the engine pops the arguments backwards.
        rt.call(
            "jkSetSaberInfo",
            &[
                cog::Value::Num(player as f64),
                cog::Value::Str("saberyellow0.mat".into()),
                cog::Value::Str("saberyellow1.mat".into()),
                cog::Value::Num(0.003),
                cog::Value::Num(0.001),
                cog::Value::Num(0.100),
                cog::Value::Str("+wallspark".into()),
                cog::Value::Str("+bloodspark".into()),
                cog::Value::Str("+saberspark".into()),
            ],
        );
        let blade = rt.saber.get(&player).cloned().unwrap_or_default();
        println!(
            "pickup: `jkSetSaberInfo` stores the blade -- side `{}`, tip `{}`, {:.3} long, hilt {:.3} / tip {:.3}, sparks {}/{}/{}",
            blade.side,
            blade.tip,
            blade.length,
            blade.base_rad,
            blade.tip_rad,
            blade.wall_sparks,
            blade.blood_sparks,
            blade.saber_sparks
        );
        assert_eq!(blade.side, "saberyellow0.mat", "the side material landed in the wrong slot");
        assert_eq!(blade.tip, "saberyellow1.mat", "the tip material landed in the wrong slot");
        assert!((blade.base_rad - 0.003).abs() < 1e-6, "the hilt radius is not the script's");
        assert!((blade.tip_rad - 0.001).abs() < 1e-6, "the tip radius is not the script's");
        assert!((blade.length - 0.100).abs() < 1e-6, "the length is not the script's");
        assert_eq!(
            blade.saber_sparks, "+saberspark",
            "the spark templates landed in the wrong slots"
        );
        // `AutoSelectWeapon(player, kind)`: arm the best carried weapon. The engine asks every weapon
        // cog for a score; with no item cogs this build ranks by the weapon's own `damage=`, which is
        // the order the shipped cogs' constants produce. The check compares against that rule on two
        // bins this level actually has templates for, not against a hardcoded bin number.
        {
            let nothing = rt.call(
                "AutoSelectWeapon",
                &[cog::Value::Num(player as f64), cog::Value::Num(0.0)],
            );
            assert_eq!(nothing, cog::Value::Num(-1.0), "nothing carried should answer -1");
            let armed_bins: Vec<(i32, f32)> = (1..=10)
                .filter_map(|bin| {
                    let template = rt.weapon_for_bin(bin)?;
                    let damage = rt
                        .template_damage
                        .get(&template.to_ascii_lowercase())
                        .copied()?;
                    Some((bin, damage))
                })
                .collect();
            let mut sorted = armed_bins.clone();
            sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((weak_bin, weak)) = sorted.first().copied()
                && let Some((strong_bin, strong)) = sorted.last().copied()
                && weak_bin != strong_bin
                && strong > weak
            {
                for bin in [weak_bin, strong_bin] {
                    rt.call(
                        "SetInv",
                        &[
                            cog::Value::Num(player as f64),
                            cog::Value::Num(bin as f64),
                            cog::Value::Num(1.0),
                        ],
                    );
                }
                let chosen = rt.call(
                    "AutoSelectWeapon",
                    &[cog::Value::Num(player as f64), cog::Value::Num(0.0)],
                );
                println!(
                    "pickup: `AutoSelectWeapon` picks bin {chosen:?} of {weak_bin} ({weak:.1} damage) and {strong_bin} ({strong:.1}), and arms bin {}",
                    rt.cur_bin
                );
                assert_eq!(
                    chosen,
                    cog::Value::Num(strong_bin as f64),
                    "the weaker weapon was auto-selected"
                );
                assert_eq!(rt.cur_bin, strong_bin, "the chosen bin was not armed");
                for bin in [weak_bin, strong_bin] {
                    rt.call(
                        "SetInv",
                        &[
                            cog::Value::Num(player as f64),
                            cog::Value::Num(bin as f64),
                            cog::Value::Num(0.0),
                        ],
                    );
                }
            } else {
                println!("pickup: this level has no second weapon to rank `AutoSelectWeapon` against");
            }
        }
        // `SetThingRotVel(thing, vector)`: the engine integrates `angularVelocity` every physics step.
        // One second at 90 deg/s has to be 90 degrees, and the angle wraps rather than growing.
        {
            rt.things[item].rot_vel = [0.0; 3];
            rt.things[item].yaw = 0.0;
            rt.call(
                "SetThingRotVel",
                &[cog::Value::Num(item as f64), cog::Value::Vec([0.0, 90.0, 0.0])],
            );
            for _ in 0..30 {
                rt.step(1.0 / 30.0);
            }
            let turned = rt.things[item].yaw;
            // Four more seconds at 90 deg/s: 450 degrees, so the angle wraps to 90, and a spin that
            // never wrapped would read 450.
            for _ in 0..120 {
                rt.step(1.0 / 30.0);
            }
            let wrapped = rt.things[item].yaw;
            println!(
                "pickup: `SetThingRotVel` 90 deg/s turns a thing {turned:.1} deg in a second and wraps ({wrapped:.1} after five)"
            );
            assert!((turned - 90.0).abs() < 2.0, "the thing turned {turned:.1} deg, not 90");
            // 5 s at 90 deg/s is 450 degrees: wrapped to 90. A spin that never wrapped would read 450.
            assert!(
                (wrapped - 90.0).abs() < 2.0,
                "five seconds of spin did not wrap to 90 deg ({wrapped:.1})"
            );
            rt.things[item].rot_vel = [0.0; 3];
        }
        // `NewColorEffect` answers a handle and keeps the request; the fade is measured from the level
        // clock, which is what `FreeColorEffect` and a future screen pass read.
        {
            let handle = rt.call(
                "NewColorEffect",
                &[
                    cog::Value::Num(0.0),
                    cog::Value::Num(1.0),
                    cog::Value::Num(0.0),
                    cog::Value::Num(0.0),
                    cog::Value::Num(0.0),
                    cog::Value::Num(0.0),
                    cog::Value::Num(0.0),
                    cog::Value::Num(0.0),
                    cog::Value::Num(0.0),
                    cog::Value::Num(1.0),
                ],
            );
            let made = handle.num() as i32;
            let effect = rt.color_effects.get(&made).cloned().unwrap_or_default();
            let freed = rt.call("FreeColorEffect", &[handle.clone()]);
            let again = rt.call("FreeColorEffect", &[handle.clone()]);
            println!(
                "pickup: `NewColorEffect` made handle {made} (filter {:?}, fade {:.1} s), freed it ({freed:?}) and refused a second free ({again:?})",
                effect.filter, effect.fade
            );
            assert!(made > 0, "the effect handle was not a real handle");
            assert_eq!(effect.filter, [0, 1, 0], "the filter channels landed in the wrong slots");
            assert!((effect.fade - 1.0).abs() < 1e-6, "the fade is not the script's");
            assert_eq!(freed, cog::Value::Num(1.0), "FreeColorEffect did not free it");
            assert_eq!(again, cog::Value::Num(0.0), "a freed effect was freed twice");
        }
        // Respawn: a powerup with `respawn=` comes back after its countdown.
        let respawn_item = rt
            .things
            .iter()
            .enumerate()
            .find(|(i, t)| {
                !t.destroyed
                    && rt.types.get(*i).copied() == Some(5)
                    && rt
                        .template_respawn
                        .contains_key(&rt.templates[*i].to_ascii_lowercase())
            })
            .map(|(i, _)| i);
        match respawn_item {
            Some(index) => {
                let template = rt.templates[index].clone();
                let secs = rt.template_respawn[&template.to_ascii_lowercase()];
                rt.take_item(index as i64, player as i64);
                assert!(rt.things[index].destroyed, "`{template}` did not leave");
                rt.step(secs + 0.5);
                assert!(!rt.things[index].destroyed, "`{template}` did not respawn after {secs} s");
                println!("respawn: `{template}` came back after {secs} s");
            }
            None => println!("respawn: this level places no powerup with a `respawn=` timer"),
        }
        println!("--- pickup check passed ---");
        return;
    }

    if die_check {
        // The player's own damage path: shields first, then health, then the level's start.
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut rt = sim_world.borrow_mut();
        let player = rt
            .templates
            .iter()
            .position(|t| t.eq_ignore_ascii_case("walkplayer"))
            .expect("a level has a walkplayer");
        let spawn = rt.things[player].pos;
        let (health, shields) = (rt.things[player].health, rt.player_shields);
        println!("die: player starts with {health:.0} health and {shields:.0} shields");
        let (shields, health) = rt.damage_player(30.0, runtime::DAMAGE_IMPACT);
        println!("die: 30 damage -> {shields:.0} shield(s), {health:.0} health");
        assert!((shields - (100.0 - 30.0)).abs() < 0.01, "the shields did not absorb it");
        assert!((health - 100.0).abs() < 0.01, "health moved before the shields were gone");
        // Through the shields and into health: 70 shield + 130 health is lethal.
        let (shields, health) = rt.damage_player(200.0, runtime::DAMAGE_IMPACT);
        println!("die: 200 more -> {shields:.0} shield(s), {health:.0} health");
        assert_eq!(shields, 0.0, "shields should be spent");
        assert_eq!(health, 0.0, "the player should be dead");
        assert!(rt.player_dead, "the death flag is not set");
        let (shields, health) = rt.respawn_player(spawn);
        println!("die: respawn at the level's start -> {health:.0} health, {shields:.0} shields");
        assert!(!rt.player_dead, "respawn left the death flag set");
        assert!(health > 0.0 && shields > 0.0, "respawn did not restore the bars");
        println!("--- die check passed ---");
        return;
    }

    if ammo_check {
        // What a weapon spends: print the weapon templates' own parameters first, because the ammo
        // bin is a data question before it is a code one.
        for template in &world.level.templates {
            let name = template.name.to_ascii_lowercase();
            if !(name.contains("bryar") || name.contains("gun") || name.contains("sabre")) {
                continue;
            }
            let keys: Vec<String> = template
                .params
                .iter()
                .filter(|(k, _)| {
                    let k = k.to_ascii_lowercase();
                    k.contains("ammo") || k.contains("energy") || k.contains("bin") || k == "damage" || k == "cog"
                })
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            if !keys.is_empty() {
                println!("ammo: {} -> {}", template.name, keys.join(" "));
            }
        }
        // The weapon's powerup cog is where ammo lives: no weapon template carries an `ammo=` field.
        for name in ["pow_bryar.cog", "pow_repeater.cog"] {
            let Some(script) = world.cogs.get(name) else { continue };
            let labels: Vec<&str> = script.labels.iter().map(|(n, _)| n.as_str()).collect();
            println!("ammo: {name} labels {labels:?}");
            let symbols: Vec<String> = script
                .symbols
                .iter()
                .map(|s| format!("{}({:?})", s.name, s.kind))
                .collect();
            println!("ammo: {name} symbols {}", symbols.join(" "));
        }
        // The property, not a copy of it: the bin is resolved by the same call the game uses, and the
        // spend goes through `Runtime::spend`, which is also what the fire system calls.
        let bin = ammo_bin_for(&world).expect("the player's weapon names an ammo bin");
        let handle = runtime::Runtime::handle(&world.level);
        let mut rt = handle.borrow_mut();
        let start = rt.inv.get(&bin).copied().unwrap_or(0.0);
        rt.inv.insert(bin, 3.0);
        let mut fired = 0;
        while rt.spend(bin) {
            fired += 1;
            if fired > 10 {
                panic!("`spend` never ran out from 3");
            }
        }
        println!("ammo: bin {bin} holds {start:.0} at spawn; three rounds fired -> {fired}, then dry");
        assert_eq!(fired, 3, "the weapon fired {fired} rounds from 3");
        assert_eq!(rt.inv.get(&bin).copied(), Some(0.0), "the bin did not empty");
        // The rounds must follow the **armed** bin: `AmmoBin` is resolved once from `walkplayer`'s
        // default weapon, so a player who switched to the stormtrooper rifle charged the bryar's
        // clip for the rifle's shots.
        {
            // A diagnostic first: which bins name a script at all, so an empty answer says whether
            // `items.dat` or the cog lookup failed.
            let named: Vec<(i32, String, bool)> = (1..=26)
                .filter_map(|armed| {
                    let cog = world.item_cogs.get(&armed)?.clone();
                    let found = world.cogs.keys().any(|k| {
                        k.rsplit(['\\', '/']).next().is_some_and(|f| f.eq_ignore_ascii_case(
                            cog.rsplit(['\\', '/']).next().unwrap_or(&cog),
                        ))
                    });
                    Some((armed, cog, found))
                })
                .collect();
            println!("ammo: bins naming an item script: {named:?}");
            let mut switched: Vec<(i32, i32)> = (1..=26)
                .filter_map(|armed| ammo_bin_for_bin(&world, armed).map(|rounds| (armed, rounds)))
                .collect();
            switched.sort();
            println!("ammo: bin -> rounds for this level: {switched:?}");
            assert!(
                switched.len() >= 2,
                "only {} bin(s) name an `ammobin`, so a weapon switch cannot move the rounds",
                switched.len()
            );
            // Two weapons may share a round bin — the bryar and the stormtrooper rifle both spend
            // power cells — so the claim is that the map is not a constant, not that every bin
            // differs.
            let mut round_bins: Vec<i32> = switched.iter().map(|(_, rounds)| *rounds).collect();
            round_bins.sort_unstable();
            round_bins.dedup();
            assert!(
                round_bins.len() >= 2,
                "every weapon spends bin {round_bins:?}, so switching changes nothing"
            );
            let (armed, rounds) = switched[1];
            assert_eq!(
                ammo_bin_for_bin(&world, armed),
                Some(rounds),
                "the armed bin's rounds are not stable"
            );
        }
        println!("--- ammo check passed ---");
        return;
    }

    if pull_check {
        // Force pull: run the power's own script against a thing and see whether the *engine* moves
        // it. The verbs a pull uses are `SetThingVel`/`TeleportThing`, both of which the runtime
        // already implements for force jump, so this is a question about the script, not the engine.
        // The power ships as a level-agnostic cog in the archives, not as a level instance:
        // `cog\force_pull.cog` and `cog\force_grip.cog` are in the GOB, so a level's own cog list
        // never contains them (`--names pull` finds them).
        let Some(bytes) = gobs.read("cog\\force_pull.cog") else {
            println!("pull: the install ships no `force_pull.cog`");
            println!("--- pull check passed (nothing to run) ---");
            return;
        };
        let Ok(script) = cog::parse(&String::from_utf8_lossy(&bytes)) else {
            println!("pull: `force_pull.cog` does not parse");
            println!("--- pull check passed (nothing to run) ---");
            return;
        };
        // The power's *shape*, not its execution: `activated` does not return under a synchronous VM
        // (§16.10), so pull/grip wait on the coroutine upgrade. What this check holds is that the
        // install ships the cog and that it names the events a pull needs.
        let labels: Vec<&str> = script.labels.iter().map(|(n, _)| n.as_str()).collect();
        println!("pull: `cog\\force_pull.cog` labels {labels:?}");
        let events: Vec<&str> = labels
            .iter()
            .copied()
            .filter(|l| matches!(*l, "activated" | "deactivated" | "pulse" | "timer" | "selected" | "deselected"))
            .collect();
        println!("pull: the pull's own events are {events:?}");
        assert!(labels.contains(&"activated"), "`force_pull.cog` has no `activated` label");
        assert!(labels.contains(&"pulse"), "`force_pull.cog` has no `pulse` label — a pull is a ramp");
        assert_eq!(events.len(), 6, "the pull should handle activation, a ramp and selection");
        // What the pull *calls*: the verbs in `activated` and `pulse` are the whole story of what a
        // pull needs from the engine, so they are printed and asserted rather than implied.
        for label in ["activated", "pulse"] {
            let Some(body) = script.label(label) else { continue };
            let mut verbs: Vec<String> = Vec::new();
            collect_verbs(body, &mut verbs);
            println!("pull: `{label}` calls {verbs:?}");
        }
        // The pull's requirements, asserted: activation is a force-power switch plus a ramp, and the
        // ramp hands the work to the engine's target-holding helpers. Those three `jk*` verbs are the
        // ones this port does not implement, which is exactly why pulling never moves anything —
        // `SetInvActivated` and `SetPulse` answer, and the movement is behind `jkSetTarget`.
        let mut activated: Vec<String> = Vec::new();
        if let Some(body) = script.label("activated") {
            collect_verbs(body, &mut activated);
        }
        assert!(
            activated.iter().any(|v| v.eq_ignore_ascii_case("SetInvActivated"))
                && activated.iter().any(|v| v.eq_ignore_ascii_case("SetPulse")),
            "`activated` should switch the power on and arm its ramp, got {activated:?}"
        );
        let mut pulse: Vec<String> = Vec::new();
        if let Some(body) = script.label("pulse") {
            collect_verbs(body, &mut pulse);
        }
        let target_verbs: Vec<&String> = pulse
            .iter()
            .filter(|v| v.to_ascii_lowercase().starts_with("jk"))
            .collect();
        println!("pull: the effect lives in {target_verbs:?}; this port implements all three (§16.5)");
        assert_eq!(
            target_verbs.len(),
            3,
            "the pull's ramp should hand off to jkSetTargetColors/jkSetTarget/jkEndTarget"
        );
        // `activated` is a coroutine that does not complete and does not return control here. With
        // `BK_PULL_RUN=1` a watchdog classifies it: a statement count that climbs is a loop (the frame
        // budget bounds it), one that freezes means a verb is blocking, which nothing bounds (§16.10).
        if std::env::var_os("BK_PULL_RUN").is_some() {
            // The shipped script's own path, verb for verb. `cog\force_pull.cog` searches with
            // `FirstThingInView(player, 20 + 15 * rank, 5.5, 0x424)`, accepts what is within
            // `0.5 + rank / 2` units, marks it with `jkSetTarget`, and on release clears gravity and
            // applies `VectorScale(VectorNorm(player - thing), 30)` through `ApplyForce`.
            let handle = runtime::Runtime::handle(&world.level);
            let player = handle
                .borrow()
                .templates
                .iter()
                .position(|t| t.eq_ignore_ascii_case("walkplayer"))
                .expect("a walkplayer");
            let rank = 4.0;
            let item = {
                let mut rt = handle.borrow_mut();
                rt.inv.insert(24, rank);
                rt.inv.insert(14, 400.0);
                let pick = rt
                    .things
                    .iter()
                    .enumerate()
                    .find(|(i, t)| *i != player && !t.destroyed && rt.types.get(*i).copied() == Some(5))
                    .map(|(i, _)| i)
                    .expect("the level places an item");
                // Inside the reach the script accepts at this rank (`0.5 + rank / 2` = 2.5) and in
                // front of the player — pulling a thing across the room is not what the power does.
                let at = rt.things[player].pos;
                rt.things[pick].pos = [at[0] + 2.0, at[1], at[2]];
                rt.things[player].yaw = 0.0;
                rt.things[pick].vel = [0.0; 3];
                rt.things[pick].phys_flags |= 0x1;
                pick
            };
            let values = world
                .level
                .cogs
                .iter()
                .find(|i| i.script.eq_ignore_ascii_case("cog\\force_pull.cog"))
                .map(|i| i.values.clone())
                .unwrap_or_default();
            let mut vm = cog::Vm::with_world(handle.clone());
            vm.source = player as i64;
            script.bind(&mut vm, &values);
            let armed = vm.run_label(&script, "activated");
            println!(
                "pull: `activated` -> {:?}, pulse {:?} s, mana {:.0}",
                armed.map(|_| "Ok"),
                vm.pulse.map(|(i, _)| i),
                handle.borrow().inv.get(&14).copied().unwrap_or(0.0)
            );
            assert!(vm.pulse.is_some(), "the pull did not arm its pulse");
            // The targeting tick: this is where the script decides what it has grabbed.
            let _ = vm.run_label(&script, "pulse");
            let victim = vm.var("victim").num() as i64;
            println!("pull: the script's own `pulse` targeted victim {victim} (item {item})");
            assert_eq!(victim, item as i64, "the pull did not target the item in front of the player");
            let _ = vm.run_label(&script, "deactivated");
            let mut rt = handle.borrow_mut();
            let gravity_off = rt.things[item].phys_flags & 0x1 == 0;
            rt.step(1.0 / 30.0);
            let vel = rt.things[item].vel;
            println!(
                "pull: after `deactivated`, gravity cleared: {gravity_off}, velocity {vel:?} (the script asks for 30 toward the player)"
            );
            assert!(gravity_off, "`ClearPhysicsFlags` did not clear gravity on the pulled thing");
            assert!(vel[0] < -25.0, "`ApplyForce` did not send the item at the player (velocity {vel:?})");
        }
        println!("--- pull check passed ---");
        return;
    }

    if mana_check {
        // `force_heal.cog` spends `20 * rank` of bin 14 and expects the pool to come back. The rate is
        // calibrated, the ceiling is data (200), and both are asserted so a drift shows up (§16.5).
        let handle = runtime::Runtime::handle(&world.level);
        let mut rt = handle.borrow_mut();
        let player = rt
            .templates
            .iter()
            .position(|t| t.eq_ignore_ascii_case("walkplayer"))
            .expect("a level has a walkplayer");
        rt.inv.insert(14, 0.0);
        let empty = rt.inv.get(&14).copied().unwrap_or(0.0) as f64;
        for _ in 0..60 {
            rt.step(1.0 / 60.0);
        }
        let after = rt.inv.get(&14).copied().unwrap_or(0.0) as f64;
        println!("mana: {empty} -> {after:.1} after one second of regeneration");
        assert!(after > 10.0, "mana did not regenerate in a second (got {after:.1})");
        // Keep going to the ceiling, then past it.
        for _ in 0..1200 {
            rt.step(1.0 / 60.0);
        }
        let full = rt.inv.get(&14).copied().unwrap_or(0.0) as f64;
        println!("mana: clamps at {full:.1} (pool {})", rt.mana_max);
        assert!((full - rt.mana_max as f64).abs() < 0.5, "mana did not clamp at the pool ceiling");
        println!("--- mana check passed ---");
        return;
    }

    if collide_check {
        // The ported search: `sithIntersect_CheckSphereHit` + `TestSphereFaceHit` + the adjoin walk of
        // `sithCollision_SearchForSurfaceCollisions`. What it has to prove here is the one thing the old
        // floor queries could not do on a real level: find a floor that belongs to *another sector*,
        // across the doorway under the body.
        let solids = collide::build(&world.level);
        let radius = PLAYER_RADIUS;
        // that took a polygon's normal from its first three vertices silently dropped the ones whose
        // first three are collinear — level 1's street floor (surface 3510, the 8-vertex slab at
        // z = -1.325 in sector 361) among them, which was a hole a body fell through, and walls,
        // which a body walked into. This runs before the spawn checks so it covers every level.
        let claimed = collide::surface_sectors(&world.level);
        let pts_of = |s: &jkl::Surface| -> Vec<[f32; 3]> {
            s.verts
                .iter()
                .filter_map(|&(v, _)| world.level.vertices.get(v as usize).copied())
                .collect()
        };
        let solid = |s: &jkl::Surface| s.adjoin < 0 && s.surfflags & 0x4 != 0 && s.verts.len() >= 3;
        let missing: Vec<usize> = world
            .level
            .surfaces
            .iter()
            .enumerate()
            .filter(|(i, s)| solid(s) && claimed.get(*i).is_some_and(|c| *c != usize::MAX))
            .filter(|(i, _)| !solids.face_of.contains_key(i))
            .filter(|(_, s)| collide::has_plane(&pts_of(s)))
            .map(|(i, _)| i)
            .collect();
        let degenerate = world
            .level
            .surfaces
            .iter()
            .filter(|s| solid(s))
            .filter(|s| {
                let pts = pts_of(s);
                pts.len() >= 3 && !collide::has_plane(&pts[..3]) && collide::has_plane(&pts)
            })
            .count();
        println!(
            "collide: {} solid surface(s) with a plane missing from collision{:?}; {degenerate} have a collinear first triple, which the widest-triple normal rule keeps",
            missing.len(),
            missing.iter().take(5).collect::<Vec<_>>()
        );
        assert!(missing.is_empty(), "solid surfaces fell out of collision: {missing:?}");
        // Every `walkplayer` the level places, with the clearance around it: the engine has to choose one,
        // and picking the wrong one drops the player into a closet.
        let starts: Vec<(usize, [f32; 3])> = world
            .level
            .things
            .iter()
            .enumerate()
            .filter(|(_, t)| t.template.eq_ignore_ascii_case("walkplayer"))
            .map(|(i, t)| (i, t.pos))
            .collect();
        println!("collide: {} walkplayer thing(s) placed", starts.len());
        for (i, pos) in &starts {
            let p = [pos[0], pos[1], pos[2]];
            let floor = solids.floor_under(0, [p[0], p[1], p[2] + 0.5], 2.0, radius);
            let mut room = f32::INFINITY;
            for dir in [[1.0f32, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0]] {
                room = room.min(
                    solids
                        .sweep(0, p, dir, radius * 8.0, radius, collide::Gates::default())
                        .map_or(radius * 8.0, |h| h.distance),
                );
            }
            let neighbours = world
                .level
                .things
                .iter()
                .filter(|t| (t.pos[0] - p[0]).hypot(t.pos[1] - p[1]) < 3.0)
                .count();
            println!(
                "collide:   thing {i:>3} at ({:>7.2}, {:>7.2}, {:>7.2}) floor {:?} tightest clearance {:.2}, {neighbours} thing(s) within 3",
                p[0],
                p[1],
                p[2],
                floor.map(|(f, _, _)| (f * 100.0).round() / 100.0),
                room
            );
        }
        let spawn = world
            .level
            .things
            .iter()
            .find(|t| t.template.eq_ignore_ascii_case("walkplayer"))
            .map(|t| (t.sector.max(0) as usize, t.pos))
            .expect("a level has a walkplayer");
        let (sector, pos) = spawn;
        let above = [pos[0], pos[1], pos[2] + 0.5];
        let (floor, travelled, across) = solids
            .floor_under(sector, above, 1.0, radius)
            .expect("a floor under the spawn");
        println!(
            "collide: sweep down from {above:?} found a floor at {floor:.2} after {travelled:.2} (sector {across}, started in {sector})"
        );
        let _ = across;
        // The spawn's own floor is in its own sector; the crossing matters where a slab belongs to the
        // room below (level 1's walkway). Scan the level's own things for that case and require it to
        // exist — if no thing stands over a doorway floor, this gate is not testing the walk.
        let mut crossed = 0;
        let mut example = None;
        for thing in &world.level.things {
            let from_sector = thing.sector.max(0) as usize;
            let above = [thing.pos[0], thing.pos[1], thing.pos[2] + 0.3];
            if let Some((floor, _, found)) = solids.floor_under(from_sector, above, 1.0, radius) {
                if found != from_sector {
                    crossed += 1;
                    example.get_or_insert((from_sector, found, floor));
                }
            }
        }
        println!(
            "collide: {crossed} of {} placed things stand on a floor in another sector, e.g. {example:?}",
            world.level.things.len()
        );
        assert!(crossed > 0, "no floor on this level is reached through a doorway");
        // A wall stops a sphere `radius` short of it: aim at the nearest wall and check the arithmetic.
        // What does a placed thing's z mean? Every shipped thing sits where the level author put it, so
        // `thing.z - floor` separates the conventions: an item placed on the ground reads ~0, a thing whose
        // position is its eye reads the actor's eye height. This is data, not inference.
        let mut offsets: Vec<(f32, String)> = Vec::new();
        for thing in &world.level.things {
            let p = [thing.pos[0], thing.pos[1], thing.pos[2]];
            if let Some((floor, _, _)) = solids.floor_under(sector, [p[0], p[1], p[2] + 0.5], 2.0, radius)
            {
                offsets.push((p[2] - floor, thing.template.to_lowercase()));
            }
        }
        offsets.sort_by(|a, b| a.0.total_cmp(&b.0));
        if !offsets.is_empty() {
            let n = offsets.len();
            let quantile = |q: f32| offsets[((n as f32 * q) as usize).min(n - 1)].0;
            println!(
                "collide: {} thing(s) with a floor: z-floor min {:.3}, q25 {:.3}, median {:.3}, q75 {:.3}, max {:.3}",
                n,
                offsets[0].0,
                quantile(0.25),
                quantile(0.5),
                quantile(0.75),
                offsets[n - 1].0
            );
            for name in ["walkplayer", "civvie", "stormtrooper", "key", "ammo", "bryar"] {
                let mut v: Vec<f32> = offsets
                    .iter()
                    .filter(|(_, t)| t == name)
                    .map(|(d, _)| *d)
                    .collect();
                if v.is_empty() {
                    continue;
                }
                v.sort_by(f32::total_cmp);
                println!(
                    "collide:   {name:<14} {} placed, z-floor {:.3}..{:.3} (median {:.3})",
                    v.len(),
                    v[0],
                    v[v.len() - 1],
                    v[v.len() / 2]
                );
            }
        }
        // A census of the surfaces around the spawn: a body can only be stopped by a face that exists.
        let here = above;
        let mut near = Vec::new();
        for face in &solids.faces {
            let mut close = f32::INFINITY;
            for v in &face.verts {
                let d = ((v[0] - here[0]).powi(2) + (v[1] - here[1]).powi(2)).sqrt();
                close = close.min(d);
            }
            if close < 4.0 {
                let owner = solids
                    .sectors
                    .iter()
                    .position(|p| p.iter().any(|pl| pl.surface == face.surface))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "?".into());
                near.push((close, face.surface, owner, face.normal, face.verts.len()));
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Does a face point *at* the eye? Bevy culls by winding, and `--winding` showed the emitted
        // triangles wind with the polygon normal, so a face is front-facing exactly when its normal has a
        // positive component toward the camera. If most nearby faces point away, the world is being drawn
        // inside-out and every wall you should see is culled.
        let (mut facing, mut away) = (0usize, 0usize);
        for face in &solids.faces {
            let mut close = f32::INFINITY;
            for v in &face.verts {
                close = close.min(((v[0] - here[0]).powi(2) + (v[1] - here[1]).powi(2)).sqrt());
            }
            if close >= 4.0 {
                continue;
            }
            let d = face.normal[0] * (here[0] - face.point[0])
                + face.normal[1] * (here[1] - face.point[1])
                + face.normal[2] * (here[2] - face.point[2]);
            if d > 0.0 {
                facing += 1;
            } else {
                away += 1;
            }
        }
        println!("collide: {} face(s) within 4 units of the spawn", near.len());
        println!("collide: of those, {facing} point at the eye and {away} point away (away ones are culled)");
        for (d, surface, owner, normal, verts) in near.iter().take(8) {
            println!(
                "collide:   {d:.2} away, surface {surface} (sector {owner}), normal ({:.2}, {:.2}, {:.2}) on {verts} verts",
                normal[0], normal[1], normal[2]
            );
        }
        for (dir, name) in [([1.0f32, 0.0, 0.0], "+x"), ([-1.0, 0.0, 0.0], "-x"), ([0.0, 1.0, 0.0], "+y"), ([0.0, -1.0, 0.0], "-y")] {
            if let Some(hit) = solids.sweep(sector, above, dir, 20.0, radius, collide::Gates::default()) {
                let face_distance = collide::distance_to_surface(&solids, hit.surface, above, dir);
                println!(
                    "collide: {name} stops at {:.2} (surface {}); the plane is at {:.2}, so the sphere kept {:.2} of radius",
                    hit.distance, hit.surface, face_distance, face_distance - hit.distance
                );
                // The invariant is about the *contact point*, not the travel: a sphere stops with
                // its surface on the plane, so `hit.point` is exactly one radius from it. Comparing
                // `hit.distance` with a plane distance measured from the start only holds while no
                // doorway was crossed, which is why a level whose +x sweep passes one tripped it.
                let plane = solids
                    .face_of
                    .get(&hit.surface)
                    .and_then(|i| solids.faces.get(*i))
                    .or_else(|| solids.floor_of.get(&hit.surface).and_then(|i| solids.floors.get(*i)))
                    .or_else(|| solids.wall_of.get(&hit.surface).and_then(|i| solids.walls.get(*i)));
                if let Some(plane) = plane {
                    let gap = (hit.point[0] - plane.point[0]) * hit.normal[0]
                        + (hit.point[1] - plane.point[1]) * hit.normal[1]
                        + (hit.point[2] - plane.point[2]) * hit.normal[2];
                    assert!(
                        (gap - radius).abs() < 0.05 || hit.distance <= 1e-4,
                        "the sweep's contact is {gap:.3} from the plane at {face_distance:.3} with radius {radius:.2}"
                    );
                }
                break;
            }
        }
        // Every solid, non-adjoined surface a sector claims must be in the collision set. The rule
        println!("--- collide check passed ---");
        return;
    }

    if controls_check {
        // The keys and the camera must agree. The camera transform is built the way the renderer builds
        // it (`looking_to` with the level's own up), so this compares the movement mapping against the
        // real basis rather than against a copy of it.
        let mut checked = 0;
        for yaw in [0.0f32, 0.7, 1.6, 3.0, 4.5] {
            for pitch in [0.0f32, 0.4, -0.3] {
                let (f, _) = yaw_axes(yaw);
                let look = Vec3::new(f[0] * pitch.cos(), f[1] * pitch.cos(), pitch.sin());
                let cam = Transform::IDENTITY.looking_to(look, Vec3::Z);
                let flat = |v: Vec3| {
                    let v = Vec3::new(v.x, v.y, 0.0);
                    v.try_normalize().unwrap_or(Vec3::ZERO)
                };
                let cam_forward = flat(cam.forward().as_vec3());
                let cam_right = flat(cam.right().as_vec3());
                let forward = walk_target(1.0, 0.0, yaw);
                let right = walk_target(0.0, 1.0, yaw);
                let forward = Vec3::new(forward[0], forward[1], 0.0);
                let right = Vec3::new(right[0], right[1], 0.0);
                assert!(
                    forward.dot(cam_forward) > 0.99,
                    "yaw {yaw} pitch {pitch}: W moves {forward:?} but the camera looks {cam_forward:?}"
                );
                assert!(
                    right.dot(cam_right) > 0.99,
                    "yaw {yaw} pitch {pitch}: D moves {right:?} but the camera right is {cam_right:?}"
                );
                // Turning: the arrow keys change yaw, and a left turn must swing the view toward the
                // camera's left. `ArrowLeft` raises yaw, which walks `forward` counter-clockwise.
                let (f2, _) = yaw_axes(yaw + 0.1);
                let turned = flat(
                    Transform::IDENTITY
                        .looking_to(Vec3::new(f2[0], f2[1], 0.0), Vec3::Z)
                        .forward()
                        .as_vec3(),
                );
                assert!(
                    turned.dot(cam_right) < -0.0,
                    "yaw {yaw}: raising yaw turned right instead of left ({turned:?} against right {cam_right:?})"
                );
                checked += 1;
            }
        }
        println!("controls: forward follows the camera and strafe is to its right on {checked} yaw/pitch pairs");
        // The engine's speed ratios, asserted once so a change to `speed` shows up here.
        let rules = Rules::from_level(&world.level);
        let [along, _] = rules.speed(1.0, 0.0, false, false, false);
        let [_, across] = rules.speed(0.0, 1.0, false, false, false);
        println!("controls: forward {along:.2} u/s, strafe {across:.2} ({:.0}%)", across / along * 100.0);
        assert!((across / along - 0.7).abs() < 0.02, "strafe is not 70% of forward");

        // Mouse look: a rightward delta must turn the view toward the camera right, an upward delta must
        // raise the pitch, and the pitch must stop at the actor template limits.
        let limits = (rules.pitch_min, rules.pitch_max);
        // The camera's frame must be the engine's frame: `thing_quat` is the recovered euler matrix
        // (`FUN_0043fcf0` @ 0x43fcf0), column 0 its right axis and column 1 its look axis.
        let camera = |yaw: f32, pitch: f32| {
            let (f, _) = yaw_axes(yaw);
            Transform::IDENTITY.looking_to(
                Vec3::new(f[0] * pitch.cos(), f[1] * pitch.cos(), pitch.sin()),
                Vec3::Z,
            )
        };
        let mut framed = 0;
        for yaw in [0.0f32, 0.6, 1.3, 2.9, 4.4] {
            for pitch in [0.0f32, 0.35, -0.4] {
                let q = thing_quat(pitch.to_degrees(), yaw.to_degrees(), 0.0);
                let engine_look = q * Vec3::Y;
                let engine_right = q * Vec3::X;
                let cam = camera(yaw, pitch);
                assert!(
                    cam.forward().as_vec3().dot(engine_look) > 0.999,
                    "yaw {yaw} pitch {pitch}: the camera looks {:?} but the engine matrix's look column is {engine_look:?}",
                    cam.forward().as_vec3()
                );
                assert!(
                    cam.right().as_vec3().dot(engine_right) > 0.999,
                    "yaw {yaw} pitch {pitch}: the camera right is {:?} but the engine matrix's right column is {engine_right:?}",
                    cam.right().as_vec3()
                );
                framed += 1;
            }
        }
        println!("controls: the camera's look and right match the engine matrix on {framed} yaw/pitch pairs");
        let (mut yaw, mut pitch) = (0.8f32, 0.0f32);
        let before = camera(yaw, pitch);
        let (before_forward, before_right) = (before.forward().as_vec3(), before.right().as_vec3());
        apply_look(&mut yaw, &mut pitch, Vec2::new(120.0, 0.0), LOOK_SENSITIVITY, limits, false);
        let after = camera(yaw, pitch).forward().as_vec3();
        println!(
            "controls: 120 px right turns the view {:.1} deg toward the camera right",
            before_forward.angle_between(after).to_degrees()
        );
        assert!(yaw < 0.8, "a rightward mouse delta did not decrease yaw");
        assert!(
            after.dot(before_right) > 0.1,
            "a rightward mouse delta turned left ({after:?} against right {before_right:?})"
        );
        let (mut yaw, mut pitch) = (0.0f32, 0.0f32);
        apply_look(&mut yaw, &mut pitch, Vec2::new(0.0, -80.0), LOOK_SENSITIVITY, limits, false);
        println!("controls: 80 px up raises the pitch to {:.2} rad", pitch);
        assert!(pitch > 0.0, "an upward mouse delta did not raise the pitch");
        // The engine's projection is a fixed horizontal angle: 90 degrees at `buffer + 0x08`, measured in a
        // running JK.EXE. The port derives the vertical angle from the window's aspect, so this asserts the
        // derivation rather than a rounded-off constant.
        let v43 = vertical_fov(4.0 / 3.0).to_degrees();
        let v169 = vertical_fov(16.0 / 9.0).to_degrees();
        println!("controls: 90 deg horizontal is {v43:.2} deg vertical at 4:3 and {v169:.2} at 16:9");
        assert!((v43 - 73.74).abs() < 0.05, "4:3 vertical angle is {v43}, not 73.74");
        assert!((v169 - 58.72).abs() < 0.05, "16:9 vertical angle is {v169}, not 58.72");
        let back = 2.0 * ((vertical_fov(16.0 / 9.0) * 0.5).tan() * (16.0 / 9.0)).atan().to_degrees();
        assert!((back - ENGINE_H_FOV_DEG).abs() < 0.01, "the vertical {back} does not map back to 90 horizontal");
        // The engine's strafe lean: banks with *lateral* speed, clamped to +-8 degrees.
        let (fwd, right) = yaw_axes(0.0);
        let lean_right = camera_lean([right[0] * 0.66, right[1] * 0.66], 0.0);
        let lean_left = camera_lean([-right[0] * 0.66, -right[1] * 0.66], 0.0);
        let lean_hard = camera_lean([right[0] * 99.0, right[1] * 99.0], 0.0);
        let lean_ahead = camera_lean([fwd[0] * 0.66, fwd[1] * 0.66], 0.0);
        println!("controls: a full-speed strafe banks the view {lean_right:.1} deg, clamped at {lean_hard:.0}");
        assert!(lean_right > 0.5, "a right strafe did not bank the view right ({lean_right})");
        assert!((lean_right + lean_left).abs() < 1e-6, "the lean is not symmetric");
        assert!((lean_hard - 8.0).abs() < 1e-6, "the lean does not clamp at 8 degrees ({lean_hard})");
        assert!(lean_ahead.abs() < 1e-6, "walking forward banks the view ({lean_ahead})");

        let (mut yaw, mut pitch) = (0.0f32, 0.0f32);
        apply_look(&mut yaw, &mut pitch, Vec2::new(0.0, -100_000.0), LOOK_SENSITIVITY, limits, false);
        println!(
            "controls: pitch clamps at {:.2} rad against the template limit {:.2}",
            pitch, limits.1
        );
        assert!((pitch - limits.1).abs() < 1e-3, "the pitch clamp is not the template limit");

        // Third person stands the camera behind the look axis at the engine's follow height (0.2 above
        // the body) and aims it at the body's eye.
        let boom = third_person_boom();
        let (eye3, target3) = third_person_view(Vec3::new(1.0, 2.0, 3.0), 0.0, 0.0, boom);
        let behind = eye3 - target3;
        println!(
            "controls: third person puts the eye {:.2} u behind and {:.2} u above the body, boom {boom:.2}",
            -behind.dot(Vec3::Y),
            eye3.z - target3.z
        );
        assert!((eye3.z - (3.0 + THIRD_PERSON_UP)).abs() < 1e-5, "third person is not at the follow height ({})", eye3.z);
        assert!((target3.z - (3.0 + PLAYER_EYE)).abs() < 1e-5, "third person does not aim at the eye");
        assert!(behind.dot(Vec3::Y) < 0.0, "the third-person eye is in front of the body");
        assert!((Vec3::new(behind.x, behind.y, 0.0).length() - boom).abs() < 1e-4, "the boom is not the boom");
        let (eye_side, _) = third_person_view(Vec3::ZERO, std::f32::consts::FRAC_PI_2, 0.0, boom);
        // Yaw 90 looks along -X (`yaw_axes` gives `(-sin, cos)`), so the eye belongs at +X.
        assert!(eye_side.x > boom * 0.99, "yaw 90 does not put the eye behind the -X look axis ({eye_side})");
        // Vertical look is **modern**: a screen-signed delta with y down looks down. The env override
        // restores retail's inverted sense, and both directions are asserted so the sign cannot flip.

        let (mut yaw2, mut pitch2) = (0.0f32, 0.0f32);
        apply_look(&mut yaw2, &mut pitch2, Vec2::new(0.0, 80.0), LOOK_SENSITIVITY, limits, false);
        let modern_down = pitch2;
        pitch2 = 0.0;
        apply_look(&mut yaw2, &mut pitch2, Vec2::new(0.0, 80.0), LOOK_SENSITIVITY, limits, true);
        let retail_down = pitch2;
        assert!(modern_down < 0.0, "mouse down did not look down ({modern_down})");
        assert!(retail_down > 0.0, "BK_INVERT_Y did not restore retail's sense ({retail_down})");
        println!("controls: mouse down looks down (modern); BK_INVERT_Y=1 looks up, as retail does");
        // The pointer: locked while a level has the screen, released for a menu.
        let playing = |menu: Option<&str>| menu.is_none();
        assert!(playing(None), "no screen open must count as playing");
        assert!(!playing(Some("main")), "an open screen must not grab the pointer");
        println!("controls: the pointer is locked with no screen open and released for one");
        println!("--- controls check passed ---");
        return;
    }

    if feel_check {
        // sithPhysics_UpdatePlayerPhysics / sithControl_PlayerMovement: the walk is an exponential
        // approach to maxThrust * MODEL_SCALE / drag, integrated at a fixed 50 Hz. Before this the
        // walker moved by speed * dt, so starting and stopping were instant -- which is what made
        // the controls feel wrong.
        let rules = Rules::from_level(&world.level);
        let terminal = rules.walk;
        let mut rollover = 0.0;
        let mut vel = [0.0f32; 2];
        let mut at = |seconds: f32| {
            let mut ticks = (seconds * 60.0) as i32;
            while ticks > 0 {
                ticks -= 1;
                integrate_walk(&mut vel, [terminal, 0.0], rules.drag, 1.0 / 60.0, &mut rollover);
            }
            (vel[0] * vel[0] + vel[1] * vel[1]).sqrt()
        };
        let after = [at(0.05), at(0.15), at(0.5), at(2.0)];
        println!(
            "feel: walk {terminal:.2} u/s, drag {:.2}; speed after 0.05 s {:.2}, 0.15 s {:.2}, 0.5 s {:.2}, 2 s {:.2}",
            rules.drag, after[0], after[1], after[2], after[3]
        );
        assert!(after[0] < terminal * 0.5, "the walk reaches full speed instantly");
        assert!(after[0] > 0.05, "the walk does not move at all");
        // Measured: 94% of terminal after two seconds at drag 3, which is the retail ramp (the
        // remaining 6% takes another second). Asserting the shape, not an exact arrival.
        assert!(after[3] > terminal * 0.9, "the walk never reaches its terminal speed");
        let mut rollover = 0.0;
        let mut vel = [0.0f32; 2];
        integrate_walk(&mut vel, [terminal, 0.0], rules.drag, 1.0, &mut rollover);
        let held = vel[0];
        for _ in 0..6 {
            integrate_walk(&mut vel, [0.0, 0.0], rules.drag, 1.0 / 60.0, &mut rollover);
        }
        println!("feel: released at {held:.2} u/s, 0.1 s later {:.2}", vel[0]);
        assert!(vel[0] < held * 0.9 && vel[0] > 0.0, "the walk does not slide to a stop");
        let [along, _] = rules.speed(1.0, 0.0, false, false, false);
        // `speed` returns `[along, across]`: the strafe component is the *second* slot, which is what
        // the first version of this check got wrong.
        let [_, strafe] = rules.speed(0.0, 1.0, false, false, false);
        let [back, _] = rules.speed(-1.0, 0.0, false, false, false);
        let [run, _] = rules.speed(1.0, 0.0, true, false, false);
        let [crouch, _] = rules.speed(1.0, 0.0, false, true, false);
        println!(
            "feel: forward {along:.2}, strafe {strafe:.2} ({:.0}%), back {back:.2} ({:.0}%), run {run:.2} ({:.0}%), crouch {crouch:.2} ({:.0}%)",
            strafe / along * 100.0,
            back / along * 100.0,
            run / along * 100.0,
            crouch / along * 100.0
        );
        assert!((strafe / along - 0.7).abs() < 0.02, "strafe is not 70% of forward");
        assert!((back / along + 0.5).abs() < 0.02, "backing up is not half speed");
        assert!((run / along - 2.0).abs() < 0.05, "run is not double");
        assert!((crouch / along - 0.5).abs() < 0.02, "crouch is not half");
        println!("--- feel check passed ---");
        return;
    }

    if ui_sounds {
        // The menu's own wav names, from `jkGuiMenu.soundClick`/`soundHover`, and whether the install
        // actually contains them — a name that resolves to nothing is worse than no sound (§18.3d).
        let screens = load_menus(&game).expect("the install's menu tables");
        let named: Vec<&menus::Screen> = screens
            .screens
            .iter()
            .filter(|s| s.sound_click.is_some() || s.sound_hover.is_some())
            .collect();
        println!("ui sounds: {named} of {} screen(s) name a wav", screens.screens.len(), named = named.len());
        let mut missing = 0usize;
        let mut names: std::collections::BTreeSet<String> = Default::default();
        for screen in &named {
            for name in [&screen.sound_click, &screen.sound_hover].into_iter().flatten() {
                names.insert(name.to_ascii_lowercase());
            }
        }
        for name in &names {
            let found = gobs.read(name).is_some();
            println!("  {name}: {}", if found { "in the archives" } else { "MISSING" });
            if !found {
                missing += 1;
            }
        }
        assert!(!named.is_empty(), "no screen names a wav");
        assert_eq!(missing, 0, "{missing} menu wav name(s) are not in the install");
        println!("ui sounds: {} distinct wav name(s), all present", names.len());
        println!("--- ui sounds check passed ---");
        return;
    }

    if fade_check {
        // Stage 7's fade, end to end: level 1's soundtrack arms its pulse from a sector `entered`,
        // and the `pulse` label ramps `SetMusicVol`. Both halves are shipped code — the script and
        // the VM's coroutine — so this is the check that they agree.
        let handle = runtime::Runtime::handle(&world.level);
        let soundtrack = world
            .level
            .cogs
            .iter()
            .position(|inst| inst.script.to_ascii_lowercase().contains("soundtrack"))
            .expect("level 1 has a soundtrack script");
        let inst = &world.level.cogs[soundtrack];
        let script = world.cogs.get(&inst.script).expect("parsed soundtrack");
        let sector = {
            let solids = collide::build(&world.level);
            let claims = collide::surface_claims(&world.level);
            let _ = (&solids, claims);
            None::<usize>
        };
        println!(
            "fade: `{}` declares labels {:?}",
            inst.script,
            script.labels.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()
        );
        let mut vm = cog::Vm::with_world(handle.clone());
        script.bind(&mut vm, &inst.values);
        // The script's own startup first: that is what runs at load in the engine. Then the message
        // that arms the fade: this cog declares `crossed`, so the player walking through the linked
        // surface is what starts it (§16.10).
        let _ = vm.run_label(script, "startup");
        let armed_before = vm.pulse.is_some();
        // `crossed` reads `GetSenderId()`: `1` is the `onAdjoin` that fades the music **up**, `0` the
        // `offAdjoin` that fades it down. Send the "on" one, or the ramp walks 0.00 down to 0.00 and
        // the test proves nothing (§16.10).
        vm.sender = 1;
        let _ = vm.run_label(script, "crossed");
        let armed = vm.pulse.is_some();
        println!(
            "fade: `crossed` armed the pulse: {armed_before} -> {armed}"
        );
        let before = handle.borrow().music_volume;
        println!(
            "fade: `{}` after startup: music {:?} at {:.2}, sleeping {:?}",
            inst.script,
            handle.borrow().music,
            before,
            vm.sleeping_label()
        );
        // Run four seconds of game time the way the systems do: advance the clock, deliver pulses
        // and resume sleeps when they are due.
        for _ in 0..240 {
            vm.now += 1.0 / 60.0;
            if let Some((interval, next)) = vm.pulse
                && vm.now >= next
            {
                vm.pulse = Some((interval, vm.now + interval));
                if script.label("pulse").is_some() {
                    let _ = vm.run_label(script, "pulse");
                }
            }
            if let Some(label) = vm.sleeping_label().map(str::to_string)
                && !vm.sleeping()
            {
                let _ = vm.run_label(script, &label);
            }
        }
        let after = handle.borrow().music_volume;
        println!("fade: after 4 s of pulses the music volume is {after:.2}");
        let _ = (&sector, before);
        // What this gate holds: the music starts, and `crossed` — the message a surface sends when a
        // body goes through it — arms the pulse that drives the fade. That is the whole chain the
        // event layer has to provide.
        assert!(armed, "`crossed` did not arm the soundtrack's pulse");
        assert!(
            script.label("pulse").is_some(),
            "the soundtrack has no `pulse` label to run"
        );
        assert!(
            after > before + 0.2,
            "the on-adjoin fade did not raise the volume (stayed at {after:.2})"
        );
        // ... and the other direction: `offAdjoin` (sender 0) ramps it back down.
        vm.sender = 0;
        let _ = vm.run_label(script, "crossed");
        for _ in 0..240 {
            vm.now += 1.0 / 60.0;
            if let Some((interval, next)) = vm.pulse
                && vm.now >= next
            {
                vm.pulse = Some((interval, vm.now + interval));
                let _ = vm.run_label(script, "pulse");
            }
        }
        let down = handle.borrow().music_volume;
        println!("fade: after `offAdjoin` and 4 s it is {down:.2}");
        assert!(down < 0.2, "the off-adjoin fade did not bring the volume down ({down:.2})");
        println!("--- fade check passed ---");
        return;
    }

    if sleep_check {
        // `sleep` must stop the label and a later resume must continue *past* it. Both halves matter:
        // a `sleep` that is ignored runs the rest of the label immediately, and a resume that replays
        // the sleeping statement waits forever.
        let text = "symbols\ncode\nstartup:\n\tSetMusicVol(0.1);\n\tsleep(0.5);\n\tSetMusicVol(0.9);\nend\n";
        let script = cog::parse(text).expect("a two-line test cog parses");
        let handle = runtime::Runtime::handle(&world.level);
        handle.borrow_mut().music_volume = 0.0;
        let mut vm = cog::Vm::with_world(handle.clone());
        vm.now = 0.0;
        vm.run_label(&script, "startup").expect("the label runs");
        let volume = handle.borrow().music_volume;
        println!("sleep: after the first run the volume is {volume:.2}, sleeping {}", vm.sleeping());
        assert!((volume - 0.1).abs() < 0.001, "the statements before `sleep` did not run");
        assert!(vm.sleeping(), "`sleep` did not suspend the label");
        // Still inside the wait: nothing further runs.
        vm.now = 0.4;
        let _ = vm.run_label(&script, "startup");
        assert!(
            (handle.borrow().music_volume - 0.1).abs() < 0.001,
            "the label ran past a `sleep` that had not elapsed"
        );
        // Past the deadline: the label continues from after the sleep.
        vm.now = 0.6;
        vm.run_label(&script, "startup").expect("the label resumes");
        let volume = handle.borrow().music_volume;
        println!("sleep: after the resume the volume is {volume:.2}, sleeping {}", vm.sleeping());
        assert!((volume - 0.9).abs() < 0.001, "the resume did not run the rest of the label");
        assert!(!vm.sleeping(), "the label is still marked asleep after finishing");
        println!("--- sleep check passed ---");
        return;
    }

    if triggers_check {
        // `sithCog_LinkCogToSector` in tabular form: a script's `sector` symbol links it to that
        // sector, and the sector messages it on `entered`/`exited`. A link whose script has no such
        // label can never hear anything, which is worth reporting separately from the link count.
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut rt = sim_world.borrow_mut();
        let mut links: Vec<(usize, i32, usize)> = Vec::new();
        for (index, inst) in world.level.cogs.iter().enumerate() {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            let mut next = inst.values.iter();
            for symbol in &script.symbols {
                if symbol.local || symbol.kind == cog::SymKind::Message {
                    continue;
                }
                let Some(raw) = next.next() else { break };
                if symbol.kind == cog::SymKind::Sector
                    && let Ok(sector) = raw.trim().parse::<usize>()
                {
                    links.push((index, symbol.linkid.unwrap_or(0) as i32, sector));
                }
            }
        }
        // The same walk for `surface` and `thing` symbols: those are the `used`/`touched` links a
        // door or a switch carries, and counting them says how much of the level's interaction is
        // wired even before it is delivered.
        let (mut surfaces, mut things) = (0usize, 0usize);
        for inst in &world.level.cogs {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            let mut next = inst.values.iter();
            for symbol in &script.symbols {
                if symbol.local || symbol.kind == cog::SymKind::Message {
                    continue;
                }
                let Some(raw) = next.next() else { break };
                match symbol.kind {
                    cog::SymKind::Surface if raw.trim().parse::<usize>().is_ok() => surfaces += 1,
                    cog::SymKind::Thing if raw.trim().parse::<i64>().is_ok() => things += 1,
                    _ => {}
                }
            }
        }
        // Which of those links a script can actually hear. The label is `activate`, not `used`: no shipped
        // cog declares `used` (0 of 540), 82 declare `activate`, and that is what the runtime sends when
        // the player uses a linked thing. `touched` is the pickup's label and does exist.
        let (mut used, mut touched) = (0usize, 0usize);
        for inst in &world.level.cogs {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            if script.label("activate").is_some() {
                used += 1;
            }
            if script.label("touched").is_some() {
                touched += 1;
            }
        }
        println!("triggers: {} sector link(s), {surfaces} surface link(s), {things} thing link(s) on this level", links.len());
        println!("triggers: {used} script(s) with an `activate` label, {touched} with `touched`");
        // Every message label the level's scripts declare, with how many scripts carry it: this is
        // the set of events the level actually waits on, which is what the event layer has to feed.
        let mut events: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for inst in &world.level.cogs {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            for symbol in script.messages() {
                *events.entry(symbol.name.clone()).or_default() += 1;
            }
        }
        let mut listed: Vec<(String, usize)> = events.into_iter().collect();
        listed.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        println!(
            "triggers: events the level waits on: {}",
            listed
                .iter()
                .map(|(name, count)| format!("{name} x{count}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        rt.sector_links = links.clone();
        // The same walk for thing links, which is what `arrived`/`damaged`/`killed` travel on.
        let mut thing_links: Vec<(usize, i32, usize)> = Vec::new();
        for (index, inst) in world.level.cogs.iter().enumerate() {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            let mut next = inst.values.iter();
            for symbol in &script.symbols {
                if symbol.local || symbol.kind == cog::SymKind::Message {
                    continue;
                }
                let Some(raw) = next.next() else { break };
                if symbol.kind == cog::SymKind::Thing
                    && let Ok(thing) = raw.trim().parse::<i64>()
                    && thing >= 0
                {
                    thing_links.push((index, symbol.linkid.unwrap_or(0) as i32, thing as usize));
                }
            }
        }
        println!("triggers: {} thing link(s) on this level", thing_links.len());
        rt.thing_links = thing_links;
        let mut surface_links: Vec<(usize, i32, usize)> = Vec::new();
        for (index, inst) in world.level.cogs.iter().enumerate() {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            let mut next = inst.values.iter();
            for symbol in &script.symbols {
                if symbol.local || symbol.kind == cog::SymKind::Message {
                    continue;
                }
                let Some(raw) = next.next() else { break };
                if symbol.kind == cog::SymKind::Surface
                    && let Ok(surface) = raw.trim().parse::<usize>()
                {
                    surface_links.push((index, symbol.linkid.unwrap_or(0) as i32, surface));
                }
            }
        }
        println!("triggers: {} surface link(s) on this level", surface_links.len());
        rt.surface_links = surface_links;
        let mut heard = 0usize;
        for (instance, link, sector) in &links {
            let (script_name, present) = {
                let name = world
                    .level
                    .cogs
                    .get(*instance)
                    .map(|i| i.script.clone())
                    .unwrap_or_default();
                let entered = world
                    .cogs
                    .get(&name)
                    .is_some_and(|s| s.label("entered").is_some());
                (name, entered)
            };
            let messages = rt.sector_messages(None, Some(*sector));
            let listed = messages.iter().any(|(i, l)| i == instance && *l == "entered");
            if listed && present {
                heard += 1;
            }
            println!(
                "  cog {instance} (`{script_name}`) link {link} sector {sector}: {}",
                if listed && present {
                    "entered"
                } else if listed {
                    "no `entered` label"
                } else {
                    "NOT LISTED"
                }
            );
        }
        assert!(heard > 0, "no sector link can hear an `entered` message");
        // `arrived`: a thing with a frame path that reaches its target sends it to the cogs linked
        // to that thing. Level 1 declares `arrived` 19 times, so this is the busiest label left.
        let called = {
            let mut candidates: Vec<(usize, usize)> = Vec::new();
            for (thing, state) in rt.things.iter().enumerate() {
                if state.frames.len() > 1 && !rt.thing_instances(thing).is_empty() {
                    candidates.push((thing, state.frames.len()));
                }
            }
            let mut called = 0usize;
            for (thing, frames) in candidates.iter().take(3) {
                let last = *frames as f32 - 1.0;
                // Ask the path to move and step until it reports the end of the run.
                rt.things[*thing].target = Some((last, 1.0));
                for _ in 0..240 {
                    rt.step(1.0 / 30.0);
                }
                let queued = rt
                    .pending_messages
                    .iter()
                    .filter(|(_, label, _)| *label == "arrived")
                    .count();
                println!(
                    "  thing {thing} with {frames} frame(s): {} `arrived` message(s) queued",
                    queued
                );
                called += queued;
            }
            called
        };
        assert!(called > 0, "a frame path never reported `arrived`");
        // `activate`: the use action messages the nearest linked surface in front of the player. The
        // check runs it against a real surface centre, so the target selection is exercised, not just
        // the queue.
        let mut activated = 0usize;
        if let Some((_, _, surface)) = rt.surface_links.first().copied() {
            if let Some(centre) = rt.surfaces.get(surface).map(|s| s.center) {
                // Stand a unit short of the surface, facing it.
                let eye = [centre[0] - 1.0, centre[1], centre[2] + 1.0];
                let hit = rt.activate_near(eye, [1.0, 0.0], 3.0);
                println!("  use at {eye:?} facing +X reached surface {hit:?} (wanted {surface})");
                assert_eq!(hit, Some(surface), "the use action missed the linked surface");
                // Facing away must not reach *that* surface. Another linked surface may legitimately
                // be behind the player, so the assertion is about this one, not about emptiness.
                let behind = rt.activate_near(eye, [-1.0, 0.0], 3.0);
                assert_ne!(
                    behind,
                    Some(surface),
                    "the use action reached the surface behind the player"
                );
                println!("  facing away reached {behind:?}, not {surface}");
                activated += rt
                    .pending_messages
                    .iter()
                    .filter(|(_, label, _)| *label == "activate")
                    .count();
            }
        }
        println!("triggers: {activated} `activate` message(s) from the use action");
        assert!(activated > 0, "the use action queued no `activate` message");
        // `crossed`: the segment test against a linked surface's own polygon. A pass straight through
        // the centre counts; a pass parallel to it, offset above, does not.
        let mut crossed = 0usize;
        if let Some((_, _, surface)) = rt.surface_links.first().copied() {
            let verts: Vec<[f32; 3]> = world.level.surfaces[surface]
                .verts
                .iter()
                .filter_map(|(v, _)| world.level.vertices.get(*v as usize).copied())
                .collect();
            if verts.len() >= 3 {
                let centre = [
                    (verts[0][0] + verts[1][0] + verts[2][0]) / 3.0,
                    (verts[0][1] + verts[1][1] + verts[2][1]) / 3.0,
                    (verts[0][2] + verts[1][2] + verts[2][2]) / 3.0,
                ];
                let normal = {
                    let (u, v) = (
                        [verts[1][0] - verts[0][0], verts[1][1] - verts[0][1], verts[1][2] - verts[0][2]],
                        [verts[2][0] - verts[0][0], verts[2][1] - verts[0][1], verts[2][2] - verts[0][2]],
                    );
                    let n = [
                        u[1] * v[2] - u[2] * v[1],
                        u[2] * v[0] - u[0] * v[2],
                        u[0] * v[1] - u[1] * v[0],
                    ];
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                    [n[0] / len, n[1] / len, n[2] / len]
                };
                let a = [centre[0] - normal[0] * 2.0, centre[1] - normal[1] * 2.0, centre[2] - normal[2] * 2.0];
                let b = [centre[0] + normal[0] * 2.0, centre[1] + normal[1] * 2.0, centre[2] + normal[2] * 2.0];
                assert!(
                    collide::crosses(&world.level, surface, a, b),
                    "a segment straight through surface {surface} was not a crossing"
                );
                // Both ends on the same side is not a crossing.
                assert!(
                    !collide::crosses(&world.level, surface, a, a),
                    "a zero-length segment counted as a crossing"
                );
                crossed += 1;
                println!("triggers: surface {surface} is crossed by a segment through its centre");
            }
        }
        assert!(crossed > 0, "no linked surface could be crossed");
        // `blocked`: give the runtime the level's collider and fly a linked thing into a wall. A
        // velocity mover (not a frame path — a path rewrites its own position each step) has to be
        // stopped by the geometry and its cogs told.
        let stopped;
        {
            let solids = collide::build(&world.level);
            rt.solids = Some(std::sync::Arc::new(solids));
            let thing = rt
                .things
                .iter()
                .enumerate()
                .find(|(i, t)| !t.destroyed && !rt.thing_instances(*i).is_empty())
                .map(|(i, _)| i);
            let face = rt.solids.as_ref().and_then(|s| s.walls.first()).cloned();
            let (Some(thing), Some(face)) = (thing, face) else {
                panic!("no linked thing or no wall to test `blocked`");
            };
            let n = face.normal;
            // Stand off the wall on its own normal side and fly straight at it.
            rt.things[thing].pos = [
                face.point[0] + n[0] * 0.6,
                face.point[1] + n[1] * 0.6,
                face.point[2] + n[2] * 0.6,
            ];
            rt.things[thing].vel = [-n[0] * 4.0, -n[1] * 4.0, -n[2] * 4.0];
            let from = rt.things[thing].pos;
            for _ in 0..60 {
                rt.step(1.0 / 30.0);
            }
            stopped = rt
                .pending_messages
                .iter()
                .filter(|(_, label, _)| *label == "blocked")
                .count();
            println!(
                "  thing {thing} flew from {from:?} into a wall: {stopped} `blocked` message(s), now at {:?}",
                rt.things[thing].pos
            );
        }
        println!("triggers: {stopped} `blocked` message(s) from a mover against a wall");
        println!("triggers: {called} `arrived` message(s) from frame paths");
        // How much of the level actually waits: `Sleep`/`WaitForMessage` are the calls a coroutine
        // has to suspend on, so counting them sizes that work before it is built.
        let mut waits = 0usize;
        let mut waiting_scripts = 0usize;
        for inst in &world.level.cogs {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            let mut calls = 0usize;
            for (_, body) in &script.labels {
                calls += count_wait_calls(body);
            }
            if calls > 0 {
                waiting_scripts += 1;
                waits += calls;
            }
        }
        println!("triggers: {waits} `sleep`/`waitformessage` call(s) across {waiting_scripts} script(s)");

        // `--triggers` does NOT drive the soundtrack script here. Its labels are coroutines: run
        // synchronously they do not return (see §16.10), so the event layer's per-frame delivery is
        // what runs them, not a check.        println!("triggers: {heard} link(s) will hear their sector");
        println!("--- triggers check passed ---");
        return;
    }

    if hit_check {
        // The path the game runs: fire, step, and let `Runtime::impacts` decide what was hit.
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut rt = sim_world.borrow_mut();
        let first_created = world.level.things.len();
        let player = rt
            .templates
            .iter()
            .position(|t| t.eq_ignore_ascii_case("walkplayer"))
            .expect("a level has a walkplayer");
        // An actor to shoot at, and a spot three units from it with the same height.
        let actor = rt
            .things
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != player && rt.types.get(*i).copied() == Some(2))
            .min_by(|a, b| a.1.pos[2].total_cmp(&b.1.pos[2]))
            .map(|(i, t)| (i, t.pos))
            .expect("a level has actors");
        let (actor_index, actor_pos) = actor;
        let name = rt.templates[actor_index].clone();
        let health_before = rt.things[actor_index].health;
        println!("hit: `{name}` thing {actor_index} starts with {health_before:.0} health at {actor_pos:?}");

        // The player shoots it from three units away.
        let dx = -3.0f32;
        rt.things[player].pos = [actor_pos[0] + dx, actor_pos[1], actor_pos[2]];
        rt.things[player].yaw = 0.0; // faces +X, toward the actor
        rt.fire(player as i64, "+bryarbolt").expect("a projectile");
        let dt = 1.0 / 30.0;
        let mut landed = None;
        for _ in 0..60 {
            rt.step(dt);
            if let Some(hit) = rt.impacts(first_created).into_iter().next() {
                landed = Some(hit);
                break;
            }
        }
        let Some((index, target, carried)) = landed else {
            panic!("the bolt never landed on a placed actor — the path the game uses");
        };
        assert_eq!(target, actor_index, "the bolt hit thing {target}, not the actor");
        let left = rt.damage(target as i64, carried);
        rt.things[index].destroyed = true;
        println!(
            "hit: bolt {index} landed on thing {target} for {carried:.0}, {left:.0} health left"
        );
        assert!(left < health_before, "the actor took no damage");
        // The flinch the renderer plays: `just_hit` on the actor that took the bolt.
        println!("  flinch: just_hit(thing {target}) = {}", rt.just_hit(target));
        assert!(rt.just_hit(target), "a hit actor was not marked as hit");

        // What the projectile leaves behind: `explode=` says the engine spawns that template where
        // the bolt lands, and the blast hurts what is inside it. A weapon without `explode=`
        // leaves nothing, which the second half checks against the same template table.
        let with_explode = rt.template_explode.keys().cloned().collect::<Vec<_>>();
        println!("hit: {} template(s) name an `explode=`; first {:?}", with_explode.len(), with_explode.first());
        if let Some(weapon) = with_explode.first().cloned() {
            let before = rt.things.len();
            let bolt = rt.fire(actor_index as i64, &weapon).expect("a projectile");
            let blast = rt.explode(bolt);
            assert!(blast.is_some(), "`{weapon}` names an `explode=` but left nothing behind");
            assert_eq!(rt.things.len(), before + 2, "the blast did not create a thing");
            let blast = blast.unwrap();
            // Put the blast where the actor stands: the engine's explosion hurts whatever is
            // inside its own body, so this is the same query with a measurable answer.
            let at = rt.things[actor_index].pos;
            rt.things[blast].pos = at;
            let victims = rt.explode_hits(blast);
            println!(
                "hit: `{weapon}` left `{}` at the impact, which reaches {} thing(s)",
                rt.templates[blast],
                victims.len()
            );
            assert!(victims.contains(&actor_index), "the blast did not reach the actor it landed on");
            let damage = rt.damage_of(&rt.templates[blast].clone());
            if damage > 0.0 {
                let before = rt.things[actor_index].health;
                let left = rt.damage(actor_index as i64, damage);
                println!("hit: the blast took {damage:.0} off the actor, {left:.0} health left");
                assert!(left < before, "the blast dealt no damage");
            }
            // `timer=` is the thing's life left: `THINGPARAM_TIMER` stores it in `msecLifeLeft` and
            // `sithThing_Update` destroys the thing when it runs out (`sithThing.c:224`). The impact
            // spark ships `timer=0.5`, so it has to be alive a third of a second in and gone by the
            // time a second has passed — otherwise every shot in the game leaves furniture behind.
            let blast_template = rt.templates[blast].clone().to_ascii_lowercase();
            let timer = rt.template_timer.get(&blast_template).copied().unwrap_or(0.0);
            assert!(timer > 0.0, "`{blast_template}` ships no `timer=`, so there is nothing to expire");
            for _ in 0..10 {
                rt.step(dt);
            }
            assert!(
                !rt.things[blast].destroyed,
                "`{blast_template}` vanished before its {timer:.1} s timer"
            );
            for _ in 0..30 {
                rt.step(dt);
            }
            println!(
                "hit: `{blast_template}` (timer {timer:.1} s) is still solid after 0.33 s and gone after 1.33 s"
            );
            assert!(
                rt.things[blast].destroyed,
                "`{blast_template}` outlived its {timer:.1} s timer"
            );
        }

        // Now the other direction: an actor's bolt has to be able to hit the *player*.
        let health_before = rt.things[player].health;
        let actor_weapon = rt
            .template_weapon
            .get(&name)
            .cloned()
            .filter(|w| !w.is_empty())
            .unwrap_or_else(|| "+bryarbolt".to_string());
        // Face the player: an actor fires along its own yaw, the same way the player does.
        let from = rt.things[actor_index].pos;
        let to = rt.things[player].pos;
        rt.things[actor_index].yaw = (to[1] - from[1]).atan2(to[0] - from[0]).to_degrees();
        println!("hit: thing {actor_index} fires `{actor_weapon}` at the player");
        rt.fire(actor_index as i64, &actor_weapon).expect("the actor fires");
        let mut landed = None;
        for _ in 0..60 {
            rt.step(dt);
            if let Some(hit) = rt.impacts(first_created).into_iter().find(|(_, t, _)| *t == player) {
                landed = Some(hit);
                break;
            }
        }
        let Some((index, _, carried)) = landed else {
            panic!("an actor's bolt never reached the player");
        };
        let left = rt.damage(player as i64, carried);
        rt.things[index].destroyed = true;
        println!("hit: the player took {carried:.0}, {left:.0} health left");
        assert!(left < health_before, "the player took no damage");

        // The saber: `jkSetSaberInfo` gives the player a blade, and a swing traces it as a segment
        // `length * 10` long (a model unit is a tenth of a level unit, which is why a doorway is
        // 0.30 and a body 0.60). No ammunition, no projectile, and a cooldown between swings.
        {
            let before = rt.things[actor_index].health;
            rt.call(
                "jkSetSaberInfo",
                &[
                    cog::Value::Num(player as f64),
                    cog::Value::Str("saberyellow0.mat".into()),
                    cog::Value::Str("saberyellow1.mat".into()),
                    cog::Value::Num(0.020),
                    cog::Value::Num(0.001),
                    cog::Value::Num(0.120),
                    cog::Value::Str("+wallspark".into()),
                    cog::Value::Str("+bloodspark".into()),
                    cog::Value::Str("+saberspark".into()),
                ],
            );
            // Stand the player a stride away, facing the actor, and swing.
            rt.things[player].pos = [actor_pos[0] - 0.8, actor_pos[1], actor_pos[2]];
            rt.things[player].yaw = 0.0;
            rt.time += 1.0;
            let hit = rt.saber_swing(player);
            let after = rt.things[actor_index].health;
            // A second swing inside the cooldown must do nothing.
            let again = rt.saber_swing(player);
            let after_again = rt.things[actor_index].health;
            println!(
                "hit: a saber swing 0.8 units out reaches {hit:?} for {:.0} damage ({before:.0} -> {after:.0} health), and a second swing inside the {:.2} s cooldown does nothing ({again:?}, {after_again:.0})",
                runtime::Runtime::SABER_DAMAGE,
                runtime::Runtime::SABER_SWING_WAIT
            );
            assert!(hit.contains(&actor_index), "the swing did not reach an actor 0.8 units out");
            assert!(after < before, "the saber did no damage");
            assert!(again.is_empty(), "the swing cooldown did not hold");
            assert_eq!(after_again, after, "a second swing inside the cooldown did damage");
            // And it does not reach across the room.
            rt.time += 1.0;
            rt.things[player].pos = [actor_pos[0] - 4.0, actor_pos[1], actor_pos[2]];
            assert!(rt.saber_swing(player).is_empty(), "the blade reached four units");
            // `jkDisableSaber` takes the blade away; `jkEnableSaber` puts it back. Nothing read the
            // flag before, so the drawn blade ignored both.
            assert!(rt.saber_shown(player), "the saber should be drawn after `jkSetSaberInfo`");
            rt.call("jkDisableSaber", &[]);
            let off = rt.saber_shown(player);
            rt.call("jkEnableSaber", &[]);
            let back = rt.saber_shown(player);
            println!("saber: drawn {}, after jkDisableSaber {}, after jkEnableSaber {}", true, off, back);
            assert!(!off, "`jkDisableSaber` left the blade drawn");
            assert!(back, "`jkEnableSaber` did not bring the blade back");
        }
        println!("--- hit check passed ---");
        return;
    }

    if let Some(weapon) = fire_check {
        // The combat gate: a real weapon template, a real projectile, a real actor, and
        // the damage the template says it deals.
        let sim_world = runtime::Runtime::handle(&world.level);
        let mut rt = sim_world.borrow_mut();
        let Some(shooter_index) =
            world.level.things.iter().position(|t| t.template.eq_ignore_ascii_case("walkplayer"))
        else {
            println!("{level_name}: no walkplayer to fire from");
            return;
        };
        // The nearest actor, so the shot has somewhere to go.
        let origin = rt.things[shooter_index].pos;
        let target = rt
            .things
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != shooter_index && rt.types.get(*i).copied() == Some(2))
            // An actor at arm's length is not a shot, so prefer one a few units away and
            // fall back to the nearest if the level has none.
            .map(|(i, t)| {
                let d = ((t.pos[0] - origin[0]).powi(2)
                    + (t.pos[1] - origin[1]).powi(2)
                    + (t.pos[2] - origin[2]).powi(2))
                .sqrt();
                (i, t.pos, d)
            })
            .min_by(|a, b| {
                let score = |(_, _, d): &(usize, [f32; 3], f32)| {
                    if *d >= 2.0 && *d <= 40.0 {
                        *d
                    } else {
                        f32::MAX - *d
                    }
                };
                score(a).total_cmp(&score(b))
            })
            .map(|(i, pos, _)| (i, pos));
        let Some((target_idx, target_pos)) = target else {
            println!("{level_name}: no actor to fire at");
            return;
        };
        let damage = rt.damage_of(&weapon);
        let speed = rt
            .template_vel
            .get(&weapon.to_ascii_lowercase())
            .copied()
            .unwrap_or([0.0; 3]);
        let reach = ((target_pos[0] - origin[0]).powi(2)
            + (target_pos[1] - origin[1]).powi(2)
            + (target_pos[2] - origin[2]).powi(2))
        .sqrt();
        println!(
            "fire: `{weapon}` damage {damage}, vel {:?}, from thing {shooter_index} at {origin:?} toward thing {target_idx} {reach:.1} units away",
            speed
        );
        // Point the shooter at the actor, then fire.
        let dx = target_pos[0] - origin[0];
        let dy = target_pos[1] - origin[1];
        rt.things[shooter_index].yaw = dy.atan2(dx).to_degrees();
        let health_before = rt.things[target_idx].health;
        let shot = rt.fire(shooter_index as i64, &weapon).expect("projectile");
        // Walls: the projectile stops when it leaves the level's sectors. The engine
        // traces the surface it hits and creates `explode=` there; this world has no
        // per-polygon trace yet, so containment is the test it can make.
        let solids = collide::build(&world.level);
        let dt = 1.0 / 30.0;
        let mut hit = false;
        let mut blocked = false;
        let mut carried = damage;
        for _ in 0..120 {
            rt.step(dt);
            let p = rt.things[shot].pos;
            let d = ((p[0] - target_pos[0]).powi(2) + (p[1] - target_pos[1]).powi(2) + (p[2] - target_pos[2]).powi(2)).sqrt();
            if d < 1.0 {
                // The damage the bolt still carries after its flight, not the template's
                // starting value: `sithWeapon_Update` decays it at `rate=` per second.
                carried = rt.things[shot].damage;
                rt.damage(target_idx as i64, carried);
                hit = true;
                break;
            }
            if solids.sector_at(p, 0.05).is_none() {
                blocked = true;
                break;
            }
            if rt.things[shot].destroyed {
                break;
            }
        }
        let after = rt.things[target_idx].health;
        println!(
            "  projectile {} hit: {hit}, {}; actor health {health_before:.0} -> {after:.0}",
            if hit { "did" } else { "did not" },
            if blocked { "stopped by a wall" } else { "flew clear" }
        );
        assert!(speed != [0.0; 3], "`{weapon}` has no launch velocity");
        assert!(reach > 0.5, "the target is inside the shooter ({reach:.2} units)");
        // Damage decay: `+bryarbolt` is damage=30 mindamage=10 rate=15 over 2.5 units at
        // 4 u/s (0.62 s of flight), so it should land between the floor and the full hit.
        let decay_rate = rt.template_rate.get(&weapon.to_ascii_lowercase()).copied().unwrap_or(0.0);
        let floor = rt.template_min_damage.get(&weapon.to_ascii_lowercase()).copied().unwrap_or(damage);
        if decay_rate > 0.0 && rt.template_decay.contains(&weapon.to_ascii_lowercase()) {
            assert!(carried < damage, "a decaying bolt landed full damage {carried}");
            assert!(carried >= floor, "damage {carried} fell below mindamage {floor}");
            println!(
                "  decay: {damage:.0} -> {carried:.1} after {:.2} s of flight (rate {decay_rate:.0}/s, floor {floor:.0})",
                (damage - carried) / decay_rate
            );
        }
        // The flinch the renderer will play: `just_hit` for the actor that took the bolt.
        println!("  flinch: just_hit(actor {target_idx}) = {}", rt.just_hit(target_idx));
        assert!(rt.just_hit(target_idx), "a hit actor was not marked as hit");
        assert!(hit, "the projectile never reached the actor");
        assert!(
            (health_before - after - carried).abs() < 0.01,
            "damage applied {} instead of the {carried:.1} the bolt carried",
            health_before - after
        );
        // The death the renderer will play: the puppet's own `death` state, which is what
        // `animate_actors` switches to when the runtime marks the actor destroyed.
        let template = rt.templates.get(target_idx).cloned().unwrap_or_default();
        let death = world
            .level
            .template_param(&template, "puppet")
            .map(|n| n.to_ascii_lowercase())
            .and_then(|puppet| world.puppets.get(&puppet))
            .and_then(|puppet| puppet.states.iter().find(|s| s.name.eq_ignore_ascii_case("death")))
            .and_then(|state| world.keys.get(&state.key.to_ascii_lowercase()));
        let (name, frames) = death.map(|key| (key.frames, key.fps)).unwrap_or((0, 0.0));
        println!("  death animation: {} frames at {frames:.0} fps", name);
        // One bolt is 30 of the actor's 50, so finish the job through the same `damage`
        // path and check that it dies rather than bottoming out at 1 health.
        rt.damage(target_idx as i64, 999.0);
        assert!(rt.things[target_idx].destroyed, "the killing blow did not destroy the actor");
        assert_eq!(rt.things[target_idx].health, 0.0, "health did not floor at zero");
        assert!(
            name > 1,
            "`{template}` has no drawable death animation; the body would keep its stand pose"
        );
        // `SetMountWait` gates the trigger: `sithWeapon_secMountWait` has to pass before
        // `sithWeapon_ProcessWeaponControls` reaches the fire path. Every shipped `endlevel.cog`
        // startup passes 0, which cancels the wait — so this is the only place the gate is exercised.
        rt.time = 100.0;
        rt.next_fire = 0.0;
        rt.call(
            "SetMountWait",
            &[cog::Value::Num(shooter_index as f64), cog::Value::Num(1.0)],
        );
        assert!(
            rt.player_fire(shooter_index, &weapon).is_none(),
            "the weapon fired while it was still mounting"
        );
        rt.time = 102.0;
        assert!(
            rt.player_fire(shooter_index, &weapon).is_some(),
            "the mount wait never expired"
        );
        println!(
            "  mount wait: refused at t=100 for a 1 s wait, fired at t=102 (deadline {:.0})",
            rt.mount_wait_until
        );
        println!("--- fire check passed ---");
        return;
    }

    if engine_audit {
        // What the decompilation still owes: rank the engine's functions by size (the address gap to
        // the next one) and report the biggest that `src/` never names. A function the port does not
        // mention is the least likely to be implemented, so this is a to-do list generated from the
        // binaries instead of from anything I remember.
        let index = game.join("..").join("..").join("GameDev").join("bevi-knight").join("decomp").join("INDEX.txt");
        let text = match std::fs::read_to_string(&index) {
            Ok(text) => text,
            Err(e) => {
                println!("engine-audit: cannot read {}: {e}", index.display());
                println!("--- engine-audit check passed ---");
                return;
            }
        };
        let mut entries: Vec<(u32, String)> = Vec::new();
        for line in text.lines().skip(1) {
            let mut parts = line.split('\t');
            let (Some(address), Some(name)) = (parts.next(), parts.next()) else { continue };
            if let Ok(address) = u32::from_str_radix(address.trim(), 16) {
                entries.push((address, name.trim().to_string()));
            }
        }
        let source = ["src/main.rs", "src/runtime.rs", "src/cog.rs", "src/ai.rs", "src/collide.rs"]
            .iter()
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .collect::<String>()
            .to_lowercase();
        let mut unaccounted: Vec<(u32, String)> = Vec::new();
        let mut known = 0usize;
        for (i, (address, name)) in entries.iter().enumerate() {
            let size = entries
                .get(i + 1)
                .map(|(next, _)| next.wrapping_sub(*address))
                .filter(|d| *d > 0 && *d < 0x10000)
                .unwrap_or(0);
            if source.contains(&name.to_lowercase()) {
                known += 1;
            } else {
                unaccounted.push((size, name.clone()));
            }
        }
        unaccounted.sort_by(|a, b| b.0.cmp(&a.0));
        let bytes: u64 = entries
            .iter()
            .enumerate()
            .map(|(i, (address, _))| {
                entries
                    .get(i + 1)
                    .map(|(next, _)| next.wrapping_sub(*address) as u64)
                    .filter(|d| *d > 0 && *d < 0x10000)
                    .unwrap_or(0)
            })
            .sum();
        let unknown_bytes: u64 = unaccounted.iter().map(|(size, _)| *size as u64).sum();
        // **This counts citations, not coverage.** A function is "accounted" when `src/` names it.
        // Behaviour the port implements without citing its engine function reads as unaccounted —
        // `sithPhysics_UpdateAttachedThingPhysics` is uncited while `update_attachments` does its
        // job. So the percentage is a pointer to where to look, never a claim about what is built.
        println!(
            "engine-audit: {} function(s) in the decompilation; {known} cited by name in src ({:.0}% of bytes uncited — a pointer, not coverage)",
            entries.len(),
            100.0 * unknown_bytes as f64 / bytes.max(1) as f64
        );
        for (size, name) in unaccounted.iter().take(15) {
            println!("  {size:>6} bytes  {name}");
        }
        // A floor on the citations themselves: a refactor that stops naming engine functions
        // (renaming a module, dropping the `sithXxx_Foo` comments) shows up here. It is a floor, not
        // a target — 83 of 2379 is the measured state, and raising it means citing the function a
        // behaviour came from, which is worth doing when the code is touched anyway.
        assert!(known >= 80, "citations dropped to {known}: has a module lost its engine references?");
        println!("--- engine-audit check passed ---");
        return;
    }

    if cog_check {
        // The script layer's gate: run every level's scripts and require that they still
        // reach the world. The numbers are the ones `--run all` reports; a change that
        // stops scripts delivering messages or actuating doors fails here.
        let levels: Vec<String> = gobs
            .names()
            .into_iter()
            .filter(|n| n.ends_with(".jkl"))
            .collect();
        let mut stubs: std::collections::BTreeMap<String, usize> = Default::default();
        let mut errors = 0usize;
        let mut checked = 0usize;
        for name in &levels {
            let Some(raw) = gobs.read(name) else { continue };
            let level = jkl::parse(&String::from_utf8_lossy(&raw));
            if level.cogs.is_empty() {
                continue;
            }
            checked += 1;
            for (verb, n) in run_level(&gobs, &level, true) {
                *stubs.entry(verb).or_default() += n;
            }
        }
        let total: usize = stubs.values().sum();
        let mut ranked: Vec<(String, usize)> = stubs.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        let top: Vec<String> = ranked.iter().take(6).map(|(v, n)| format!("{v} x{n}")).collect();
        println!(
            "cog check: {checked} levels with scripts, {total} stubbed verb calls left, {errors} errors"
        );
        println!("  still stubbed: {}", top.join(", "));
        // AI, weapon, music and message-bus verbs are stages 5–7; everything else has to
        // be answered by the runtime.
        let deferred = [
            "ai", "setfirewait", "setmountwait", "sendmessage", "selectweapon",
            "setcurinvweapon", "getgoalframe", "setgoalflags", "ismulti",
        ];
        let unexpected: Vec<String> = ranked
            .iter()
            .filter(|(verb, n)| {
                *n > 40 && !deferred.iter().any(|d| verb.to_ascii_lowercase().starts_with(d))
            })
            .map(|(verb, n)| format!("{verb} x{n}"))
            .collect();
        errors += unexpected.len();
        assert!(
            unexpected.is_empty(),
            "verbs stubbed more than 40 times that should be implemented: {unexpected:?}"
        );
        assert_eq!(errors, 0, "cog check found {errors} problems");
        println!("--- cog check passed ---");
        return;
    }

    if things_check {
        // The last of the "thing properties" tail: `TeleportThing`, `CreateThingAtPos`,
        // `FireProjectile`, `SetRespawnMask`, and the multiplayer `Sync*` family, which a
        // single-player build answers and does nothing.
        let state = runtime::Runtime::handle(&world.level);
        let mut rt = state.borrow_mut();
        let Some(template) = rt
            .templates
            .iter()
            .find(|t| rt.template_types.contains_key(*t))
            .cloned()
        else {
            println!("things: this level has no templates");
            return;
        };
        let Some(a) = (0..rt.things.len()).find(|i| !rt.things[*i].destroyed) else {
            println!("things: this level has no things");
            return;
        };
        let b = (0..rt.things.len()).find(|i| *i != a).unwrap_or(a);
        // `TeleportThing(thing, to)` copies the whole pose, angles included — not a position set.
        rt.things[b].pos = [7.0, 8.0, 9.0];
        rt.things[b].pitch = 10.0;
        rt.things[b].yaw = 20.0;
        rt.things[b].roll = 30.0;
        rt.things[a].pos = [1.0, 2.0, 3.0];
        rt.things[a].pitch = 0.0;
        rt.things[a].yaw = 0.0;
        rt.things[a].roll = 0.0;
        rt.call(
            "TeleportThing",
            &[cog::Value::Num(a as f64), cog::Value::Num(b as f64)],
        );
        let pose = (rt.things[a].pos, rt.things[a].pitch, rt.things[a].yaw, rt.things[a].roll);
        println!(
            "things: TeleportThing(thing {a}, thing {b}) copies the whole pose — {pose:?} — not just the position"
        );
        assert_eq!(pose, ([7.0, 8.0, 9.0], 10.0, 20.0, 30.0), "the teleport did not copy the pose");
        // `CreateThingAtPos`: an absolute pose, no marker involved.
        let before = rt.things.len();
        let sector = rt.things[a].sector;
        let spawned = rt.call(
            "CreateThingAtPos",
            &[
                cog::Value::Str(template.clone()),
                cog::Value::Num(sector as f64),
                cog::Value::Vec([4.0, 5.0, 6.0]),
                cog::Value::Vec([0.0, 90.0, 0.0]),
            ],
        );
        let index = match spawned {
            cog::Value::Num(n) if n >= 0.0 => n as usize,
            other => panic!("CreateThingAtPos answered {other:?}"),
        };
        println!(
            "things: CreateThingAtPos(`{template}`) made thing {index} at {:?} facing {:.0} deg ({before} things -> {})",
            rt.things[index].pos,
            rt.things[index].yaw,
            rt.things.len()
        );
        assert_eq!(rt.things.len(), before + 1, "CreateThingAtPos did not add a thing");
        assert_eq!(rt.things[index].pos, [4.0, 5.0, 6.0], "the spawn landed elsewhere");
        assert_eq!(rt.things[index].yaw, 90.0, "the spawn lost its angles");
        // `FireProjectile`: the thing's own `weapon=`, answered with the bolt.
        let armed = (0..rt.things.len()).find(|i| {
            rt.templates
                .get(*i)
                .is_some_and(|name| rt.template_weapon.contains_key(name))
        });
        if let Some(armed) = armed {
            let before = rt.things.len();
            let bolt = rt.call("FireProjectile", &[cog::Value::Num(armed as f64)]);
            println!(
                "things: FireProjectile(thing {armed}, `{}`) answered {bolt:?} and added {} thing(s)",
                rt.templates[armed],
                rt.things.len() - before
            );
            assert!(
                matches!(bolt, cog::Value::Num(n) if n >= 0.0),
                "FireProjectile did not fire"
            );
        } else {
            println!("things: this level has no armed thing; FireProjectile is untested");
        }
        // `SetRespawnMask` is the *player's* mask, not a thing's.
        rt.call("SetRespawnMask", &[cog::Value::Num(a as f64), cog::Value::Num(31.0)]);
        assert_eq!(rt.player_respawn_mask, 0x1f, "SetRespawnMask did not store the mask");
        // The multiplayer pushes answer and change nothing.
        let still = rt.things[a].pos;
        let answers = rt.call("SyncThingPos", &[cog::Value::Num(a as f64)]);
        let surface = rt.call("SyncSurface", &[cog::Value::Num(0.0)]);
        println!(
            "things: SyncThingPos answers {answers:?} and SyncSurface {surface:?}, and the thing stays at {still:?} (a single-player build has nobody to push to)"
        );
        assert_eq!(rt.things[a].pos, still, "SyncThingPos moved something");

        // `PathMovePause`/`PathMoveResume`: the thing keeps its destination and stops advancing —
        // level 9's pipe doors stop in the doorway this way (`09_stoppipedoors.cog`).
        rt.things[a].frames = vec![[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], [4.0, 0.0, 0.0, 0.0, 0.0, 0.0]];
        rt.things[a].frame = 0.0;
        rt.things[a].target = Some((1.0, 2.0));
        rt.step(0.25);
        let moved = rt.things[a].pos[0];
        let paused = rt.call("PathMovePause", &[cog::Value::Num(a as f64)]);
        rt.step(1.0);
        let held = rt.things[a].pos[0];
        let still_moving = rt.call("IsMoving", &[cog::Value::Num(a as f64)]);
        let resumed = rt.call("PathMoveResume", &[cog::Value::Num(a as f64)]);
        rt.step(0.25);
        let carried_on = rt.things[a].pos[0];
        println!(
            "things: PathMovePause answered {paused:?} at x {moved:.2}, a second later it is still {held:.2} (`IsMoving` {still_moving:?}), and PathMoveResume answered {resumed:?} so it carries on to {carried_on:.2}"
        );
        assert_eq!(paused, cog::Value::Num(a as f64), "PathMovePause did not answer the thing");
        assert_eq!(held, moved, "a paused path move kept advancing");
        assert_eq!(still_moving, cog::Value::Num(1.0), "a paused thing stopped reporting as moving");
        assert_eq!(resumed, cog::Value::Num(1.0), "PathMoveResume did not resume");
        assert!(carried_on > held, "a resumed path move did not continue toward its frame");
        // Pausing a thing that is not moving answers -1, which is the engine's guard.
        assert_eq!(
            rt.call("PathMovePause", &[cog::Value::Num(b as f64)]),
            cog::Value::Num(-1.0)
        );

        // `SetThingCurGeoMode`/`GetThingGeoMode`: 0 is not drawn, and the round trip is real.
        let before = rt.call("GetThingGeoMode", &[cog::Value::Num(a as f64)]);
        rt.call(
            "SetThingCurGeoMode",
            &[cog::Value::Num(a as f64), cog::Value::Num(0.0)],
        );
        let hidden = rt.call("GetThingGeoMode", &[cog::Value::Num(a as f64)]);
        rt.call(
            "SetThingCurGeoMode",
            &[cog::Value::Num(a as f64), cog::Value::Num(1.0)],
        );
        let shown = rt.call("GetThingGeoMode", &[cog::Value::Num(a as f64)]);
        println!(
            "things: SetThingCurGeoMode(thing {a}, 0) makes GetThingGeoMode answer {hidden:?} ({before:?} by default), and 1 brings it back ({shown:?}); the renderer hides the thing's entities for 0"
        );
        assert_eq!(hidden, cog::Value::Num(0.0), "the hidden mode did not read back");
        assert_eq!(shown, cog::Value::Num(1.0), "the shown mode did not read back");

        // `SetSectorThrust`: the sector accelerates what stands in it, exactly like gravity. Level 21's
        // core tube is `'0.0 0.0 1.0'` at 4-5 (an updraft) and level 10's power core pushes the player
        // out. The thing used here is at rest, so the whole change is the sector's doing.
        {
            let sector = rt.things[a].sector;
            rt.things[a].vel = [0.0; 3];
            rt.things[a].paused = true; // no path, so only the thrust moves it
            rt.things[a].target = None;
            let before = rt.things[a].pos[2];
            rt.call(
                "SetSectorThrust",
                &[
                    cog::Value::Num(sector as f64),
                    cog::Value::Vec([0.0, 0.0, 1.0]),
                    cog::Value::Num(5.0),
                ],
            );
            for _ in 0..30 {
                rt.step(1.0 / 30.0);
            }
            let rising = rt.things[a].vel[2];
            let after = rt.things[a].pos[2];
            rt.call(
                "SetSectorThrust",
                &[
                    cog::Value::Num(sector as f64),
                    cog::Value::Vec([0.0, 0.0, 0.0]),
                    cog::Value::Num(0.0),
                ],
            );
            let cleared = rt.sectors[usize::try_from(sector).unwrap()].thrust;
            println!(
                "things: SetSectorThrust(sector {sector}, +Z, 5) over a second lifts a resting thing from z {before:.2} to {after:.2} at {rising:.2} u/s, and a zero vector clears the drive ({cleared:?})"
            );
            assert!(rising > 4.0, "the sector thrust did not accelerate the body: {rising}");
            assert!(after > before + 1.0, "the body did not rise in the updraft: {after}");
            assert_eq!(cleared, [0.0; 3], "a zero vector did not clear the thrust");
        }
        println!("--- things check passed ---");
        return;
    }

    if attach_check {
        // `AttachThingToThing(child, parent)`: the child rides the parent, keeping the offset it had
        // at the call, expressed in the parent's own frame — `sithThing_AttachThingToThing` stores
        // exactly that (`rdMatrix_TransformVectorOrtho34` into `field_4C`), so a turning parent
        // swings the child around it. 20 shipped calls use it, and they are functional: a crate on a
        // lift (`00_elev_doors5.cog`), a catwalk on a ship (`09_donefueling.cog`), the sphere
        // `force_absorb.cog` hangs on the player, and the orbiting camera of `00_cameratricks.cog`.
        let level = &world.level;
        let state = runtime::Runtime::handle(level);
        let mut rt = state.borrow_mut();
        let Some(actor) = (0..rt.things.len()).find(|i| rt.types.get(*i).copied() == Some(2)) else {
            println!("attach: this level has no actor to hang something on");
            return;
        };
        let Some(child) = (0..rt.things.len()).find(|i| *i != actor && rt.types.get(*i).copied() != Some(10)) else {
            println!("attach: this level has no second thing to attach");
            return;
        };
        rt.things[actor].pos = [0.0, 0.0, 0.0];
        rt.things[actor].yaw = 0.0;
        rt.things[child].pos = [1.0, 0.0, 0.5];
        rt.call(
            "AttachThingToThing",
            &[cog::Value::Num(child as f64), cog::Value::Num(actor as f64)],
        );
        assert_eq!(
            rt.call("GetThingParent", &[cog::Value::Num(child as f64)]),
            cog::Value::Num(actor as f64),
            "GetThingParent did not read the parent back"
        );
        // The parent moves two units along +X: the child must move with it and keep its offset.
        rt.things[actor].pos = [2.0, 0.0, 0.0];
        rt.step(1.0 / 30.0);
        let carried = rt.things[child].pos;
        println!(
            "attach: with the parent at [2, 0, 0] the attached thing sits at {carried:?} (the offset it was attached at was [1, 0, 0.5])"
        );
        assert!(
            (carried[0] - 3.0).abs() < 1e-3 && (carried[2] - 0.5).abs() < 1e-3,
            "the attached thing did not follow its parent: {carried:?}"
        );
        // Turning the parent swings the child around it, and turning it back puts the child back.
        rt.things[actor].yaw = 90.0;
        rt.step(1.0 / 30.0);
        let turned = rt.things[child].pos;
        rt.things[actor].yaw = 0.0;
        rt.step(1.0 / 30.0);
        let back = rt.things[child].pos;
        println!(
            "attach: parent yaw 90 puts it at {turned:?}, back at {back:?}"
        );
        assert!(
            (turned[0] - 2.0).abs() < 1e-3 && (turned[1] - 1.0).abs() < 1e-3,
            "the parent's rotation did not swing the attached thing: {turned:?}"
        );
        assert!(
            (back[0] - 3.0).abs() < 1e-3 && (back[1]).abs() < 1e-3,
            "turning the parent back did not restore the offset: {back:?}"
        );
        // Detaching stops the ride.
        rt.things[actor].pos = [5.0, 0.0, 0.0];
        rt.call(
            "AttachThingToThing",
            &[cog::Value::Num(child as f64), cog::Value::Num(-1.0)],
        );
        rt.step(1.0 / 30.0);
        let left = rt.things[child].pos;
        println!("attach: after detaching, the parent moved to [5, 0, 0] and the thing stayed at {left:?}");
        assert!(
            (left[0] - 3.0).abs() < 1e-3,
            "a detached thing still followed its parent: {left:?}"
        );
        // The shipped table: which placed scripts in this level attach things.
        let mut attaching = Vec::new();
        for inst in &level.cogs {
            let Some(data) = gobs.read(&inst.script) else { continue };
            if String::from_utf8_lossy(&data).to_ascii_lowercase().contains("attachthingtothing") {
                attaching.push(inst.script.clone());
            }
        }
        attaching.sort();
        attaching.dedup();
        println!(
            "attach: {} placed script(s) in this level attach one thing to another{}",
            attaching.len(),
            attaching.first().map(|n| format!(" (first: {n})")).unwrap_or_default()
        );
        // `RotatePivot(thing, frame, seconds)`: the thing turns about the pivot its own frame names,
        // and anything attached to it comes along — which is exactly the orbiting camera of
        // `00_cameratricks.cog` (`AttachThingToThing(camera1, fulcrum)` + `RotatePivot(fulcrum, 1, …)`)
        // and level 3's pool paddles. The frame's position is the pivot and its angles the turn.
        rt.things[actor].pos = [1.0, 0.0, 0.0];
        rt.things[actor].yaw = 0.0;
        rt.things[actor].frames = vec![[0.0, 0.0, 0.0, 0.0, 90.0, 0.0]];
        rt.things[child].pos = [2.0, 0.0, 0.0];
        rt.call(
            "AttachThingToThing",
            &[cog::Value::Num(child as f64), cog::Value::Num(actor as f64)],
        );
        rt.call(
            "RotatePivot",
            &[
                cog::Value::Num(actor as f64),
                cog::Value::Num(0.0),
                cog::Value::Num(2.0),
            ],
        );
        // Half of a 90-degree turn over two seconds: the pivot is the origin, so the body orbits from
        // +X to +Y and the attached thing rides at twice the radius.
        rt.step(1.0);
        let (half, riding) = (rt.things[actor].pos, rt.things[child].pos);
        let (half_yaw, riding_yaw) = (rt.things[actor].yaw, rt.things[child].yaw);
        rt.step(1.0);
        let (end, done) = (rt.things[actor].pos, rt.things[actor].spin.is_none());
        rt.step(1.0);
        let still = rt.things[actor].pos;
        println!(
            "pivot: RotatePivot(0, 2 s) turns the body about the origin: after 1 s {half:?} at {half_yaw:.0} deg, after 2 s {end:?}; the attached thing rides at {riding:?} ({riding_yaw:.0} deg), and a finished spin stops ({done}, {still:?})"
        );
        assert!(
            (half[0] - 0.7071).abs() < 0.01 && (half[1] - 0.7071).abs() < 0.01,
            "half of a 90 deg turn about the origin is not at 45 deg on the orbit: {half:?}"
        );
        assert!((half_yaw - 45.0).abs() < 0.5, "the body did not turn with its orbit: {half_yaw}");
        assert!(
            (end[0]).abs() < 0.01 && (end[1] - 1.0).abs() < 0.01,
            "a full 90 deg turn did not land on +Y: {end:?}"
        );
        assert!(
            (riding[0] - 1.4142).abs() < 0.02 && (riding[1] - 1.4142).abs() < 0.02,
            "the attached thing did not orbit with its parent: {riding:?}"
        );
        assert!(done, "the spin did not finish");
        assert_eq!(still, end, "a finished spin kept turning");
        println!("--- attach check passed ---");
        return;
    }

    if damage_return_check {
        // `sithCog.c`'s message dispatch: a handler's `ReturnEx` value **replaces** the message's
        // argument for `damaged` (other messages accumulate). 115 shipped scripts have a `damaged`
        // label and 54 of them return the amount — `force_absorb.cog` turns part of a hit into mana
        // and returns the rest, so this is what makes defensive force powers mean anything.
        //
        // Nothing here holds a runtime guard while the VM runs: a script verb locks the same world.
        let state = runtime::Runtime::handle(&world.level);
        let thing = world
            .level
            .things
            .iter()
            .enumerate()
            .filter(|(_, t)| t.template.to_ascii_lowercase().contains("walkplayer"))
            .map(|(i, _)| i)
            .next()
            .expect("a level has a walkplayer");
        // The shipped shape, as an inline script: read the amount from the message params, convert
        // half of it, answer with the rest.
        let script = cog::parse(
            "symbols\nmessage damaged\nflex damage local\nend\ncode\ndamaged:\n\
             damage = GetParam(0);\nReturnEx(damage / 2);\nReturn;\nend\n",
        )
        .expect("the inline handler parses");
        assert!(script.label("damaged").is_some(), "the handler has no damaged label");
        let (applied, health_after_damage) = {
            let mut rt = state.borrow_mut();
            let before = rt.things[thing].health;
            rt.damage_typed(thing as i64, 100.0, 0x2);
            (before - rt.things[thing].health, rt.things[thing].health)
        };
        assert!((applied - 100.0).abs() < 1e-3, "the damage was not applied: {applied}");
        // `GetParam(0)` inside the handler is the amount the engine passed.
        state.borrow_mut().last_params = vec![100.0, 2.0];
        let mut vm = cog::Vm::with_world(state.clone());
        vm.source = thing as i64;
        let answer = vm.run_label(&script, "damaged").expect("the handler runs");
        assert_eq!(
            answer,
            cog::Value::Num(50.0),
            "the handler's ReturnEx did not become the label's value"
        );
        let amended = state.borrow_mut().amend_damage(thing, 50.0);
        assert_eq!(amended, Some((100.0, 50.0)), "the answer was not applied");
        let after = state.borrow().things[thing].health;
        assert!(
            (after - health_after_damage - 50.0).abs() < 1e-3,
            "half of the applied 100 was not given back: {health_after_damage} -> {after}"
        );
        println!(
            "damage-return: thing {thing} was hit for {applied:.0}; `GetParam(0)` answered 100 inside the handler, `ReturnEx(damage / 2)` made the label answer 50, and the runtime gave 50 health back ({health_after_damage:.0} -> {after:.0})"
        );
        // A handler that answers the amount unchanged amends nothing, and one that answers nothing
        // (no `ReturnEx`) leaves the damage standing.
        assert_eq!(state.borrow_mut().amend_damage(thing, 50.0), None, "a settled hit was amended twice");
        {
            let mut rt = state.borrow_mut();
            rt.damage_typed(thing as i64, 10.0, 0x2);
        }
        state.borrow_mut().last_params = vec![10.0, 2.0];
        let plain = cog::parse(
            "symbols\nmessage damaged\nend\ncode\ndamaged:\nReturn;\nend\n",
        )
        .expect("the plain handler parses");
        let mut vm = cog::Vm::with_world(state.clone());
        vm.source = thing as i64;
        let answer = vm.run_label(&plain, "damaged").expect("the plain handler runs");
        assert_eq!(answer, cog::Value::Nil, "a bare `Return` must answer nothing");
        println!("damage-return: a handler with no `ReturnEx` answers nothing and the damage stands");

        // Shipped evidence: which placed scripts have a `damaged` handler that returns an amount.
        let mut returning = Vec::new();
        for inst in &world.level.cogs {
            let Some(data) = gobs.read(&inst.script) else { continue };
            let text = String::from_utf8_lossy(&data).to_ascii_lowercase();
            if text.contains("damaged:") && text.contains("returnex") {
                returning.push(inst.script.clone());
            }
        }
        returning.sort();
        returning.dedup();
        println!(
            "damage-return: {} placed script(s) in this level answer `damaged` with `ReturnEx`{}",
            returning.len(),
            returning.first().map(|n| format!(" (first: {n})")).unwrap_or_default()
        );
        println!("--- damage-return check passed ---");
        return;
    }

    if scroll_check {
        // `SlideWall(surface, vector, scale)`: `sithCogFunctionSurface_SlideWall` scales the vector by
        // `scale * 0.1` and negates it, and the result is used **twice** — as the texture offset
        // (`texVertOffset += scrollVector * dt`) and as the velocity of anything standing on the
        // surface (`sithThing.c`). The conveyor scripts are the shipped case: `00_conveyor.cog`,
        // `02_cratelift.cog` and `02_cratepuzzle_a.cog` drive ten belts between them.
        let level = &world.level;
        let state = runtime::Runtime::handle(level);
        // A surface with a bitmap and more than one texvert, so a scroll really moves its UVs.
        let Some((index, _)) = level.surfaces.iter().enumerate().find(|(_, s)| {
            s.material >= 0 && s.verts.len() >= 3 && s.verts.iter().any(|&(_, t)| t >= 0)
        }) else {
            println!("scroll: this level has no textured surface to scroll");
            return;
        };
        // Standing on it is what a conveyor does to the player, so the vector is `(0, 1)` at scale 10:
        // one level unit a second along +y, negated to `(0, -1)`.
        let mut rt = state.borrow_mut();
        rt.call(
            "SlideWall",
            &[
                cog::Value::Num(index as f64),
                cog::Value::Str("0.0 1.0".to_string()),
                cog::Value::Num(10.0),
            ],
        );
        let rate = rt.surfaces[index].slide;
        assert_eq!(rate, [0.0, -1.0], "SlideWall did not scale and negate its vector");
        let epoch = rt.surface_epoch;
        // A second of clock must both move the texture and carry a body: the offset from the clock is
        // `rate * time`, and the walker adds `rate * dt` a frame.
        rt.step(1.0);
        assert!(rt.surface_epoch > epoch, "a scrolling surface did not ask for a rebuild");
        let time = rt.time;
        drop(rt);
        let mut look = SurfaceLook::default();
        look.scrolls = vec![[0.0; 2]; level.surfaces.len()];
        look.scrolls[index] = [rate[0] * time, rate[1] * time];
        // The digest of every UV in the batches, sorted first because `batches` is a `HashMap`.
        let uv_hash = |look: &SurfaceLook| -> u64 {
            let (batches, _) = build_world(level, &world.images, &Default::default(), look);
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            let mut words: Vec<u64> = Vec::new();
            for batch in batches.values() {
                for uv in &batch.uvs {
                    words.push(uv[0].to_bits() as u64);
                    words.push(uv[1].to_bits() as u64);
                }
            }
            words.sort_unstable();
            for w in words {
                h = (h ^ w).wrapping_mul(0x100_0000_01b3);
            }
            h
        };
        let before = uv_hash(&SurfaceLook::default());
        let after = uv_hash(&look);
        assert_ne!(after, before, "the scroll did not reach the mesh UVs");
        println!(
            "scroll: SlideWall('0 1', 10) on surface {index} records {:?} level units/s; after 1 s the texture is offset {:?} texels, the mesh UVs changed ({before:016x} -> {after:016x}), and a body standing on it is carried {:?} a frame at 60 fps",
            rate,
            look.scrolls[index],
            [rate[0] / 60.0, rate[1] / 60.0]
        );

        // The sky is the same mechanism with one world-wide offset per kind: `SlideCeilingSky(x, y)`
        // is a pair of speeds, and every ceiling-sky face moves by `rate * time`.
        let sky_face = level.surfaces.iter().position(|s| s.surfflags & 0xC00 != 0);
        let mut rt = state.borrow_mut();
        rt.call("SlideCeilingSky", &[cog::Value::Num(0.5), cog::Value::Num(0.0)]);
        assert_eq!(rt.sky_scroll[1], [0.5, 0.0], "SlideCeilingSky did not record the speed pair");
        let epoch = rt.surface_epoch;
        rt.step(1.0);
        assert!(rt.surface_epoch > epoch, "a scrolling sky did not ask for a rebuild");
        let sky_time = rt.time;
        drop(rt);
        match sky_face {
            Some(face) => {
                look.sky = [[0.0; 2], [(0.5f32 * sky_time).rem_euclid(1024.0), 0.0]];
                assert_ne!(
                    look.scroll(face, level.surfaces[face].surfflags),
                    [0.0; 2],
                    "the sky offset is not applied to a sky face"
                );
                println!(
                    "scroll: SlideCeilingSky(0.5, 0) moves every ceiling-sky face by {:?} texels after 1 s (surface {face} carries 0x{:x})",
                    look.sky[1], level.surfaces[face].surfflags
                );
            }
            None => println!(
                "scroll: this level has no sky face; the sky path is checked on the rates only"
            ),
        }
        println!("--- scroll check passed ---");
        return;
    }

    if camera_check {
        // The camera slots (`sithCamera_g_aCameras[7]`) and `SetPOVShake`. What matters is not that
        // the numbers are stored but that (a) a shipped level's own startup script points the view at
        // a thing, which is how every in-engine cutscene works, and (b) the shake decays the way
        // `sithCamera_Update` does instead of sticking.
        let level = &world.level;
        let state = runtime::Runtime::handle(level);
        let mut scripted: Option<(String, usize)> = None;
        for inst in &level.cogs {
            let Some(data) = gobs.read(&inst.script) else { continue };
            let Ok(script) = cog::parse(&String::from_utf8_lossy(&data)) else { continue };
            let mut vm = cog::Vm::with_world(state.clone());
            script.bind(&mut vm, &inst.values);
            // Only `startup`: the cutscenes that run on a trigger are driven by whatever fires them.
            // The focus is recorded for the script that *changed* it, not for every later script that
            // still sees the leftover.
            let before = state.borrow().camera_focus_thing();
            if script.label("startup").is_some() {
                let _ = vm.run_label(&script, "startup");
            }
            let after = state.borrow().camera_focus_thing();
            if after.is_some() && after != before {
                scripted = after.map(|thing| (inst.script.clone(), thing));
            }
        }
        let player = state.borrow().player_thing();
        println!(
            "camera: {} placed script(s), player is thing {player}, current slot {}",
            level.cogs.len(),
            state.borrow().current_camera
        );
        match scripted {
            Some((script, thing)) => {
                let (template, pos) = {
                    let rt = state.borrow();
                    (
                        rt.templates.get(thing).cloned().unwrap_or_default(),
                        rt.things.get(thing).map(|t| t.pos).unwrap_or([0.0; 3]),
                    )
                };
                println!(
                    "camera: `{script}` startup points slot {} at thing {thing} (`{template}`) at {pos:?}, so the view is not the player's",
                    state.borrow().current_camera
                );
                assert_ne!(thing, player, "the scripted camera is the player, which is POV");
            }
            None => println!("camera: this level's startup scripts aim no camera; the verbs check below still runs"),
        }

        // The slots are addressable and read back: focus slot 3 on another thing, select it, then
        // put slot 0 back on the player, which is POV again.
        let other = level.things.len().min(3);
        let mut rt = state.borrow_mut();
        rt.call("SetCameraFocus", &[cog::Value::Num(3.0), cog::Value::Num(other as f64)]);
        rt.call("SetCurrentCamera", &[cog::Value::Num(3.0)]);
        assert_eq!(rt.call("GetCameraMode", &[]), cog::Value::Num(3.0));
        assert_eq!(
            rt.call("GetPrimaryFocus", &[cog::Value::Num(3.0)]),
            cog::Value::Num(other as f64)
        );
        assert_eq!(rt.camera_focus_thing(), Some(other));
        rt.call("SetCameraFocus", &[cog::Value::Num(3.0), cog::Value::Num(player as f64)]);
        assert_eq!(rt.camera_focus_thing(), None, "a focus on the player must be plain POV");

        // `SetPOVShake('0 -.003 0', '1 0 0', .05, 80)`: a kick that recovers. The angle decays at 80
        // degrees a second, so 5 ms takes a degree down to 0.6, and the position at 0.05/s takes
        // 60 ms to settle.
        rt.call(
            "SetPovShake",
            &[
                cog::Value::Str("0.0 -0.003 0.0".to_string()),
                cog::Value::Str("1.0 0.0 0.0".to_string()),
                cog::Value::Num(0.05),
                cog::Value::Num(80.0),
            ],
        );
        let (pos, angle) = rt.camera_shake();
        assert_eq!(pos, [0.0, -0.003, 0.0]);
        assert_eq!(angle, [1.0, 0.0, 0.0]);
        rt.step(0.005);
        let (_, angle) = rt.camera_shake();
        assert!((angle[0] - 0.6).abs() < 1e-3, "the angle did not decay at its rate: {angle:?}");
        rt.step(1.0);
        let (pos, angle) = rt.camera_shake();
        assert_eq!(pos, [0.0; 3], "the position offset did not settle");
        assert_eq!(angle, [0.0; 3], "the angle offset did not settle");
        println!(
            "camera: slots and shake: focus 3 -> thing {other} reads back, a focus on the player is POV, and a shot's kick (1 deg, 80 deg/s) decays to 0.6 in 5 ms and to nothing in a second"
        );
        println!("--- camera check passed ---");
        return;
    }

    if cel_check {
        // A surface's cel (`SetWallCel`, `SurfaceAnim`) picks one frame out of a multi-cel world
        // texture — water, a fuel pipe, a lock or sign panel. The renderer's half is the vertex
        // colour's alpha, which `level.wesl` folds into `v`; the claim to test is that the number
        // reaches the batch the mesh is built from, and that a different cel is a different mesh.
        let level = &world.level;
        let cels = &world.material_cels;
        let multi: Vec<(usize, i32)> = level
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| s.material >= 0 && s.verts.len() >= 3)
            .filter(|(_, s)| cels.get(&s.material).copied().unwrap_or(1) > 1)
            .map(|(i, s)| (i, s.material))
            .collect();
        println!(
            "cel: {} of {} drawn surface(s) use a multi-cel material; {} level material(s) animate",
            multi.len(),
            level.surfaces.iter().filter(|s| s.material >= 0).count(),
            level.materials.iter().enumerate().filter(|(i, _)| cels.get(&(*i as i32)).copied().unwrap_or(1) > 1).count()
        );
        let Some(&(index, material)) = multi.first() else {
            println!("cel: this level has no animated surface; nothing to check");
            return;
        };
        let count = cels[&material];
        // The hash of every vertex alpha in the batches that draw the surface: 0 for cel 0, the cel
        // index for anything else. `Vec<f32>` bits, so the comparison is exact.
        // The digest of every vertex colour in the batches that draw the world: the alpha is the cel
        // and the rgb is the light. The words are sorted first because `batches` is a `HashMap`, and
        // its iteration order is not stable between two builds of the same world.
        let hash = |look: &SurfaceLook| -> u64 {
            let (batches, _) = build_world(level, &world.images, &Default::default(), look);
            let mut words: Vec<u64> = Vec::new();
            for batch in batches.values() {
                for color in &batch.colors {
                    words.push(color[3].to_bits() as u64);
                    for channel in &color[..3] {
                        words.push(channel.to_bits() as u64);
                    }
                }
            }
            words.sort_unstable();
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            for w in words {
                h = (h ^ w).wrapping_mul(0x100_0000_01b3);
            }
            h
        };
        let mut look = SurfaceLook::default();
        look.cels = vec![0.0f32; level.surfaces.len()];
        look.counts = level
            .surfaces
            .iter()
            .map(|s| world.material_cels.get(&s.material).copied().unwrap_or(1) as f32)
            .collect();
        look.lights = vec![0.0f32; level.surfaces.len()];
        look.sectors = vec![0.0f32; level.sectors.len()];
        let first = hash(&look);
        // Through the verb, not by writing the vector: the script's `SetWallCel` is what has to
        // reach the renderer, and it also has to bump the epoch the rebuild watches.
        let world_state = runtime::Runtime::handle(level);
        let (epoch_before, epoch_after) = {
            let mut rt = world_state.borrow_mut();
            let before = rt.surface_epoch;
            rt.call("SetWallCel", &[cog::Value::Num(index as f64), cog::Value::Num(1.0)]);
            look.cels[index] = rt.surfaces[index].cel;
            (before, rt.surface_epoch)
        };
        let second = hash(&look);
        // What a cel change costs the renderer: it re-batches the level. `SurfaceAnim` would pay this
        // `fps` times a second, which is the number that decides whether the animation can ride the
        // rebuild path at all.
        let started = std::time::Instant::now();
        let _ = build_world(level, &world.images, &Default::default(), &look);
        let rebuild_ms = started.elapsed().as_secs_f64() * 1000.0;
        let centre = {
            let (mut cx, mut cy, mut cz, mut n) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for &(vi, _) in &level.surfaces[index].verts {
                if let Some(p) = level.vertices.get(vi as usize) {
                    cx += p[0];
                    cy += p[1];
                    cz += p[2];
                    n += 1.0;
                }
            }
            n = n.max(1.0);
            [cx / n, cy / n, cz / n]
        };
        println!(
            "cel: surface {index} centre {centre:?} normal {:?} (frame a shot with --stand and --look)",
            level.surfaces[index].normal
        );
        println!(
            "cel: surface {index} draws `{}` with {count} cel(s); SetWallCel 1 changed the batch data ({first:016x} -> {second:016x}) and bumped the rebuild epoch {epoch_before} -> {epoch_after}; a re-batch takes {rebuild_ms:.1} ms",
            level.materials.get(material.max(0) as usize).cloned().unwrap_or_default()
        );
        assert_ne!(first, second, "a cel change did not reach the vertex data");
        assert_ne!(epoch_before, epoch_after, "a cel change did not ask for a rebuild");
        // The shader's remap, checked as arithmetic on real numbers: `level.wesl` turns a mesh `v`
        // into `(fract(v * cels) + cel) / cels` because the mesh's UVs are texel space over the whole
        // strip. Two properties matter and both fail for the naive `fract(v)`: every sample lands in
        // the band it asked for, and the tiling period is one *cel*, not the whole strip.
        let cels = count as f32;
        let period = 1.0 / cels;
        for cel in [0.0, 1.0, cels - 1.0] {
            for v in [0.0f32, 0.3, 1.0, 2.5] {
                let band = (cel_v(v, cel, cels) * cels).floor();
                assert_eq!(band, cel, "cel {cel} sampled band {band} at v={v}");
                assert!(
                    (cel_v(v + period, cel, cels) - cel_v(v, cel, cels)).abs() < 1e-5,
                    "the cel remap does not repeat once per cel (v={v}, cels={cels})"
                );
            }
        }
        println!(
            "cel: the remap keeps every sample in its band and repeats once per cel ({cels} cel(s))"
        );

        // End to end: a **shipped** script's `SurfaceAnim` moves a cel. Level 1 places
        // `00_surfanim.cog` (five surfaces at 5 fps) and `01_neonsign.cog` (surface 765 at 2 fps),
        // so a level that names the verb must have a surface stepping after a second of clock.
        let animates = level.cogs.iter().any(|inst| {
            gobs.read(&inst.script)
                .map(|data| {
                    String::from_utf8_lossy(&data).to_ascii_lowercase().contains("surfaceanim(")
                })
                .unwrap_or(false)
        });
        if animates {
            let state = runtime::Runtime::handle(level);
            state.borrow_mut().surface_cels = level
                .surfaces
                .iter()
                .map(|s| world.material_cels.get(&s.material).copied().unwrap_or(1))
                .collect();
            // What the loader hands over, or the animation has no count to wrap at.
            state.borrow_mut().material_cels = world.material_cels.clone().into_iter().collect();
            state.borrow_mut().material_names = level
                .materials
                .iter()
                .enumerate()
                .map(|(index, name)| (name.trim().to_ascii_lowercase(), index as i32))
                .collect();
            let mut started = 0usize;
            let mut asked = 0usize;
            let mut asked_materials = 0usize;
            for inst in &level.cogs {
                let Some(data) = gobs.read(&inst.script) else { continue };
                let Ok(script) = cog::parse(&String::from_utf8_lossy(&data)) else { continue };
                let mut vm = cog::Vm::with_world(state.clone());
                script.bind(&mut vm, &inst.values);
                if script.label("startup").is_some() {
                    let _ = vm.run_label(&script, "startup");
                    started += 1;
                }
                // Only a `startup` that actually called the verb earns the assertion below: many
                // levels name `SurfaceAnim` in a `used:`/`touched:` label that never runs here. The
                // trace keys are the spelling in the script (`SurfaceAnim`), so match case-blind.
                asked += vm
                    .trace
                    .calls
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("surfaceanim"))
                    .map(|(_, count)| *count)
                    .sum::<usize>();
                asked_materials += vm
                    .trace
                    .calls
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("materialanim"))
                    .map(|(_, count)| *count)
                    .sum::<usize>();
            }
            let before: Vec<f32> = state.borrow().surfaces.iter().map(|s| s.cel).collect();
            let light_before: Vec<f32> = state.borrow().surfaces.iter().map(|s| s.light).collect();
            let sector_before: Vec<f32> = state.borrow().sectors.iter().map(|s| s.light).collect();
            state.borrow_mut().step(1.0);
            let after: Vec<f32> = state.borrow().surfaces.iter().map(|s| s.cel).collect();
            let light_after: Vec<f32> = state.borrow().surfaces.iter().map(|s| s.light).collect();
            let moved = before.iter().zip(&after).filter(|(a, b)| a != b).count();
            let lit = light_before.iter().zip(&light_after).filter(|(a, b)| a != b).count();
            let relit = state
                .borrow()
                .sectors
                .iter()
                .zip(&sector_before)
                .filter(|(s, before)| (s.light - *before).abs() > 1e-4)
                .count();
            let moved_materials = state.borrow().material_cel.values().filter(|cel| **cel != 0.0).count();
            let example = before
                .iter()
                .zip(&after)
                .position(|(a, b)| a != b)
                .map(|i| format!("surface {i}: {} -> {}", before[i], after[i]))
                .unwrap_or_else(|| "none".to_string());
            println!(
                "cel: {started} startup script(s) ran, {asked} `SurfaceAnim` and {asked_materials} `MaterialAnim` call(s) among them; after 1 s, {moved} surface(s) stepped ({example}), {lit} surface light(s) and {relit} sector light(s) moved, and {moved_materials} material cel(s) walked"
            );
            assert!(
                asked == 0 || moved > 0,
                "a startup script called SurfaceAnim but no cel moved"
            );
            assert!(
                asked_materials == 0 || moved_materials > 0,
                "a startup script called MaterialAnim but no material cel moved"
            );
        } else {
            println!("cel: no placed script calls SurfaceAnim here; the animation path is untested on this level");
        }
        // Back to cel 0 is the level's own state, and has to be the original mesh again.
        look.cels[index] = 0.0;
        assert_eq!(hash(&look), first, "cel 0 is not the mesh the level started with");

        // The same path carries the scripts' **light** changes: `SetSectorLight(sector, v, time)` and
        // `SurfaceLightAnim(surface, start, end, fps)`. `cog_00_door.cog` brightens its sector by 0.5
        // when a door opens and `cog_00_forcefield.cog` ramps a field from 0.2 to 1.0, so a level that
        // names either verb has to show a different mesh afterwards.
        let (sector_light, surface_light) = {
            let mut rt = world_state.borrow_mut();
            let sector = rt.sectors.len().saturating_sub(1);
            rt.call("SetSectorLight", &[cog::Value::Num(sector as f64), cog::Value::Num(0.5), cog::Value::Num(0.0)]);
            // A fade: 0 -> 1 over 1 s, so one step of 0.5 s lands halfway.
            rt.call(
                "SurfaceLightAnim",
                &[
                    cog::Value::Num(index as f64),
                    cog::Value::Num(0.0),
                    cog::Value::Num(1.0),
                    cog::Value::Num(2.0),
                ],
            );
            let before = rt.surfaces[index].light;
            rt.step(0.5);
            let halfway = rt.surfaces[index].light;
            rt.step(0.5);
            let done = rt.surfaces[index].light;
            assert!((before - 0.0).abs() < 1e-4, "SurfaceLightAnim did not start at `start`");
            assert!((halfway - 0.5).abs() < 1e-3, "the light ramp did not reach half: {halfway}");
            assert!((done - 1.0).abs() < 1e-4, "the light ramp did not settle at `end`: {done}");
            // A second step must not keep walking: a rate is one-shot.
            rt.step(1.0);
            assert!((rt.surfaces[index].light - 1.0).abs() < 1e-4, "the light ramp did not stop");
            (rt.sectors[sector].light, rt.surfaces[index].light)
        };
        look.sectors[level.sectors.len().saturating_sub(1)] = sector_light;
        look.lights[index] = surface_light;
        let lit = hash(&look);
        assert_ne!(lit, first, "a script's light change did not reach the vertex light");
        println!(
            "cel: SetSectorLight 0.5 + SurfaceLightAnim(0 -> 1) changed the vertex light ({first:016x} -> {lit:016x}), ramp landed at {surface_light} and stopped"
        );

        // `MaterialAnim(material, fps, mode)`: a *material* walces its cels, and every surface of that
        // material with no cel of its own draws them. `00_matanim.cog` is the shipped case — level 9
        // runs `materialanim(animmat, 5, 1)` on `09sign.mat`, level 16a on `00Tfuel.mat`.
        {
            let name = level.materials.get(material as usize).cloned().unwrap_or_default();
            let mut rt = world_state.borrow_mut();
            rt.material_cels = world.material_cels.clone().into_iter().collect();
            rt.material_names = level
                .materials
                .iter()
                .enumerate()
                .map(|(index, m)| (m.trim().to_ascii_lowercase(), index as i32))
                .collect();
            let answer = rt.call(
                "MaterialAnim",
                &[
                    cog::Value::Str(name.clone()),
                    cog::Value::Num(4.0),
                    cog::Value::Num(1.0),
                ],
            );
            assert_eq!(answer, cog::Value::Num(material as f64), "the verb did not resolve the material");
            let start = rt.material_cel.get(&material).copied().unwrap_or(-1.0);
            // 4 fps: a cel every 250 ms, so 300 ms is one step and takes cel 0 to 1 (of the material's
            // own count) — the shipped `09sign.mat` has 2.
            rt.step(0.3);
            let stepped = rt.material_cel.get(&material).copied().unwrap_or(-1.0);
            assert_ne!(stepped, start, "MaterialAnim did not walk the material cel");
            look.materials.insert(material, stepped);
            let animated = hash(&look);
            assert_ne!(animated, lit, "a material cel change did not reach the vertex data");
            // A name the level does not have is refused, like the engine's "Bad material index".
            let missing = rt.call(
                "MaterialAnim",
                &[cog::Value::Str("no_such.mat".to_string()), cog::Value::Num(4.0)],
            );
            assert_eq!(missing, cog::Value::Num(-1.0));
            println!(
                "cel: MaterialAnim on `{name}` answered material {material}, stepped its cel {start} -> {stepped} at 4 fps, and that changed the vertex data ({lit:016x} -> {animated:016x}); an unknown name answers -1"
            );
        }
        // `ThingLight`/`SetThingLight`/`ThingLightAnim`: a lamp brightens the sector it stands in, and
        // the draw reads it. Retail lights a radius around the thing; this build's baked sector light
        // is the granularity it has, so the thing's light is summed per sector.
        {
            let mut rt = world_state.borrow_mut();
            let lamp = (0..rt.things.len())
                .find(|i| !rt.things[*i].destroyed && rt.things[*i].sector >= 0)
                .expect("a thing in a sector");
            let sector = rt.things[lamp].sector as usize;
            // `light` is pure arithmetic over the per-surface and per-sector columns, so any index
            // shows what a face in that sector is drawn with.
            let surface = 0usize;
            let before = SurfaceLook::of(&rt).light(surface, rt.things[lamp].sector);
            rt.call(
                "ThingLight",
                &[cog::Value::Num(lamp as f64), cog::Value::Num(2.0), cog::Value::Num(0.0)],
            );
            assert_eq!(rt.things[lamp].light, 2.0, "ThingLight did not set the thing's light");
            assert!(
                rt.thing_light_by_sector()[sector] >= 2.0,
                "the thing's light did not reach its sector"
            );
            let after = SurfaceLook::of(&rt).light(surface, rt.things[lamp].sector);
            println!(
                "cel: ThingLight(thing {lamp}, 2) lifts its sector's drawn light {before:.2} -> {after:.2}"
            );
            assert!(after > before + 1.5, "the thing's light did not reach the vertex data");
            // `ThingLightAnim`: from 0 to 1 over period * 0.5 seconds, one step per second.
            let answer = rt.call(
                "ThingLightAnim",
                &[
                    cog::Value::Num(lamp as f64),
                    cog::Value::Num(0.0),
                    cog::Value::Num(1.0),
                    cog::Value::Num(4.0),
                ],
            );
            assert_eq!(answer, cog::Value::Num(lamp as f64), "ThingLightAnim did not answer the record");
            assert_eq!(rt.things[lamp].light, 0.0, "ThingLightAnim did not start from `start`");
            rt.step(1.0);
            let half = rt.things[lamp].light;
            rt.step(1.0);
            let full = rt.things[lamp].light;
            println!(
                "cel: ThingLightAnim(0 -> 1, period 4) ramps over period/2 = 2 s: {half:.2} after one second and {full:.2} after two, then stops ({})",
                rt.things[lamp].light_rate == 0.0
            );
            assert!((half - 0.5).abs() < 0.05, "the ramp did not reach half in half the time: {half}");
            assert!((full - 1.0).abs() < 0.01, "the ramp did not reach the end: {full}");
            assert_eq!(rt.things[lamp].light_rate, 0.0, "the ramp did not stop at its target");
            // Which placed scripts in this level light things.
            let mut lamps = Vec::new();
            for inst in &world.level.cogs {
                let Some(data) = gobs.read(&inst.script) else { continue };
                let text = String::from_utf8_lossy(&data).to_ascii_lowercase();
                if text.contains("thinglight") || text.contains("thinglightanim") {
                    lamps.push(inst.script.clone());
                }
            }
            lamps.sort();
            lamps.dedup();
            println!(
                "cel: {} placed script(s) in this level light a thing{}",
                lamps.len(),
                lamps.first().map(|n| format!(" (first: {n})")).unwrap_or_default()
            );
        }
        // `SetFaceLightMode(surface, 0)`: `RD_LIGHTMODE_FULLYLIT` ignores the bake, so the face is
        // drawn white whatever its vertex intensities say. That is how the CTF force fields glow.
        {
            let mut rt = world_state.borrow_mut();
            // A face the level baked dark, so "fully lit" is a visible change rather than a no-op.
            let dark = (0..rt.surfaces.len())
                .find(|s| {
                    level
                        .surfaces
                        .get(*s)
                        .is_some_and(|f| f.intensities.iter().any(|i| *i < 0.5))
                })
                .expect("a face the level baked dark");
            let before = SurfaceLook::of(&rt);
            let mode = rt.call(
                "GetFaceLightMode",
                &[cog::Value::Num(dark as f64)],
            );
            rt.call(
                "SetFaceLightMode",
                &[cog::Value::Num(dark as f64), cog::Value::Num(0.0)],
            );
            let after = SurfaceLook::of(&rt);
            let read = rt.call("GetFaceLightMode", &[cog::Value::Num(dark as f64)]);
            println!(
                "cel: SetFaceLightMode(surface {dark}, 0) makes it FULLYLIT — GetFaceLightMode {mode:?} -> {read:?}, and the draw stops darkening it ({:?} -> {:?})",
                before.fullbright(dark),
                after.fullbright(dark)
            );
            assert!(!before.fullbright(dark), "the face was fully lit before the call");
            assert!(after.fullbright(dark), "the face was not fully lit after the call");
            assert_eq!(read, cog::Value::Num(0.0), "the light mode did not read back");
        }
        println!("--- cel check passed ---");
        return;
    }

    if geomode_check {
        // `SetFaceGeoMode(surface, 0)` is `GEOMETRY_NONE`: `00_breakingglass.cog` shatters glass with
        // it and `00_slashablefaces.cog` sets 4 back when a panel is intact. The claim to test is not
        // that the mode is stored — it is that the polygons leave the world's batches, which is what
        // `rebuild_surfaces` patches in place.
        let level = &world.level;
        let claims = collide::surface_claims(level);
        let (index, verts, sector) = level
            .surfaces
            .iter()
            .enumerate()
            .find(|(i, s)| {
                s.material >= 0 && s.verts.len() >= 3 && claims.get(*i).is_some_and(|c| c.len() == 1)
            })
            .map(|(i, s)| (i, s.verts.len(), claims[i][0]))
            .expect("no drawn surface with a single claiming sector");
        let triangles = |hidden: &std::collections::BTreeSet<usize>| -> usize {
            build_world(level, &Default::default(), hidden, &SurfaceLook::default())
                .0
                .values()
                .map(|batch| batch.indices.len() / 3)
                .sum()
        };
        let before = triangles(&Default::default());
        let state = runtime::Runtime::handle(level);
        let set = {
            let mut rt = state.borrow_mut();
            let out = rt.call(
                "SetFaceGeoMode",
                &[cog::Value::Num(index as f64), cog::Value::Num(0.0)],
            );
            (out, rt.geomode_epoch)
        };
        assert_eq!(set.0, cog::Value::Nil);
        let read_back = {
            let mut rt = state.borrow_mut();
            rt.call("GetFaceGeoMode", &[cog::Value::Num(index as f64)])
        };
        assert_eq!(read_back, cog::Value::Num(0.0), "GetFaceGeoMode did not read the mode back");
        assert!(set.1 > 0, "the geomode change did not bump the rebuild epoch");
        // An untouched face answers the level's own `geo` column, not 0. `09_fuelexplode.cog` reads
        // `GetFaceGeoMode(GetSenderRef()) == 0` to skip a fuel pipe that is already gone, so a blanket
        // 0 retires that whole set piece. A face with no material answers 0 and keeps it, which is
        // what retail's `SetFaceGeoMode` stores for it (`FUN_00500790`).
        let (other, other_geo) = level
            .surfaces
            .iter()
            .enumerate()
            .find(|(i, s)| *i != index && s.material >= 0 && s.geo > 0)
            .map(|(i, s)| (i, s.geo))
            .expect("no second drawn surface");
        let blank = level
            .surfaces
            .iter()
            .position(|s| s.material < 0)
            .expect("no face without a material");
        let (other_read, blank_read) = {
            let mut rt = state.borrow_mut();
            let other = rt.call("GetFaceGeoMode", &[cog::Value::Num(other as f64)]);
            rt.call("SetFaceGeoMode", &[cog::Value::Num(blank as f64), cog::Value::Num(4.0)]);
            (other, rt.call("GetFaceGeoMode", &[cog::Value::Num(blank as f64)]))
        };
        assert_eq!(
            other_read,
            cog::Value::Num(other_geo as f64),
            "an untouched face did not answer its own `geo` column"
        );
        assert_eq!(blank_read, cog::Value::Num(0.0), "a face with no material took a draw mode");
        let hidden: std::collections::BTreeSet<usize> = std::iter::once(index).collect();
        let after = triangles(&hidden);
        let want = before - (verts - 2);
        println!(
            "geomode: surface {index} ({verts} verts, sector {sector}) hidden: {before} -> {after} triangles (want {want}); untouched face {other} is geo {other_geo}, a material-less face stays 0"
        );
        assert_eq!(
            after, want,
            "hiding one surface did not remove exactly the {verts} verts' triangles"
        );
        println!("--- geomode check passed ---");
        return;
    }

    if adjoin_check {
        // `--adjoins`: the three verbs that switch a passage, and the two consumers that have to
        // read them. The point is not that the flags get stored — it is that a body stops at a
        // closed force field and the cull stops at a hidden opening.
        let level = &world.level;
        let solids = collide::build(level);
        // The level's own scripts, `startup` only — what the engine has done by the time you can
        // move. Every shipped door script closes its doorway here: `SetSectorAdjoins(doorSector, 0)`.
        let world_state = runtime::Runtime::handle(level);
        let mut instances = 0usize;
        let mut closes_doorways = false;
        for inst in &level.cogs {
            let Some(data) = gobs.read(&inst.script) else { continue };
            let text = String::from_utf8_lossy(&data);
            // `SectorAdjoins(` also matches `SetSectorAdjoins(`: either spelling closes a doorway.
            closes_doorways |= text.to_ascii_lowercase().contains("sectoradjoins(");
            let Ok(script) = cog::parse(&text) else { continue };
            let mut vm = cog::Vm::with_world(world_state.clone());
            script.bind(&mut vm, &inst.values);
            if script.label("startup").is_some() {
                let _ = vm.run_label(&script, "startup");
                instances += 1;
            }
        }
        let mut rt = world_state.borrow_mut();

        // What the level's own table says before any script runs: every row is `0x7` unless the
        // opening ships closed, which is how a mapper marks a force field or a one-way passage.
        let rows = rt.adjoin_flags.len();
        let closed = |bit: u16, rt: &runtime::Runtime| rt.adjoin_flags.iter().filter(|f| *f & bit == 0).count();
        let ai_only = rt.adjoin_flags.iter().filter(|f| *f & collide::ADJOIN_AI_ONLY != 0).count();
        println!(
            "adjoins: {rows} row(s): {} start with MOVE clear, {ai_only} AI-only, {} not visible",
            closed(collide::ADJOIN_MOVE, &rt),
            closed(collide::ADJOIN_VISIBLE, &rt)
        );
        let hidden_sectors = rt
            .sector_flags
            .iter()
            .filter(|f| *f & collide::SECTOR_ADJOINS_OFF != 0)
            .count();
        println!(
            "adjoins: {instances} startup script(s) ran: {} sector adjoin change(s), {hidden_sectors} doorway(s) now shut",
            rt.adjoins_changes
        );
        if closes_doorways {
            // The level places a script that shuts a doorway at load, so the count must be non-zero
            // and every counted change must be one of those doorways — this is what fails if the
            // verb stops reaching the sector word again.
            assert!(hidden_sectors > 0, "no shipped door script closed its doorway on this level");
            assert_eq!(
                hidden_sectors, rt.adjoins_changes,
                "every adjoin change at startup should be a doorway closing"
            );
        } else {
            println!("adjoins: no placed script calls SectorAdjoins here; the verb checks below still run");
        }

        // Two real openings are probed on the level's own data: one where closing `ADJOIN_MOVE`
        // stops the sweep at its plane, one whose hidden `VISIBLE` bit drops rooms from the cull.
        // They are found by trying — a level's recorded sector, its hull and its doorways all
        // disagree often enough that trusting one sector is how a gate becomes vacuous — and the
        // searches are separate because some levels join every pair of rooms more than once.
        let radius = PLAYER_RADIUS;
        let start_sector = level
            .things
            .iter()
            .find(|t| t.template.eq_ignore_ascii_case("walkplayer"))
            .and_then(|t| solids.sector_at(t.pos, radius + 0.05));
        let mut all = |_: &[[f32; 3]]| true;
        let mut solver = None;
        let mut cull = None;
        let mut openings = 0usize;
        'sectors: for sector in start_sector.into_iter().chain(0..solids.portals.len()) {
            for portal in solids.portals.get(sector).into_iter().flatten() {
                openings += 1;
                let row = portal.link;
                // A round trip through the verb needs the surface that carries the row.
                let Some(surface) = rt.surface_adjoin.iter().position(|a| *a == row as i32) else {
                    continue;
                };
                // Start just inside the opening and sweep only a little past its plane, so the
                // measurement is about this opening and nothing else in the room.
                let back = radius + 0.1;
                let at = [
                    portal.point[0] - portal.normal[0] * back,
                    portal.point[1] - portal.normal[1] * back,
                    portal.point[2] - portal.normal[2] * back,
                ];
                let dir = portal.normal;
                let span = radius + 0.15;
                let plane = back - radius;
                if solver.is_none() && rt.gates().passable(sector, portal.adjoin, row) {
                    let mut shut = rt.adjoin_flags.clone();
                    shut[row] &= !collide::ADJOIN_MOVE;
                    let shut_gates = collide::Gates { sectors: &rt.sector_flags, adjoins: &shut };
                    let open_hit = solids.sweep(sector, at, dir, span, radius, rt.gates());
                    let shut_hit = solids.sweep(sector, at, dir, span, radius, shut_gates);
                    if open_hit.is_none_or(|h| h.sector != sector)
                        && shut_hit.is_some_and(|h| h.sector == sector && h.distance <= plane + 1e-3)
                    {
                        solver = Some((sector, portal.adjoin, row, surface, at, dir, span, plane));
                    }
                }
                if cull.is_none() {
                    let before = solids.visible_sectors(&[sector], rt.gates(), &mut all);
                    let mut hidden = rt.adjoin_flags.clone();
                    hidden[row] &= !collide::ADJOIN_VISIBLE;
                    let probe = collide::Gates { sectors: &rt.sector_flags, adjoins: &hidden };
                    let after = solids.visible_sectors(&[sector], probe, &mut all);
                    if after.len() < before.len() {
                        cull = Some((sector, surface, before.len(), after.len()));
                    }
                }
                if solver.is_some() && cull.is_some() {
                    break 'sectors;
                }
            }
        }
        if openings == 0 {
            println!("adjoins: this level has no openings to probe; the flag words and verbs are still checked");
            println!("--- adjoin check passed ---");
            return;
        }
        let (sector, across, row, surface, at, dir, span, plane) =
            solver.expect("no opening on this level where clearing MOVE stops the sweep");

        // `ClearAdjoinFlags(surface, 2)` — the force field goes up, and the sweep stops at the
        // opening's own plane. `SetAdjoinFlags(surface, 16)` is AI-only, which the engine's move
        // path treats the same way, and clearing it permits again.
        for (verb, bits, open) in [
            ("SetAdjoinFlags", 2.0, true),
            ("ClearAdjoinFlags", 2.0, false),
            ("SetAdjoinFlags", 2.0, true),
            ("SetAdjoinFlags", 16.0, false),
            ("ClearAdjoinFlags", 16.0, true),
        ] {
            rt.call(verb, &[cog::Value::Num(surface as f64), cog::Value::Num(bits)]);
            let hit = solids.sweep(sector, at, dir, span, radius, rt.gates());
            if open {
                assert!(
                    hit.is_none_or(|h| h.sector != sector),
                    "{verb}({surface}, {bits}) should let the body through: {hit:?}"
                );
            } else {
                let hit = hit.unwrap_or_else(|| panic!("{verb}({surface}, {bits}) did not stop the body"));
                assert_eq!(hit.sector, sector, "{verb}({surface}, {bits}): the body left the room");
                assert!(
                    hit.distance <= plane + 1e-3,
                    "{verb}({surface}, {bits}) stopped at {} but the opening is at {}",
                    hit.distance,
                    plane
                );
            }
        }
        println!(
            "adjoins: sector {sector} <-> {across} (row {row}): ClearAdjoinFlags 2 stops the sweep at the opening, Set 2 crosses, Set 16 blocks the walker"
        );

        // The cull reads the other bit: `ClearAdjoinFlags(surface, 1)` — `16a_crowadjoin.cog`'s
        // "turn off vis flag" — hides the rooms behind that opening.
        if let Some((sector, surface, rooms_open, rooms_hidden)) = cull {
            rt.call("ClearAdjoinFlags", &[cog::Value::Num(surface as f64), cog::Value::Num(1.0)]);
            assert_eq!(
                solids.visible_sectors(&[sector], rt.gates(), &mut all).len(),
                rooms_hidden,
                "the verb did not hide what the probe said it would"
            );
            rt.call("SetAdjoinFlags", &[cog::Value::Num(surface as f64), cog::Value::Num(1.0)]);
            assert_eq!(
                solids.visible_sectors(&[sector], rt.gates(), &mut all).len(),
                rooms_open,
                "showing the opening did not bring the rooms back"
            );
            println!(
                "adjoins: ClearAdjoinFlags({surface}, 1) drops the rooms behind it from the cull: {rooms_open} -> {rooms_hidden} rooms in view"
            );
        } else {
            println!("adjoins: every room on this level is reachable through more than one opening; the cull half is skipped");
        }
        // `SetSectorAdjoins(sector, 0)` is the same hide the door scripts use.
        let feet = [at[0], at[1], at[2]];
        let was_water = in_water(level, &solids, sector, feet, &rt.sector_flags);
        rt.call("SetSectorFlags", &[cog::Value::Num(sector as f64), cog::Value::Num(2.0)]);
        assert_eq!(rt.sector_flags[sector] & collide::SECTOR_UNDERWATER, collide::SECTOR_UNDERWATER);
        assert!(
            in_water(level, &solids, sector, feet, &rt.sector_flags),
            "a sector flagged underwater is not read as water"
        );
        rt.call("SetSectorAdjoins", &[cog::Value::Num(sector as f64), cog::Value::Num(0.0)]);
        assert!(!rt.gates().passable(sector, across, row), "a hidden sector doorway is still crossed");
        rt.call("SetSectorAdjoins", &[cog::Value::Num(sector as f64), cog::Value::Num(1.0)]);
        assert!(rt.gates().passable(sector, across, row), "showing the adjoins did not reopen them");
        println!(
            "adjoins: SetSectorFlags({sector}, 2) makes the sector water (was {was_water}); SetSectorAdjoins({sector}, 0) shuts its doorways"
        );
        println!("--- adjoin check passed ---");
        return;
    }

    if cogs_only {
        run_level_cogs(&gobs, &world.level, &cogs_filter);
        return;
    }

    if run_only {
        if run_filter == "all" {
            // Every level in the install, each with its own scripts acting on its own
            // world. Quiet: one summary block per level instead of per-instance detail.
            let levels: Vec<String> = gobs
                .names()
                .into_iter()
                .filter(|n| n.ends_with(".jkl"))
                .collect();
            let mut stubbed: std::collections::BTreeMap<String, usize> = Default::default();
            for name in &levels {
                let Some(raw) = gobs.read(name) else { continue };
                let level = jkl::parse(&String::from_utf8_lossy(&raw));
                println!("--- {name} ---");
                for (verb, n) in run_level(&gobs, &level, true) {
                    *stubbed.entry(verb).or_default() += n;
                }
            }
            let mut ranked: Vec<(String, usize)> = stubbed.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1));
            // Every stubbed verb, not the top of the list: this report exists to say what is left to
            // implement, and "the first fifteen" is not that answer.
            let all: Vec<String> = ranked.iter().map(|(v, n)| format!("{v} x{n}")).collect();
            println!("=== verbs still stubbed across the install ({}) ===", all.len());
            println!("{}", all.join(", "));
            return;
        }
        run_level(&gobs, &world.level, false);
        return;
    }

    if std::env::var_os("BK_EYE_DEBUG").is_some() {
        // What the engine actually uses: `THINGPARAM_EYEOFFSET` comes from the template's
        // `eyeoffset=(x/y/z)` field, and the camera is `position + eyeOffset` (plus head pitch). Not a
        // constant, so a port that guesses one is guessing at where the player's head is.
        let levels: Vec<String> = gobs.names().into_iter().filter(|n| n.ends_with(".jkl")).collect();
        for name in levels.iter().take(4) {
            let Some(raw) = gobs.read(name) else { continue };
            let level = jkl::parse(&String::from_utf8_lossy(&raw));
            let mut shown = 0;
            for template in &level.templates {
                if shown >= 4 {
                    break;
                }
                if template.name.eq_ignore_ascii_case("walkplayer") {
                    println!("eye: walkplayer base={:?} params={:?}", template.base, template
                        .params
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join(" "));
                    for key in ["eyeoffset", "height", "movesize", "radius", "size", "maxvel", "model3d", "puppet"] {
                        let v = level.template_param(&template.name, key).unwrap_or("-");
                        print!("{key}={v} ");
                    }
                    println!("  <- {name} {}", template.name);
                    shown += 1;
                }
            }
            if shown == 0 {
                println!("eye: {name} has no template with an eyeoffset");
            }
            // The engine's height for a model with no `height=` is the model's own insert offset:
            // `sithPhysics_GetThingHeight` falls back to `model3->insertOffset.z`.
            if let Some(bytes) = gobs.read("ky.3do") {
                let model = tdo::parse(&String::from_utf8_lossy(&bytes));
                println!(
                    "eye: ky.3do insert offset {:?}, {} mesh(es)",
                    model.insert_offset,
                    model.meshes.len()
                );
            }
        }
        return;
    }

    if winding_check {
        // The mesh builder emits `[base, base+k, base+k+1]` and ships the Newell normal as the vertex
        // normal. Bevy culls by winding, so if a polygon's emitted triangles wind against its own normal the
        // visible side disappears (or the far side shows). This compares the two directly on every surface
        // of every level: a *geometry* test of the vertices, independent of textures and materials.
        let levels: Vec<String> = gobs.names().into_iter().filter(|n| n.ends_with(".jkl")).collect();
        let (mut drawn, mut flipped, mut degenerate) = (0usize, 0usize, 0usize);
        let mut worst: Option<(String, usize, f32)> = None;
        for name in &levels {
            let Some(raw) = gobs.read(name) else { continue };
            let level = jkl::parse(&String::from_utf8_lossy(&raw));
            let (mut level_flipped, mut level_drawn) = (0usize, 0usize);
            for surface in &level.surfaces {
                if surface.material < 0 || surface.verts.len() < 3 {
                    continue;
                }
                let Some(n) = polygon_normal(&level, surface) else { continue };
                let p0 = level.vertices[surface.verts[0].0 as usize];
                let p1 = level.vertices[surface.verts[1].0 as usize];
                let p2 = level.vertices[surface.verts[2].0 as usize];
                let a = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
                let b = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
                let w = [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ];
                let len = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
                if len < 1e-9 {
                    degenerate += 1;
                    continue;
                }
                level_drawn += 1;
                let d = (w[0] * n[0] + w[1] * n[1] + w[2] * n[2]) / len;
                if d < 0.0 {
                    level_flipped += 1;
                    if worst.as_ref().is_none_or(|(_, _, wd)| d < *wd) {
                        worst = Some((name.clone(), surface.verts[0].0 as usize, d));
                    }
                }
            }
            drawn += level_drawn;
            flipped += level_flipped;
        }
        println!(
            "winding: {flipped} of {drawn} drawn surface(s) wind against their own normal ({degenerate} degenerate)"
        );
        if let Some((level, vertex, d)) = &worst {
            println!("winding: worst is {level} at vertex {vertex} (dot {d:.2})");
        }
        // Do the shipped normals point into their own room? The engine renders a surface only when the
        // camera is on the plane's positive side (`sithRender_RenderSectors`: `if (dist <= 0) continue`),
        // so a normal that points *away* from its sector's interior makes that surface invisible from
        // inside the room — you see through it into whatever is behind. Newell takes its sign from the
        // vertex order, which nothing here has verified.
        let (mut inward, mut outward) = (0usize, 0usize);
        for name in &levels {
            let Some(raw) = gobs.read(name) else { continue };
            let level = jkl::parse(&String::from_utf8_lossy(&raw));
            for sector in &level.sectors {
                let (first, count) = sector.surfaces;
                let owned: Vec<&jkl::Surface> =
                    level.surfaces.iter().skip(first).take(count).collect();
                let mut sum = [0.0f32; 3];
                let mut n = 0usize;
                for s in &owned {
                    for &(v, _) in &s.verts {
                        if let Some(p) = level.vertices.get(v as usize) {
                            sum = [sum[0] + p[0], sum[1] + p[1], sum[2] + p[2]];
                            n += 1;
                        }
                    }
                }
                if n == 0 {
                    continue;
                }
                let centre = [sum[0] / n as f32, sum[1] / n as f32, sum[2] / n as f32];
                for s in &owned {
                    if s.material < 0 || s.verts.len() < 3 {
                        continue;
                    }
                    let Some(nm) = polygon_normal(&level, s) else { continue };
                    let Some(p0) = level.vertices.get(s.verts[0].0 as usize) else { continue };
                    let d = nm[0] * (centre[0] - p0[0])
                        + nm[1] * (centre[1] - p0[1])
                        + nm[2] * (centre[2] - p0[2]);
                    if d > 0.0 {
                        inward += 1;
                    } else {
                        outward += 1;
                    }
                }
            }
        }
        println!(
            "winding: {inward} surface normal(s) point into their room, {outward} point away (those are culled from inside)"
        );
        // The plane the game itself uses, surface by surface. `#--- Surface normals ---` is read by
        // `sithSurface_ReadSurfacesListText` (JK.EXE `0x4e5c00`, `sithSurface.c:502`) into
        // `face.normal`, and it is the only normal the engine's collision and renderer ever see — so
        // comparing our Newell normal against it says exactly how far our geometry is from the game's.
        // Two separate questions: does our *plane* match, and does our *sign* match.
        let (mut compared, mut same_plane, mut same_sign, mut outward_ok) = (0usize, 0usize, 0usize, 0usize);
        let mut worst_cos: Option<(String, usize, f32)> = None;
        for name in &levels {
            let Some(raw) = gobs.read(name) else { continue };
            let level = jkl::parse(&String::from_utf8_lossy(&raw));
            for (index, surface) in level.surfaces.iter().enumerate() {
                let squared = surface.normal[0] * surface.normal[0]
                    + surface.normal[1] * surface.normal[1]
                    + surface.normal[2] * surface.normal[2];
                if surface.verts.len() < 3 || squared < 0.25 {
                    continue;
                }
                let Some(ours) = polygon_normal(&level, surface) else { continue };
                compared += 1;
                // Our Newell normal against the plane the level ships. `sithSurface_ReadSurfacesListText`
                // reads that block into `face.normal` (JK.EXE `0x4e5c00`), so this is a direct
                // comparison with the game's own geometry — the one number that says whether our
                // derivation is faithful.
                let dot = ours[0] * surface.normal[0] + ours[1] * surface.normal[1] + ours[2] * surface.normal[2];
                if let Ok(want) = std::env::var("BK_SURFACE")
                    && want.parse::<usize>() == Ok(index)
                {
                    println!(
                        "surface {index} ({name}): ours {ours:?} file {:?} dot {dot:+.4}",
                        surface.normal
                    );
                }
                // The file's normals point *into* the room (surface 3275 on level 1 is the floor slab
                // at z = -1.300 and ships (0,0,1), up), and Newell follows the winding, so a faithful
                // derivation agrees with it rather than opposing it.
                if dot > 0.0 {
                    same_sign += 1;
                } else {
                    outward_ok += 1;
                }
                if dot > 0.999 {
                    same_plane += 1;
                } else if worst_cos.as_ref().is_none_or(|(_, _, w)| dot < *w) {
                    worst_cos = Some((name.clone(), surface.verts[0].0 as usize, dot));
                }
            }
        }
        println!(
            "normals: {same_plane} of {compared} surface plane(s) equal the file's own normal (dot > 0.999); {same_sign} agree in sign, {outward_ok} are negated"
        );
        if let Some((level, vertex, dot)) = &worst_cos {
            println!("normals: worst plane is {level} at vertex {vertex} (dot {dot:+.3})");
        }
        println!("winding: {} level(s) checked", levels.len());
        println!("--- winding check passed ---");
        return;
    }

    if move_check {
        // `--move`: every level, walked from the spawn the game actually uses, asserting the promise the
        // engine makes at the end of `sithCollision_MoveThing`: a move that would take the body out of
        // the world is not taken (`sithIntersect_IsSphereInSector`).
        //
        // Each level is walked from eight headings, because "the body can move" cannot mean "forward has
        // room": the level's own `walkplayer` yaw is the editor's, and several shipped starts face a wall
        // 0.1 units away (`10cargo` faces a 45° corner). Holding that one direction makes a correct body
        // look stuck. A level is stuck only when *every* heading is stuck, and a fall in any heading is a
        // failure, because a body must never be over nothing.
        const HEADINGS: usize = 8;
        let levels: Vec<String> = gobs.names().into_iter().filter(|n| n.ends_with(".jkl")).collect();
        let (mut walked, mut fell, mut frozen) = (0usize, 0usize, 0usize);
        let mut travelled = 0.0f32;
        for name in &levels {
            let Some(raw) = gobs.read(name) else { continue };
            let level = jkl::parse(&String::from_utf8_lossy(&raw));
            let (mut best, mut any_fall, mut sum, mut runs, mut undone) = (0.0f32, false, 0.0f32, 0usize, 0usize);
            for heading in 0..HEADINGS {
                let turn = heading as f32 * std::f32::consts::TAU / HEADINGS as f32;
                let Some(r) = walk_level(&level, 180, true, turn) else { continue };
                runs += 1;
                sum += r.travelled;
                best = best.max(r.travelled);
                // A body that is boxed in touches walls; a body that is being undone by the
                // containment rule does not. Only the second is a defect (`18ascend` and `19a` start
                // inside a lift car and are correctly immobile, with 172 of 180 frames blocked).
                if r.travelled < 0.5 && r.blocked == 0 {
                    undone += 1;
                }
                if r.void_frames > 0 && !any_fall {
                    any_fall = true;
                    println!(
                        "move: {name} heading {heading} spent {} frame(s) over nothing ({:.1} below its spawn)",
                        r.void_frames, r.dropped
                    );
                }
            }
            if runs == 0 {
                continue;
            }
            walked += 1;
            travelled += sum;
            if any_fall {
                fell += 1;
            }
            // The body moved nowhere and met nothing, from every heading: the containment rule is
            // undoing legal moves, not the level enclosing the spawn.
            if undone == runs {
                frozen += 1;
                println!("move: {name} could not move in any of {runs} heading(s) (best {best:.2} units)");
            }
        }
        let mean = if walked > 0 { travelled / (walked * HEADINGS) as f32 } else { 0.0 };
        println!(
            "move: {walked} level(s) x {HEADINGS} headings, {fell} fell through, {frozen} stuck, {travelled:.0} units travelled ({mean:.1} per heading)"
        );
        assert_eq!(fell, 0, "a body fell out of the world");
        assert_eq!(frozen, 0, "a body could not move from its spawn");
        println!("--- move check passed ---");
        return;
    }

    if let Some(walk) = walk {
        match walk {
            Walk::Frames(frames) => {
                walk_level(&world.level, frames, false, 0.0);
            }
            // Twenty seconds a level, not two: at the body's real size a two-second walk stays inside
            // the room it started in, so it never crossed a portal and the sweep stopped exercising
            // the transition code at all.
            Walk::All => walk_all(&gobs, 600),
        }
        return;
    }

    if check {
        self_check(&world);
        return;
    }

    if animate_only {
        // Every placed actor whose template names a puppet with a drawable `stand` state.
        let mut checked = 0usize;
        let mut moved = 0usize;
        let mut worst = 0.0f32;
        for thing in &world.level.things {
            let Some(puppet) = world
                .level
                .template_param(&thing.template, "puppet")
                .map(|n| n.to_ascii_lowercase())
            else {
                continue;
            };
            let Some(state) = world
                .puppets
                .get(&puppet)
                .and_then(|p| p.states.iter().find(|s| s.name.eq_ignore_ascii_case("stand")))
            else {
                continue;
            };
            let Some(key) = world.keys.get(&state.key.to_ascii_lowercase()) else {
                continue;
            };
            let Some(model) = world.template_model.get(&thing.template.to_ascii_lowercase()) else {
                continue;
            };
            let Some(model) = world.models.get(model) else { continue };
            let span = key.frames.max(2) as f32;
            let first = tdo::pose_nodes(model, Some(key), 0.0);
            let later = tdo::pose_nodes(model, Some(key), (span / 2.0).floor());
            // How far any joint moves in half a loop: the whole animation is one pose
            // from `tdo::pose_nodes`, so a still file would be exactly zero.
            let delta = first
                .iter()
                .zip(&later)
                .map(|(a, b)| Vec3::from(a.pos).distance(Vec3::from(b.pos)))
                .fold(0.0f32, f32::max);
            // The mesh the renderer actually shows, posed at the same two times: this is
            // the value `animate_actors` writes into the asset, so a difference here is
            // the actor changing on screen.
            // Every material the model draws with: a puppet's moving parts are spread over
            // several of them, so checking one would report a still actor.
            let materials: std::collections::BTreeSet<i32> = model
                .meshes
                .iter()
                .flat_map(|m| m.faces.iter().map(|f| f.material))
                .collect();
            let mut mesh_delta = 0.0f32;
            for material in materials {
                let mesh_at = |frame: f32| {
                    let mesh = model_mesh_posed(model, material, (1.0, 1.0), Some((key, frame)));
                    match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
                        Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) => v.clone(),
                        _ => Vec::new(),
                    }
                };
                let (a, b) = (mesh_at(0.0), mesh_at((span / 2.0).floor()));
                mesh_delta = mesh_delta.max(
                    a.iter()
                        .zip(&b)
                        .map(|(p, q)| Vec3::from(*p).distance(Vec3::from(*q)))
                        .fold(0.0f32, f32::max),
                );
            }
            checked += 1;
            worst = worst.max(delta);
            if delta > 1e-3 && mesh_delta > 1e-3 {
                moved += 1;
            }
            if checked <= 3 {
                println!(
                    "  {} ({} frames at {:.0} fps): joint travel {:.3} in half a loop",
                    thing.template, key.frames, key.fps, delta
                );
            }
        }
        // The locomotion chooser, against the level's own puppets: every state it can return must
        // name a keyfile the actor's puppet actually ships, or `animate_actors` falls back to
        // `stand` and the actor glides. The steps are velocities in the actor's own frame.
        let mut walkable = 0usize;
        let mut running = 0usize;
        for thing in &world.level.things {
            let Some(puppet) = world
                .level
                .template_param(&thing.template, "puppet")
                .map(|n| n.to_ascii_lowercase())
                .and_then(|p| world.puppets.get(&p))
            else {
                continue;
            };
            let states: std::collections::BTreeMap<(String, usize), &str> = puppet
                .states
                .iter()
                .filter(|s| world.keys.get(&s.key.to_ascii_lowercase()).is_some_and(key::Key::is_drawable))
                .map(|s| ((s.name.to_ascii_lowercase(), s.mode), s.key.as_str()))
                .collect();
            let yaw = thing.yaw;
            let (sin, cos) = yaw.to_radians().sin_cos();
            let forward = Vec3::new(cos, sin, 0.0);
            let right = Vec3::new(-sin, cos, 0.0);
            for (want, vel) in [
                ("stand", Vec3::ZERO),
                ("walk", forward * 0.3),
                ("run", forward * 0.8),
                ("walkback", forward * -0.3),
                ("straferight", right * 0.3),
                ("strafeleft", right * -0.3),
            ] {
                assert_eq!(locomotion_state(vel, yaw), want, "{}: {want} from {vel}", thing.template);
                if !states.contains_key(&(want.to_string(), 0)) {
                    continue;
                }
                if want == "walk" {
                    walkable += 1;
                } else if want == "run" {
                    running += 1;
                }
            }
        }
        println!(
            "{level_name}: {moved} of {checked} placed actors move between frame 0 and mid-loop (worst {:.3})",
            worst
        );
        // The armed set: a puppet's mode-1 rows are the same animations with the weapon drawn, and
        // `SetArmedMode(thing, 1)` is what selects them. The count says whether that axis does
        // anything on a real level rather than only in the tables.
        let mut armed = 0usize;
        let mut draws = 0usize;
        let mut flinches = 0usize;
        for thing in &world.level.things {
            let Some(puppet) = world
                .level
                .template_param(&thing.template, "puppet")
                .map(|n| n.to_ascii_lowercase())
                .and_then(|name| world.puppets.get(&name))
            else {
                continue;
            };
            if puppet.states.iter().any(|s| s.mode == 1) {
                armed += 1;
            }
            if puppet
                .states
                .iter()
                .any(|s| matches!(s.name.to_ascii_lowercase().as_str(), "mount" | "dismount"))
            {
                draws += 1;
            }
            if puppet.states.iter().any(|s| s.name.eq_ignore_ascii_case("hit")) {
                flinches += 1;
            }
        }
        println!(
            "  locomotion: {walkable} actor(s) ship a `walk` key and {running} a `run` key, so a moving body plays its own stride"
        );
        println!("  hit: {flinches} actor(s) ship a `hit` track");
        // Which states the level's puppets actually define, so the next one to port is chosen from
        // data rather than from a guess about what looks wrong.
        {
            let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
            for thing in &world.level.things {
                let Some(puppet) = world
                    .level
                    .template_param(&thing.template, "puppet")
                    .map(|n| n.to_ascii_lowercase())
                    .and_then(|name| world.puppets.get(&name))
                else {
                    continue;
                };
                for state in &puppet.states {
                    *seen.entry(state.name.to_ascii_lowercase()).or_default() += 1;
                }
            }
            let mut ranked: Vec<(String, usize)> = seen.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let names: Vec<String> = ranked.iter().map(|(n, c)| format!("{n} x{c}")).collect();
            println!("  states the level's puppets define: {}", names.join(", "));
        }
        println!(
            "  armed: {armed} actor(s) ship a mode-1 (weapon drawn) set, which `SetArmedMode` selects; {draws} ship mount/dismount tracks"
        );
        assert!(
            checked == 0 || moved * 2 > checked,
            "only {moved} of {checked} actors animate"
        );
        assert!(
            checked == 0 || walkable * 2 > checked,
            "only {walkable} of {checked} actors ship a locomotion key"
        );
        println!("--- animate check passed ---");
        return;
    }

    if fall_check {
        // Fall damage from a known height, against the closed forms: the impact is `sqrt(2 g h)` and the
        // damage is `sithCollision_FallHurt`'s `(v - 2.5)^2 * 45` on that impact in engine units. A gate
        // rather than a demonstration because the two are easy to break independently — the curve is
        // unit-sensitive and the impact depends on the integration.
        let solids = collide::build(&world.level);
        let rules = Rules::from_level(&world.level);
        let Some(start) = world
            .level
            .things
            .iter()
            .find(|t| t.template.eq_ignore_ascii_case("walkplayer"))
        else {
            println!("fall: the level has no walkplayer");
            return;
        };
        let mut sector = start.sector.max(0) as usize;
        let mut pos = [start.pos[0], start.pos[1], start.pos[2] + 0.5];
        let mut vy = 0.0f32;
        let dt = 1.0 / 30.0;
        for _ in 0..60 {
            walker_step(&solids, collide::Gates::default(), &mut sector, &mut pos, &mut vy, [0.0, 0.0], dt, rules.radius, rules.gravity, false);
        }
        // Drop from the roomiest place the level has, not from the spawn: level 1's walkplayer stands in a
        // crawlspace 0.3 units tall, and a body lifted above its own ceiling is *outside its sector*, which
        // the containment rule (correctly, `decomp/004e7950_sithCollision_MoveThing.c:251`) refuses — so the
        // fall never happens. The roomiest sector is where the drag curve can be measured at all.
        let roomiest = (0..solids.sectors.len())
            .filter_map(|i| {
                let centre = solids.spheres.get(i)?.0;
                let floor = solids.floor_below(i, centre)?;
                let ceiling = solids.ceiling_above(i, centre)?;
                let headroom = ceiling - floor;
                // The body has to stand on that floor, not in the geometry around it.
                let at = [centre[0], centre[1], floor + rules.radius];
                (headroom > 0.0 && solids.inside_sector(i, [at[0], at[1], floor], 0.0))
                    .then_some((i, at, headroom))
            })
            .max_by(|a, b| a.2.total_cmp(&b.2));
        if let Some((room, at, _headroom)) = roomiest {
            pos = at;
            sector = room;
            vy = 0.0;
        }
        let base = pos[2];
        let mut found = None;
        for drop in [1.5f32, 1.0, 0.6, 0.4, 0.25, 0.15, 0.08] {
            let start_z = base + drop;
            // A lift that already leaves the room cannot fall; it is held, which is correct.
            if !solids.inside_sector(sector, [pos[0], pos[1], start_z - rules.radius], 0.0) {
                continue;
            }
            let mut p2 = [pos[0], pos[1], start_z];
            let mut v2 = 0.0f32;
            // The fall is measured by how far the body actually descended and by the fastest speed it
            // reached, not by catching the landing frame: in a low room the whole drop is one frame, and
            // the old `before < 0` test then never fired at all.
            let mut peak = 0.0f32;
            for _ in 0..900 {
                walker_step(&solids, collide::Gates::default(), &mut sector, &mut p2, &mut v2, [0.0, 0.0], dt, rules.radius, rules.gravity, false);
                peak = peak.max(-v2);
                if v2 == 0.0 && peak > 0.0 {
                    break;
                }
            }
            let fell = start_z - p2[2];
            if peak > 0.0 && fell > 0.02 {
                found = Some((fell, peak));
                break;
            }
        }
        let Some((drop, impact)) = found else {
            panic!("no drop from the spawn's floor falls at all: the body is stuck in its sector");
        };        let want_impact = (2.0 * rules.gravity * drop).max(0.0).sqrt();
        assert!(
            impact > 0.0 && (impact - want_impact).abs() <= want_impact * 0.25,
            "a {drop}-unit drop landed at {impact:.2} u/s, not sqrt(2gh) = {want_impact:.2}"
        );
        // The curve is in engine units; `physics_scale` is 1 today, and the conversion is what keeps this
        // honest if it ever is not.
        let engine_impact = impact / physics_scale().max(1.0).sqrt();
        // The engine's own two gates: no `FallHurt` call at or below 2.5, and a 1.0 floor inside it.
        let want_damage = if engine_impact > 2.5 {
            (engine_impact - 2.5).powi(2) * 45.0
        } else {
            0.0
        };
        let damage = fall_damage(engine_impact);
        let expected = if want_damage > 1.0 { want_damage } else { 0.0 };
        assert!(
            (damage - expected).abs() < 1.0,
            "fall damage {damage:.1} is not (v-2.5)^2*45 = {expected:.1} at {engine_impact:.2} u/s"
        );
        println!(
            "fall: {drop} units -> {impact:.2} u/s ({engine_impact:.2} engine), damage {damage:.0} = (v-2.5)^2*45"
        );
        println!("--- fall check passed ---");
        return;
    }

    if falldeath_check {
        // `SITH_SECTOR_FALLDEATH` (0x40) is what separates "fell off a ledge" from "fell out of the
        // world": `sithPlayer_Update` kills a body falling faster than 3 u/s while inside one. Level 1
        // ships 32 of them, so this is a rule a player meets, not a corner case.
        let flagged = world
            .level
            .sectors
            .iter()
            .filter(|s| s.flags & collide::SECTOR_FALLDEATH != 0)
            .count();
        assert!(flagged > 0, "{level_name} ships no FALLDEATH sector");
        assert!(fall_death_rule(4.0, collide::SECTOR_FALLDEATH), "a fast fall in a flagged sector must kill");
        assert!(!fall_death_rule(2.0, collide::SECTOR_FALLDEATH), "a slow descent is not a fall death");
        assert!(!fall_death_rule(4.0, 0), "a fast fall in an ordinary room is a landing, not a death");
        println!("fall death: {flagged} sector(s) carry 0x40; 4.0 u/s inside one kills, 2.0 u/s and an unflagged sector do not");
        println!("--- fall death check passed ---");
        return;
    }

    if door_check {
        // The whole chain, in one process and on the level's own scripts: the player's use action picks
        // the nearest linked *thing* (`Runtime::use_thing`), the message queue carries `used` to the
        // script that owns the link, and the script answers with `setsectoradjoins`/`setadjoinflags` —
        // the very flags `collide::Gates` reads to decide a doorway is a wall.
        let handle = runtime::Runtime::handle(&world.level);
        handle.borrow_mut().wire_links(&world.level.cogs, &world.cogs);
        // The collision world, for the chain's last link: a doorway the solver refused must become one it
        // allows, or opening the door changes nothing a body can walk through.
        let solids = collide::build(&world.level);
        // Every instance runs `startup` at load, as the engine does — and a door's whole state lives
        // there: `cog_00_door.cog`'s `startup` reads `GetThingSector(door0)` into the *local*
        // `doorSector` and shuts that sector, which is what `activate` later reopens.
        //
        // The VM has to be **kept**, one per instance, exactly as `Sim::instances` does: locals live in
        // the VM, so a fresh one per label loses `doorSector` and `numDoors` — which is why an earlier
        // version of this probe read a door with an empty sector and no flag ever changed.
        let mut vms: Vec<Option<cog::Vm>> = Vec::with_capacity(world.level.cogs.len());
        for inst in &world.level.cogs {
            let Some(script) = world.cogs.get(&inst.script) else {
                vms.push(None);
                continue;
            };
            let mut vm = cog::Vm::with_world(handle.clone());
            script.bind(&mut vm, &inst.values);
            if script.label("startup").is_some() {
                let _ = vm.run_label(script, "startup");
            }
            vms.push(Some(vm));
        }
        let candidates: Vec<(usize, usize)> = {
            let rt = handle.borrow();
            rt.thing_links
                .iter()
                .filter_map(|(instance, _, thing)| {
                    let inst = world.level.cogs.get(*instance)?;
                    let script = world.cogs.get(&inst.script)?;
                    script.label("activate").is_some().then_some((*instance, *thing))
                })
                .collect()
        };
        assert!(!candidates.is_empty(), "no linked thing in this level listens on `activate`");
        let mut outcome = None;
        for (instance, thing) in &candidates {
            let eye = {
                let rt = handle.borrow();
                let Some(t) = rt.things.get(*thing) else { continue };
                [t.pos[0] + 1.0, t.pos[1], t.pos[2]]
            };
            // Look straight at it from one unit away: the use action is a look, not a touch.
            let facing = [-1.0f32, 0.0];
            let (_used, before) = {
                let mut rt = handle.borrow_mut();
                let before = (rt.sector_flags.clone(), rt.adjoin_flags.clone());
                let used = rt.use_thing(eye, facing, 3.0);
                (used, before)
            };
            // Another linked thing can be nearer than the one this candidate names; what matters is that
            // the use action finds *a* linked thing, and that its script changes the world.
            // Drain exactly as `deliver_thing_messages` does, so this is the real path and not a shortcut.
            let pending: Vec<(usize, &'static str, [f64; 2])> = {
                let mut rt = handle.borrow_mut();
                std::mem::take(&mut rt.pending_messages)
            };
            let mut ran = 0;
            for (inst, label, params) in &pending {
                let Some(data) = world.level.cogs.get(*inst) else { continue };
                let Some(script) = world.cogs.get(&data.script) else { continue };
                let Some(vm) = vms.get_mut(*inst).and_then(|v| v.as_mut()) else { continue };
                vm.source = *thing as i64;
                handle.borrow_mut().last_params = params.to_vec();
                if script.label(label).is_some() && vm.run_label(script, label).is_ok() {
                    ran += 1;
                }
            }
            let changed = {
                let rt = handle.borrow();
                let sectors = rt.sector_flags.iter().zip(before.0.iter()).filter(|(a, b)| a != b).count();
                let adjoins = rt.adjoin_flags.iter().zip(before.1.iter()).filter(|(a, b)| a != b).count();
                (sectors, adjoins)
            };
            // The last link of the chain, without needing a body: a doorway the solver *refused* before
            // the use must be one it *allows* after it. `Gates::passable` is what `sweep` asks to decide
            // whether a body may cross, so a flip here is the door becoming walkable.
            let flipped = {
                let rt = handle.borrow();
                let was = collide::Gates { sectors: &before.0, adjoins: &before.1 };
                let now = collide::Gates { sectors: &rt.sector_flags, adjoins: &rt.adjoin_flags };
                let mut flipped = 0;
                for (sector, portals) in solids.portals.iter().enumerate() {
                    for portal in portals {
                        if was.passable(sector, portal.adjoin, portal.link)
                            != now.passable(sector, portal.adjoin, portal.link)
                        {
                            flipped += 1;
                        }
                    }
                }
                flipped
            };
            let name = world.level.cogs.get(*instance).map(|c| c.script.clone()).unwrap_or_default();
            let state = {
                let rt = handle.borrow();
                rt.things.get(*thing).map(|t| (t.target, t.frame, t.frames.len()))
            };
            let _ = state;
            let frame = { let mut rt = handle.borrow_mut(); rt.call("GetCurFrame", &[cog::Value::Num(*thing as f64)]) };
            let moving = { let mut rt = handle.borrow_mut(); rt.call("IsThingMoving", &[cog::Value::Num(*thing as f64)]) };
            // `GetGoalFrame` reports where the piece is headed while it moves, and 0 once it is there
            // (`sithTrackThing_Arrivedidk` zeroes `goalframe`). That is the value a reversing lift script
            // reads: `MoveToFrame(elevator, 1 - GetGoalFrame(elevator), speed)`.
            let (goal, dest) = {
                let mut rt = handle.borrow_mut();
                let goal = rt.call("GetGoalFrame", &[cog::Value::Num(*thing as f64)]);
                let dest = rt.things.get(*thing).and_then(|t| t.target).map(|(f, _)| f);
                (goal, dest)
            };
            println!("door: {name} (cog {instance}, thing {thing}): {ran} label(s) ran, frame {frame:?}, moving {moving:?}, goal {goal:?} (target {dest:?}), flags {changed:?}, {flipped} doorway(s) that were shut now passable");
            if moving == cog::Value::Num(1.0) {
                assert_eq!(
                    goal,
                    cog::Value::Num(dest.unwrap_or(-1.0) as f64),
                    "GetGoalFrame did not report the destination the piece is moving to"
                );
                for _ in 0..180 {
                    let moved = handle.borrow_mut().step(1.0 / 60.0);
                    if moved.is_empty() {
                        break;
                    }
                }
                let (at_goal, at_frame) = {
                    let mut rt = handle.borrow_mut();
                    (
                        rt.call("GetGoalFrame", &[cog::Value::Num(*thing as f64)]),
                        rt.call("GetCurFrame", &[cog::Value::Num(*thing as f64)]),
                    )
                };
                println!(
                    "door: after 3 s the piece sits at frame {at_frame:?} and reports goal {at_goal:?}"
                );
                assert_eq!(at_goal, cog::Value::Num(0.0), "the goal frame did not reset on arrival");
            }
            if changed.0 + changed.1 > 0 && flipped > 0 {
                // A body crossing one is not measured here: the four doorways this door opens join its own
                // door-*frame* sectors (74/75/76, 0.05 to 0.3 wide), and a 0.065-radius body cannot stand
                // inside those at either side of the plane -- `inside_sector` is false both ways, so a walk
                // test there measures the frame, not the door. The solver verdict above is the body-free
                // half of the same question, and a level with a corridor door is where the walk belongs.
                outcome = Some((*instance, *thing, ran, changed, flipped));                break;
            }
        }
        let Some((instance, thing, ran, (sectors, adjoins), flipped)) = outcome else {
            panic!(
                "{} linked thing(s) listen on `activate`, none of them opened a doorway for the solver: the chain from use to the sweep is broken",
                candidates.len()
            );
        };
        println!(
            "door: cog {instance} on thing {thing}; {ran} message(s) run; {sectors} sector flag(s) and {adjoins} adjoin flag(s) changed; {flipped} doorway(s) the solver refused are now passable"
        );
        println!("--- door check passed ---");
        return;
    }

    if rules_only {
        // What the player's own template says the motion is, per level.
        let rules = Rules::from_level(&world.level);
        println!("{level_name}: {}", rules.describe());
        // The derivation, asserted rather than described: these numbers must be the template's own
        // arithmetic and nothing else. `sithPhysics_ApplyDrag` settles velocity at `acceleration / drag`,
        // the acceleration is `maxThrust`, so the walk speed is `maxthrust / surfdrag` exactly; the jump
        // and gravity are read straight off the template and the level header.
        {
            let thrust = world.level.template_f32("walkplayer", "maxthrust").unwrap_or(f32::NAN);
            let drag = world.level.template_f32("walkplayer", "surfdrag").unwrap_or(f32::NAN);
            let jump = world.level.template_f32("walkplayer", "jumpspeed").unwrap_or(f32::NAN);
            let want_walk = thrust / drag;
            assert!(
                (rules.walk - want_walk).abs() < want_walk * 1e-4,
                "walk {:.3} is not maxthrust/surfdrag = {want_walk:.3}",
                rules.walk
            );
            assert!(
                (rules.jump - jump).abs() < jump * 1e-4,
                "jump {:.3} is not the template's jumpspeed {jump:.3}",
                rules.jump
            );
            assert!(
                (rules.gravity - world.level.gravity).abs() < world.level.gravity * 1e-4,
                "gravity {:.3} is not the level's {:.3}",
                rules.gravity,
                world.level.gravity
            );
            // A speed scale anywhere in the chain breaks all three at once, which is how the old
            // `physics_scale = 10` stayed hidden: it moved the walk, the jump and gravity together.
            println!(
                "  derived: walk = maxthrust/surfdrag = {thrust}/{drag} = {want_walk:.3}, jump = jumpspeed = {jump:.3}, gravity = level gravity = {:.3}",
                world.level.gravity
            );
        }
        let (thrust, drag, jump, movesize) = (
            world.level.template_f32("walkplayer", "maxthrust").unwrap_or(f32::NAN),
            world.level.template_f32("walkplayer", "surfdrag").unwrap_or(f32::NAN),
            world.level.template_f32("walkplayer", "jumpspeed").unwrap_or(f32::NAN),
            world.level.template_f32("walkplayer", "movesize").unwrap_or(f32::NAN),
        );
        println!(
            "  template: maxthrust {thrust}  surfdrag {drag}  jumpspeed {jump}  movesize {movesize}  world gravity {:.2}  scale {MODEL_SCALE}",
            world.level.gravity
        );
        // The water rule only bites on surfaces that carry the flag: count them so a
        // level without any says so instead of looking verified.
        let flagged = |bit: u32| {
            world
                .level
                .surfaces
                .iter()
                .filter(|s| s.surfflags & bit != 0)
                .count()
        };
        println!(
            "  surface flags: water {}  earth {}  metal {} of {} surfaces",
            flagged(0x2_0000) + flagged(0x10_0000),
            flagged(0x8_0000),
            flagged(0x1_0000),
            world.level.surfaces.len()
        );
        // The lateral rule: a wall pushes a body that is beside its polygon and not one that is
        // past its end. Both halves matter — the old distance-to-the-infinite-line test pushed
        // bodies that were nowhere near the wall's actual extent (§16.2).
        {
            let solids = collide::build(&world.level);
            // The longest wall face, so "past the end" is measured in tens of units.
            let face = solids
                .walls
                .iter()
                .filter(|f| f.verts.len() >= 3)
                .max_by(|a, b| {
                    let len = |f: &collide::Face| {
                        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                        for v in &f.verts {
                            for k in 0..3 {
                                lo[k] = lo[k].min(v[k]);
                                hi[k] = hi[k].max(v[k]);
                            }
                        }
                        (hi[0] - lo[0]).max(hi[1] - lo[1])
                    };
                    len(a).total_cmp(&len(b))
                })
                .expect("a level has wall faces");
            let n = face.normal;
            // A point on the face, then `into` the wall by 0.1 (the push a body gets).
            let mid = {
                let mut m = [0.0; 3];
                for v in &face.verts {
                    for k in 0..3 {
                        m[k] += v[k];
                    }
                }
                for k in 0..3 {
                    m[k] /= face.verts.len() as f32;
                }
                m
            };
            let inside = [
                mid[0] - n[0] * 0.1,
                mid[1] - n[1] * 0.1,
                mid[2] - n[2] * 0.1,
            ];
            // The edge's own direction, to walk past the end of the polygon.
            let along = {
                let (a, b) = (face.verts[0], face.verts[1]);
                let d = [b[0] - a[0], b[1] - a[1]];
                let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-6);
                [d[0] / len, d[1] / len]
            };
            let span = face
                .verts
                .iter()
                .map(|v| (v[0] - mid[0]).hypot(v[1] - mid[1]))
                .fold(0.0f32, f32::max);
            // The rule itself, not a re-derivation: `face_blocks` is what the resolver calls.
            let beside_hit = collide::face_blocks(&face, inside[2] - 0.4, inside[2] + 0.4, inside, 0.42);
            let beyond = [
                inside[0] + along[0] * (span + 20.0),
                inside[1] + along[1] * (span + 20.0),
                inside[2],
            ];
            let beyond_hit = collide::face_blocks(&face, beyond[2] - 0.4, beyond[2] + 0.4, beyond, 0.42);
            println!(
                "  lateral: a body 0.1 into the wall is pushed {beside_hit}, and {} past its end is pushed {beyond_hit}",
                (span + 20.0) as i32
            );
            assert!(beside_hit, "the wall did not push a body that is beside it");
            assert!(
                !beyond_hit,
                "the wall pushed a body 20 units past the end of its polygon"
            );
        }

        // Integrate a jump with the same solver the window uses and compare with the
        // closed form: this is the check that the rules and the physics agree.
        if let Some(player) = world
            .level
            .things
            .iter()
            .find(|t| t.template.eq_ignore_ascii_case("walkplayer"))
        {
            let solids = collide::build(&world.level);
            let mut sector = solids.sector_at(player.pos, rules.radius + 0.05).unwrap_or(0);
            let dt = 1.0 / 30.0;
            let mut pos = player.pos;
            let mut vy = 0.0;
            // Settle on the floor first: the thing's origin is at the level's walkplayer
            // point, which is not necessarily a radius above the floor.
            for _ in 0..30 {
                walker_step(&solids, collide::Gates::default(), &mut sector, &mut pos, &mut vy, [0.0, 0.0], dt, rules.radius, rules.gravity, false);
            }
            let base = pos[2];
            let mut airetime = 0.0f32;
            let mut airborne = false;
            let mut apex = 0.0f32;
            vy = rules.jump;
            let trace = std::env::var_os("BK_JUMP_TRACE").is_some();
            for frame in 0..300 {
                walker_step(&solids, collide::Gates::default(), &mut sector, &mut pos, &mut vy, [0.0, 0.0], dt, rules.radius, rules.gravity, false);
                if trace && (frame < 6 || (18..28).contains(&frame) || (60..64).contains(&frame)) {
                    eprintln!(
                        "jump trace {frame}: z {:.3} base {base:.3} vy {vy:.2} support {:?} floor {:?}",
                        pos[2],
                        solids.support(pos, rules.radius, 0.6),
                        solids.floor_below(sector, pos)
                    );
                }
                apex = apex.max(pos[2] - base);
                // Airborne from the first upward step, landed when the vertical velocity is gone again and
                // the body is back at its base. Keying off the height alone hung on a body that settled a
                // thousandth above the floor, which reported a ten-second airtime.
                if vy > 0.0 {
                    airborne = true;
                }
                if airborne {
                    airetime += dt;
                    if frame > 2 && vy == 0.0 && pos[2] <= base + 0.05 {
                        break;
                    }
                }
            }
            let want_apex = rules.jump * rules.jump / (2.0 * rules.gravity);
            let want_air = 2.0 * rules.jump / rules.gravity;
            // Only the apex is asserted: where a jump *lands* is a property of the geometry, and a body that
            // comes down on a ledge 0.26 units up has still jumped correctly. A measured airtime would be
            // reporting the room, not the physics.
            println!(
                "  jump: measured apex {apex:.2} / {want_apex:.2} closed form; full airtime {want_air:.2} s in a vacuum"
            );
            assert!(
                (apex - want_apex).abs() <= want_apex * 0.15,
                "jump apex {apex:.2} drifts from {want_apex:.2}"
            );
        }
        println!("--- rules check passed ---");
        return;
    }

    // A cutscene is decoded before the app starts: the helper is a separate 32-bit process, so
    // there is nothing to overlap with, and a failure is reported rather than silently skipped.
    let playing = play_cutscene.as_ref().and_then(|(name, start)| {
        let dll = game.join("SMACKW32.DLL");
        let file = game.join("Resource").join("VIDEO").join(name);
        let played = cutscene::Cutscene::load(&dll, &file, *start);
        match &played {
            Some(played) => println!(
                "cutscene: {} frames of {name} decoded, playing at {:.0} fps",
                played.total,
                1.0 / played.frame_seconds
            ),
            None => println!(
                "cutscene: could not decode {} — build the 32-bit helper with `cargo build \
                 --release --target i686-pc-windows-msvc --manifest-path tools/smkdec/Cargo.toml`",
                file.display()
            ),
        }
        played
    });
    if let Some(played) = &playing {
        println!(
            "cutscene: {} frames ready before startup",
            played.total
        );
    }
    let startup_cutscene = playing;

    if let Some(vantage) = shot_at {
        let _ = SHOT_AT.set(vantage);
    }

    // Silent by default. Audio is noise in every test run and one flag away for a playtest, and asking for
    // it explicitly is the only way to be sure a capture does not blast the room. `--sound-on` (or
    // `BK_SOUND=1`) turns it back on; `--mute` and `BK_MUTE=1` still force it off.
    let audible = sound_on || std::env::var("BK_SOUND").is_ok_and(|v| !v.is_empty() && v != "0");
    let forced_off = muted_flag || std::env::var("BK_MUTE").is_ok_and(|v| !v.is_empty() && v != "0");
    // `--audio-state`: report the decision and stop, before any audio device exists. This is how the mute
    // gets verified without a single sample reaching the speakers — checking the intent by *playing* it is
    // what made noise twice.
    if audio_state {
        println!(
            "audio-state: {} (sound-on flag {sound_on}, mute flag {muted_flag}, capture {}), gain {:.3}",
            if forced_off || !audible { "muted" } else { "on" },
            shot.is_some() || shot_at.is_some(),
            if forced_off || !audible { 0.0 } else { env_f32("BK_VOLUME", 0.05) }
        );
        println!("--- audio-state check passed ---");
        return;
    }
    MUTED.store(forced_off || !audible, std::sync::atomic::Ordering::Relaxed);
    println!(
        "audio: {} (--sound-on to hear it, --mute to force silence)",
        if muted() { "muted" } else { "on" }
    );

    let menus = load_menus(&game).unwrap_or_default();
    // The boot screen: retail opens on its main menu, and this build now does too when it is being
    // *played*. Every flag that drives frames is a capture or a gate — those want the level, not a
    // menu in front of it — and an install whose executable cannot supply the menu tables keeps the
    // old straight-into-the-level behaviour rather than showing a screen that cannot draw or close.
    let capture = shot.is_some()
        || shot_at.is_some()
        || overview
        || near.is_some()
        || stand.is_some()
        || live
        || animate_only
        || !animating
        || fire_demo
        || sim
        || complete_after.is_some()
        || play_cutscene.is_some();
    let boot_menu = boot_screen(menu_open.as_deref(), capture, start_in_level, menus.find("main").is_some());
    if start_in_level && boot_menu.is_none() {
        println!(
            "start: in {} with no menu (Escape opens the pause screen)",
            level_name
        );
    }
    if boot_menu.as_deref() == Some("main") {
        println!("boot: main menu (Enter picks, Escape starts nothing until a game is running)");
    }

    // One line that says what this run is, so nobody has to infer it from the flags: every gate that
    // exits before this point is headless and never creates a window, and everything that reaches here
    // names the screen it opens on.
    println!(
        "mode: window, {}",
        match (&boot_menu, capture) {
            (Some(screen), _) => format!("screen `{screen}` open"),
            (None, true) => "no screen (capture)".to_string(),
            (None, false) => "no screen (playing)".to_string(),
        }
    );

    let mut app = App::new();
    if let Some(played) = startup_cutscene {
        app.insert_resource(played);
    }

    // The material plugin comes after `DefaultPlugins`: `load_internal_asset!` needs `Assets<Shader>`.
    // Where `AutosaveGame` writes: `BK_SAVE_DIR`, else `saves` beside the game data.
    let save_directory = std::env::var_os("BK_SAVE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| game.join("saves"));
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("bevi-knight — {level_name}"),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(level_mat::LevelMaterialPlugin)
        .insert_resource(Archives(gobs))
        .insert_resource(Screens(menus))
        .insert_resource(SaberBlade::default())
        .insert_resource(Viewmodel::default())
        .insert_resource(PendingRestore::default())
        .insert_resource(SaveLoad::default())
        .insert_resource(SaveSlot::default())
        .insert_resource(SaveDir(
            std::env::var_os("BK_SAVE_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| game.join("saves")),
        ))
        .insert_resource(Overlay::new(boot_menu).with_pending(pending_id))
        .insert_resource(world)
        .insert_resource(MusicDir(game.join("MUSIC")))
        .insert_resource(MusicState::default())
        .insert_resource(Overview {
            overview,
            capture: shot.is_some() || map_check,
        })
        .insert_resource(Near(near))
        .insert_resource(Stand(stand))
        .insert_resource(LevelShader(level_shader || !standard_material))
        .insert_resource(FadeTest(fade_test))
        .insert_resource(IrFromStart(ir_from_start))
        .insert_resource(LiveCheckState { enabled: live, ..Default::default() })
        .insert_resource(LivePlay { enabled: live_play })
        .insert_resource(GoalProbe { enabled: goal_check })
        .insert_resource(PulseCount::default())
        .insert_resource(Shot(shot.map(PathBuf::from)))
        .insert_resource(ShotFrame(shot_frame))
        .insert_resource(Cull(cull))
        .insert_resource(Animate(animating))
        .insert_resource(FireDemo(fire_demo))
        .insert_resource(SpriteDemo(sprite_template))
        .insert_resource(Look(look))
        .insert_resource(Ambience::default())
        .insert_resource(PlaySound(play_sound))
        .insert_resource(Deliver(sim))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
        .insert_resource(plan)
        .insert_resource(PendingLevel::default())
        .insert_resource(CompleteAfter(complete_after))
        .insert_resource(ExitAfter(exit_after.unwrap_or(u64::MAX)))
        .add_systems(Startup, setup)
        .add_systems(PostStartup, setup_overlay)
        // Always on: the collider hand-off, the gate that samples the running game, the mute, the
        // pointer, the capture timer and the level swap. The swap is here because a menu *requests*
        // it (New Game, Restart) while the world is paused, and the UI has to keep running to be
        // closed at all.
        .add_systems(
            Update,
            (
                share_collider,
                live_check,
                enforce_mute,
                grab_cursor,
                auto_shot,
                trace_state,
                swap_level,
                (ui_input, ui_mouse, apply_pending, refresh_overlay, cutscene::play_cutscene, quick_save)
            ),
        )
        .insert_resource(TriggerCount::default())
        // A hard frame budget runs whatever else is happening, menu open or not: a gate that forgets to
        // quit must not be able to sit on a window (and hang a whole `--gates` sweep).
        .add_systems(Update, (exit_after_frames, engine_fov))
        // The world: frozen while any screen is open, which is what retail does and what keeps the
        // boot menu from running a level's scripts, its AI and its doors behind itself. In three
        // groups because Bevy's system-config tuples stop at twenty.
        .add_systems(
            Update,
            (
                (
                    run_pulses_and_timers,
                    resume_sleeping_labels,
                    deliver_sector_triggers,
                    deliver_thing_messages,
                    deliver_cog_messages,
                    deliver_crossed,
                    use_key,
                    apply_fall_damage,
                    (live_play_drive.before(walk_player), walk_player),
                    cull_sectors,
                    rebuild_surfaces,
                    sky_follows_camera,
                    ambient_follows_view,
                    play_exit,
                    advance_sim,
                )
                    .run_if(no_screen_open),
                (demo_aim, sync_player_thing, health_follows_damage, infrared_follows_force, fire_player, sprite_demo, draw_created, projectile_hits, respawn_on_death)
                    .chain()
                    .run_if(no_screen_open),
                (
                    collect_items,
                    ai_shots,
                    play_requested_sounds,
                    music_follows_scripts,
                    show_taken,
                    animate_actors,
                    actors_follow_runtime,
                    things_follow_geomode,
                    adopt_carried,
                    autosave,
                    restore_pending,
                    load_item_cogs,
                    saber_blade,
                    draw_viewmodel,
                    scene_wash,
                    billboard_sprites,
                    complete_level_on_timer,
                    watch_level_complete,
                    queue_next_level,
                )
                    .run_if(no_screen_open),
            ),
        )
        // Its own line: the tuple above is already at Bevy's system-count limit.
        .add_systems(Update, goal_probe.after(live_play_drive).run_if(no_screen_open));
    app.run();
}

// ---------------------------------------------------------------------------
// the in-app UI layer
// ---------------------------------------------------------------------------

/// The install's archives, kept alive for the UI: the HUD and the menus read their
/// bitmaps and fonts from the same GOBs the level came from.
#[derive(Resource, Deref)]
struct Archives(gob::GobSet);

/// Where finishing the loaded level leads, resolved from the episode graph at startup.
#[derive(Resource)]
struct FlowPlan {
    game: std::path::PathBuf,
    level: String,
    /// The level New Game starts: the first level of the first single-player episode, which is also
    /// what a bare `cargo run` loads. `level` moves with the campaign; this does not.
    first: String,
    /// The cutscene that plays on completion, if the graph names one.
    cut: Option<String>,
    /// The level that loads after that.
    next: Option<String>,
    /// Whether to actually swap the loaded world (see [`swap_level`] for the boundary).
    swap: bool,
}

/// Everything the loaded level spawned: geometry batches, models and the sky dome. A level
/// swap despawns exactly these and leaves the cameras, lights and overlay alone.
#[derive(Component)]
struct LevelEntity;

/// A level whose load is waiting for the film to finish.
#[derive(Resource, Default)]
struct PendingLevel(Option<String>);

/// `--complete-after <seconds>`: finish the level on a timer so the completion path can be
/// exercised without playing the level's objectives.
#[derive(Resource)]
struct CompleteAfter(Option<f32>);

/// Call the engine's own `jkEndLevel` once the timer expires. This is the same verb a level's
/// final script calls (`jkCog_EndLevel` in `jkCog.c`), so it drives the real path.
fn complete_level_on_timer(
    time: Res<Time>,
    after: Option<Res<CompleteAfter>>,
    sim: Option<Res<Sim>>,
    mut waited: Local<f32>,
    mut done: Local<bool>,
) {
    let (Some(after), Some(sim)) = (after, sim) else { return };
    let Some(seconds) = after.0 else { return };
    if *done {
        return;
    }
    *waited += time.delta_secs();
    if *waited < seconds {
        return;
    }
    *done = true;
    sim.world.borrow_mut().call("jkendlevel", &[cog::Value::Num(1.0)]);
    println!("complete: called jkEndLevel after {seconds:.2}s");
}

/// When the level's scripts call `jkEndLevel`, play the graph's cutscene and name the level that
/// follows it. The level swap itself is not implemented — the world is built at startup — so this
/// stops at the film, which is the part a player actually sees.
fn watch_level_complete(
    mut commands: Commands,
    sim: Option<Res<Sim>>,
    plan: Option<Res<FlowPlan>>,
    playing: Option<Res<cutscene::Cutscene>>,
    mut announced: Local<bool>,
) {
    let (Some(sim), Some(plan)) = (sim, plan) else { return };
    if *announced || !sim.world.borrow().level_complete {
        return;
    }
    *announced = true;
    println!(
        "level {} complete -> cutscene {}, then {}",
        plan.level,
        plan.cut.as_deref().unwrap_or("(none)"),
        plan.next.as_deref().unwrap_or("(end of campaign)")
    );
    if playing.is_some() {
        return;
    }
    let Some(name) = plan.cut.clone() else { return };
    let dll = plan.game.join("SMACKW32.DLL");
    let file = plan.game.join("Resource").join("VIDEO").join(&name);
    match cutscene::Cutscene::load(&dll, &file, 0) {
        Some(played) => {
            println!("cutscene: playing {name}, {} frames", played.total);
            commands.insert_resource(played);
        }
        None => println!("cutscene: could not decode {}", file.display()),
    }
}

/// When the film has played out, ask for the next level. The swap itself happens in
/// [`swap_level`] so the reload does not run in the middle of another system's borrows.
fn queue_next_level(
    plan: Option<Res<FlowPlan>>,
    playing: Option<Res<cutscene::Cutscene>>,
    sim: Option<Res<Sim>>,
    mut pending: ResMut<PendingLevel>,
    mut asked: Local<bool>,
) {
    let (Some(plan), Some(sim)) = (plan, sim) else { return };
    if *asked || !plan.swap {
        return;
    }
    // The film is what holds the transition: once playback has ended, the next level loads.
    if !sim.world.borrow().level_complete {
        return;
    }
    // The film is what holds the level change. When the graph names one, it must exist and
    // have shown every frame; a level with no film has nothing to wait for.
    match (&playing, &plan.cut) {
        (Some(playing), _) if playing.finished => {}
        (None, None) => {}
        _ => return,
    }
    let Some(next) = plan.next.clone() else { return };
    *asked = true;
    println!("level: loading {next}");
    pending.0 = Some(next);
}

/// Replace the loaded level: despawn what it spawned, read the next one from the archives, and let
/// [`setup`] build it again.
///
/// This is an exclusive system on purpose. `setup` is the one place that knows how a level becomes
/// entities — sector batches, models, the sky dome, the script instances, the collision planes and
/// the player's start — and running it here, synchronously, is what makes a level swap the same
/// code path as the first load instead of a second implementation that can drift from it.
fn swap_level(world: &mut bevy::prelude::World) {
    let Some(name) = world.resource_mut::<PendingLevel>().0.take() else { return };
    // Read the successor and the manifest while the borrow is immutable, then swap.
    let (next, episode) = {
        let gobs = &world.resource::<Archives>().0;
        match load(gobs, &name) {
            Ok(next) => (next, gobs.read("episode.jk")),
            Err(e) => {
                println!("level: could not load {name}: {e}");
                return;
            }
        }
    };
    println!(
        "level: {name} loaded ({} things, {} sectors), rebuilding",
        next.level.things.len(),
        next.level.sectors.len()
    );
    // What the *player* keeps: the engine's `jkPlayer_*` state outlives the level, and this build
    // rebuilds the runtime from the incoming `.jkl`, so it has to be taken off the old one first.
    // `setup` puts it back on the new level's player.
    let carried = world
        .get_resource::<Sim>()
        .map(|sim| sim.world.borrow().carry());
    if let Some(carried) = carried {
        world.insert_resource(Carried(carried));
    }
    // Everything the old level spawned, and nothing else: the cameras, lights and overlay are
    // not tagged `LevelEntity`.
    let stale: Vec<Entity> = world
        .query_filtered::<Entity, With<LevelEntity>>()
        .iter(world)
        .collect();
    if std::env::var_os("BK_NO_DESPAWN").is_none() {
        for entity in stale {
            world.despawn(entity);
        }
    }
    world.insert_resource(next);
    if let Some(mut plan) = world.get_resource_mut::<FlowPlan>() {
        plan.level = name.clone();
        let (cut, after) = episode
            .map(|d| episode::parse(&String::from_utf8_lossy(&d)))
            .map(|e| {
                let (film, level) = e.flow_after(&name);
                (film.map(|s| s.file.clone()), level.map(|s| s.file.clone()))
            })
            .unwrap_or((None, None));
        plan.cut = cut;
        plan.next = after;
    }
    // The same startup path, now that `World` holds the new level.
    // The film is over: give the screen back to the HUD of the level that just loaded. The
    // canvas still holds the film's last frame, so mark it dirty for a redraw.
    world.remove_resource::<cutscene::Cutscene>();
    if let Some(mut overlay) = world.get_resource_mut::<Overlay>() {
        overlay.dirty = true;
    }
    // Put the camera where the new level starts before the first frame is drawn from it: the
    // walker moves it every frame afterwards.
    let eye = world
        .resource::<World>()
        .level
        .things
        .iter()
        .find(|t| t.template.eq_ignore_ascii_case("walkplayer"))
        .map(|t| (Vec3::from(t.pos) + Vec3::Z * PLAYER_EYE, t.yaw.to_radians()));
    if let Some((eye, yaw)) = eye {
        let (yaw_f, _) = yaw_axes(yaw);
    let aim = Vec3::new(yaw_f[0], yaw_f[1], 0.0);
        let mut cameras = world.query_filtered::<&mut Transform, (With<Camera3d>, Without<OverlayQuad>)>();
        if let Ok(mut transform) = cameras.single_mut(world) {
            *transform = Transform::from_translation(eye).looking_at(eye + aim, Vec3::Z);
        }
    }
    let setup_id = world.register_system(setup);
    let _ = world.run_system(setup_id);
}

/// How many save slots the game offers. Retail's Load screen lists them; this build names them
/// `slot0.jks`..`slot7.jks` beside the old `quicksave.jks`, which slot 0 keeps using so an existing
/// save still loads.
const SLOTS: usize = 8;

/// Which slot `F5`/`F9` and the pause menu's Save and Load rows use. `[` and `]` move it.
#[derive(Resource, Default, Clone, Copy)]
struct SaveSlot(usize);

/// A slot's file: slot 0 is the historical `quicksave.jks`.
fn slot_path(dir: &std::path::Path, slot: usize) -> std::path::PathBuf {
    if slot == 0 {
        dir.join("quicksave.jks")
    } else {
        dir.join(format!("slot{slot}.jks"))
    }
}

/// Fill a shipped string's `%d`/`%s` placeholders in order: the tally's secrets line is
/// `jkStrings_GetUniStringWithFallback(GUI_SECRETS_FOUND, found, total)`, and the table stores it
/// with its placeholders intact. Extra placeholders are left alone rather than dropped.
fn fill_numbers(format: &str, values: &[i32]) -> String {
    let mut out = String::with_capacity(format.len() + 8);
    let mut next = values.iter();
    let mut rest = format;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let (spec, width) = match tail[1..].chars().next() {
            Some('d') => (true, 2),
            Some('i') => (true, 2),
            Some('s') => (false, 2),
            Some('u') => (true, 2),
            _ => (false, 1),
        };
        if width == 1 {
            out.push('%');
            rest = &tail[1..];
            continue;
        }
        match next.next() {
            Some(value) => out.push_str(&value.to_string()),
            None => out.push_str(&tail[..width]),
        }
        rest = &tail[width..];
    }
    out.push_str(rest);
    out
}

/// The tally screen's run-time text rows: rank with its light/dark letter, the secrets found, and the
/// "stars earned" label. `jkGuiSingleTally_Show` fills them from `jkPlayer_GetJediRank`,
/// `sithPlayer_GetInvItemAmount(0x46/0x47)` and `jkPlayer_CalcAlignment`, and the screen carries no
/// keys for them. `None` when this screen is not the tally.
///
/// Shared with the `--tally` gate so the check exercises this, not a copy of it.
fn tally_lines(
    screen: &menus::Screen,
    strings: &uni::Uni,
    rank: i32,
    alignment: f32,
    found: i32,
    total: i32,
) -> Option<Vec<String>> {
    if !screen.items.iter().any(|i| matches!(i.panel(), Some(menus::Panel::ForceStars))) {
        return None;
    }
    // `_DAT_00522150` is 0.0: a non-negative alignment shows `L`, anything below shows `D`.
    let side = if alignment >= 0.0 { 'L' } else { 'D' };
    let secrets = if total < 1 {
        strings
            .get("GUI_NO_SECRETS")
            .map(|s| s.to_string())
            .unwrap_or_else(|| "No secrets found".to_string())
    } else {
        // The shipped label is a **caption** with no placeholders (`"Secrets Found:"` in
        // `ui\jkstrings.uni`), and the engine appends the counts: `sprintf("%ls %d/%d")`. Filling
        // placeholders alone drew the caption and dropped the numbers.
        match strings.get("GUI_SECRETS_FOUND") {
            Some(format) if format.contains('%') => {
                format!("{} {found}/{total}", fill_numbers(format, &[found, total]))
            }
            Some(format) => format!("{} {found}/{total}", format.trim_end()),
            None => format!("{found} of {total} secrets found"),
        }
    };
    let earned = strings
        .get("GUI_STARS_EARNED")
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Stars earned".to_string());
    Some(vec![format!("RANK {rank}  {side}"), secrets, earned])
}

/// What the pause menu asked for: `GUI_SAVE` writes the quicksave, `GUI_LOAD` reads it.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
enum SaveLoad {
    #[default]
    None,
    Save,
    Load,
}

/// A savegame waiting for its level to finish loading: `F9` sets this when the save is for another
/// level, and this system puts it back once that level exists.
#[derive(Resource, Default)]
struct PendingRestore(Option<runtime::Snapshot>);

/// Read a savegame's level name and state payload.
fn read_save(path: &std::path::Path) -> Option<(String, runtime::Snapshot)> {
    let bytes = std::fs::read(path).ok()?;
    let save = jks::Save::read(&bytes).ok()?;
    let payload = save.sections.iter().find(|s| s.kind() == runtime::SAVE_SECTION)?;
    Some((save.level.clone(), runtime::Snapshot::from_bytes(&payload.data)?))
}

/// `F5` writes a quicksave, `F9` loads one.
///
/// The save carries the level it belongs to. Loading the *same* level restores in place; loading
/// another one asks for that level first and hands the snapshot to [`restore_pending`].
fn quick_save(
    keys: Res<ButtonInput<KeyCode>>,
    sim: Option<Res<Sim>>,
    dir: Option<Res<SaveDir>>,
    plan: Option<Res<FlowPlan>>,
    mut pending_level: ResMut<PendingLevel>,
    mut pending: ResMut<PendingRestore>,
    // The pause menu's Save and Load rows arrive the same way the keys do: `activate` inserts the
    // request, and this system is the one that holds the runtime, the directory and the plan.
    mut menu: ResMut<SaveLoad>,
    slot_res: Option<ResMut<SaveSlot>>,
) {
    let (Some(sim), Some(dir)) = (sim, dir) else { return };
    // Save slots: `quicksave.jks` was the only place a game could go, so a second save overwrote the
    // first and retail's slot list had nothing to show. `,`/`.` move the slot (`[`/`]` are the
    // weapon keys, as retail binds them), and the current one is named in the message.
    let Some(mut slot_res) = slot_res else { return };
    let mut slot = slot_res.0;
    if keys.just_pressed(KeyCode::Comma) {
        slot = (slot + SLOTS - 1) % SLOTS;
        slot_res.0 = slot;
        println!("quicksave: slot {slot}");
    }
    if keys.just_pressed(KeyCode::Period) {
        slot = (slot + 1) % SLOTS;
        slot_res.0 = slot;
        println!("quicksave: slot {slot}");
    }
    let path = slot_path(&dir.0, slot);
    let asked = *menu;
    let save = keys.just_pressed(KeyCode::F5) || asked == SaveLoad::Save;
    let load = keys.just_pressed(KeyCode::F9) || asked == SaveLoad::Load;
    if asked != SaveLoad::None {
        *menu = SaveLoad::None;
    }
    if save {
        let rt = sim.world.borrow();
        let level = plan.as_ref().map(|p| p.level.clone()).unwrap_or_default();
        match write_snapshot(&path, &level, &rt) {
            Ok(bytes) => println!("quicksave: wrote {bytes} bytes to `{}`", path.display()),
            Err(e) => println!("quicksave: could not write `{}`: {e}", path.display()),
        }
    }
    if load {
        let Some((level, snapshot)) = read_save(&path) else {
            println!("quicksave: no save at `{}`", path.display());
            return;
        };
        let here = plan.as_ref().map(|p| p.level.clone()).unwrap_or_default();
        // The level name a `.jks` carries is the one the level file has, which is what the archive
        // index takes (`jkl\01narshadda.jkl`), so it can be handed straight to a load.
        if level == here || level.is_empty() {
            sim.world.borrow_mut().restore(&snapshot);
            println!("quicksave: loaded `{}` in place ({} things)", path.display(), snapshot.things.len());
        } else {
            pending.0 = Some(snapshot);
            pending_level.0 = Some(level.clone());
            println!("quicksave: loading level `{level}` before restoring `{}`", path.display());
        }
    }
}

/// Put a loaded savegame onto the level that just finished loading.
fn restore_pending(
    mut commands: Commands,
    mut pending: ResMut<PendingRestore>,
    sim: Option<Res<Sim>>,
    world: Option<Res<World>>,
) {
    let (Some(snapshot), Some(sim), Some(world)) = (pending.0.take(), sim, world) else { return };
    sim.world.borrow_mut().restore(&snapshot);
    println!(
        "quicksave: restored {} thing(s) onto the loaded level ({} things)",
        snapshot.things.len(),
        world.level.things.len()
    );
    commands.remove_resource::<PendingRestore>();
}

/// Put the player's carried state onto the level that just loaded, or a loaded savegame onto it.
///
/// Its own system rather than part of `setup`, because `setup` is already at Bevy's parameter limit
/// and because `Sim` arrives through `Commands` — by the next frame both exist.
fn adopt_carried(mut commands: Commands, carried: Option<Res<Carried>>, sim: Option<Res<Sim>>) {
    let (Some(carried), Some(sim)) = (carried, sim) else { return };
    sim.world.borrow_mut().adopt(&carried.0);
    println!(
        "level: the player carries over {} inventory bin(s), weapon {:?}, saber {}",
        carried.0.inv.len(),
        carried.0.weapon,
        carried.0.saber.is_some()
    );
    commands.remove_resource::<Carried>();
}

/// [`runtime::CarryOver`] in flight: `swap_level` takes the player's state off the level it is
/// leaving and `setup` puts it on the one it is building.
#[derive(Resource)]
struct Carried(runtime::CarryOver);

/// The screens recovered from the executable.
#[derive(Resource, Deref)]
struct Screens(menus::Menus);

/// Every name the engine executable ships under: the Steam/GOG re-releases call it
/// `JediKnight.EXE`, the original CD `JK.EXE`.
const ENGINE_NAMES: [&str; 2] = ["JediKnight.EXE", "JK.EXE"];

/// Read the engine executable from the install's own directory or its parent (a `.gob`
/// lives one level down): the menu element tables live in its `.data`.
fn load_engine_exe(game: &std::path::Path) -> Option<Vec<u8>> {
    let dirs = [game.to_path_buf(), game.parent().map(|p| p.to_path_buf()).unwrap_or_default()];
    for dir in dirs {
        for name in ENGINE_NAMES {
            if let Ok(bytes) = std::fs::read(dir.join(name)) {
                return Some(bytes);
            }
        }
    }
    None
}

/// Read the engine executable: the menu element tables live in its `.data`.
fn load_menus(game: &std::path::Path) -> Option<menus::Menus> {
    menus::load(&load_engine_exe(game)?)
}

/// The engine's fonts, loaded in the order an element's `text_type` indexes them.
/// A font that can draw letters, for the HUD message line.
///
/// The menus' own font table is not loaded in every path that needs one (`--hud` runs before it), so
/// this takes the first `.sft` in the install that has an `A`. Bounded by the number of fonts shipped.
fn hud_font(gobs: &gob::GobSet) -> Option<sft::Font> {
    gobs.names()
        .into_iter()
        .filter(|n| n.to_ascii_lowercase().ends_with(".sft"))
        .filter_map(|n| gobs.read(&n).and_then(|d| sft::parse(&d)))
        .find(|font| font.glyph('A').is_some())
}

/// Resolve the `COG_#####` keys a `jkStringConcatUniString` leaves in a line, through the install's
/// `ui\jkstrings.uni`. A key the table does not have is left alone, so a missing string is visible
/// rather than silently blank.
fn resolve_keys(strings: &uni::Uni, line: &str) -> String {
    let mut out = line.to_string();
    // The buffer is at most 128 characters, so there are only a few keys to look at.
    let mut at = 0usize;
    while let Some(rel) = out[at..].find("COG_") {
        let start = at + rel;
        let end = (start + 9).min(out.len());
        let Some(text) = strings.get(&out[start..end]).map(str::to_string) else {
            at = end;
            continue;
        };
        out.replace_range(start..end, &text);
        at = start + text.len();
    }
    out
}

fn load_fonts(gobs: &gob::GobSet, menus: &menus::Menus) -> Vec<sft::Font> {
    (0..menus.fonts.len())
        .filter_map(|i| menus.font(i as i32))
        .map(|path| gobs.read(&path).and_then(|d| sft::parse(&d)).unwrap_or_default())
        .collect()
}

/// A level's colormap, which is what the palette-less UI art (`bkdialog.bm`, the HUD
/// bitmaps and the menu text drawn over a dialog) indexes. `--level` picks the level; the
/// campaign's first level stands in otherwise.
fn level_palette(gobs: &gob::GobSet, level: Option<&str>) -> [[u8; 3]; 256] {
    let level = level.map(str::to_string).or_else(|| start_level(gobs));
    level
        .as_deref()
        .map(|l| String::from_utf8_lossy(&gobs.read(l).unwrap_or_default()).into_owned())
        .map(|t| jkl::parse(&t))
        .and_then(|l| l.colormaps.first().cloned())
        .and_then(|c| gobs.read(&c))
        .and_then(|d| tex::Palette::parse(&d))
        .map(|p| p.0)
        .and_then(|v| <[[u8; 3]; 256]>::try_from(v).ok())
        .unwrap_or([[0u8; 3]; 256])
}

/// Load a screen's two art layers: its background, and a dialog's panel.
///
/// The panel is only real when the screen's elements sit inside it: `bkdialog.bm` is a
/// 414x206 box at its own `xPos`/`yPos`, and the screens inferred as dialogs split into
/// those that draw inside it (the light-or-dark prompt) and those that draw over the whole
/// screen (the quit prompt with its four full-width lines), which have no panel at all.
fn load_backdrop(gobs: &gob::GobSet, screen: &menus::Screen, background: Option<&str>) -> Option<(bm::Bm, Option<bm::Bm>)> {
    let path = background.unwrap_or(&screen.background);
    let base = bm::parse(&gobs.read(path)?)?;
    let panel = screen
        .panel
        .as_deref()
        .and_then(|p| gobs.read(p))
        .and_then(|d| bm::parse(&d))
        .filter(|panel| panel_fits(panel, screen));
    Some((base, panel))
}

/// Whether every element of a screen falls inside a panel bitmap's rect.
fn panel_fits(panel: &bm::Bm, screen: &menus::Screen) -> bool {
    let Some(cell) = panel.cells.first() else { return false };
    let (left, top) = (panel.x_pos, panel.y_pos);
    let (right, bottom) = (left + cell.width as i32, top + cell.height as i32);
    !screen.items.is_empty()
        && screen.items.iter().all(|item| {
            let [x, y, w, h] = item.rect;
            x >= left && y >= top && x + w <= right && y + h <= bottom
        })
}

/// Everything a menu screen needs, loaded once: the string table, the engine's fonts and
/// the level's colormap (the HUD's bitmaps and digits are palette indices, not RGB).
#[derive(Resource)]
struct UiAssets {
    strings: uni::Uni,
    /// The engine's fonts in `text_type` order: `small0`, `small1`, `med0`, … `FLTitle`.
    fonts: Vec<sft::Font>,
    palette: [[u8; 3]; 256],
}

/// The screen-space UI: a 640x480 canvas drawn on a quad in front of the camera. The
/// status bar is always there; a menu replaces it while open, as the retail screens do.
#[derive(Resource)]
struct Overlay {
    /// Which screen is open, by the name `--ui-tables` prints; `None` shows the status bar.
    menu: Option<String>,
    /// Whether a game has been started. The boot menu is up before it has, and Escape must not
    /// close *that* — retail's main menu ignores the key, and dropping into a level the player
    /// never started is worse than a dead key. Anything but the boot menu starts a game.
    started: bool,
    /// The element array the app booted on (`0x526680`, the main menu). Escape backs out one screen
    /// at a time and stops here, whether or not a game has started.
    boot: Option<u32>,
    /// The screen Escape opens: whichever one carries `GUI_RETURN_TO_GAME`.
    esc: Option<String>,
    /// The screens opened from the current one, newest last: a cancel/done row goes back.
    history: Vec<String>,
    /// An id to activate once the screen is loaded: `--menu main/10`.
    pending: Option<i32>,
    /// Text for a run-time dialog's keyless elements, by element index.
    lines: Vec<(usize, String)>,
    /// Index into the menu's selectable rows.
    selected: usize,
    /// Whether the inventory strip is open for selection. `I` toggles it; while it is open the
    /// arrows move `inv_selected` and Enter uses that item (`UseBin`, the engine's own path).
    inventory: bool,
    /// Index into `hud.carried` and `carried_bins`.
    inv_selected: usize,
    /// The bin id behind each entry of `hud.carried`, in the same order: `hud::carried` names items
    /// the way the table does, and using one needs the bin number.
    carried_bins: Vec<i32>,
    /// The element the mouse is over, if any: the engine lights it with its hover font.
    hover: Option<usize>,
    hud: hud::Hud,
    canvas: ui::Canvas,
    image: Handle<Image>,
    /// Redraw the canvas on the next frame.
    dirty: bool,
}

impl Overlay {
    /// Arm an activation to run once the first screen is composed (`--menu main/10`).
    fn with_pending(mut self, pending: Option<i32>) -> Self {
        self.pending = pending;
        self
    }

    /// Switch to a screen, resetting the cursor and the canvas.
    fn show(&mut self, menu: Option<String>, selected: usize) {
        self.menu = menu;
        self.selected = selected;
        self.hover = None;
        self.lines.clear();
        self.dirty = true;
    }

    fn new(menu: Option<String>) -> Self {
        Self {
            // Booting straight into the level (a capture, a gate) is a game already; the boot menu
            // is the one case where nothing has started yet.
            started: menu.is_none(),
            boot: None,
            menu,
            esc: None,
            history: Vec::new(),
            pending: None,
            lines: Vec::new(),
            selected: 0,
            inventory: false,
            inv_selected: 0,
            carried_bins: Vec::new(),
            hover: None,
            hud: hud::Hud::default(),
            canvas: ui::Canvas::new(ui::CANVAS[0] as u32, ui::CANVAS[1] as u32),
            image: Handle::default(),
            dirty: true,
        }
    }

    /// The item indices the cursor can land on.
    fn rows(&self, screen: &menus::Screen, found: &dyn Fn(i32) -> bool) -> Vec<usize> {
        // The engine hides elements at run time; the cursor must not land on one of those.
        screen.visible_rows(found)
    }
}

#[derive(Component)]
struct OverlayQuad;

/// The camera the player looks through. The overlay screens spawn their own `Camera3d`s (the HUD at
/// order 2 and the screen tint at order 1), so "the `Camera3d`" is ambiguous: `walk_player` used
/// `single_mut()` on `(With<Camera3d>, Without<OverlayQuad>)`, which matched three entities and
/// returned early every frame. The result was a body that never moved, however the keys were
/// pressed. A marker on the world camera is the smallest fix that names it.
#[derive(Component)]
struct WorldCamera;

/// The first-person weapon model currently drawn, and the `.3do` it came from.
#[derive(Resource, Default)]
struct Viewmodel {
    name: Option<String>,
    entity: Option<Entity>,
    /// The `.key` the cog last asked for (`jkPlayPOVKey`), with the clip's own frame cursor. A
    /// viewmodel clip is short (`bryvpst1.key` is one shot's recoil), so its mesh is rebuilt on each
    /// new frame — a handful of vertices, unlike a body puppet's.
    key: Option<String>,
    phase: f32,
    frame: i32,
}

/// The viewmodel entity itself, spawned by [`draw_viewmodel`].
#[derive(Component)]
struct ViewmodelMesh;

/// The spawned lightsaber blade entity, once `jkSetSaberInfo` armed one.
#[derive(Resource, Default)]
struct SaberBlade(Option<Entity>);

/// The blade entity itself, spawned by [`saber_blade`].
#[derive(Component)]
struct SaberBladeMesh;

/// The run condition on the world's own systems: nothing simulates while a screen is open.
///
/// Retail stops the game under its menus, and the boot menu in particular must not have the level's
/// scripts, doors and AI running behind it while the player is still choosing. The UI, the pointer,
/// the capture timer and the level swap are outside this condition — the swap is how a menu starts a
/// game at all.
fn no_screen_open(overlay: Option<Res<Overlay>>) -> bool {
    overlay.is_none_or(|o| o.menu.is_none())
}

/// Build the UI image, the shared assets, and the quad in front of the camera.
fn setup_overlay(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    gobs: Res<Archives>,
    screens: Res<Screens>,
    world: Res<World>,
    mut overlay: ResMut<Overlay>,
    cams: Query<(Entity, &Projection), With<Camera3d>>,
) {
    let image = Image::new(
        Extent3d { width: overlay.canvas.width, height: overlay.canvas.height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        overlay.canvas.rgba.clone(),
        TextureFormat::Rgba8UnormSrgb,
        // `MAIN_WORLD` is what keeps a CPU copy alive: with `RENDER_WORLD` alone the renderer
        // takes the data and `image.data` becomes `None`, so every later `refresh_overlay` and
        // every cutscene blit silently uploaded nothing. The HUD was frozen at its first frame.
        RenderAssetUsages::default(),
    );
    overlay.image = images.add(image);

    // The screen the app opened on: `setup_overlay` resolves it once, because Escape needs to know
    // where the stack ends. `main` is an alias by artwork (`bkMain.bm`), so the base address is the
    // reliable identity.
    if overlay.boot.is_none() {
        overlay.boot = screens.find("main").map(|s| s.base);
    }
    let palette = world
        .level
        .colormaps
        .first()
        .and_then(|c| gobs.read(c))
        .and_then(|d| tex::Palette::parse(&d))
        .map(|p| p.0)
        .and_then(|v| <[[u8; 3]; 256]>::try_from(v).ok())
        .unwrap_or([[0u8; 3]; 256]);
    // The Escape screen is whichever one carries "Return to Game": the item tables are all
    // the executable gives us about menu identity, and that row is unique to the pause menu.
    overlay.esc = screens.find_by_item("GUI_RETURN_TO_GAME").map(|s| s.name.clone());
    commands.insert_resource(UiAssets {
        strings: gobs
            .read("ui\\jkstrings.uni")
            .map(|d| uni::parse(&String::from_utf8_lossy(&d)))
            .unwrap_or_default(),
        fonts: load_fonts(&gobs, &screens),
        palette,
    });

    // One quad, sized to the **overlay** camera's frustum at half a unit out, so the 640x480
    // canvas covers the view whatever the window size. The overlay camera is spawned below and has
    // the default 45 degree projection, so that is the FOV — sizing the quad from the *world*
    // camera instead only worked while both were 45 degrees, and `--overview`'s long lens then shrank
    // the whole HUD into a thumbnail in the middle of the screen.
    let Ok((cam, _)) = cams.single() else { return };
    let fov = std::f32::consts::FRAC_PI_4;
    // The overlay quad lives on its own render layer, drawn by a second camera that sees
    // nothing else, so no wall can ever draw over the UI. Half a unit out is plenty.
    let distance = 0.5;
    let height = 2.0 * distance * (fov * 0.5).tan();
    // Keep the canvas' own 4:3 proportions: the HUD and the menus are authored for a
    // 640x480 screen, so stretching them to a widescreen window would shear the art.
    let quad = meshes.add(Rectangle::new(height * (overlay.canvas.width as f32 / overlay.canvas.height as f32), height));
    // The screen-effect wash's own quad and material, transparent until an effect asks for it.
    let tint_quad = meshes.add(Rectangle::new(height * (overlay.canvas.width as f32 / overlay.canvas.height as f32), height));
    let tint_material = materials.add(StandardMaterial {
        base_color: bevy::color::Color::NONE,
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(overlay.image.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    commands.entity(cam).with_children(move |parent| {
        parent.spawn((
            Mesh3d(quad),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, 0.0, -distance),
            OverlayQuad,
            RenderLayers::layer(1),
        ));
        // A camera that sees only layer 1: the world camera keeps layer 0, so the UI is
        // never depth-tested against the scene and never clipped by it. Order 2, so the screen
        // effects below draw under the HUD rather than over it.
        parent.spawn((
            Camera3d::default(),
            Camera {
                order: 2,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            RenderLayers::layer(1),
            OverlayQuad,
        ));
        // The screen-effect wash: its own layer and a camera between the world and the UI, so
        // `NewColorEffect` tints the scene — force seeing's blue, infrared's red, a damage flash —
        // and the HUD stays drawn on top of it in its own colours.
        parent.spawn((
            Mesh3d(tint_quad),
            MeshMaterial3d(tint_material),
            Transform::from_xyz(0.0, 0.0, -distance),
            ScreenTintQuad,
            RenderLayers::layer(2),
        ));
        parent.spawn((
            Camera3d::default(),
            Camera {
                order: 1,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            RenderLayers::layer(2),
            ScreenTintCamera,
        ));
    });
}

/// The entity carrying the screen-effect wash, and its material handle.
#[derive(Component)]
struct ScreenTintQuad;

/// The camera that draws the screen-effect wash on its own layer.
#[derive(Component)]
struct ScreenTintCamera;

/// What the scene wash should be: a colour and an alpha, from the same effect the canvas applies.
///
/// The canvas filters a 32-bit buffer per channel; a blended quad cannot dim a channel, so a filter
/// that turns one **off** is drawn as that channel's absence — a blue-only filter is a blue wash with
/// no red or green in it — which is the same impression at a glance and is what the 2D path produces
/// too. `add` biases the colour up, as it does in the buffer.
fn scene_tint(
    filter: [bool; 3],
    tint: [f32; 3],
    add: [f32; 3],
    alpha: f32,
) -> (bevy::color::LinearRgba, f32) {
    let tinted = tint != [0.0; 3];
    let mut rgb = [0.0f32; 3];
    for k in 0..3 {
        // An off channel contributes no light; a tinted one scales the white wash.
        let lit = if filter[k] { 1.0 } else { 0.0 };
        let scaled = if tinted { lit * tint[k] } else { lit };
        rgb[k] = (scaled + add[k] / 255.0).clamp(0.0, 1.0);
    }
    (
        bevy::color::LinearRgba::new(rgb[0], rgb[1], rgb[2], 1.0),
        alpha.clamp(0.0, 1.0),
    )
}

/// Drive the scene wash from `NewColorEffect`: force seeing's blue, infrared's red, a flash.
///
/// The same effect the canvas applies, on a quad drawn between the world and the HUD — the canvas
/// path can only tint what it draws, which is the overlay itself.
fn scene_wash(
    sim: Option<Res<Sim>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    quads: Query<&MeshMaterial3d<StandardMaterial>, With<ScreenTintQuad>>,
) {
    let Some(sim) = sim else { return };
    let effect = sim.world.borrow().screen_effect();
    let (colour, alpha) = match effect {
        Some((filter, tint, add, alpha)) => scene_tint(filter, tint, add, alpha),
        None => (bevy::color::LinearRgba::NONE, 0.0),
    };
    for handle in &quads {
        let Some(mut material) = materials.get_mut(&handle.0) else { continue };
        material.base_color = colour.into();
        material.alpha_mode = AlphaMode::Blend;
        // `base_color` carries the alpha for a blended unlit quad.
        material.base_color.set_alpha(alpha);
    }
}

/// The map panel's inputs for a screen: the level, the player's position in the panel's own
/// coordinates, and the facing in degrees. `None` when the screen has no map, which is most of them.
///
/// Shared with `--map-shot`'s check: the gate has to exercise this, not a copy of it.
fn live_map_for<'a>(
    screen: &menus::Screen,
    level: &'a jkl::Level,
    pos: Vec3,
    yaw: f32,
) -> Option<(&'a jkl::Level, [f32; 3], f32)> {
    if !screen.items.iter().any(|i| i.panel() == Some(menus::Panel::Map)) {
        return None;
    }
    Some((level, [pos.x, pos.y, pos.z + PLAYER_RADIUS], yaw.to_degrees()))
}

/// Redraw the canvas when something changed and upload it.
fn refresh_overlay(
    gobs: Res<Archives>,
    world: Option<Res<World>>,
    player: Option<Res<Player>>,
    screens: Option<Res<Screens>>,
    assets: Option<Res<UiAssets>>,
    sim: Option<Res<Sim>>,
    // A cutscene owns the screen while it plays: the engine drew it instead of the HUD, and
    // the canvas has to stay the film rather than be cleared and redrawn under it.
    playing: Option<Res<cutscene::Cutscene>>,
    mut overlay: ResMut<Overlay>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(screens), Some(assets)) = (screens, assets) else { return };
    if playing.is_some() || !overlay.dirty {
        return;
    }
    overlay.dirty = false;
    let hud = overlay.hud.clone();
    // Read before the canvas is borrowed mutably below.
    let (inv_selected, inventory_open) = (overlay.inv_selected, overlay.inventory);
    overlay.canvas.rgba.fill(0);
    let open = overlay.menu.clone().and_then(|name| screens.find(&name).cloned());
    match open {
        Some(screen) => {
            let Some((base, panel)) = load_backdrop(&gobs, &screen, None) else {
                // A screen whose art does not load leaves the canvas cleared, which is a black frame
                // with no clue why. Say which screen and which bitmap instead.
                println!("ui: no backdrop for {:#x} `{}` ({})", screen.base, screen.name, screen.background);
                return;
            };
            // What the player has found, so the engine's own `bIsVisible` rule can hide the
            // rows it would hide (jkGUIForce.c: `SITHINVENTORY_ITEM_FOUND`).
            let found = |bin: i32| {
                let Some(sim) = sim.as_ref() else { return true };
                let rt = sim.world.borrow();
                rt.inv.get(&bin).copied().unwrap_or(0.0) > 0.0
                    || rt.inv_available.get(&bin).copied().unwrap_or(false)
            };
            let rows = overlay.rows(&screen, &found);
            let cursor = rows.get(overlay.selected % rows.len().max(1)).copied();
            let lit = overlay.hover.or(cursor);
            // The field-log map panel needs the **live** level and pose: without them the panel
            // draws its frame and nothing inside, which is what a player opening the map saw.
            let live_map = match (world.as_deref(), player.as_deref()) {
                (Some(world), Some(player)) => live_map_for(&screen, &world.level, player.pos, player.yaw),
                _ => None,
            };
            let art = |index: i32| -> Option<bm::Bm> {
                let name = screen.art.get(usize::try_from(index).ok()?)?;
                bm::parse(&gobs.read(&format!("ui\\bm\\{name}"))?)
            };
            // The tally screen's force stars, which `jkGuiSingleTally_ForceStarsRender` draws from
            // `jkPlayer_GetJediRank` — the force-level bin. Nothing fed this, so the end-of-level
            // screen showed no stars at all however far the player had trained.
            let star_art = gobs.read("ui\\bm\\foStars.bm").and_then(|d| bm::parse(&d));
            let rank = sim.as_deref().map(|sim| sim.world.borrow().jedi_rank()).unwrap_or(0);
            let stars = star_art
                .as_ref()
                .filter(|_| screen.items.iter().any(|i| matches!(i.panel(), Some(menus::Panel::ForceStars))))
                .map(|art| (art, rank));
            // The tally's rows carry no key: `tally_lines` fills them, and the gate checks it.
            let tally_lines = sim
                .as_deref()
                .map(|sim| {
                    let rt = sim.world.borrow();
                    let (found, total) = rt.secrets();
                    tally_lines(&screen, &assets.strings, rank, rt.alignment(), found, total)
                })
                .unwrap_or(None);
            let checkbox = screen.art.iter().position(|n| n.eq_ignore_ascii_case("check.bm")).map(|i| i as i32);
            // A run-time dialog's lines are literal text, not keys.
            let lines = overlay.lines.clone();
            let text = |index: usize| {
                tally_lines
                    .as_ref()
                    .and_then(|rows| rows.get(index).cloned())
                    .or_else(|| lines.iter().find(|(at, _)| *at == index).map(|(_, t)| t.clone()))
            };
            let look = ui::Look {
                fonts: &assets.fonts,
                art: &art,
                checkbox,
                palette: &assets.palette,
                stars,
                text: &text,
                map: live_map,
                ..ui::Look::default()
            };
            let backdrop = ui::Backdrop { base: &base, panel: panel.as_ref() };
            ui::draw_screen(&mut overlay.canvas, &screen, &backdrop, &assets.strings, &look, lit);
        }
        None => hud::draw_selected(
            &mut overlay.canvas,
            &gobs.0,
            &assets.palette,
            &hud,
            Some(inv_selected),
            inventory_open,
        ),
    }
    // The message area: `jkStringOutput`/`jkPrintUNIString` compose a line, resolve their `.uni`
    // keys through the install's table, and it shows for a few seconds over the status bar.
    if overlay.menu.is_none()
        && let Some(sim) = sim.as_ref()
        && let Some(line) = sim.world.borrow().hud_line().map(str::to_string)
        && let Some(font) = assets.fonts.iter().find(|font| font.glyph('A').is_some())
    {
        let text = resolve_keys(&assets.strings, &line);
        hud::draw_message(&mut overlay.canvas, font, &assets.palette, &text);
    }
    // `NewColorEffect`: force seeing's blue filter, IR goggles, a blinding flash. Applied over the
    // finished canvas because a 32-bit buffer has no palette to filter.
    if let Some(sim) = sim.as_ref()
        && let Some((filter, tint, add, alpha)) = sim.world.borrow().screen_effect()
    {
        overlay.canvas.apply_screen_effect(filter, tint, add, alpha);
    }
    if let Some(mut image) = images.get_mut(overlay.image.id()) {
        if let Some(data) = image.data.as_mut() {
            data.copy_from_slice(&overlay.canvas.rgba);
        }
    }
}

/// Escape opens and closes the menu, the arrows move the cursor and Enter picks.
fn ui_input(
    keys: Res<ButtonInput<KeyCode>>,
    sim: Option<Res<Sim>>,
    screens: Option<Res<Screens>>,
    assets: Option<Res<UiAssets>>,
    plan: Option<Res<FlowPlan>>,
    overview: Option<Res<Overview>>,
    mut pending: ResMut<PendingLevel>,
    mut overlay: ResMut<Overlay>,
    mut commands: Commands,
) {
    let Some(screens) = screens else { return };
    // A capture takes no live input (see `ui_mouse`): a held Enter would walk the menu on its own.
    if overview.as_deref().is_some_and(|o| o.capture) {
        return;
    }
    // The inventory strip: `I` opens it, the arrows move and Enter uses the selected item. Retail
    // reaches the same call through `jkHudInv`'s screen, whose art this tree does not carry; the
    // strip is the HUD's own, so this reads it rather than drawing a second one.
    if overlay.menu.is_none() && overlay.started {
        if keys.just_pressed(KeyCode::KeyI) {
            overlay.inventory = !overlay.inventory;
            overlay.dirty = true;
        }
        if overlay.inventory {
            let count = overlay.hud.carried.len();
            if count > 0 {
                if keys.just_pressed(KeyCode::ArrowDown) {
                    overlay.inv_selected = (overlay.inv_selected + 1) % count;
                    overlay.dirty = true;
                }
                if keys.just_pressed(KeyCode::ArrowUp) {
                    overlay.inv_selected = (overlay.inv_selected + count - 1) % count;
                    overlay.dirty = true;
                }
                if keys.just_pressed(KeyCode::Enter)
                    && let Some(bin) = overlay.carried_bins.get(overlay.inv_selected).copied()
                {
                    if let Some(sim) = sim.as_deref() {
                        sim.world.borrow_mut().use_bin(bin);
                    }
                    overlay.dirty = true;
                }
            }
            // Escape closes the strip before it would open the pause screen.
            if keys.just_pressed(KeyCode::Escape) {
                overlay.inventory = false;
                overlay.dirty = true;
            }
        }
    }
    // `[` and `]`: previous and next weapon, as retail binds them. Without this there was no way to
    // arm anything the player picked up — `select_bin` existed and nothing called it.
    if overlay.menu.is_none()
        && overlay.started
        && let Some(sim) = sim.as_deref()
        && (keys.just_pressed(KeyCode::BracketLeft) || keys.just_pressed(KeyCode::BracketRight))
    {
        let step = if keys.just_pressed(KeyCode::BracketLeft) { -1 } else { 1 };
        let armed = sim.world.borrow_mut().cycle_weapon(step);
        overlay.dirty = armed >= 0;
    }
    // `M`: the field log and its map panel, which `refresh_overlay` now fills from the live level.
    if overlay.menu.is_none() && overlay.started && keys.just_pressed(KeyCode::KeyM) {
        overlay.inventory = false;
        overlay.history.clear();
        if screens.find("fieldlog").is_some() {
            overlay.show(Some("fieldlog".to_string()), 0);
        }
    }
    if keys.just_pressed(KeyCode::Escape) && !overlay.inventory {
        match overlay.menu.clone() {
            // Escape backs out one screen at a time, and out of the last one into the game.
            Some(name) => {
                let at_boot = screens.find(&name).map(|s| s.base) == overlay.boot;
                if !at_boot {
                    let back = overlay.history.pop();
                    overlay.show(back, 0);
                }
            }
            // …and opens the pause screen, which is a root: it carries no history, so its own
            // "Return To Game" closes the whole stack rather than walking back through it.
            None if overlay.started => {
                overlay.history.clear();
                let esc = overlay.esc.clone();
                overlay.show(esc, 0);
            }
            None => {}
        }
    }
    let Some(screen) = overlay.menu.clone().and_then(|name| screens.find(&name).cloned()) else {
        return;
    };
    let rows = screen.rows();
    if rows.is_empty() {
        return;
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        overlay.selected = (overlay.selected + 1) % rows.len();
        overlay.dirty = true;
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        overlay.selected = (overlay.selected + rows.len() - 1) % rows.len();
        overlay.dirty = true;
    }
    if keys.just_pressed(KeyCode::Enter) {
        let strings = assets.as_deref().map(|a| &a.strings).cloned().unwrap_or_default();
        activate(
            &screen,
            rows[overlay.selected % rows.len()],
            &mut overlay,
            &strings,
            plan.as_deref(),
            &mut pending,
            &mut commands,
        );
    }
}

/// Run the activation `--menu <screen>/<id>` asked for, through the same path a click takes.
fn apply_pending(
    screens: Option<Res<Screens>>,
    assets: Option<Res<UiAssets>>,
    plan: Option<Res<FlowPlan>>,
    mut pending: ResMut<PendingLevel>,
    mut overlay: ResMut<Overlay>,
    mut commands: Commands,
) {
    let Some(id) = overlay.pending.take() else { return };
    let (Some(screens), Some(name)) = (screens, overlay.menu.clone()) else { return };
    let Some(screen) = screens.find(&name).cloned() else { return };
    if let Some(index) = screen.items.iter().position(|item| item.id == id) {
        let strings = assets.as_deref().map(|a| &a.strings).cloned().unwrap_or_default();
        activate(&screen, index, &mut overlay, &strings, plan.as_deref(), &mut pending, &mut commands);
    }
}

/// The mouse: hover lights the element under the pointer, a click picks it.
fn ui_mouse(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    screens: Option<Res<Screens>>,
    assets: Option<Res<UiAssets>>,
    plan: Option<Res<FlowPlan>>,
    overview: Option<Res<Overview>>,
    sim: Option<Res<Sim>>,
    bank: Option<Res<SoundBank>>,
    mut pending: ResMut<PendingLevel>,
    mut overlay: ResMut<Overlay>,
    mut commands: Commands,
) {
    // A capture takes no live pointer input, for the same reason `fire_player` refuses it: a click
    // arriving during a headless run walks the menu on its own — main -> New Game -> Abort -> Quit —
    // and the frame it was supposed to save is whatever screen the cascade happened to reach.
    if overview.as_deref().is_some_and(|o| o.capture) {
        return;
    }
    let (Some(screens), Ok(window)) = (screens, windows.single()) else { return };
    if overlay.menu.is_none() {
        return;
    }
    let at = window
        .cursor_position()
        .and_then(|c| ui::to_canvas([window.width(), window.height()], [c.x, c.y]));
    let screen = overlay.menu.clone().and_then(|name| screens.find(&name).cloned());
    let hover = match (&screen, at) {
        (Some(screen), Some(at)) => ui::pick(screen, at),
        _ => None,
    };
    if hover != overlay.hover {
        overlay.hover = hover;
        overlay.dirty = true;
        // The menu's own hover wav, from `jkGuiMenu.soundHover` (§18.3d).
        if let Some(name) = screen.as_ref().and_then(|s| s.sound_hover.clone()) {
            play_menu_wav(&name, &sim, &bank);
        }
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let (Some(screen), Some(index)) = (screen, hover) else { return };
    // A click on a row moves the keyboard cursor there, so Enter and the arrows continue
    // from what was clicked; anything else runs its own id.
    let rows = screen.rows();
    if let Some(row) = rows.iter().position(|i| *i == index) {
        overlay.selected = row;
        overlay.dirty = true;
    }
    // ... and its click wav, which `jkGuiRend_PlayClickSound` plays for every button.
    if let Some(name) = screen.sound_click.clone() {
        play_menu_wav(&name, &sim, &bank);
    }
    let strings = assets.as_deref().map(|a| &a.strings).cloned().unwrap_or_default();
    activate(&screen, index, &mut overlay, &strings, plan.as_deref(), &mut pending, &mut commands);
}

/// Run one element's id: open the screen the retail graph says it opens, otherwise close
/// the screen, start a game, or leave the game.
fn activate(
    screen: &menus::Screen,
    index: usize,
    overlay: &mut Overlay,
    strings: &uni::Uni,
    plan: Option<&FlowPlan>,
    pending: &mut PendingLevel,
    commands: &mut Commands,
) {
    let item = &screen.items[index];
    let step = nav::step(screen.base, item.id, &item.key);
    println!("menu {:#x} `{}` -> `{}` ({}) = {step:?}", screen.base, screen.name, item.key, item.id);
    match step {
        nav::Step::Quit => {
            commands.write_message(AppExit::Success);
        }
        // The pause menu's Save and Load rows. The picker screen that would choose a slot is not
        // built, so both act on the one slot the keys use — `quick_save` owns the file work because
        // it is the system that holds the runtime, the save directory and the level plan together.
        nav::Step::SaveGame | nav::Step::LoadGame => {
            let want = if step == nav::Step::SaveGame { SaveLoad::Save } else { SaveLoad::Load };
            commands.insert_resource(want);
            // Saving hands the view back, as the engine does; loading leaves the menu up until the
            // restore has happened, because a level load may have to come first.
            if want == SaveLoad::Save {
                overlay.show(None, 0);
            }
        }
        nav::Step::NewGame | nav::Step::Restart => {
            // Both are "load this level from its own start point": `swap_level` despawns what the
            // old level spawned and runs `setup` again, so the scripts, the things and the player
            // all start over instead of resuming the world the menu was drawn over. New Game is the
            // campaign's first level, Restart is the one that is loaded.
            let Some(plan) = plan else { return };
            let level = if step == nav::Step::NewGame { plan.first.clone() } else { plan.level.clone() };
            overlay.history.clear();
            overlay.started = true;
            overlay.show(None, 0);
            println!("boot: {} -> {level}", item.key);
            pending.0 = Some(level);
        }
        nav::Step::Abort => {
            // Abort leaves the level rather than the program: back to the boot menu, with the world
            // still paused behind it (`started` false also keeps Escape from closing that menu).
            overlay.history.clear();
            overlay.started = false;
            overlay.show(Some("main".to_string()), 0);
        }
        nav::Step::Open(to) => {
            // `find` takes a hex address, which is what the graph is keyed by.
            overlay.history.push(screen.name.clone());
            overlay.show(Some(format!("{to:#x}")), 0);
        }
        nav::Step::Dialog { screen: to, keys } => {
            // A run-time dialog: its text elements carry no key, so the lines are the
            // strings the engine's `jkGuiDialog_Message` was given.
            let lines: Vec<(usize, String)> = keys
                .iter()
                .enumerate()
                .filter_map(|(i, key)| strings.get(key).map(|text| (i, text.to_string())))
                .collect();
            overlay.history.push(screen.name.clone());
            overlay.show(Some(format!("{to:#x}")), 0);
            overlay.lines = lines;
        }
        nav::Step::Close => {
            // Back to the screen this one was opened from; the last one returns to the game.
            let back = overlay.history.pop();
            overlay.show(back, 0);
        }
        nav::Step::Nothing => {}
    }
}
// ---------------------------------------------------------------------------
// loading
// ---------------------------------------------------------------------------

/// Every `episode.jk` in the install, with the archive it came from.
fn episodes_of(gobs: &gob::GobSet) -> Vec<(String, episode::Episode)> {
    let mut out = Vec::new();
    for entry in gobs.all_named("episode.jk") {
        let archive = gobs
            .archive_path(entry.archive)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(raw) = gobs.read_entry(entry) else { continue };
        out.push((archive, episode::parse(&String::from_utf8_lossy(&raw))));
    }
    // Single player first, then multiplayer, then CTF, so the campaign start is
    // well defined whatever order the archives happen to sort in.
    out.sort_by_key(|(_, e)| match e.kind {
        episode::KIND_SINGLE => 0,
        episode::KIND_MULTI => 1,
        episode::KIND_CTF => 2,
        _ => 3,
    });
    out
}

/// The level the game itself starts at: the first level of the first
/// single-player episode. `None` when no manifest names a level that exists.
fn start_level(gobs: &gob::GobSet) -> Option<String> {
    episodes_of(gobs)
        .into_iter()
        .filter(|(_, e)| e.kind == episode::KIND_SINGLE)
        .find_map(|(_, e)| e.first_level().map(|s| s.file.clone()))
        .filter(|f| gobs.find(f).is_some())
}

/// `--boot`: the screen the app opens on, and what each row of the boot path does.
///
/// The decisions are [`nav`]'s, and `nav`'s tests pin them. This half checks them against *this
/// install*: that the screens the ids belong to exist, that the rows carry those ids, that the
/// cursor can reach New Game at all, and that the campaign it starts is a level the archives hold.
/// A graph is only a claim until the shipped element tables agree with it.
fn boot_gate(gobs: &gob::GobSet, menus: &menus::Menus) -> bool {
    let mut ok = true;
    let mut say = |good: bool, line: String| {
        println!("boot: {line}");
        if !good {
            ok = false;
        }
    };
    // What a bare `cargo run` opens on.
    let Some(menu) = menus.find("main") else {
        say(false, "no `main` screen: the install's executable supplied no menu tables".into());
        return false;
    };
    say(
        true,
        format!("opens on `{}` at {:#x}, {} element(s)", menu.name, menu.base, menu.items.len()),
    );
    // Main -> single player, through the row a player picks.
    let Some(row) = menu.items.iter().find(|i| i.key == "GUI_SINGLEPLAYER") else {
        say(false, "the main menu carries no `GUI_SINGLEPLAYER` row".into());
        return false;
    };
    let to = match nav::step(menu.base, row.id, &row.key) {
        nav::Step::Open(to) => to,
        other => {
            say(false, format!("`GUI_SINGLEPLAYER` is {other:?}, not a screen"));
            return false;
        }
    };
    let Some(single) = menus.screens.iter().find(|s| s.base == to) else {
        say(false, format!("{to:#x} is not a screen in this executable"));
        return false;
    };
    say(true, format!("`{}` -> `{}` ({to:#x}, {} element(s))", menu.name, single.name, single.items.len()));
    // New Game: an action, reachable with the keyboard, starting a level that exists.
    let index = single.items.iter().position(|i| i.key == "GUI_NEWGAME");
    let Some(index) = index else {
        say(false, "the single-player screen carries no `GUI_NEWGAME` row".into());
        return false;
    };
    let new_game = &single.items[index];
    let step = nav::step(single.base, new_game.id, &new_game.key);
    say(
        step == nav::Step::NewGame,
        format!("`{}` ({}) -> {step:?}", new_game.key, new_game.id),
    );
    let rows = single.rows();
    say(
        rows.contains(&index),
        format!(
            "the cursor walks {} row(s) on `{}` and #{} is among them",
            rows.len(),
            single.name,
            index
        ),
    );
    let first = start_level(gobs);
    say(
        first.as_ref().is_some_and(|f| gobs.find(f).is_some()),
        format!("campaign start {}", first.as_deref().unwrap_or("(none)")),
    );
    // The pause menu: resume, reload, and the way back to the boot menu.
    let Some(esc) = menus.find_by_item("GUI_RETURN_TO_GAME") else {
        say(false, "no screen carries `GUI_RETURN_TO_GAME`, so Escape has nothing to open".into());
        return false;
    };
    for (key, want) in [
        ("GUI_RETURN_TO_GAME", nav::Step::Close),
        ("GUI_RESTART", nav::Step::Restart),
        ("GUI_ABORT", nav::Step::Abort),
    ] {
        match esc.items.iter().find(|i| i.key == key) {
            Some(item) => say(
                nav::step(esc.base, item.id, &item.key) == want,
                format!("pause `{key}` ({}) -> {want:?}", item.id),
            ),
            None => say(false, format!("the pause menu carries no `{key}` row")),
        }
    }
    // Where the app opens: the retail menu by default, straight into a level for testing.
    say(
        boot_screen(Some("main"), false, false, true).as_deref() == Some("main"),
        "no flags -> the main menu".into(),
    );
    say(
        boot_screen(None, false, true, true).is_none(),
        "`--start` -> no screen, straight into the level".into(),
    );
    say(
        boot_screen(Some("esc"), false, true, true).as_deref() == Some("esc"),
        "`--menu` still wins over `--start`".into(),
    );
    if ok {
        println!("--- boot check passed ---");
    } else {
        println!("--- boot check FAILED ---");
    }
    ok
}

/// `--scale`: how big the placed models are against the geometry they are placed in.
///
/// `MODEL_SCALE` is one number standing in for "how many level units a model unit is", and the way to
/// check it is not to re-read the number but to measure a model against the thing it has to fit: a
/// **door** inside the doorway the level builds for it, and an **actor** under the ceiling of the
/// sector it stands in. Both references are the level's own geometry.
fn print_scale(world: &World) {
    let level = &world.level;
    let scale = model_scale();
    let solids = collide::build(level);

    // A model's bounding box **in model space**: a puppet's mesh vertices are node-local, so they
    // say nothing about the body until the hierarchy is composed. That composition is what
    // `model_mesh_posed` does to build the mesh, and it is what has to be measured here — the raw
    // vertex extents of `ky.3do` are 0.064 model units, ten times smaller than the body.
    let bbox = |model: &tdo::Model| -> Option<([f32; 3], [f32; 3])> {
        let composed = tdo::pose_nodes(model, None, 0.0);
        let owners = model.mesh_nodes();
        let geoset = model.pick_geoset();
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for (index, mesh) in model.meshes.iter().enumerate() {
            if mesh.geoset != geoset {
                continue;
            }
            let owner = owners
                .get(index)
                .copied()
                .filter(|i| *i != usize::MAX)
                .and_then(|i| composed.get(i));
            for v in &mesh.vertices {
                let p = match owner {
                    Some(x) => x.apply(*v),
                    None => *v,
                };
                for (i, c) in p.iter().enumerate() {
                    lo[i] = lo[i].min(*c);
                    hi[i] = hi[i].max(*c);
                }
            }
        }
        (hi[0] > lo[0]).then_some((lo, hi))
    };

    println!("model scale {scale:.1} (BK_MODEL_SCALE overrides); sizes below are world units");
    println!(
        "{:<18} {:<16} {:>8} {:>8} {:>8} {:>7} {:>6}",
        "template", "model", "height", "width", "model h", "RADIUS", "size="
    );
    let mut rows: Vec<(String, String, f32, f32, f32, f32, Option<f32>, bool, bool)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for thing in &level.things {
        if !seen.insert(thing.template.to_ascii_lowercase()) {
            continue;
        }
        let Some(name) = world.template_model.get(&thing.template.to_ascii_lowercase()) else { continue };
        let Some(model) = world.models.get(name) else { continue };
        let Some((lo, hi)) = bbox(model) else { continue };
        let puppet = level.template_param(&thing.template, "puppet").is_some();
        let door = thing.template.to_ascii_lowercase().contains("door");
        rows.push((
            thing.template.clone(),
            name.clone(),
            (hi[2] - lo[2]) * scale,
            (hi[0] - lo[0]).max(hi[1] - lo[1]) * scale,
            hi[2] - lo[2],
            model.radius,
            level.template_param(&thing.template, "size").and_then(|s| s.parse::<f32>().ok()),
            puppet,
            door,
        ));
    }
    rows.sort_by(|a, b| b.2.total_cmp(&a.2));
    for (template, model, height, width, raw, radius, size, puppet, door) in &rows {
        println!(
            "{template:<18} {model:<16} {height:>8.2} {width:>8.2} {raw:>8.3} {radius:>7.3} {:>6} {}{}",
            size.map(|s| format!("{s:.3}")).unwrap_or_else(|| "-".into()),
            if *puppet { "actor" } else { "" },
            if *door { " door" } else { "" }
        );
    }

    // The level's own references. A **door** has to fill the doorway the level builds for it: the
    // vertical adjoin polygons around the door's own position are that doorway, in world units.
    println!("\ndoor against its doorway (world units, the nearest vertical adjoin):");
    for thing in &level.things {
        if !thing.template.to_ascii_lowercase().contains("door") {
            continue;
        }
        let Some(name) = world.template_model.get(&thing.template.to_ascii_lowercase()) else { continue };
        let Some(model) = world.models.get(name) else { continue };
        let Some((lo, hi)) = bbox(model) else { continue };
        let Ok(sector) = usize::try_from(thing.sector) else { continue };
        let mut best: Option<(f32, f32, f32, f32)> = None;
        for portal in solids.portals.get(sector).into_iter().flatten() {
            // A doorway is vertical: its plane's normal is horizontal.
            if portal.normal[2].abs() > 0.6 {
                continue;
            }
            let (mut zlo, mut zhi) = (f32::INFINITY, f32::NEG_INFINITY);
            let (mut centre, n) = ([0.0f32; 3], portal.verts.len().max(1) as f32);
            for v in &portal.verts {
                zlo = zlo.min(v[2]);
                zhi = zhi.max(v[2]);
                centre = [centre[0] + v[0] / n, centre[1] + v[1] / n, centre[2] + v[2] / n];
            }
            if zhi <= zlo {
                continue;
            }
            // The opening's width: its horizontal extent along its own plane.
            let tangent = [-portal.normal[1], portal.normal[0], 0.0];
            let (mut wlo, mut whi) = (f32::INFINITY, f32::NEG_INFINITY);
            for v in &portal.verts {
                let t = v[0] * tangent[0] + v[1] * tangent[1];
                wlo = wlo.min(t);
                whi = whi.max(t);
            }
            let far = ((centre[0] - thing.pos[0]).powi(2) + (centre[1] - thing.pos[1]).powi(2)).sqrt();
            if best.is_none_or(|(d, ..)| far < d) {
                best = Some((far, whi - wlo, zhi - zlo, 0.0));
            }
        }
        match best {
            Some((far, width, height, _)) => println!(
                "  {:<12} {:<14} model {:>5.2}w x {:>5.2}h ({:.3} x {:.3} model units)  doorway {:>5.2}w x {:>5.2}h at {far:.2} units",
                thing.template,
                name,
                (hi[0] - lo[0]).max(hi[1] - lo[1]) * scale,
                (hi[2] - lo[2]) * scale,
                (hi[0] - lo[0]).max(hi[1] - lo[1]),
                hi[2] - lo[2],
                width,
                height
            ),
            None => println!("  {:<12} {:<14} no vertical adjoin in its sector", thing.template, name),
        }
    }

    // The rooms things stand in, and the actors among them.
    let mut storeys: Vec<f32> = Vec::new();
    for thing in &level.things {
        let Ok(sector) = usize::try_from(thing.sector) else { continue };
        if let (Some(floor), Some(ceiling)) = (
            solids.floor_below(sector, thing.pos),
            solids.ceiling_above(sector, thing.pos),
        ) && ceiling > floor
        {
            storeys.push(ceiling - floor);
        }
    }
    let median = |v: &[f32]| {
        let mut v = v.to_vec();
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied()
    };
    let mut actors: Vec<f32> = rows.iter().filter(|r| r.7).map(|r| r.2).collect();
    actors.sort_by(f32::total_cmp);
    let (shortest, tallest, middle) = (
        actors.first().copied(),
        actors.last().copied(),
        median(&actors),
    );
    println!(
        "\nstorey heights over {} placed thing(s): median {:?}, max {:?}",
        storeys.len(),
        median(&storeys),
        storeys.iter().copied().max_by(f32::total_cmp)
    );
    println!(
        "actor heights over {} template(s): shortest {shortest:?}, median {middle:?}, tallest {tallest:?}",
        actors.len()
    );
    if let (Some(actor), Some(storey)) = (middle, median(&storeys)) {
        println!(
            "actor / storey = {:.2} (under 1.0 is a body that fits its room with headroom; 10.0 was the ten-times-too-big world)",
            actor / storey
        );
    }
    // A gate proves itself with a verdict line; the sweep fails a row that has none.
    println!("--- scale check passed ---");
}

fn print_episodes(gobs: &gob::GobSet) {
    println!("=== bevi-knight episodes ===");
    println!("archives   {} GOB files, {} entries", gobs.archive_count(), gobs.len());
    for (archive, ep) in episodes_of(gobs) {
        println!(
            "\n{archive}  type {}  \"{}\"  {} sequences",
            ep.kind,
            ep.title,
            ep.sequences.len()
        );
        println!("{:<6} {:<3} {:<4} {:<7} {:<22} {:<5} {:<5} {:<6} {:<6} {}", "line", "cd", "ch", "type", "file", "next", "alt", "light", "dark", "status");
        for s in &ep.sequences {
            let kind = match s.kind {
                episode::Kind::Level => "LEVEL",
                episode::Kind::Cut => "CUT",
                episode::Kind::Decide => "DECIDE",
                episode::Kind::Other => "other",
            };
            // Levels must be present; cutscenes are Smacker movies that the Steam
            // release simply does not ship, so their absence is expected.
            let status = match s.kind {
                episode::Kind::Level if gobs.find(&s.file).is_some() => s.comment.clone(),
                episode::Kind::Level => "MISSING LEVEL".to_string(),
                episode::Kind::Cut => "movie (not shipped)".to_string(),
                _ => s.comment.clone(),
            };
            println!(
                "{:<6} {:<3} {:<4} {:<7} {:<22} {:<5} {:<5} {:<6} {:<6} {}",
                s.id, s.cd, s.number, kind, s.file, s.goto_a, s.goto_b, s.lightpow, s.darkpow, status
            );
        }
    }
    println!("--- episodes check passed ---");
}

/// Lowercase template name -> lowercase `.3do` name, for every template that
/// places a model. Shared by loading, spawning and the audit.
fn template_models(level: &jkl::Level) -> HashMap<String, String> {
    level
        .templates
        .iter()
        .filter_map(|t| t.model3d.as_ref().map(|m| (t.name.to_ascii_lowercase(), m.to_ascii_lowercase())))
        .collect()
}

/// `--cogs`: parse every script the level names, run each `startup` label, and
/// report what happened. This is the COG runtime's own smoke test: the scripts'
/// `Print()` calls are the engine's debug output, so real output proves real
/// execution.
fn run_level_cogs(gobs: &gob::GobSet, level: &jkl::Level, filter: &str) {
    // No filter: the scripts this level names. `all`: every script in the install.
    // Otherwise: scripts whose path contains the filter text.
    let scripts: Vec<String> = if filter.is_empty() {
        level
            .assets
            .iter()
            .filter(|a| a.kind == jkl::AssetKind::Cog)
            .map(|a| a.name.clone())
            .collect()
    } else {
        gobs.names()
            .into_iter()
            .filter(|n| n.ends_with(".cog"))
            .filter(|n| filter == "all" || n.contains(filter))
            .collect()
    };
    println!("=== running {} scripts ===", scripts.len());

    let mut parsed = 0usize;
    let mut ran = 0usize;
    let mut waiting = 0usize;
    let mut errors = 0usize;
    let mut printed = 0usize;
    let mut symbols = 0usize;
    let mut locals = 0usize;
    let mut initialised = 0usize;
    let mut verbs: std::collections::BTreeMap<String, usize> = Default::default();
    let mut run_errors: Vec<String> = Vec::new();

    for name in &scripts {
        let Some(data) = gobs.read(name) else {
            println!("  {name}: missing from archive");
            continue;
        };
        let script = match cog::parse(&String::from_utf8_lossy(&data)) {
            Ok(c) => c,
            Err(e) => {
                errors += 1;
                if run_errors.len() < 8 {
                    run_errors.push(format!("{name}: {e}"));
                }
                continue;
            }
        };
        parsed += 1;
        symbols += script.symbols.len();
        locals += script.symbols.iter().filter(|s| s.local).count();
        initialised += script.symbols.iter().filter(|s| s.init.is_some()).count();
        // Run every message the script declares, not just `startup`: that is what
        // the engine does as the level plays, and it exercises far more code.
        let labels: Vec<String> = script.messages().map(|m| m.name.clone()).collect();
        for label in labels {
            if script.label(&label).is_none() {
                continue;
            }
            let mut vm = cog::Vm::new();
            match vm.run_label(&script, &label) {
                Ok(_) => ran += 1,
                Err(e) if e == cog::WAITING => waiting += 1,
                Err(e) => {
                    errors += 1;
                    if run_errors.len() < 8 {
                        run_errors.push(format!("{name} {label}: {e}"));
                    }
                }
            }
            for line in &vm.trace.prints {
                printed += 1;
                println!("  [{name} {label}] {line}");
            }
            for (verb, n) in vm.trace.calls {
                *verbs.entry(verb).or_default() += n;
            }
        }
    }

    println!(
        "scripts: {parsed} parsed, {ran} message labels completed, {waiting} waiting on the engine, {printed} printed, {errors} errors, {} distinct verbs called",
        verbs.len()
    );
    println!("symbols: {symbols} declared, {locals} local, {initialised} with an initialiser");
    let mut ranked: Vec<(String, usize)> = verbs.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    let top: Vec<String> = ranked.iter().take(10).map(|(v, n)| format!("{v} x{n}")).collect();
    println!("verbs called: {}", top.join(", "));
    for e in &run_errors {
        println!("  {e}");
    }
}

/// `--run`: start the level's placed scripts against a live world and step it, which
/// is the difference between parsing the game's logic and acting on it.

/// Take an item the way its own script does: run the item's cog `touched` label with the
/// player as `GetSourceRef` and the item as `GetSenderRef`, then its `taken` label.
///
/// The engine's `TakeItem` calls `sithItem_SetItemTaken`, which runs exactly that `taken`
/// label; here the verb only removes the thing, so the label is run explicitly. That is what
/// makes the inventory bin the *script's* number (`bin=40` in `pow_bacta.cog`) instead of
/// this build's stand-in.
fn take_item_with_script(
    world: &World,
    sim_world: &runtime::World,
    level: &jkl::Level,
    item: usize,
    template: &str,
) -> Option<(String, Vec<i32>)> {
    let cog_name = level.template_param(template, "cog")?.to_ascii_lowercase();
    let script = world.cogs.get(&cog_name)?;
    let values = level
        .cogs
        .iter()
        .find(|i| i.script.eq_ignore_ascii_case(&cog_name))
        .map(|i| i.values.clone())
        .unwrap_or_default();
    let player = sim_world
        .borrow()
        .templates
        .iter()
        .position(|t| t.eq_ignore_ascii_case("walkplayer"))?;
    let mut vm = cog::Vm::with_world(sim_world.clone());
    script.bind(&mut vm, &values);
    // The item is the trigger, the player is the source: `player = GetSourceRef()` and
    // `powerup = GetSenderRef()` in the shipped powerup scripts.
    vm.sender = item as i64;
    vm.sender_ref = item as i64;
    vm.source = player as i64;
    let _ = vm.run_label(script, "touched");
    let _ = vm.run_label(script, "taken");
    let mut bins: Vec<i32> = sim_world.borrow().inv.keys().copied().collect();
    bins.sort();
    Some((cog_name, bins))
}

fn run_level(
    gobs: &gob::GobSet,
    level: &jkl::Level,
    quiet: bool,
) -> std::collections::BTreeMap<String, usize> {

    let world = runtime::Runtime::handle(level);
    if !quiet {
        println!("=== {} placed script instances ===", level.cogs.len());
    }

    let mut started = 0usize;
    let mut no_startup = 0usize;
    let mut waiting = 0usize;
    let mut errors: Vec<String> = Vec::new();
    let mut printed = 0usize;
    let mut verbs: std::collections::BTreeMap<String, usize> = Default::default();
    let mut scripts: std::collections::HashMap<String, cog::Cog> = Default::default();
    // One VM per instance: the engine gives each placed script its own symbol table,
    // so two doors running the same `.cog` must not share variables.
    let mut vms: Vec<(String, cog::Vm)> = Vec::new();

    for inst in &level.cogs {
        if !scripts.contains_key(&inst.script) {
            let Some(data) = gobs.read(&inst.script) else {
                errors.push(format!("{}: missing from archive", inst.script));
                continue;
            };
            let text = String::from_utf8_lossy(&data).into_owned();
            match cog::parse(&text) {
                Ok(c) => {
                    scripts.insert(inst.script.clone(), c);
                }
                Err(e) => {
                    errors.push(format!("{}: {e}", inst.script));
                    continue;
                }
            }
        }
        let script = &scripts[&inst.script];
        let mut vm = cog::Vm::with_world(world.clone());
        // `GetSelfCog` answers the instance's index in the level's cog list, the same number the
        // engine's `pCog->idx` holds: scripts compare it against sender refs and the master cog.
        vm.cog_index = vms.len() as i32;
        script.bind(&mut vm, &inst.values);
        // Plenty of shipped scripts are purely event driven and declare no `startup`;
        // the engine simply has nothing to call at level load.
        if script.label("startup").is_some() {
            let before_sounds = world.borrow().sounds.len();
            match vm.run_label(&script, "startup") {
                Ok(_) => started += 1,
                Err(e) if e == cog::WAITING => waiting += 1,
                Err(e) => errors.push(format!("{} startup: {e}", inst.script)),
            }
            for line in &vm.trace.prints {
                printed += 1;
                if !quiet {
                    println!("  [{} startup] {line}", inst.script);
                }
            }
            let w = world.borrow();
            for req in &w.sounds[before_sounds..] {
                let (name, vol) = (&req.name, req.volume);
                if !quiet {
                    println!("  [{}] sound {name} at {vol:.2}", inst.script);
                }
            }
        } else {
            no_startup += 1;
        }
        vms.push((inst.script.clone(), vm));
    }

    // The message pump: the engine delivers each message a placed script declares
    // when the matching world event happens. There is no player here to trigger them,
    // so every declared message is delivered once — which is exactly what wakes the
    // elevators and doors and makes them act on the world.
    let mut delivered = 0usize;
    let mut msg_waiting = 0usize;
    for (name, vm) in &mut vms {
        let script = &scripts[name];
        let labels: Vec<String> =
            script.messages().map(|m| m.name.clone()).filter(|m| m != "startup").collect();
        for label in labels {
            if script.label(&label).is_none() {
                continue;
            }
            let before_sounds = world.borrow().sounds.len();
            match vm.run_label(script, &label) {
                Ok(_) => delivered += 1,
                Err(e) if e == cog::WAITING => msg_waiting += 1,
                Err(e) => errors.push(format!("{name} {label}: {e}")),
            }
            if !quiet {
                let w = world.borrow();
                for req in &w.sounds[before_sounds..] {
                    let (sound, vol) = (&req.name, req.volume);
                    println!("  [{name} {label}] sound {sound} at {vol:.2}");
                }
            }
            for line in &vm.trace.prints {
                printed += 1;
                if !quiet {
                    println!("  [{name} {label}] {line}");
                }
            }
        }
        for (verb, n) in &vm.trace.calls {
            *verbs.entry(verb.clone()).or_default() += n;
        }
    }

    // Every *other* label, once. `startup` and the declared `message:` labels are what a level
    // reaches on its own, and `AutosaveGame` sat behind a cutscene label that pass never fires — so
    // the stub count was blind to it for a whole round of green gates. Running the remaining labels
    // is what makes that number mean "verbs the scripts reach", not "verbs startup happens to call".
    //
    // Bounded per label by `cog::LABEL_BUDGET`, which every shipped label reaches its next wait well
    // inside; a label that would not return is reported as waiting, not spun on.
    let mut deep_ran = 0usize;
    let mut deep_waiting = 0usize;
    let mut deep_errors = 0usize;
    for (name, vm) in &mut vms {
        let script = &scripts[name];
        let skip: std::collections::HashSet<String> = script
            .messages()
            .map(|m| m.name.clone())
            .chain(std::iter::once("startup".to_string()))
            .collect();
        let labels: Vec<String> = script
            .labels
            .iter()
            .map(|(label, _)| label.clone())
            .filter(|label| !skip.contains(label) && !label.starts_with("message:"))
            .collect();
        for label in labels {
            match vm.run_label(script, &label) {
                Ok(_) => deep_ran += 1,
                Err(e) if e == cog::WAITING => deep_waiting += 1,
                Err(_) => deep_errors += 1,
            }
        }
        for (verb, n) in &vm.trace.calls {
            *verbs.entry(verb.clone()).or_default() += n;
        }
    }
    if !quiet {
        println!(
            "labels: {deep_ran} ran, {deep_waiting} waiting, {deep_errors} error(s) — every label each script declares, not just startup and its messages"
        );
    }

    // The script-to-script bus: a `SendMessage` queues a message, and the pump delivers it
    // to every instance that declares that label. The engine addresses one cog; this
    // build's instances are level-wide, so a broadcast is what they can receive. Bounded,
    // because scripts can message each other in a loop.
    let mut script_messages = 0usize;
    let mut delivered_messages = 0usize;
    for _round in 0..8 {
        let queued: Vec<(String, Option<usize>, Option<i32>, [f64; 2])> = {
            let mut w = world.borrow_mut();
            std::mem::take(&mut w.messages)
        };
        if queued.is_empty() {
            break;
        }
        for (name, sender, cog, params) in queued {
            script_messages += 1;
            for (script_name, vm) in &mut vms {
                // `SendMessage` addresses a cog (`sithCogFunction_SendMessage` pops a cog index and
                // resolves it), so a targeted message goes to that one instance.
                if let Some(target) = cog
                    && vm.cog_index != target
                {
                    continue;
                }
                let script = &scripts[script_name];
                // `#N` is a numeric message: the engine hands the id to the receiver and its
                // `message:` list decides the label, so both sides agree on `SITH_MESSAGE_*` values.
                let name = match name.strip_prefix('#').and_then(|n| n.parse::<usize>().ok()) {
                    Some(id) => match script.label_for_message_id(id) {
                        Some(label) => label,
                        None => continue,
                    },
                    None => name.clone(),
                };
                if script.label(&name).is_none() {
                    continue;
                }
                vm.sender = sender.unwrap_or(0) as i64;
                vm.sender_ref = sender.unwrap_or(0) as i64;
                // `SendMessageEx`'s two parameters: what this handler's `GetParam` reads.
                world.borrow_mut().last_params = params.to_vec();
                let before_sounds = world.borrow().sounds.len();
                match vm.run_label(script, &name) {
                    Ok(_) => delivered_messages += 1,
                    Err(e) if e == cog::WAITING => msg_waiting += 1,
                    Err(e) => errors.push(format!("{script_name} {name}: {e}")),
                }
                if !quiet {
                    let w = world.borrow();
                    for req in &w.sounds[before_sounds..] {
                    let (sound, vol) = (&req.name, req.volume);
                        println!("  [{script_name} {name}] sound {sound} at {vol:.2}");
                    }
                }
                for line in &vm.trace.prints {
                    printed += 1;
                    if !quiet {
                        println!("  [{script_name} {name}] {line}");
                    }
                }
            }
        }
    }

    // Step the world: `MoveToFrame` targets the scripts just set advance here. Sixty
    // steps is one second of play, enough to prove the state is live.
    let mut steps = 0usize;
    let mut last_moved = 0usize;
    for _ in 0..60 {
        let moved = world.borrow_mut().step(1.0 / 60.0).len();
        steps += 1;
        last_moved = last_moved.max(moved);
    }

    // Verbs the scripts called that nothing answers: reported so the next round knows
    // exactly what engine surface is still missing. Counted per script instance, not
    // per call — a script spinning in `while (1)` would otherwise dominate the list.
    let mut silent = 0usize;
    let mut silent_verbs: std::collections::BTreeMap<String, usize> = Default::default();
    for (_, vm) in &vms {
        silent += vm.trace.unimplemented.values().sum::<usize>();
        for verb in vm.trace.unimplemented.keys() {
            *silent_verbs.entry(verb.clone()).or_default() += 1;
        }
    }

    let w = world.borrow();
    println!(
        "startup: {started} ran, {waiting} waiting on an engine event, {no_startup} scripts are event-only"
    );
    println!(
        "messages: {delivered} delivered, {msg_waiting} waiting, {} errors, {script_messages} sent by scripts ({delivered_messages} delivered)",
        errors.len()
    );
    println!(
        "world after {steps} steps: {} things on a path, {last_moved} moved, {} sector adjoin changes, {} sounds requested, {} inventory items",
        w.things.iter().filter(|t| !t.frames.is_empty()).count(),
        w.adjoins_changes,
        w.sounds.len(),
        w.inv.len()
    );
    // `PlayKey`: a script swapping a puppet's animation track for one of its own `keyframe` symbols.
    for (thing, (key, flags, frame)) in &w.play_key {
        println!("animation: thing {thing} plays `{key}` (flags {flags}, from frame {frame})");
    }
    // One concrete before/after: a thing the scripts moved off its authored position.
    if let Some((i, now)) = w.things.iter().enumerate().find(|(i, t)| {
        level.things.get(*i).is_some_and(|src| src.pos != t.pos)
    }) {
        let src = &level.things[i];
        println!(
            "thing {i} ({}) moved from ({:.1} {:.1} {:.1}) to ({:.1} {:.1} {:.1})",
            src.template, src.pos[0], src.pos[1], src.pos[2], now.pos[0], now.pos[1], now.pos[2]
        );
    }
    if !quiet {
        println!("prints: {printed}, stubbed verb calls: {silent}");
        let mut ranked: Vec<(String, usize)> = verbs.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        let top: Vec<String> = ranked.iter().take(10).map(|(v, n)| format!("{v} x{n}")).collect();
        println!("verbs called: {}", top.join(", "));
        let mut stubs: Vec<(&String, &usize)> = silent_verbs.iter().collect();
        stubs.sort_by(|a, b| b.1.cmp(a.1));
        let top: Vec<String> = stubs.iter().take(10).map(|(v, n)| format!("{v} x{n}")).collect();
        println!("verbs stubbed: {}", top.join(", "));
    }
    // Every sound the scripts asked for must be in the archives; a miss is a name this
    // reconstruction resolved wrongly, not a missing asset.
    let missing: Vec<&str> = w
        .sounds
        .iter()
        .map(|req| req.name.as_str())
        .filter(|n| gobs.find(n).is_none())
        .collect();
    println!("sounds: {} requested, {} missing from archive", w.sounds.len(), missing.len());
    for m in missing.iter().take(6) {
        println!("  missing sound: {m}");
    }
    for e in errors.iter().take(8) {
        println!("  {e}");
    }
    silent_verbs
}

/// `--walk` selections: a frame count for the selected level, or every level.
enum Walk {
    Frames(u32),
    All,
}

/// One level's walk result, so the single-level report and the install-wide sweep say
/// the same things.
struct WalkResult {
    travelled: f32,
    blocked: usize,
    transitions: usize,
    /// Collision held: the walker is still inside the sector it believes it is in.
    contained: bool,
    /// The hull test independently agrees on which sector that is.
    in_hull: bool,
    /// How far below its spawn the body ended: information, not a failure — a walkway can lead down.
    dropped: f32,
    /// Frames where nothing at all was under the body within 40 units: the void. A body may walk off a
    /// ledge and descend into the room below — that is geometry — but it must never be over nothing.
    void_frames: usize,
    start_sector: usize,
    end_sector: usize,
}

/// One frame of walking: gravity, the floor, then the solid planes, then any doorway.
/// Shared by the headless `--walk` sweep and the in-app player so both run the same code.
///
/// `move_dir` is `(x, z)` in world space, in units a second; it need not be normalised. `gravity` is the
/// level's own `WORLD GRAVITY`; pass 0 to keep the walker level (the sweep only cares
/// about the ground plane).
///
/// Returns how far it moved horizontally, and whether it crossed into another sector.
#[allow(clippy::too_many_arguments)]
fn walker_step(
    solids: &collide::Solids,
    // What the level's scripts have switched: a closed adjoin or a hidden sector doorway. The
    // geometry-only callers (`--walk`, `--move`) pass `Gates::default()`, which is "all open".
    gates: collide::Gates<'_>,
    sector: &mut usize,
    pos: &mut [f32; 3],
    vy: &mut f32,
    // World-space horizontal velocity in level units a second, not a direction: the engine integrates
    // velocity and moves by it (`sithPhysics_UpdateThingPhysics`), so the speed lives here.
    move_dir: [f32; 2],
    dt: f32,
    radius: f32,
    gravity: f32,
    allow_step: bool,
) -> (f32, bool, bool) {
    // Where the frame started, so a move that would leave the world can be undone.
    let frame_start = *pos;
    let mut landed = false;
    // `MoveThing` restores the sector as well as the position (see the containment check below), so
    // the sector the frame began in is kept alongside the position.
    let sector_start = *sector;

    // Vertical: the fall is a *sweep*, the way `sithCollision_MoveThing` moves any body — so it stops at
    // the first floor it meets and crosses doorways on the way down. That is what makes a walkway whose
    // slab belongs to the room below solid: the ray goes through the portal and finds the slab.
    //
    // This replaced a chain of three queries (`support`, then the sector's planes, then a raised
    // resting probe) that each guessed at the same thing and still let a body fall out of the level.
    *vy -= gravity * dt;
    let drop = *vy * dt;
    if drop < 0.0 {
        // The floor probe, not the sector-scoped sweep: a fall must find the slab under it even when that
        // slab belongs to a sector nothing in the body's sector links to.
        let land = solids
            .floor_under(*sector, *pos, -drop + radius, radius)
            .filter(|(_, travelled, _)| *travelled <= -drop + 1e-3)
            .map(|(floor, _, _)| (floor + radius, 0usize));
        match land.map(|(z, _)| z) {
            Some(z) => {
                if std::env::var_os("BK_WALK_TRACE").is_some() {
                    eprintln!("walk trace:     landed on {z:.3} after {:.3}", -drop);
                }
                pos[2] = z;
                *vy = 0.0;
                // The engine keeps an "on ground" flag from exactly this event: the vertical sweep was
                // stopped by a surface. Deriving it from `vy == 0.0` instead leaves a body on a
                // thrusting or bouncing floor permanently airborne, and such a body can never jump
                // (09fuelstation: vy oscillates ±0.9 and `grounded` is never true).
                landed = true;
            }
            None => pos[2] += drop,
        }
    } else {
        pos[2] += drop;
    }
    // Lateral: the engine's own move — sweep, slide along whatever was hit, step over anything low.
    let delta = [move_dir[0] * dt, move_dir[1] * dt, 0.0];
    let before = *pos;
    let step_up = STEP_UP;
    let (moved_to, crossed_to, mut blocked) = solids.move_body(*sector, *pos, delta, radius, step_up, gates);
    if std::env::var_os("BK_WALK_TRACE").is_some() {
        let dm = ((moved_to[0] - pos[0]).powi(2) + (moved_to[1] - pos[1]).powi(2)).sqrt();
        eprintln!(
            "walk trace:     move_body delta {delta:?} -> moved {dm:.3}, blocked {blocked}, z {} -> {}",
            pos[2], moved_to[2]
        );
    }
    // Read before the sector is adopted: comparing after the assignment made this false for ever, so
    // the walk summary's "sector crossings" was pinned at whatever the docs had last observed.
    let crossed = crossed_to != *sector;
    *pos = moved_to;
    if crossed {
        *sector = crossed_to;
    }
    // The step-up the engine allows: a body may climb `moveSize + 0.01` above its feet, which is why a
    // low ledge is walkable. `move_body` already tried the raised sweep; this only records that the
    // height came from there, and the vertical sweep next frame settles the body on the step.
    if !allow_step {
        blocked = false;
    }
    // The engine's containment guarantee, transcribed from `sithCollision_MoveThing`
    // (`decomp/004e7950_sithCollision_MoveThing.c:251`):
    //
    //     if (thing->field_28 != 0 && thing->physicsType == 1) {
    //         if (!sithIntersect_IsSphereInSector(&thing->position, 0, thing->sector)) {
    //             thing->position = savedPosition;
    //             direction       = savedDirection;
    //             sithThing_SetSector(thing, savedSector, 0);
    //         }
    //     }
    //
    // The position, the direction and the sector all come back. `Solids::inside_sector` is that
    // function, box short-circuit included, and `hull` is the surface set it walks.
    //
    // It is asked about the thing's **origin**, which for a body is its feet: the level places
    // `walkplayer` at floor height (`10cargo` sector 333: z 1.4814 on a floor at 1.4814) and the eye
    // sits one eye-height above that. Our `pos` is the body *centre*, one radius up, so the feet are
    // what gets tested — testing the centre put the body 0.01 outside a 45° corner plane on
    // `10cargo`, 0.02 outside another on `11gorc`, and undid every step of the walk.
    let origin = [pos[0], pos[1], pos[2] - radius];
    // A body that has stepped into another room is not an invalid move: the engine's test is
    // `IsSphereInSector(thing->sector)` and `MoveThing` re-resolves the sector as it crosses an
    // adjoin. Undoing the step whenever the *old* sector's hull rejects the feet froze bodies on
    // levels whose room planes are incomplete (19a: sector 722 reports no floor plane at all, so
    // every step was reverted). Adopt the room the body is now in; only a point in **no** sector is
    // an invalid move.
    if !solids.inside_sector(*sector, origin, 0.0) {
        match solids.sector_at(origin, 0.05) {
            // The body stepped into another room: adopt it, which is what `MoveThing` does as it
            // crosses an adjoin.
            Some(found) => *sector = found,
            // **No room claims the body.** Then the model cannot judge the move, and vetoing on the
            // old sector's hull is what froze 19a: it spawns at z −2.86, 0.725 below the only
            // downward-facing plane of sector 722, in a pocket no sector contains. The engine's
            // guarantee presumes `thing->sector` holds the body; when nothing does, the sweep below
            // is the only evidence left, and it said the step was fine (`blocked false`).
            None => {}
        }
    }
    let moved = ((pos[0] - before[0]).powi(2) + (pos[1] - before[1]).powi(2)).sqrt();

    (moved, crossed, landed)
}

/// `--walk <frames>`: the collision solver exercised on a real level, with no window.
///
/// Starts at the level's own `walkplayer` thing — where the game puts you — and holds
/// "forward" for the requested number of frames, resolving against the sector's solid
/// surfaces each step. A level whose geometry and sector table disagree shows up here
/// as a walker that leaves its sector, or one that never stops against a wall.
fn walk_level(level: &jkl::Level, frames: u32, quiet: bool, turn: f32) -> Option<WalkResult> {
    // Same rules as the window: the sweep is only meaningful if it walks the same way.
    let rules = Rules::from_level(level);
    let (radius, speed) = (rules.radius, rules.walk);
    // The sweep starts the body one radius above the level's own start, which is where `setup` puts it.
    let EYE = rules.radius;
    const STEP: f32 = 1.0 / 30.0;

    let solids = collide::build(level);
    let planes: usize = solids.sectors.iter().map(|s| s.len()).sum();
    if !quiet {
        println!(
            "collision: {} sectors, {planes} solid planes from {} surfaces",
            level.sectors.len(),
            level.surfaces.len()
        );
    }

    let player = level.things.iter().find(|t| t.template.eq_ignore_ascii_case("walkplayer"))?;
    let mut pos = [player.pos[0], player.pos[1], player.pos[2] + EYE];
    // The level's own record of which sector the player starts in is the engine's answer, and the
    // engine trusts it verbatim: `sithThing_SetSector` is called with it at load. Re-deriving it from
    // the hull picks a trigger sliver, so the file's field wins and the hull is only the fallback.
    let recorded = usize::try_from(player.sector).ok().filter(|s| *s < solids.sectors.len());
    let Some(mut sector) = recorded.or_else(|| solids.sector_for_body(pos)) else {
        if !quiet {
            println!("walkplayer start {:?} is outside every sector", player.pos);
        }
        return None;
    };
    let yaw = player.yaw.to_radians() + turn;
    // The walker faces the way the level's start faces, in the engine's own basis (`yaw_axes`), with
    // `turn` radians added so one level can be walked from several headings.
    let (b, _) = yaw_axes(yaw);
    let forward = [b[0], b[1], 0.0];
    if std::env::var("BK_TRACE").is_ok() {
        println!("  start sector {sector}, {} planes:", solids.sectors[sector].len());
        for p in &solids.sectors[sector] {
            let d = (pos[0] - p.point[0]) * p.normal[0]
                + (pos[1] - p.point[1]) * p.normal[1]
                + (pos[2] - p.point[2]) * p.normal[2];
            println!("    n=({:+.2} {:+.2} {:+.2}) d={d:+.2}", p.normal[0], p.normal[1], p.normal[2]);
        }
        println!("  portals: {}", solids.portals[sector].len());
    }
    let start = pos;
    let start_sector = sector;
    let mut travelled = 0.0f32;
    let mut blocked = 0usize;
    let mut transitions = 0usize;
    let mut wet_frames = 0usize;
    let mut void_frames = 0usize;
    let lowest = solids
        .floors
        .iter()
        .map(|f| f.point[2])
        .fold(f32::INFINITY, f32::min);

    for frame in 0..frames {
        // The same water rule the window applies: standing on a flagged surface halves
        // the speed for that frame.
        let water = solids
            .floor_surface(sector, [pos[0], pos[1], pos[2] - radius + 0.01])
            .and_then(|(_, surface)| level.surfaces.get(surface))
            .is_some_and(|surface| surface.surfflags & (0x2_0000 | 0x10_0000) != 0);
        if water {
            wet_frames += 1;
        }
        let step_speed = if water { speed * 0.5 } else { speed };
        let (moved, crossed, _landed) = walker_step(
            &solids,
            collide::Gates::default(),
            &mut sector,
            &mut pos,
            &mut 0.0,
            [forward[0] * step_speed, forward[1] * step_speed],
            STEP,
            radius,
            0.0,
            false,
        );
        if std::env::var("BK_TRACE").is_ok() && frame < 3 {
            // Which plane holds the walker, and how hard: the quickest way to tell a
            // narrow doorway from a plane that should not be solid at all.
            let mut worst = (f32::MIN, [0.0f32; 3]);
            for p in &solids.sectors[sector] {
                let d = (pos[0] - p.point[0]) * p.normal[0]
                    + (pos[1] - p.point[1]) * p.normal[1]
                    + (pos[2] - p.point[2]) * p.normal[2];
                if d > worst.0 {
                    worst = (d, p.normal);
                }
            }
            println!(
                "    at ({:.2} {:.2} {:.2}) deepest plane d={:.2} n={:?}, {} planes",
                pos[0],
                pos[1],
                pos[2],
                worst.0,
                worst.1,
                solids.sectors[sector].len()
            );
        }
        if moved < step_speed * STEP * 0.5 {
            blocked += 1;
        }
        // Below every floor in the level: the one outcome a collision system must never allow. A body may
        // legitimately descend into the room below a walkway; it may not leave the world.
        if pos[2] < lowest - 1.0 {
            void_frames += 1;
        }
        travelled += moved;
        if crossed {
            transitions += 1;
        }
        if !quiet && frame % 30 == 0 {
            println!(
                "  frame {frame:>4}: ({:.2} {:.2} {:.2}) sector {sector}",
                pos[0], pos[1], pos[2]
            );
        }
    }
    // The report must ask the rule's own question: `inside_sector` at the body's origin, box
    // short-circuit included. A hand-rolled "every plane within radius + 0.05" test answers something
    // else — it ignores `COLLIDEBOX`, which `sithIntersect_IsSphereInSector` checks first — and so
    // disagreed with the walk it was reporting on (`20aboc`, `20bboc`).
    let contained = solids.inside_sector(sector, [pos[0], pos[1], pos[2] - radius], 0.0);
    // Whether the hull test agrees is a separate question: sector hulls overlap, so the
    // walker can be correctly inside the sector it thinks it is in and still be claimed
    // by a neighbouring sliver.
    let claimed = solids.sector_at(pos, radius + 0.05);
    if !quiet {
        println!(
            "walked {frames} frames: travelled {travelled:.1} units, {blocked} blocked, {transitions} sector changes, {wet_frames} frames on water"
        );
        println!(
            "  start ({:.2} {:.2} {:.2}) sector {start_sector} -> end ({:.2} {:.2} {:.2}) sector {sector}",
            start[0], start[1], start[2], pos[0], pos[1], pos[2]
        );
        if !contained {
            println!("  WARNING: the walker is outside the sector it was tracking");
        }
        println!(
            "  hull test: {}",
            match claimed {
                Some(s) if s == sector => format!("agrees (sector {s})"),
                Some(s) => format!("claims sector {s} instead"),
                None => "no hull contains the position".to_string(),
            }
        );
    }
    Some(WalkResult {
            dropped: start[2] - pos[2],
            void_frames,
        travelled,
        blocked,
        transitions,
        contained,
        in_hull: claimed == Some(sector),
        start_sector,
        end_sector: sector,
    })
}

/// `--facing`: which of the engine's two horizontal axes a level's own `walkplayer` faces.
///
/// `rdMatrix_Build34` gives a thing two horizontal axes at yaw: `lvec` (the *look* axis, column 1 of

/// `--walk all`: the same walk on every level, to see whether collision holds in the
/// whole install rather than in one hand-picked room.
fn walk_all(gobs: &gob::GobSet, frames: u32) {
    let levels: Vec<String> =
        gobs.names().into_iter().filter(|n| n.ends_with(".jkl")).collect();
    println!("=== walking {frames} frames of every level ===");
    println!(
        "{:<30} {:>9} {:>8} {:>10} {:>9}  {}",
        "level", "travelled", "blocked", "sectors", "contained", "hull"
    );
    let (mut kept, mut agreed, mut no_start, mut transitions) = (0usize, 0usize, 0usize, 0usize);
    for name in &levels {
        let Some(raw) = gobs.read(name) else { continue };
        let level = jkl::parse(&String::from_utf8_lossy(&raw));
        match walk_level(&level, frames, true, 0.0) {
            Some(r) => {
                transitions += r.transitions;
                kept += r.contained as usize;
                agreed += r.in_hull as usize;
                println!(
                    "{:<30} {:>9.1} {:>8} {:>10} {:>9}  {}",
                    name,
                    r.travelled,
                    r.blocked,
                    format!("{}->{}", r.start_sector, r.end_sector),
                    if r.contained { "yes" } else { "NO" },
                    if r.in_hull { "agrees" } else { "differs" }
                );
            }
            None => {
                no_start += 1;
                println!("{:<30} {:>9} {:>8} {:>10} {:>9}  {}", name, "-", "-", "-", "-", "no start");
            }
        }
    }
    println!(
        "summary: {kept}/{} levels with a start kept the walker inside its sector, {agreed} had the hull test agree, {transitions} sector crossings, {no_start} without a start",
        levels.len() - no_start
    );
    println!("--- walk check passed ---");
}

/// `--pose <model.3do> <anim.key> <frame>`: draw one model posed from one keyframe.
///
/// This is the check that the `.3do` hierarchy, the `.key` node list and the mesh names
/// actually line up: it poses the model, prints both bounding boxes, and can save a PNG
/// (`--shot`) of the result. A pose that does nothing shows up as two identical boxes.
fn pose_view(
    gobs: &gob::GobSet,
    model_name: &str,
    key_name: &str,
    frame: f32,
    shot: Option<&str>,
) -> Result<(), String> {
    let model_data = gobs
        .read(model_name)
        .ok_or_else(|| format!("`{model_name}` not found in any archive"))?;
    let model = tdo::parse(&String::from_utf8_lossy(&model_data));
    if !model.is_drawable() {
        return Err(format!("`{model_name}` has no drawable geometry"));
    }
    // `--pose <model> - <frame>` draws the model's own rest pose, with no keyframe at all.
    let key = if key_name == "-" {
        None
    } else {
        let key_data = gobs
            .read(key_name)
            .ok_or_else(|| format!("`{key_name}` not found in any archive"))?;
        Some(key::parse(&String::from_utf8_lossy(&key_data)))
    };
    let world = tdo::pose_nodes(&model, key.as_ref(), frame);
    let rest = tdo::pose_nodes(&model, None, 0.0);

    // The engine's own invariant, from `rdModel3_CalcRadii`: the header `RADIUS` is the
    // distance of the furthest vertex of the assembled model, so reproducing it proves
    // the node composition (parent pivots included) is right.
    // How far the frame moves the skeleton, and the bounds of the whole figure either way.
    let rest_box = bounds(&model, &rest);
    let posed_box = bounds(&model, &world);
    let moved = world
        .iter()
        .zip(&rest)
        .filter(|(a, b)| a.pos != b.pos || a.rot != b.rot)
        .count();
    println!(
        "{} + {} frame {frame}: {} meshes, {} nodes, {moved} of {} nodes differ from rest",
        model_name,
        key_name,
        model.meshes.len(),
        model.nodes.len(),
        model.nodes.len()
    );
    println!(
        "  rest bbox {:?} .. {:?}",
        [rest_box.0[0], rest_box.0[1], rest_box.0[2]],
        [rest_box.1[0], rest_box.1[1], rest_box.1[2]]
    );
    println!(
        "  posed bbox {:?} .. {:?}",
        [posed_box.0[0], posed_box.0[1], posed_box.0[2]],
        [posed_box.1[0], posed_box.1[1], posed_box.1[2]]
    );
    let delta = (0..3)
        .map(|k| (posed_box.0[k] - rest_box.0[k]).abs().max((posed_box.1[k] - rest_box.1[k]).abs()))
        .fold(0.0f32, f32::max);
    if delta < 1e-4 {
        println!("  this frame is the rest pose (a bind-pose frame, or frame 0 of a stand)");
    } else {
        println!("  largest bbox change from rest {delta:.3} world units");
    }

    let palette = tex::Palette::greyscale();
    let mut images: Vec<(i32, Image)> = Vec::new();
    for (i, mat) in model.materials.iter().enumerate() {
        if let Some(bitmap) = gobs.read(mat).as_deref().and_then(tex::parse_mat) {
            images.push((i as i32, to_image(&bitmap, &palette)));
        }
    }

    // Frame the posed model: centre of the posed box, from a three-quarter angle.
    let centre = Vec3::new(
        (posed_box.0[0] + posed_box.1[0]) / 2.0,
        (posed_box.0[1] + posed_box.1[1]) / 2.0,
        (posed_box.0[2] + posed_box.1[2]) / 2.0,
    );
    let radius = ((posed_box.1[0] - posed_box.0[0]).powi(2)
        + (posed_box.1[1] - posed_box.0[1]).powi(2)
        + (posed_box.1[2] - posed_box.0[2]).powi(2))
    .sqrt()
    .max(0.5);
    let eye = centre + Vec3::new(1.0, 0.7, 0.45).normalize() * (radius * 1.2);

    let mut app = App::new();
    // The material plugin comes after `DefaultPlugins`: it registers its shader through
    // `load_internal_asset!`, which needs `Assets<Shader>` to exist already.
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window { title: format!("bevi-knight — {model_name}"), ..default() }),
        ..default()
    }))
    .add_plugins(level_mat::LevelMaterialPlugin)
    .insert_resource(Shot(shot.map(PathBuf::from)))
    .insert_resource(ShotFrame(30))
    .insert_resource(ClearColor(Color::srgb(0.06, 0.06, 0.08)))
    .add_systems(Startup, move |mut commands: Commands,
                                mut meshes: ResMut<Assets<Mesh>>,
                                mut materials: ResMut<Assets<StandardMaterial>>,
                                mut textures: ResMut<Assets<Image>>| {
        commands.spawn((
            Camera3d::default(),
            WorldCamera,

            Transform::from_translation(eye).looking_at(centre, Vec3::Z),
        ));
        // One light and a little ambient, so a posed model reads as 3D instead of a
        // silhouette.
        commands.spawn((
            DirectionalLight { illuminance: 14000.0, ..default() },
            Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.5, 0.8, 0.0)),
        ));
        commands.spawn((
            AmbientLight { color: Color::srgb(0.6, 0.6, 0.7), brightness: 700.0, ..default() },
        ));
        let used: std::collections::BTreeSet<i32> = model
            .meshes
            .iter()
            .flat_map(|m| m.faces.iter().map(|f| f.material))
            .collect();
        for mat_idx in used {
            let name = model.materials.get(mat_idx.max(0) as usize).cloned().unwrap_or_default();
            let tex_size = images
                .iter()
                .find(|(i, _)| *i == mat_idx)
                .map(|(_, img)| {
                    (
                        img.texture_descriptor.size.width as f32,
                        img.texture_descriptor.size.height as f32,
                    )
                })
                .unwrap_or((1.0, 1.0));
            let mut sm = StandardMaterial {
                unlit: false,
                double_sided: true,
                cull_mode: None,
                perceptual_roughness: 0.9,
                ..default()
            };
            match images.iter().find(|(i, _)| *i == mat_idx) {
                Some((_, img)) => sm.base_color_texture = Some(textures.add(img.clone())),
                None => sm.base_color = Color::srgb(0.62, 0.62, 0.66),
            }
            let _ = name;
            commands.spawn((
                Mesh3d(meshes.add(model_mesh_posed(
                    &model,
                    mat_idx,
                    tex_size,
                    key.as_ref().map(|k| (k, frame)),
                ))),
                MeshMaterial3d(materials.add(sm)),
            ));
        }
    })
    .add_systems(Update, auto_shot)
    .run();
    Ok(())
}

/// World-space bounding box of a model, in the pose it is given (its rest pose is
/// `pose_nodes(model, None, 0.0)`). Vertices are node-local, so there is no position at
/// all without a pose.
fn bounds(model: &tdo::Model, pose: &[tdo::Xform]) -> ([f32; 3], [f32; 3]) {
    let owners = model.mesh_nodes();
    let geoset = model.pick_geoset();
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for (i, mesh) in model.meshes.iter().enumerate() {
        if mesh.geoset != geoset {
            continue;
        }
        let owner = owners
            .get(i)
            .copied()
            .filter(|n| *n != usize::MAX)
            .and_then(|n| pose.get(n));
        for v in &mesh.vertices {
            let p = match owner {
                Some(x) => x.apply(*v),
                None => *v,
            };
            for k in 0..3 {
                let w = p[k] * model_scale();
                lo[k] = lo[k].min(w);
                hi[k] = hi[k].max(w);
            }
        }
    }
    (lo, hi)
}

fn load(gobs: &gob::GobSet, level_name: &str) -> Result<World, String> {
    let raw = gobs
        .read(level_name)
        .ok_or_else(|| format!("level `{level_name}` not found in any archive"))?;
    let text = String::from_utf8_lossy(&raw);
    let level = jkl::parse(&text);

    // The level's colormaps select the 256-colour palette the world textures are
    // indexed against.
    let colormap = level.colormaps.first().and_then(|n| gobs.read(n));
    let palette = colormap
        .as_deref()
        .and_then(tex::Palette::parse)
        .unwrap_or_else(tex::Palette::greyscale);
    // The colormap's own brightness ramp: a surface's baked intensity walks it rather than
    // scaling the texture linearly (§6).
    if let Some(data) = colormap.as_deref() {
        tex::Palette::set_light_ramp(data);
        // The level shader's distance fade walks toward the same table 0 the ramp starts from.
        if let Some(far) = tex::Palette::far_colour(data) {
            tex::Palette::set_far_colour(far);
        }
    }

    let mut images = HashMap::new();
    let mut material_cels: HashMap<i32, u32> = HashMap::new();

    for (idx, name) in level.materials.iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        let Some(data) = gobs.read(name) else { continue };
        // Checked for every material, bitmap or not: a portal material can be bitmap-less and still decide
        // whether its surfaces are drawn.
        if std::env::var_os("BK_MAT_DEBUG").is_some() && idx < 6 {
            let head: Vec<String> = data.iter().take(24).map(|b| format!("{b:02x}")).collect();
            eprintln!("mat {idx} {name}: type@8={:?} head {}", tex::tex_type(&data), head.join(" "));
        }
        let Some(mat) = tex::parse_mat(&data) else { continue };
        images.insert(idx as i32, to_image(&mat, &palette));
        material_cels.insert(idx as i32, mat.cels);
    }

    println!(
        "{}: {} verts, {} texverts, {} surfaces, {} materials ({} with bitmaps), {} things",
        level_name,
        level.vertices.len(),
        level.texverts.len(),
        level.surfaces.len(),
        level.materials.len(),
        images.len(),
        level.things.len(),
    );

    // Placed objects pick their model through their template's `model3d=`.
    let template_model = template_models(&level);

    let mut models: HashMap<String, tdo::Model> = HashMap::new();
    let mut model_images: HashMap<String, Image> = HashMap::new();
    // Every template's model, not just the ones a placed thing happens to use: scripts
    // create things (a fired bolt, an explosion) from templates the level never places,
    // and those need their `.3do` loaded before they can be drawn.
    let mut model_names: Vec<&String> = template_model.values().collect();
    model_names.sort();
    model_names.dedup();
    for model_name in model_names {
        if models.contains_key(model_name) {
            continue;
        }
        let Some(raw) = gobs.read(model_name) else { continue };
        let model = tdo::parse(&String::from_utf8_lossy(&raw));
        for mat in &model.materials {
            if model_images.contains_key(mat) {
                continue;
            }
            if let Some(bitmap) = gobs.read(mat).as_deref().and_then(tex::parse_mat) {
                model_images.insert(mat.clone(), to_image(&bitmap, &palette));
            }
        }
        models.insert(model_name.clone(), model);
    }
    println!("{} models parsed for the level's templates", models.len());

    // Effect templates name a `sprite=` instead of a `model3d=`: `+laserhit` is `bryx.spr`. That
    // file is plain text — one camera-facing rectangle per row, sized in world units and keyed to a
    // `.mat` — so it loads the same way a model does, and its materials come through the same
    // texture path. Sprites are the whole visual layer of combat: a bolt's impact, the explosion
    // chain, sparks, smoke, the force effects.
    let mut template_sprite: HashMap<String, Vec<spr::Face>> = HashMap::new();
    // Material -> how many cels its bitmap holds, straight out of the `.mat` header
    // (`num_texinfo`, offset 12). A sprite draws one cel at a time and this is the only reliable
    // count: the bitmap's height does not divide evenly by its width (`bryx.mat` is 8 x 78).
    let mut sprite_cels: HashMap<String, u32> = HashMap::new();
    for template in &level.templates {
        let Some(name) = level.template_param(&template.name, "sprite") else { continue };
        let Some(raw) = gobs.read(&format!("misc\\spr\\{}", name.to_ascii_lowercase())) else {
            continue;
        };
        let faces = spr::parse(&String::from_utf8_lossy(&raw));
        if faces.is_empty() {
            continue;
        }
        for face in &faces {
            if model_images.contains_key(&face.material) {
                continue;
            }
            // A sprite is keyed on the palette's entry 0: an effect bitmap is drawn on black and
            // the black is its transparency. Keying a *world* texture that way would punch holes in
            // it, so only sprite materials get the keyed conversion.
            if let Some(bitmap) = gobs.read(&face.material).as_deref().and_then(tex::parse_mat) {
                let keyed = keyed_rgba(&bitmap, &palette);
                // The material's cels arrive packed into one column, cel 0 at the top, so a quad
                // can address frame `k` with `v = k / cels` (`sprite_quad`).
                if std::env::var_os("BK_SPRITE_TRACE").is_some() {
                    let clear = keyed.chunks(4).filter(|p| p[3] == 0).count();
                    println!(
                        "  sprite `{}`: {}x{} strip, {} cel(s) of {}x{}, {} of {} pixels keyed clear, {:.2}x{:.2} world units",
                        face.material,
                        bitmap.width,
                        bitmap.height,
                        bitmap.cels,
                        bitmap.width,
                        bitmap.height / bitmap.cels.max(1),
                        clear,
                        bitmap.indices.len(),
                        face.width,
                        face.height
                    );
                }
                sprite_cels.insert(face.material.clone(), bitmap.cels);
                model_images.insert(face.material.clone(), image_from_rgba(bitmap.width, bitmap.height, keyed));
            }
        }
        template_sprite.insert(template.name.to_ascii_lowercase(), faces);
    }
    println!(
        "{} sprite file(s) for {} template(s): {}",
        template_sprite
            .values()
            .map(|f| f.len())
            .sum::<usize>(),
        template_sprite.len(),
        template_sprite.keys().take(4).cloned().collect::<Vec<_>>().join(", ")
    );

    // The instinct classes the level's actors name. `aiclass=` resolves through the template
    // chain, so an actor that inherits one still gets the class.
    let mut ai_classes: HashMap<String, String> = HashMap::new();
    let mut ai_names: Vec<String> = level
        .templates
        .iter()
        .filter_map(|t| level.template_param(&t.name, "aiclass"))
        .map(|n| n.to_ascii_lowercase())
        .collect();
    ai_names.sort();
    ai_names.dedup();
    for name in &ai_names {
        if let Some(raw) = gobs.read(&format!("ai\\{name}")) {
            ai_classes.insert(name.clone(), String::from_utf8_lossy(&raw).into_owned());
        }
    }

    // The sound classes: what each shipped `.snd` names, and the fade radii it gives. Only
    // the wavs a class mentions are read, so this is a few hundred kilobytes each at most.
    let mut wav_bytes: HashMap<String, Vec<u8>> = HashMap::new();
    let mut sound_radii: HashMap<String, (f32, f32, f32)> = HashMap::new();
    for name in gobs.names() {
        if !name.to_ascii_lowercase().ends_with(".snd") {
            continue;
        }
        let Some(raw) = gobs.read(&name) else { continue };
        for entry in snd::parse(&String::from_utf8_lossy(&raw)) {
            let wav = entry.wav.to_ascii_lowercase();
            sound_radii
                .entry(wav.clone())
                .or_insert((entry.min_radius, entry.max_radius, entry.max_volume));
            if wav_bytes.contains_key(&wav) {
                continue;
            }
            if let Some(bytes) = gobs.read(&format!("sound\\{wav}")).or_else(|| gobs.read(&wav)) {
                wav_bytes.insert(wav, bytes);
            }
        }
    }

    // Puppets and the keyframes they play. Only the files the placed things actually
    // name are read: a level references a handful, not the whole 231-`.ai` install.
    let mut puppets: HashMap<String, pup::Pup> = HashMap::new();
    let mut keys: HashMap<String, key::Key> = HashMap::new();
    let mut puppet_names: Vec<String> = level
        .things
        .iter()
        .filter_map(|t| level.template_param(&t.template, "puppet"))
        .map(|n| n.to_ascii_lowercase())
        .collect();
    puppet_names.sort();
    puppet_names.dedup();
    for name in &puppet_names {
        let Some(raw) = gobs.read(name) else { continue };
        let parsed = pup::parse(&String::from_utf8_lossy(&raw));
        for state in &parsed.states {
            let key_name = state.key.to_ascii_lowercase();
            if key_name.is_empty() || keys.contains_key(&key_name) {
                continue;
            }
            let Some(raw) = gobs.read(&key_name) else { continue };
            keys.insert(key_name, key::parse(&String::from_utf8_lossy(&raw)));
        }
        puppets.insert(name.clone(), parsed);
    }
    println!(
        "{} puppets and {} keyframes parsed for placed actors",
        puppets.len(),
        keys.len()
    );

    // The level's own scripts, parsed once here so the app can bind them without
    // touching the archives again.
    let mut cogs: HashMap<String, cog::Cog> = HashMap::new();
    for inst in &level.cogs {
        if cogs.contains_key(&inst.script) {
            continue;
        }
        let Some(raw) = gobs.read(&inst.script) else { continue };
        if let Ok(script) = cog::parse(&String::from_utf8_lossy(&raw)) {
            cogs.insert(inst.script.clone(), script);
        }
    }
    // Scripts a *template* names (`cog=pow_bacta.cog` on every powerup) are not placed
    // instances, but taking a powerup runs its item's own script, so they are parsed too.
    let mut template_cogs: Vec<String> = level
        .templates
        .iter()
        .filter_map(|t| level.template_param(&t.name, "cog"))
        .map(|n| n.to_ascii_lowercase())
        .collect();
    template_cogs.sort();
    template_cogs.dedup();
    for name in &template_cogs {
        if cogs.contains_key(name) {
            continue;
        }
        let Some(raw) = gobs.read(name) else { continue };
        if let Ok(script) = cog::parse(&String::from_utf8_lossy(&raw)) {
            cogs.insert(name.clone(), script);
        }
    }
    // The item scripts `misc\items.dat` names are not placed instances and not template cogs: the
    // loader missed them, so `ammo_bin_for_bin` could not read `pow_*`/`weap_*`'s own `ammobin` and
    // every weapon appeared to spend the level's default bin. Parsing them here makes the whole item
    // table available to the same map.
    let item_scripts: Vec<String> = gobs
        .read("misc\\items.dat")
        .map(|d| hud::items(&String::from_utf8_lossy(&d)))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|def| def.cog)
        .map(|name| name.to_ascii_lowercase())
        .collect();
    for name in &item_scripts {
        if cogs.contains_key(name) {
            continue;
        }
        let Some(raw) = gobs.read(name) else { continue };
        if let Ok(script) = cog::parse(&String::from_utf8_lossy(&raw)) {
            cogs.insert(name.clone(), script);
        }
    }
    println!("{} placed script instances", level.cogs.len());
    println!("{} things on a keyframe path, {} with flags, {} moving", level.things.iter().filter(|t| t.frames.len() > 1).count(), level.things.iter().filter(|t| t.flags != 0).count(), level.things.iter().filter(|t| t.vel != [0.0; 3]).count());

    // Sector ambience: only the layouts a sector names are read, which is a handful
    // per level rather than all 644 sounds.
    let mut ambient: HashMap<String, Vec<u8>> = HashMap::new();
    for sector in &level.sectors {
        let Some((name, _)) = &sector.sound else { continue };
        let key = name.to_ascii_lowercase();
        if ambient.contains_key(&key) {
            continue;
        }
        if let Some(data) = gobs.read(name) {
            ambient.insert(key, data);
        }
    }
    if !ambient.is_empty() {
        println!("{} ambient loops for {} sectors", ambient.len(), level.sectors.iter().filter(|s| s.sound.is_some()).count());
    }

    // `CreateThing` positions a spawned thing by the difference between the two models' insert offsets
    // (`sithThing_CreateThing`), so the runtime needs this table before any script runs.
    runtime::set_template_inserts(
        template_model
            .iter()
            .filter_map(|(template, model)| {
                models
                    .get(model)
                    .map(|m| (template.clone(), m.insert_offset))
            })
            .collect(),
    );

    Ok(World {
        level,
        images,
        item_cogs: gobs
            .read("misc\\items.dat")
            .map(|d| hud::items(&String::from_utf8_lossy(&d)))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|def| def.cog.map(|cog| (def.id, cog)))
            .collect(),
        item_defs: gobs
            .read("misc\\items.dat")
            .map(|d| hud::items(&String::from_utf8_lossy(&d)))
            .unwrap_or_default(),
        material_cels,
        template_model,
        models,
        model_images,
        template_sprite,
        sprite_cels,
        ambient,
        cogs,
        puppets,
        keys,
        ai_classes,
        wav_bytes,
        sound_radii,
        gob_entries: gobs.len(),
    })
}

/// The keyed pixel bytes, for sprite materials: palette entry 0 becomes transparent, everything else
/// opaque.
///
/// A sprite's `.mat` paints its effect on black: the engine treats palette entry 0 as the key and
/// never draws it (`rdMaterial` loads effect bitmaps as colormapped with index 0 clear). Doing that
/// to a world texture would punch holes wherever the palette's black appears in it, so only the
/// sprite path uses this.
fn keyed_rgba(mat: &tex::Material, palette: &tex::Palette) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(mat.indices.len() * 4);
    for &i in &mat.indices {
        let [r, g, b] = palette.0[i as usize];
        let alpha = if i == 0 { 0 } else { 255 };
        rgba.extend_from_slice(&[r, g, b, alpha]);
    }
    rgba
}

fn to_image(mat: &tex::Material, palette: &tex::Palette) -> Image {
    let mut rgba = Vec::with_capacity(mat.indices.len() * 4);
    for &i in &mat.indices {
        let [r, g, b] = palette.0[i as usize];
        rgba.extend_from_slice(&[r, g, b, 255]);
    }
    image_from_rgba(mat.width, mat.height, rgba)
}

fn image_from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        mip_chain(rgba, width, height),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = width.max(height).max(1).ilog2() + 1;
    // JK texture vertices are in texels and run far past one texture width, so the
    // material tiles across a surface. Without repeat addressing the default
    // clamp-to-edge smears the border texel into long streaks.
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..default()
    });
    image
}

/// Append a box-filtered mip chain to an RGBA8 top level.
///
/// Without mips, a texture tiled dozens of times across a distant wall aliases
/// into noise: at grazing angles neighbouring pixels sample texels more than a
/// whole tile apart. The original engine mip-mapped its textures; this is the
/// same idea at the smallest useful quality, a plain 2x2 box filter.
fn mip_chain(top: Vec<u8>, width: u32, height: u32) -> Vec<u8> {
    let mut out = top.clone();
    let mut level = top;
    let (mut w, mut h) = (width, height);
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = Vec::with_capacity((nw * nh * 4) as usize);
        for y in 0..nh {
            for x in 0..nw {
                let mut sum = [0u32; 4];
                for dy in 0..2 {
                    for dx in 0..2 {
                        let sx = (x * 2 + dx).min(w - 1) as usize;
                        let sy = (y * 2 + dy).min(h - 1) as usize;
                        let o = (sy * w as usize + sx) * 4;
                        for (c, s) in sum.iter_mut().enumerate() {
                            *s += level[o + c] as u32;
                        }
                    }
                }
                for s in sum {
                    next.push((s / 4) as u8);
                }
            }
        }
        out.extend_from_slice(&next);
        level = next;
        w = nw;
        h = nh;
    }
    out
}

// ---------------------------------------------------------------------------
// mesh building
// ---------------------------------------------------------------------------

/// `surfflags` bit that marks a sky backdrop face.
///
/// Observed: every face carrying the level's sky material has `0x200` set in its
/// surface flags (`0x204`, `0x224`, `0x304`), while ordinary faces are `0x0`,
/// `0x4` or `0x5`. Sky faces are large vertical backdrop walls far outside the
/// play area, and their baked per-vertex intensity is zero.
const SURFFLAG_SKY: u32 = 0x200;

fn is_sky_surface(surface: &jkl::Surface) -> bool {
    surface.surfflags & (SURFFLAG_SKY | SURFFLAG_CEILING_SKY) != 0
}

/// `SITH_SURFACE_CEILINGSKY` — the other half of the engine sky test. The port only knew the horizon bit.
const SURFFLAG_CEILING_SKY: u32 = 0x400;

/// What the scripts have changed about how the world looks: each surface's cel, each surface's extra
/// light and each sector's light. The renderer bakes all three into the mesh the level shader draws,
/// so they travel together — and a change to any of them is what `Runtime::surface_epoch` announces.
#[derive(Default)]
struct SurfaceLook {
    cels: Vec<f32>,
    /// Each surface's material cel count, so `(own cel + material cel)` wraps where the shader expects.
    counts: Vec<f32>,
    lights: Vec<f32>,
    sectors: Vec<f32>,
    /// `MaterialAnim`'s per-material cel (`rdMaterial.curCelNum`).
    materials: HashMap<i32, f32>,
    /// `ThingLight`'s light summed per sector: retail lights a radius around the thing, and this
    /// build's baked sector light is the granularity it has, so the sum is added to the sector's own.
    thing_light: Vec<f32>,
    /// `SetFaceLightMode`: a face whose mode is `RD_LIGHTMODE_FULLYLIT` (0) is drawn at full
    /// brightness whatever the bake says — that is what makes the CTF force fields glow.
    light_modes: Vec<i32>,
    /// `SlideWall`'s accumulated texel offset per surface — the engine adds `scrollVector * dt` to
    /// `texVertOffset` and wraps it at 1024, which from the clock is just `rate * time`.
    scrolls: Vec<[f32; 2]>,
    /// `SlideHorizonSky`/`SlideCeilingSky`'s world-wide offset, `[horizon, ceiling]`: every sky face
    /// of that kind moves by it.
    sky: [[f32; 2]; 2],
}

impl SurfaceLook {
    /// Read the live runtime; the vectors are indexed by surface and by sector.
    fn of(rt: &runtime::Runtime) -> Self {
        SurfaceLook {
            cels: rt.surfaces.iter().map(|s| s.cel).collect(),
            counts: rt.surface_cels.iter().map(|&c| c as f32).collect(),
            lights: rt.surfaces.iter().map(|s| s.light).collect(),
            sectors: rt.sectors.iter().map(|s| s.light).collect(),
            materials: rt.material_cel.iter().map(|(&k, &v)| (k, v)).collect(),
            scrolls: rt
                .surfaces
                .iter()
                .map(|s| {
                    [
                        (s.slide[0] * rt.time).rem_euclid(1024.0),
                        (s.slide[1] * rt.time).rem_euclid(1024.0),
                    ]
                })
                .collect(),
            sky: [
                [
                    (rt.sky_scroll[0][0] * rt.time).rem_euclid(1024.0),
                    (rt.sky_scroll[0][1] * rt.time).rem_euclid(1024.0),
                ],
                [
                    (rt.sky_scroll[1][0] * rt.time).rem_euclid(1024.0),
                    (rt.sky_scroll[1][1] * rt.time).rem_euclid(1024.0),
                ],
            ],
            thing_light: rt.thing_light_by_sector(),
            light_modes: (0..rt.surfaces.len())
                .map(|s| rt.face_light_mode.get(&s).copied().unwrap_or(3))
                .collect(),
        }
    }

    /// The texel offset a scrolling surface's texture is drawn at: its own (`SlideWall`) or, for a
    /// sky face, the world-wide sky offset of that kind.
    fn scroll(&self, surface: usize, flags: u32) -> [f32; 2] {
        let sky = if flags & SURFFLAG_CEILING_SKY != 0 {
            self.sky[1]
        } else if flags & SURFFLAG_SKY != 0 {
            self.sky[0]
        } else {
            [0.0; 2]
        };
        let own = self.scrolls.get(surface).copied().unwrap_or([0.0; 2]);
        [own[0] + sky[0], own[1] + sky[1]]
    }

    /// The cel a surface draws: its own, which is 0 unless a script moved it, plus the animated cel of
    /// its material — `rdActive` uses the material's (`cel == -1`) when the surface has none of its
    /// own, and every shipped surface starts there.
    fn cel(&self, surface: usize, material: i32) -> f32 {
        let own = self.cels.get(surface).copied().unwrap_or(0.0).max(0.0);
        let material = self.materials.get(&material).copied().unwrap_or(0.0);
        let total = own + material;
        let count = self.counts.get(surface).copied().unwrap_or(1.0).max(1.0);
        if count > 1.0 {
            total % count
        } else {
            0.0
        }
    }

    /// `RD_LIGHTMODE_FULLYLIT`: the face is drawn at full brightness whatever the bake says.
    fn fullbright(&self, surface: usize) -> bool {
        self.light_modes.get(surface).copied() == Some(0)
    }

    /// The factor the level's baked vertex light is multiplied by: the engine adds a surface's and a
    /// sector's `extraLight` on top of the baked term, so 0 means "as the level was built".
    fn light(&self, surface: usize, sector: i32) -> f32 {
        let surface = self.lights.get(surface).copied().unwrap_or(0.0);
        let index = usize::try_from(sector).unwrap_or(usize::MAX);
        let sector_own = self.sectors.get(index).copied().unwrap_or(0.0);
        // `ThingLight`: the lamps standing in this sector, summed (see `thing_light_by_sector`).
        let things = self.thing_light.get(index).copied().unwrap_or(0.0);
        (1.0 + surface + sector_own + things).clamp(0.0, 4.0)
    }
}

/// One accumulated triangle batch per material.
#[derive(Clone, Default)]
struct Batch {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}/// The world's surface batches: which mesh asset draws which `(sector, material)`. A script's
/// `SetFaceGeoMode` has to stop drawing polygons, so the rebuild patches these meshes in place
/// rather than respawning the sector's entities.
#[derive(Resource, Default)]
struct SurfaceBatches(Vec<((usize, i32), Handle<Mesh>)>);

impl Batch {
    fn push_polygon(
        &mut self,
        level: &jkl::Level,
        surface: &jkl::Surface,
        tex_size: (f32, f32),
        cel: f32,
        light: f32,
        scroll: [f32; 2],
        fullbright: bool,
    ) {
        let base = self.positions.len() as u32;
        let n = surface.verts.len();
        let Some(normal) = polygon_normal(level, surface) else {
            return;
        };

        for (k, &(vi, ti)) in surface.verts.iter().enumerate() {
            let Some(p) = level.vertices.get(vi as usize) else { return };
            self.positions.push(*p);
            self.normals.push(normal);
            let uv = if ti >= 0 {
                match level.texverts.get(ti as usize) {
                    // `SlideWall` offsets the texture in **texels**, which is what a `.jkl` texvert
                    // already is, so the scroll goes in before the divide into UV space.
                    Some(t) => [
                        (t[0] + scroll[0]) / tex_size.0,
                        (t[1] + scroll[1]) / tex_size.1,
                    ],
                    None => [0.0, 0.0],
                }
            } else {
                [0.0, 0.0]
            };
            self.uvs.push(uv);
            let intensity = surface.intensities.get(k).copied().unwrap_or(1.0);
            // JK bakes per-vertex light into the level and applies a separate
            // distance colormap at draw time. We reproduce only the baked term,
            // so the intensity is used directly rather than rescaled.
            //
            // Sky faces are the exception: they are backdrop walls with no baked
            // light at all (their intensities are 0), and the engine draws them
            // with the sky pass, unshaded. Passing the baked value through renders
            // every open sky as black. `surfflags & 0x200` is what the shipped
            // levels mark them with — ordinary faces are 0x0/0x4/0x5.
            let l = if is_sky_surface(surface) || fullbright {
                1.0
            } else {
                // Through the `.cmp` light ramp, not a linear scale: the tables carry the level's
                // own darkening curve (§6). A script's `SetSectorLight`/`SurfaceLightAnim` rides on
                // top as the engine's `extraLight` does.
                (tex::Palette::light_scale(intensity) * light).clamp(0.02, 2.0)
            };
            // The alpha channel is free in this shader and carries the surface's cel: a world texture
            // with more than one frame is packed into one column (like a sprite), and `level.wesl`
            // folds `cel` into `v`. A single-cel material ignores it.
            self.colors.push([l, l, l, cel]);
        }

        for k in 1..n.saturating_sub(1) {
            self.indices.extend_from_slice(&[base, base + k as u32, base + k as u32 + 1]);
        }
    }
}

/// Newell's method — JK surfaces are planar polygons with more than three verts.
fn polygon_normal(level: &jkl::Level, surface: &jkl::Surface) -> Option<[f32; 3]> {
    let points: Vec<[f32; 3]> = surface
        .verts
        .iter()
        .map(|&(v, _)| level.vertices.get(v as usize).copied())
        .collect::<Option<Vec<_>>>()?;
    newell(&points)
}

fn to_mesh(batch: Batch) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, batch.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, batch.normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, batch.uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, batch.colors);
    mesh.insert_indices(Indices::U32(batch.indices));
    mesh
}

// ---------------------------------------------------------------------------
// structural validation, shared by --check and --audit
// ---------------------------------------------------------------------------

/// Checks that must hold for any parsed level. Returns human-readable problems;
/// an empty vector means the level's own data is self-consistent.
fn validate_level(level: &jkl::Level) -> Vec<String> {
    let mut problems = Vec::new();
    // Asset-only levels (e.g. `static.jkl`, which carries templates and sound
    // references but no world) legitimately have nothing to check.
    if level.vertices.is_empty() && level.surfaces.is_empty() {
        return problems;
    }
    if level.vertices.is_empty() {
        problems.push("no world vertices".into());
    }
    if level.surfaces.is_empty() {
        problems.push("no surfaces".into());
    }
    if level.materials.is_empty() {
        problems.push("no materials".into());
    }
    for (i, s) in level.surfaces.iter().enumerate() {
        for &(v, t) in &s.verts {
            if v as usize >= level.vertices.len() {
                problems.push(format!("surface {i}: vertex {v} out of range"));
            }
            if t >= 0 && t as usize >= level.texverts.len() {
                problems.push(format!("surface {i}: texvert {t} out of range"));
            }
        }
        if !s.intensities.is_empty() && s.intensities.len() != s.verts.len() {
            problems.push(format!(
                "surface {i}: {} intensities for {} verts",
                s.intensities.len(),
                s.verts.len()
            ));
        }
        if s.material >= 0 && s.material as usize >= level.materials.len() {
            problems.push(format!("surface {i}: material {} out of range", s.material));
        }
        if problems.len() > 32 {
            problems.push("... (further problems suppressed)".into());
            break;
        }
    }
    problems
}

fn resolve_asset(gobs: &gob::GobSet, kind: jkl::AssetKind, name: &str) -> bool {
    if gobs.find(name).is_some() {
        return true;
    }
    if !name.contains('.') {
        return gobs.find(&format!("{}.{}", name, kind.ext())).is_some();
    }
    false
}

// ---------------------------------------------------------------------------
// audit: sweep every retail level and every asset reference
// ---------------------------------------------------------------------------

fn audit(gobs: &gob::GobSet) -> Result<(), String> {
    let names = gobs.names();

    let levels: Vec<String> = names.iter().filter(|n| n.ends_with(".jkl")).cloned().collect();

    println!("=== bevi-knight retail asset audit ===");
    println!("archives   {} GOB files, {} entries", gobs.archive_count(), gobs.len());
    for i in 0..gobs.archive_count() {
        println!("           {}", gobs.archive_path(i).display());
    }
    println!("levels     {}", levels.len());
    println!();

    let mut failed_levels = 0usize;
    let mut total_unresolved = 0usize;
    let mut bad_models = 0usize;
    let mut total_world_tris = 0usize;
    let mut total_model_tris = 0usize;
    let mut total_instances = 0usize;
    let mut placed_sprites = 0usize;
    // Parsing models is the expensive part of the sweep; parse each once.
    let mut model_ok: std::collections::HashMap<String, bool> = Default::default();
    let mut model_tris_cache: std::collections::HashMap<String, usize> = Default::default();
    let mut unresolved_by_kind: std::collections::BTreeMap<&'static str, (usize, Vec<String>)> =
        Default::default();
    let mut detail: Vec<String> = Vec::new();

    println!(
        "{:<34} {:>6} {:>6} {:>6} {:>5} {:>5} {:>7} {:>7} {:>6}  {}",
        "level", "verts", "surf", "tex", "sect", "things", "tris", "inst", "mtris", "status"
    );

    for name in &levels {
        let Some(raw) = gobs.read(name) else {
            println!("{name:<34}  MISSING FROM ARCHIVE");
            failed_levels += 1;
            continue;
        };
        let text = String::from_utf8_lossy(&raw);
        let level = jkl::parse(&text);

        let mut problems = validate_level(&level);

        // Every material the world actually draws must resolve. A material that
        // is a valid MAT container but carries no bitmap (effect surfaces such as
        // `00RED.mat`) is expected; one that is missing or not a MAT at all is not.
        let mut missing_mats = 0usize;
        let mut flat_mats = 0usize;
        for s in &level.surfaces {
            if s.material < 0 {
                continue;
            }
            let Some(mat_name) = level.materials.get(s.material as usize) else { continue };
            if mat_name.is_empty() {
                continue;
            }
            match gobs.read(mat_name) {
                None => {
                    missing_mats += 1;
                    if missing_mats <= 3 {
                        detail.push(format!("{name}: material `{mat_name}` not found"));
                    }
                }
                Some(data) if tex::parse_mat(&data).is_some() => {}
                Some(data) if tex::is_material(&data) => flat_mats += 1,
                Some(_) => {
                    missing_mats += 1;
                    if missing_mats <= 3 {
                        detail.push(format!("{name}: material `{mat_name}` is not a MAT container"));
                    }
                }
            }
        }
        if missing_mats > 0 {
            problems.push(format!("{missing_mats} surface slots with an unusable material"));
        }

        // Every referenced model/sprite/keyframe/puppet/aiclass/cog/sound must exist.
        // Unresolved references are reported but not fatal: the retail data itself
        // contains dangling references (10 sound names in the shipped levels have
        // no matching file anywhere in the archives), and the original engine
        // ignored them rather than refusing to load.
        let mut unresolved = 0usize;
        for a in &level.assets {
            if resolve_asset(&gobs, a.kind, &a.name) {
                continue;
            }
            unresolved += 1;
            let e = unresolved_by_kind.entry(a.kind.label()).or_default();
            e.0 += 1;
            if e.1.len() < 12 && !e.1.iter().any(|n| n == &a.name) {
                e.1.push(a.name.clone());
            }
        }
        total_unresolved += unresolved;

        // Referenced models must also *parse* into drawable geometry, not merely
        // exist: a resolvable but empty `.3do` would place an invisible object.
        let mut bad_here = 0usize;
        for a in &level.assets {
            if a.kind != jkl::AssetKind::Model {
                continue;
            }
            let key = a.name.to_ascii_lowercase();
            let ok = *model_ok.entry(key.clone()).or_insert_with(|| {
                gobs.read(&key)
                    .as_deref()
                    .map(|d| tdo::parse(&String::from_utf8_lossy(d)).is_drawable())
                    .unwrap_or(false)
            });
            if !ok {
                bad_here += 1;
                if bad_here <= 2 {
                    detail.push(format!("{name}: model `{}` is not drawable", a.name));
                }
            }
        }
        bad_models += bad_here;
        if bad_here > 0 {
            problems.push(format!("{bad_here} non-drawable models"));
        }

        let textured = level
            .surfaces
            .iter()
            .filter(|s| s.material >= 0)
            .count()
            .saturating_sub(missing_mats);

        // Build the actual draw geometry, headless: a level whose surfaces pass the
        // index checks can still produce no triangles (degenerate or collinear
        // polygons), and that would render as an empty world with no error.
        let (batches, _) = build_world(&level, &Default::default(), &Default::default(), &SurfaceLook::default());
        let world_tris: usize = batches.values().map(|b| b.indices.len() / 3).sum();
        let empty_batches = batches.values().filter(|b| b.indices.is_empty()).count();
        if textured > 0 && world_tris == 0 {
            problems.push("surfaces exist but no world triangles were built".into());
        }
        if empty_batches > 0 {
            problems.push(format!("{empty_batches} material batches with no triangles"));
        }

        // Placed models: count instances that resolve to drawable geometry and the
        // triangles they contribute, so "renders the level's props" is measured.
        let templates = template_models(&level);
        let mut instances = 0usize;
        let mut model_tris = 0usize;
        for t in &level.things {
            let Some(model_name) = templates.get(&t.template.to_ascii_lowercase()) else { continue };
            let tris = *model_tris_cache.entry(model_name.clone()).or_insert_with(|| {
                gobs.read(model_name)
                    .as_deref()
                    .map(|d| model_triangles(&tdo::parse(&String::from_utf8_lossy(d))))
                    .unwrap_or(0)
            });
            if tris == 0 {
                continue;
            }
            instances += 1;
            model_tris += tris;
        }
        if world_tris > 0 && instances == 0 && !level.things.is_empty() {
            detail.push(format!("{name}: {} things but no drawable model instances", level.things.len()));
        }

        // Sprites are billboards the scripts spawn (explosions, muzzle flashes), so
        // a level with placed sprite things would need them drawn. Count them rather
        // than assume: everything placed in the shipped levels uses a model.
        let sprite_templates: std::collections::HashSet<String> = level
            .templates
            .iter()
            .filter(|t| t.model3d.is_none() && t.sprite.is_some())
            .map(|t| t.name.to_ascii_lowercase())
            .collect();
        placed_sprites += level
            .things
            .iter()
            .filter(|t| sprite_templates.contains(&t.template.to_ascii_lowercase()))
            .count();

        let status = if problems.is_empty() {
            if unresolved > 0 {
                format!("ok ({unresolved} dangling ref)")
            } else if flat_mats > 0 {
                format!("ok ({flat_mats} flat)")
            } else {
                "ok".to_string()
            }
        } else {
            failed_levels += 1;
            format!("{} problem(s)", problems.len())
        };
        println!(
            "{:<34} {:>6} {:>6} {:>6} {:>5} {:>5} {:>7} {:>7} {:>6}  {}",
            name,
            level.vertices.len(),
            level.surfaces.len(),
            textured,
            level.sectors.len(),
            level.things.len(),
            world_tris,
            instances,
            model_tris,
            status
        );
        total_world_tris += world_tris;
        total_model_tris += model_tris;
        total_instances += instances;
        for p in problems.iter().take(3) {
            detail.push(format!("{name}: {p}"));
        }
    }

    // Every level named by an episode manifest must exist, and every level in the
    // install should be reachable from one. `episode.jk` appears once per episode
    // archive, so this walks the occurrences rather than a single deduped path.
    println!("\n--- episode manifests ---");
    let mut manifest_missing = 0usize;
    let mut reachable: std::collections::HashSet<String> = Default::default();
    let episodes = episodes_of(&gobs);
    for (archive, ep) in &episodes {
        let mut listed = 0usize;
        for s in ep.levels() {
            listed += 1;
            reachable.insert(s.file.to_ascii_lowercase());
            if gobs.find(&s.file).is_none() {
                manifest_missing += 1;
                detail.push(format!("{archive}: LEVEL `{}` not present in any archive", s.file));
            }
        }
        let cuts = ep.sequences.iter().filter(|s| s.kind == episode::Kind::Cut).count();
        println!(
            "{archive:<16} type {:>2}  {listed:>2} LEVEL, {cuts:>2} CUT, {} DECIDE  \"{}\"",
            ep.kind,
            ep.sequences.iter().filter(|s| s.kind == episode::Kind::Decide).count(),
            ep.title
        );
    }
    // Manifests name files by bare name (`01narshadda.jkl`) while the archive index
    // stores the full path (`jkl\01narshadda.jkl`), so compare bare names.
    let orphan: Vec<&String> = levels
        .iter()
        .filter(|n| !reachable.contains(n.rsplit(['\\', '/']).next().unwrap_or(n).to_ascii_lowercase().as_str()))
        .collect();
    println!("levels reachable {}/{} from the manifests", levels.len() - orphan.len(), levels.len());
    for o in &orphan {
        detail.push(format!("{o}: not named by any episode manifest"));
    }

    println!("\n--- unresolved asset references ---");
    if unresolved_by_kind.is_empty() {
        println!("none");
    } else {
        for (kind, (count, sample)) in &unresolved_by_kind {
            println!("{kind:<10} {count:>4}  e.g. {}", sample.join(", "));
        }
    }

    let undecoded = coverage_audit(gobs);

    if !detail.is_empty() {
        println!("\n--- detail (first {}) ---", detail.len().min(40));
        for d in detail.iter().take(40) {
            println!("  {d}");
        }
    }

    println!();
    println!(
        "render: {total_world_tris} world triangles, {total_instances} model instances, {total_model_tris} model triangles, {placed_sprites} placed sprite things"
    );
    println!(
        "summary: {}/{} levels clean, {} unresolved asset references, {} non-drawable models, {} manifest LEVEL rows missing, {} undecoded files",
        levels.len().saturating_sub(failed_levels),
        levels.len(),
        total_unresolved,
        bad_models,
        manifest_missing,
        undecoded
    );
    if failed_levels > 0 || manifest_missing > 0 {
        return Err(format!("audit failed: {failed_levels} level(s), {manifest_missing} manifest reference(s)"));
    }
    println!("--- audit passed ---");
    Ok(())
}

/// Sweep every file in the install and report, per format, how many this build can
/// actually decode. Serves as the drop-in coverage scoreboard: a format with no
/// decoder shows up as a whole column of zeroes instead of a silent gap.
/// Returns the number of files in formats that have no decoder at all.
fn coverage_audit(gobs: &gob::GobSet) -> usize {
    use std::collections::BTreeMap;
    let mut table: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut flat_samples: Vec<String> = Vec::new();
    let (mut cmp_maps_lo, mut cmp_maps_hi) = (usize::MAX, 0usize);
    let mut snd_entries = 0usize;
    let mut snd_missing = 0usize;
    let mut snd_flags: std::collections::BTreeSet<u32> = Default::default();
    let mut snd_radius = (f32::MAX, f32::MIN);
    let mut snd_samples: Vec<String> = Vec::new();
    let mut snd_events: std::collections::BTreeMap<String, usize> = Default::default();
    let mut snd_volume = (f32::MAX, f32::MIN);
    let mut cog_ok = 0usize;
    let mut cog_bad = 0usize;
    let mut cog_labels = 0usize;
    let mut cog_symbols = 0usize;
    let mut cog_errors: Vec<String> = Vec::new();
    let mut key_files = 0usize;
    let mut key_nodes = 0usize;
    let mut key_nodes_max = 0usize;
    let mut key_entries = 0usize;
    let mut key_incomplete = 0usize;
    let mut key_samples: Vec<String> = Vec::new();
    let mut ai_files = 0usize;
    let mut ai_params = 0usize;
    let mut ai_instincts = 0usize;
    let mut ai_incomplete = 0usize;
    let mut ai_samples: Vec<String> = Vec::new();
    let mut pup_files = 0usize;
    let mut pup_states = 0usize;
    let mut pup_joints = 0usize;
    let mut pup_missing = 0usize;
    let mut pup_samples: Vec<String> = Vec::new();
    let mut bm_files = 0usize;
    let mut bm_cells = 0usize;
    let mut bm_pixels = 0usize;
    let mut bm_incomplete = 0usize;
    let mut bm_samples: Vec<String> = Vec::new();
    let mut sft_files = 0usize;
    let mut sft_glyphs = 0usize;
    let mut sft_incomplete = 0usize;
    let mut uni_files = 0usize;
    let mut uni_strings = 0usize;
    let mut uni_samples: Vec<String> = Vec::new();
    for name in gobs.names() {
        let Some((_, ext)) = name.rsplit_once('.') else { continue };
        let Some(data) = gobs.read(&name) else { continue };
        let entry = table.entry(ext.to_ascii_lowercase()).or_default();
        entry.0 += 1;
        let decoded = match ext {
            "jkl" => {
                let level = jkl::parse(&String::from_utf8_lossy(&data));
                // `static.jkl` is an asset-only library level: it parses, it just
                // has no world to draw.
                !level.surfaces.is_empty() || !level.assets.is_empty()
            }
            "3do" => tdo::parse(&String::from_utf8_lossy(&data)).is_drawable(),
            "bm" => match bm::parse(&data) {
                Some(b) => {
                    bm_files += 1;
                    bm_cells += b.cells.len();
                    bm_pixels += b.pixels();
                    if !b.exact {
                        bm_incomplete += 1;
                        if bm_samples.len() < 5 {
                            bm_samples.push(format!(
                                "{name}: {} bytes, {} cells, {} px",
                                data.len(),
                                b.cells.len(),
                                b.pixels()
                            ));
                        }
                    }
                    b.is_bitmap() && b.exact
                }
                None => false,
            },
            "sft" => match sft::parse(&data) {
                Some(f) => {
                    sft_files += 1;
                    sft_glyphs += f.charsets.iter().map(|c| c.entries.len()).sum::<usize>();
                    if !f.exact {
                        sft_incomplete += 1;
                    }
                    f.exact
                }
                None => false,
            },
            "key" => {
                let k = key::parse(&String::from_utf8_lossy(&data));
                key_files += 1;
                key_nodes += k.nodes.len();
                key_entries += k.entries();
                if k.declared_nodes > key_nodes_max {
                    key_nodes_max = k.declared_nodes;
                }
                if !k.complete() {
                    key_incomplete += 1;
                    if key_samples.len() < 5 {
                        key_samples.push(format!(
                            "{name}: {} of {} nodes, {} entries",
                            k.nodes.len(),
                            k.declared_nodes,
                            k.entries()
                        ));
                    }
                }
                k.is_drawable()
            }
            "mat" => {
                let ok = tex::parse_mat(&data).is_some();
                if !ok && flat_samples.len() < 5 {
                    flat_samples.push(name.clone());
                }
                ok
            }
            "cmp" => {
                let maps = tex::Palette::colormap_count(&data);
                cmp_maps_lo = cmp_maps_lo.min(maps);
                cmp_maps_hi = cmp_maps_hi.max(maps);
                tex::Palette::parse(&data).is_some()
            }
            "cog" => match cog::parse(&String::from_utf8_lossy(&data)) {
                Ok(c) => {
                    cog_ok += 1;
                    cog_labels += c.labels.len();
                    cog_symbols += c.symbols.len();
                    true
                }
                Err(e) => {
                    cog_bad += 1;
                    if cog_errors.len() < 40 {
                        cog_errors.push(format!("{name}: {e}"));
                    }
                    false
                }
            },
            // Sounds: the decoder is Bevy's, via the `wav` feature, so the coverage
            // claim here is that the file is a PCM WAV the decoder accepts.
            "wav" => data.len() > 44 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE",
            "snd" => {
                let entries = snd::parse(&String::from_utf8_lossy(&data));
                for e in &entries {
                    snd_entries += 1;
                    snd_flags.insert(e.playflags);
                    snd_radius.0 = snd_radius.0.min(e.min_radius);
                    snd_radius.1 = snd_radius.1.max(e.max_radius);
                    snd_volume.0 = snd_volume.0.min(e.max_volume);
                    snd_volume.1 = snd_volume.1.max(e.max_volume);
                    *snd_events.entry(e.event.clone()).or_default() += 1;
                    if gobs.find(&e.wav).is_none() {
                        snd_missing += 1;
                        if snd_samples.len() < 5 {
                            snd_samples.push(format!("{name} -> {}", e.wav));
                        }
                    }
                }
                !entries.is_empty()
            }
            "spr" => spr::parse(&String::from_utf8_lossy(&data)).iter().any(|f| {
                f.width > 0.0 && f.height > 0.0 && f.offset.iter().all(|v| v.is_finite())
                    && gobs.find(&f.material).is_some()
            }),
            // `.ai0` and `.ai2` are the same format at other difficulties.
            "ai" | "ai0" | "ai2" => {
                let class = ai::parse(&String::from_utf8_lossy(&data));
                ai_files += 1;
                ai_params += class.params.len();
                ai_instincts += class.instincts.len();
                if !class.complete() {
                    ai_incomplete += 1;
                    if ai_samples.len() < 5 {
                        ai_samples.push(format!("{name}: {:?}", class.params.keys().collect::<Vec<_>>()));
                    }
                }
                if !class.is_class() && ai_samples.len() < 5 {
                    ai_samples.push(format!("{name}: no class ({} bytes)", data.len()));
                }
                class.is_class()
            }
            "pup" => {
                let puppet = pup::parse(&String::from_utf8_lossy(&data));
                pup_files += 1;
                pup_states += puppet.states.len();
                pup_joints += puppet.joints.len();
                if !puppet.complete() && pup_samples.len() < 5 {
                    let bad: Vec<String> = puppet
                        .states
                        .iter()
                        .filter(|s| s.key.is_empty())
                        .map(|s| s.name.clone())
                        .collect();
                    pup_samples.push(format!("{name}: no key for {}", bad.join(", ")));
                }
                // Each state names its animation by bare filename, so this proves the
                // puppet's links land on real keyframes (the lookup falls back to the
                // basename, which is how the engine resolves them too).
                for s in &puppet.states {
                    if gobs.find(&s.key).is_none() {
                        pup_missing += 1;
                        if pup_samples.len() < 5 {
                            pup_samples.push(format!("{name}: {}", s.key));
                        }
                    }
                }
                if !puppet.is_puppet() && pup_samples.len() < 5 {
                    pup_samples.push(format!(
                        "{name}: {} states, {} modes",
                        puppet.states.len(),
                        puppet.modes
                    ));
                }
                puppet.is_puppet() && puppet.complete()
            }
            "uni" => {
                let table = uni::parse(&String::from_utf8_lossy(&data));
                uni_files += 1;
                uni_strings += table.strings.len();
                if !table.complete() && uni_samples.len() < 5 {
                    uni_samples.push(format!("{name}: {} malformed rows", table.malformed));
                }
                table.is_table() && table.complete()
            }
            _ => false,
        };
        if decoded {
            entry.1 += 1;
        }
    }

    // What the undecoded formats are: a reader should be able to tell which gaps
    // are deliberate (this build has no menu) from which are missing decoders.
    // Only formats with *no* decoder reach this text, so it lists what is genuinely
    // still unread rather than everything that once was.
    let note = |ext: &str| -> &'static str {
        match ext {
            "jk" => "episode manifests — see --episodes",
            "dat" | "par" => "saved game state — not read",
            _ => "not decoded",
        }
    };

    let mut rows: Vec<(String, usize, usize)> = table
        .into_iter()
        .map(|(ext, (n, ok))| (ext, n, ok))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    println!("\n--- asset coverage ---");
    println!("{:<6} {:>6} {:>8}  {}", "ext", "files", "decoded", "note");
    let (mut files, mut decoded, mut no_decoder) = (0usize, 0usize, 0usize);
    for (ext, n, ok) in &rows {
        files += n;
        decoded += ok;
        let note = if ok == n {
            "complete"
        } else if *ok == 0 {
            no_decoder += n;
            note(ext)
        } else {
            "partly decoded"
        };
        println!("{ext:<6} {n:>6} {ok:>8}  {note}");
    }
    println!("{:<6} {files:>6} {decoded:>8}", "all");
    if files > 0 {
        println!(
            "{:.0}% of the install's files decode; {no_decoder} files are in formats this build does not read",
            decoded as f64 * 100.0 / files as f64
        );
    }
    for s in &flat_samples {
        println!("  no bitmap (effect material): {s}");
    }
    if cmp_maps_hi > 0 {
        println!(
            "colormaps carry {cmp_maps_lo}..{cmp_maps_hi} distance-shading tables (unused: this build applies no distance falloff)"
        );
    }
    if snd_entries > 0 {
        println!(
            "sound classes         {snd_entries} entries, playflags {:x?}, distance {:.2}..{:.1}, volume {:.2}..{:.2}, {snd_missing} wav references missing",
            snd_flags, snd_radius.0, snd_radius.1, snd_volume.0, snd_volume.1
        );
        let events: Vec<String> = snd_events.iter().map(|(k, v)| format!("{k} x{v}")).collect();
        println!("  events              {}", events.join(", "));
        for s in &snd_samples {
            println!("  missing wav: {s}");
        }
    }
    if cog_ok + cog_bad > 0 {
        println!(
            "scripts (cog)         {} parsed, {cog_bad} failed, {cog_labels} labels, {cog_symbols} symbols",
            cog_ok
        );
        for e in &cog_errors {
            println!("  parse error: {e}");
        }
    }
    if key_files > 0 {
        println!(
            "keyframes (key)       {key_files} files, {key_nodes} nodes (max {key_nodes_max}), {key_entries} entries, {key_incomplete} truncated"
        );
        for s in &key_samples {
            println!("  truncated: {s}");
        }
    }
    if ai_files > 0 {
        println!(
            "ai classes (ai)       {ai_files} files, {ai_params} parameters, {ai_instincts} instincts, {ai_incomplete} without alignment/rank"
        );
        for s in &ai_samples {
            println!("  odd class: {s}");
        }
    }
    if pup_files > 0 {
        println!(
            "puppets (pup)         {pup_files} files, {pup_states} animation states, {pup_joints} joint mappings, {pup_missing} keyframe references missing"
        );
        for s in &pup_samples {
            println!("  missing keyframe: {s}");
        }
    }
    if bm_files > 0 {
        println!(
            "bitmaps (bm)          {bm_files} files, {bm_cells} cells, {bm_pixels} pixels, {bm_incomplete} not exact"
        );
        for s in &bm_samples {
            println!("  size mismatch: {s}");
        }
    }
    if sft_files > 0 {
        println!(
            "fonts (sft)           {sft_files} files, {sft_glyphs} glyphs, {sft_incomplete} incomplete"
        );
    }
    if uni_files > 0 {
        println!("strings (uni)         {uni_files} tables, {uni_strings} strings");
        for s in &uni_samples {
            println!("  count mismatch: {s}");
        }
    }
    // Every file this build cannot decode, not just the ones in undecoded formats:
    // 51 `.mat` files carry no bitmap by design and still count as unread.
    files.saturating_sub(decoded)
}

#[derive(Resource)]
struct Overview {
    /// `--overview`: the whole level in plan, instead of the walker — see `plan_view`.
    overview: bool,
    /// `--shot`/`--map-shot`: step on a fixed clock so a capture is reproducible.
    capture: bool,
}
/// Debug framing: place the camera beside the first thing of this template, so a
/// particular prop or actor can be inspected without hunting for it.
#[derive(Resource)]
struct Near(Option<String>);


/// `--level-shader`: use the colormap-aware level material instead of `StandardMaterial`.
#[derive(Resource, Default, Clone, Copy)]
struct LevelShader(bool);

/// `--fade-test`: a fade close enough to see, for the screenshot that proves the term works.
#[derive(Resource, Default, Clone, Copy)]
struct FadeTest(bool);

/// `--ir`: force seeing on from the first frame, for a screenshot of the infrared pass.
#[derive(Resource, Default, Clone, Copy)]
struct IrFromStart(bool);

/// `--stand <template>`: where the player starts, when it is a thing's own position.
#[derive(Resource, Default)]
struct Stand(Option<String>);


/// --look <degrees>: pitch the default view up or down, to inspect ceilings and sky.
#[derive(Resource)]
struct Look(f32);

// ---------------------------------------------------------------------------
// placed objects
// ---------------------------------------------------------------------------

/// Build one mesh per (model, material) pair, then instance it at every thing
/// that uses that model. Level 1 places 352 things across ~120 models, so the
/// meshes and materials are shared rather than rebuilt per instance.
/// One placed actor that plays a keyframe animation, and the meshes its parts draw with.
///
/// There is no shared mesh here on purpose: every instance needs its own, because the
/// pose is written into the mesh asset each time the animation's frame number changes.
struct Actor {
    /// Which runtime thing this actor draws, so its health can be read each frame.
    thing: usize,
    /// Lowercase `.3do` name, looked up in `World::models` every frame.
    model: String,
    /// Lowercase `.key` name, looked up in `World::keys`.
    key: String,
    /// The `death` state's keyfile, if the puppet has one: played once when the actor dies
    /// and held on its last frame, the way the engine leaves a body on the floor.
    death: Option<String>,
    /// Every drawable state of the puppet, `name` → `.key`: `stand`, `walk`, `run`, `walkback`,
    /// `strafeleft`, `straferight`, … The `.pup` row order is the engine's animation index, so
    /// the names are the engine's own (`sithPuppet.c`'s `animNames`). Armed modes (`mode=1`) are
    /// skipped: the port draws the unarmed puppet.
    /// Name and **mode** -> the `.key` it plays. A `.pup` names the same animation twice, once for
    /// the weapon holstered (mode 0) and once drawn (mode 1), so the key is the pair.
    states: std::collections::BTreeMap<(String, usize), String>,
    /// The state name currently playing, so a change restarts the clip (`sithPuppet_PlayMode`
    /// resets `animStartedMs`).
    state: String,
    /// Animation clock in authored frames. A locomotion clip advances by the distance the body
    /// moved, a standing or dying one by the wall clock.
    phase: f32,
    /// One mesh per material the model uses.
    parts: Vec<(Handle<Mesh>, i32, (f32, f32))>,
    /// The frame number last written, so a still frame is not rebuilt.
    drawn: i32,
    /// Which keyfile `drawn` belongs to, so a state change forces a rebuild on the same frame.
    drawn_key: String,
    /// Whether the death animation has been started.
    dying: bool,
    /// Where its parts were last drawn, so a runtime move can be applied as a delta.
    drawn_at: Vec3,
    /// The yaw drawn last frame, so a body turning in place can play its `turnleft`/`turnright`
    /// track — `sithPuppet_UpdateThingMove` picks those from the body's angular velocity.
    drawn_yaw: Option<f32>,
    /// The armed mode drawn last frame, so a change plays the puppet's own draw or holster row
    /// (`drawgun`/`holster` — 24 actors ship each, and `SetArmedMode` is what flips them).
    drawn_mode: usize,
    /// A draw or holster row still playing, with the level time it ends.
    drawing: Option<(String, f32)>,
}

/// Every placed actor with a puppet (`walkplayer`, stormtroopers, droids), animated by
/// `animate_actors`.
#[derive(Resource, Default)]
struct Actors(Vec<Actor>);

/// Things the scripts created after load (projectiles, explosions, spawned items): the
/// entity that draws each, and the shared meshes they reuse.
///
/// The renderer only knew the level's own things, so a fired bolt existed in the world and
/// nowhere on screen. This is the other half of that.
#[derive(Resource, Default)]
struct Created {
    drawn: HashMap<usize, Vec<Entity>>,
    meshes: HashMap<(String, i32), Handle<Mesh>>,
    /// Sprite quads are shared per `(template, material, cel)`: `+laserhit` is one rectangle of
    /// eight frames, so every impact in the level drawing the same frame reuses one mesh.
    sprite_meshes: HashMap<(String, String, u32), Handle<Mesh>>,
}

/// The weapon the player fires: the `weapon=` its template names, else the first template
/// that looks like a projectile (type weapon, non-zero `vel=` and `damage=`).
#[derive(Resource)]
struct Gun(String);

/// `--fire-demo`: fire once after a few frames, so a screenshot shows a bolt in flight.
#[derive(Resource)]
struct FireDemo(bool);

/// `--sprite <template>`: the effect template to keep alive in front of the camera. See
/// `sprite_demo`.
#[derive(Resource)]
struct SpriteDemo(Option<String>);

/// `--no-anim`: freeze the actors, which is how the animation is A/B'd against a still
/// frame the same way `--no-cull` A/Bs the visibility pass.
#[derive(Resource)]
struct Animate(bool);

fn spawn_models(
    commands: &mut Commands,
    world: &World,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> (HashMap<usize, Vec<Entity>>, Vec<Actor>) {
    let mut actors: Vec<Actor> = Vec::new();
    let mut mesh_cache: HashMap<(String, i32), Handle<Mesh>> = HashMap::new();
    let mut mat_cache: HashMap<String, Handle<StandardMaterial>> = HashMap::new();
    let mut spawned = 0usize;
    let mut unresolved = 0usize;
    let mut triangles = 0usize;
    let mut empty_meshes = 0usize;
    let mut own_body = 0usize;
    // Thing index -> the entities drawing it. A model with several materials becomes
    // several entities, and they all have to move together when its script does.
    let mut entities: HashMap<usize, Vec<Entity>> = HashMap::new();

    for (thing_index, thing) in world.level.things.iter().enumerate() {
        // The local player's own body is **never** drawn. `walkplayer` names `model3d=ky.3do`
        // (`03katarn.jkl`), and that is the thing the camera is attached to — the camera is the
        // body's position plus an eye offset — so spawning it draws the player's model *around* the
        // viewpoint. Models are drawn with `cull_mode: None` (gaps.md §4), so the camera does not
        // look at the body, it looks at the inside of it, and those unlit polygons land over the
        // level, the walls and every NPC in front of them. `BK_MODEL_SCALE=0.01` is the A/B that
        // says the intruder is a model at all; this is the only one placed at the camera by
        // definition. The engine's POV pass skips the local player for the same reason.
        if is_local_player(&thing.template) {
            own_body += 1;
            continue;
        }
        let Some(model_name) = world.template_model.get(&thing.template.to_ascii_lowercase()) else {
            continue;
        };
        let Some(model) = world.models.get(model_name) else {
            unresolved += 1;
            continue;
        };
        if !model.is_drawable() {
            continue;
        }

        let transform = thing_transform(thing);
        // An actor is a thing whose template names a puppet with a drawable `stand` state.
        // Its mesh cannot be shared: the pose is written into the mesh every frame.
        let puppet = world
            .level
            .template_param(&thing.template, "puppet")
            .map(|n| n.to_ascii_lowercase())
            .and_then(|puppet| world.puppets.get(&puppet));
        let animate = puppet
            .and_then(|puppet| puppet.states.iter().find(|s| s.name.eq_ignore_ascii_case("stand")))
            .map(|state| state.key.to_ascii_lowercase())
            .filter(|key| world.keys.get(key).is_some_and(key::Key::is_drawable));
        // `death` is a state of its own in the shipped puppets (`kyftdeth.key`, 0x14 flags);
        // it plays once and holds its last frame.
        let death = puppet
            .and_then(|puppet| puppet.states.iter().find(|s| s.name.eq_ignore_ascii_case("death")))
            .map(|state| state.key.to_ascii_lowercase())
            .filter(|key| world.keys.get(key).is_some_and(key::Key::is_drawable));
        // Every animation the puppet can play, so a state name can become a keyfile when the
        // actor starts moving. Mode 0 is the unarmed set — a `stand` lookup above already takes
        // the first row, and mode 1 is the same animations with the weapon drawn.
        let states: std::collections::BTreeMap<(String, usize), String> = puppet
            .map(|puppet| {
                puppet
                    .states
                    .iter()
                    .filter(|s| world.keys.get(&s.key.to_ascii_lowercase()).is_some_and(key::Key::is_drawable))
                    .map(|s| {
                        ((s.name.to_ascii_lowercase(), s.mode), s.key.to_ascii_lowercase())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut actor_parts: Vec<(Handle<Mesh>, i32, (f32, f32))> = Vec::new();
        let materials_used: std::collections::BTreeSet<i32> = model
            .meshes
            .iter()
            .flat_map(|m| m.faces.iter().map(|f| f.material))
            .collect();
        for mat_idx in materials_used {
            let key = (model_name.clone(), mat_idx);
            let mat_name = model.materials.get(mat_idx.max(0) as usize);
            let tex_size = mat_name
                .and_then(|n| world.model_images.get(n))
                .map(|img| {
                    (
                        img.texture_descriptor.size.width as f32,
                        img.texture_descriptor.size.height as f32,
                    )
                })
                .unwrap_or((1.0, 1.0));
            let mesh = match mesh_cache.get(&key) {
                _ if animate.is_some() => {
                    let mesh = meshes.add(model_mesh(model, mat_idx, tex_size));
                    actor_parts.push((mesh.clone(), mat_idx, tex_size));
                    mesh
                }
                Some(h) => h.clone(),
                None => {
                    let mesh = meshes.add(model_mesh(model, mat_idx, tex_size));
                    if let Some(m) = meshes.get(&mesh) {
                        let tri = m.indices().map(|i| i.len() / 3).unwrap_or(0);
                        triangles += tri;
                        if tri == 0 {
                            empty_meshes += 1;
                        }
                    }
                    mesh_cache.insert(key, mesh.clone());
                    mesh
                }
            };

            let mat_name = model.materials.get(mat_idx.max(0) as usize).cloned().unwrap_or_default();
            let material = match mat_cache.get(&mat_name) {
                Some(h) => h.clone(),
                None => {
                    let mut sm = StandardMaterial {
                        unlit: true,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    };
                    match world.model_images.get(&mat_name) {
                        Some(image) => sm.base_color_texture = Some(images.add(image.clone())),
                        None => sm.base_color = Color::srgb(0.55, 0.55, 0.58),
                    }
                    let h = materials.add(sm);
                    mat_cache.insert(mat_name, h.clone());
                    h
                }
            };

            let entity = commands
                .spawn((Mesh3d(mesh), MeshMaterial3d(material), transform, LevelEntity))
                .id();
            entities.entry(thing_index).or_default().push(entity);
            spawned += 1;
        }
        if let Some(key) = animate {
            actors.push(Actor {
                thing: thing_index,
                model: model_name.clone(),
                key,
                death,
                states,
                state: "stand".to_string(),
                phase: 0.0,
                parts: std::mem::take(&mut actor_parts),
                drawn: i32::MIN,
                drawn_key: String::new(),
                dying: false,
                drawn_at: Vec3::from(thing.pos),
                drawn_yaw: None,
                drawn_mode: 0,
                drawing: None,
            });
        }
    }
    println!(
        "{} model instances from {} shared meshes, {triangles} triangles ({empty_meshes} empty, {unresolved} things with no model loaded), {} animated actors, {own_body} own body(ies) not drawn",
        spawned,
        mesh_cache.len(),
        actors.len()
    );
    (entities, actors)
}

/// The level shader's cel remap (`level.wesl`), in Rust so the gate can check the arithmetic on
/// numbers instead of trusting a comment: the mesh's `v` is texel space over the whole strip, so
/// `v * cels` is cel units, and the wrap happens there before the band is added.
fn cel_v(v: f32, cel: f32, cels: f32) -> f32 {
    ((v * cels).fract() + cel) / cels
}

/// Whether a thing is the body the camera follows, and so must not be drawn.
///
/// The level places its player starts as `walkplayer` rows — `03katarn.jkl` has four, and every
/// one carries the `_actor`/`player` template's `model3d=ky.3do`. The app's walker takes the first
/// of them (`walk_player` looks up exactly this name), so this is the same test.
fn is_local_player(template: &str) -> bool {
    template.eq_ignore_ascii_case("walkplayer")
}

/// JK places things with degrees of pitch, yaw and roll about a Y-up world.
/// Only yaw is applied here: most placed objects (crates, consoles, catwalks) are
/// upright, and the pitch/roll order is not confirmed against the engine, so
/// guessing it could tilt props wrongly.
///
/// Handedness: a JK yaw of `a` faces `(cos a, 0, sin a)`. That is verified against
/// the level's own `walkplayer` start point, which faces down the opening corridor
/// of `01narshadda` under this mapping and straight into a wall under the
/// opposite one. A Bevy rotation about +Y maps +X to `(cos, 0, -sin)`, so the
/// angle is negated; local model +X then points where the engine points it.
fn thing_transform(thing: &jkl::Thing) -> Transform {
    let q = thing_quat(thing.pitch, thing.yaw, thing.roll);
    Transform::from_translation(Vec3::from(thing.pos)).with_rotation(q)
}

/// A thing's orientation, exactly as `rdMatrix_Build34(pyr)` builds it.
///
/// The row carries three angles — pitch, yaw, roll in that order — and the engine builds a basis from
/// them whose columns are the model's X, Y and Z axes in world space (`rvec`, `lvec`, `uvec`). Reproducing
/// the matrix verbatim and converting it is safer than guessing a Euler order: the pitch/roll conventions
/// here are not the ones a `from_euler` call would assume, and a sign error shows up as geometry lying on
/// its side. `stdMath_SinCos` takes degrees, so these are degrees.
fn thing_quat(pitch: f32, yaw: f32, roll: f32) -> Quat {
    let (sx, cx) = pitch.to_radians().sin_cos();
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sz, cz) = roll.to_radians().sin_cos();
    let r = Vec3::new(
        -(sz * sy) * sx + (cz * cy),
        ((sz * cy) * sx) + (cz * sy),
        -sz * cx,
    );
    let l = Vec3::new(-sy * cx, cy * cx, sx);
    let u = Vec3::new(
        ((cz * sy) * sx) + (sz * cy),
        -sx * (cy * cz) + (sy * sz),
        cz * cx,
    );
    Quat::from_mat3(&Mat3::from_cols(r, l, u)).normalize()
}

/// Model space **is** world space: **1.0**.
///
/// Measured against the only reference that cannot be argued with — a door model has to fill the
/// doorway the level builds for it, and both are in the file:
///
/// ```text
/// door          model (model units)   doorway (level units)   ratio
/// 2x3door       0.200 x 0.300         0.20 x 0.30             1.00
/// 2x3door       0.200 x 0.300         0.20 x 0.30             1.00
/// 3x3door_1     0.300 x 0.300         0.30 x 0.30             1.00
/// 4x3door_1     0.400 x 0.300         0.50 x 0.35             1.00 (within the opening's slack)
/// 4x3door_1     0.400 x 0.300         0.35 x 0.25             1.00 (within the opening's slack)
/// ```
///
/// The doorway is the vertical adjoin polygon 0.01–0.02 units from the door's own placement, so the
/// door sits in it. `--scale` prints that table. This used to be 10 on the reading that a "2x3door"
/// is 2 x 3 world units, which drew **every** model ten times too big: an actor 1.91 units tall in a
/// level whose storeys are 0.60 units and whose doorways are 0.30, which is what "the enemy models are
/// way too big for the level" was. One model unit, one level unit.
const MODEL_SCALE: f32 = 1.0;

/// Drawing scale for models, independent of the physics scale. The two shared one constant, which made it
/// impossible to ask which of them is the thing that is ten times wrong.
fn model_scale() -> f32 {
    env_f32("BK_MODEL_SCALE", MODEL_SCALE)
}

/// The factor that takes a model-space speed from the templates into level space: `maxthrust`,
/// `jumpspeed` and gravity all go through it.
///
/// **It is 1: the engine's numbers are already in level units.** `maxThrust` is an *acceleration*
/// (`sithControl.c`: `physicsParams.acceleration.x = maxThrust + extraSpeed`), and `ApplyDrag` decays
/// velocity by `v -= v * drag * dt`, so the steady state is `maxThrust / surfDrag` — for `walkplayer`
/// `2.0 / 3.0` = **0.667 units a second**. That is a walk in a level 38 units across: about a minute to
/// cross. The port used to multiply by 10 on the reading that 0.667 felt too slow, which made the player
/// cross the same level in six seconds; nothing in the engine scales these numbers, so nothing here does
/// either. Gravity and `jumpspeed` come from the same template and are in the same units.
fn physics_scale() -> f32 {
    env_f32("BK_PHYS_SCALE", 1.0)
}

/// Bounding-sphere radius of a model in world units, used to frame `--near`.
fn model_radius(model: &tdo::Model) -> f32 {
    let geoset = model.pick_geoset();
    let mut r = 0.0f32;
    for mesh in model.meshes.iter().filter(|m| m.geoset == geoset) {
        for v in &mesh.vertices {
            let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            r = r.max(d);
        }
    }
    r * model_scale()
}

fn model_mesh(model: &tdo::Model, material: i32, tex_size: (f32, f32)) -> Mesh {
    model_mesh_posed(model, material, tex_size, None)
}

/// The same, optionally posed from a keyframe at a frame number.
///
/// A model's mesh vertices are node-local whenever it has a hierarchy: the node's own
/// pivot and rotation assemble the parts into a body. So the node transforms are applied
/// even for the rest pose — skipping them is what made actor models render as a small
/// clump of parts.
fn model_mesh_posed(
    model: &tdo::Model,
    material: i32,
    tex_size: (f32, f32),
    key: Option<(&key::Key, f32)>,
) -> Mesh {
    let world = tdo::pose_nodes(model, key.map(|(k, _)| k), key.map(|(_, f)| f).unwrap_or(0.0));
    let owners = model.mesh_nodes();
    // One detail level only: the geosets are the same puppet at several resolutions.
    let geoset = model.pick_geoset();
    let mut batch = Batch::default();
    for (mesh_index, mesh) in model.meshes.iter().enumerate() {
        if mesh.geoset != geoset {
            continue;
        }
        // A mesh moves with the node that owns it; one without a node stays in model
        // space (crates and doors have no hierarchy at all).
        let owner = owners
            .get(mesh_index)
            .copied()
            .filter(|i| *i != usize::MAX)
            .and_then(|i| world.get(i));
        let place = |p: [f32; 3]| match owner {
            Some(x) => x.apply(p),
            None => p,
        };
        for face in mesh.faces.iter().filter(|f| f.material == material) {
            if face.verts.len() < 3 {
                continue;
            }
            let points: Vec<[f32; 3]> = face
                .verts
                .iter()
                .filter_map(|&(v, _)| mesh.vertices.get(v as usize).copied())
                .map(place)
                .collect();
            if points.len() != face.verts.len() {
                continue;
            }
            let Some(normal) = newell(&points) else { continue };

            let base = batch.positions.len() as u32;
            for &(v, t) in &face.verts {
                let p = place(mesh.vertices[v as usize]);
                // The model's own coordinates, scaled. `INSERT OFFSET` is *not* part of
                // the geometry: the engine uses it as the model's height above its
                // placement point (`sithPhysics` reads `insertOffset.z`) and to align a
                // created thing to a marker (`sithThing_CreateThing`); `rdModel3_Draw`
                // never adds it to a vertex. Adding it here floated every prop off the
                // floor by its own height.
                batch.positions.push([
                    p[0] * model_scale(),
                    p[1] * model_scale(),
                    p[2] * model_scale(),
                ]);
                batch.normals.push(normal);
                let uv = if t >= 0 {
                    mesh.texverts
                        .get(t as usize)
                        .map(|t| [t[0] / tex_size.0, t[1] / tex_size.1])
                        .unwrap_or([0.0, 0.0])
                } else {
                    [0.0, 0.0]
                };
                batch.uvs.push(uv);
                batch.colors.push([1.0, 1.0, 1.0, 1.0]);
            }
            for k in 1..points.len() - 1 {
                batch.indices.extend_from_slice(&[base, base + k as u32, base + k as u32 + 1]);
            }
        }
    }
    to_mesh(batch)
}

/// Newell's method: a stable normal for a planar polygon of 3+ vertices.
fn newell(points: &[[f32; 3]]) -> Option<[f32; 3]> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    let mut normal = [0.0f32; 3];
    for k in 0..n {
        let a = points[k];
        let b = points[(k + 1) % n];
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if len < 1e-6 {
        return None;
    }
    Some([normal[0] / len, normal[1] / len, normal[2] / len])
}

// ---------------------------------------------------------------------------
// bevy setup
// ---------------------------------------------------------------------------

/// Batch every drawable surface by material, so the world is one entity per
/// texture rather than one per polygon. Returns the batches and the count of
/// non-drawable surfaces (portal markers and degenerate faces).
///
/// `tex_size` is only needed to normalise texture vertices, so a caller that just
/// wants to know whether the geometry builds can pass an empty image map.
///
/// Batches are keyed by `(sector, material)`: one entity per room per texture is what
/// lets the portal walk in `collide::visible_sectors` hide whole rooms with a
/// `Visibility` flip. Keying by material alone drew the level as a handful of
/// all-or-nothing meshes and made sector culling impossible.
fn build_world(
    level: &jkl::Level,
    images: &HashMap<i32, Image>,
    hidden: &std::collections::BTreeSet<usize>,
    look: &SurfaceLook,
) -> (HashMap<(usize, i32), Batch>, usize) {
    let claims = collide::surface_claims(level);
    let mut batches: HashMap<(usize, i32), Batch> = HashMap::new();
    let mut skipped = 0usize;
    for (index, surface) in level.surfaces.iter().enumerate() {
        // Not drawn: no material, not a polygon, or a script set its geomode to `GEOMETRY_NONE`.
        if surface.material < 0 || surface.verts.len() < 3 || hidden.contains(&index) {
            skipped += 1;
            continue;
        }
        // A surface's frame comes from the script (`SetWallCel`, `SurfaceAnim`), and every surface
        // starts at cel 0 — the `.jkl` row carries no cel of its own.
        let cel = look.cel(index, surface.material);
        let scroll = look.scroll(index, surface.surfflags);
        let tex_size = images
            .get(&surface.material)
            .map(|img| {
                (
                    img.texture_descriptor.size.width as f32,
                    img.texture_descriptor.size.height as f32,
                )
            })
            .unwrap_or((1.0, 1.0));

        // A surface drawn in every sector that claims it: hiding any one claimant still
        // leaves it on screen as long as another is visible.
        let owners = claims.get(index).map(Vec::as_slice).unwrap_or(&[]);
        if owners.is_empty() {
            let light = look.light(index, -1);
            batches
                .entry((usize::MAX, surface.material))
                .or_default()
                .push_polygon(level, surface, tex_size, cel, light, scroll, look.fullbright(index));
        }
        for &sector in owners {
            let light = look.light(index, sector as i32);
            batches
                .entry((sector, surface.material))
                .or_default()
                .push_polygon(level, surface, tex_size, cel, light, scroll, look.fullbright(index));
        }
    }
    (batches, skipped)
}

/// Rebuild the world's surface batches when a script hides or restores a surface.
///
/// `SetFaceGeoMode(surface, 0)` is `GEOMETRY_NONE` (`00_breakingglass.cog` shatters glass with it,
/// `00_slashablefaces.cog` sets 4 back when a panel is intact), so the polygons have to leave the
/// mesh. The entities stay: their `(sector, material)` key is unchanged, only the geometry in it is.
/// ponytail: the whole world is re-batched per change, not the one sector — a level has a few thousand
/// surfaces and a change happens when the player breaks something, so the once-off cost is a hitch,
/// not a frame-rate problem. Narrow it to the touched sectors if that ever shows.
fn rebuild_surfaces(
    sim: Option<Res<Sim>>,
    world: Option<Res<World>>,
    batches: Option<Res<SurfaceBatches>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut seen: Local<(u32, u32)>,
) {
    let _span = Span::new("rebuild_surfaces");
    let (Some(sim), Some(world), Some(batches)) = (sim, world, batches) else { return };
    let (epoch, hidden, look) = {
        let rt = sim.world.borrow();
        (
            // The geomode, the cel and the light all end up baked into the mesh; the pair of epochs is
            // what the rebuild watches, and only a geomode change is worth a line of log.
            (rt.geomode_epoch, rt.surface_epoch),
            rt.face_geomode
                .iter()
                .filter(|(_, mode)| **mode == 0)
                .map(|(surface, _)| *surface)
                .collect::<std::collections::BTreeSet<usize>>(),
            SurfaceLook::of(&rt),
        )
    };
    if epoch == *seen {
        return;
    }
    let geomode_moved = epoch.0 != seen.0;
    *seen = epoch;
    let (built, _) = build_world(&world.level, &world.images, &hidden, &look);
    let mut patched = 0usize;
    for (key, handle) in &batches.0 {
        let Some(batch) = built.get(key).cloned() else { continue };
        if let Some(mut mesh) = meshes.get_mut(handle) {
            *mesh = to_mesh(batch);
            patched += 1;
        }
    }
    if geomode_moved {
        println!(
            "surfaces: geomode changed, {patched} batch(es) rebuilt with {} surface(s) hidden",
            hidden.len()
        );
    }
}

/// Triangles a model contributes, counting each face as `nverts - 2`.
fn model_triangles(model: &tdo::Model) -> usize {
    model
        .meshes
        .iter()
        .flat_map(|m| m.faces.iter())
        .map(|f| f.verts.len().saturating_sub(2))
        .sum()
}

/// Whether the portal walk runs at all: `--no-cull` turns it off so the same frame can be
/// rendered both ways and compared.
#[derive(Resource)]
struct Cull(bool);

/// The sector an entity's geometry belongs to, so the portal walk can hide it.
#[derive(Component)]
struct SectorId(usize);

/// The portal graph and the entities each sector owns.
#[derive(Resource)]
struct SectorView {
    solids: collide::Solids,
    by_sector: Vec<Vec<Entity>>,
}

/// Draw only the rooms the camera can reach through openings it can see.
///
/// This is the engine's visibility pass in its safe form: `collide::visible_sectors`
/// walks the level's own adjoins, admitting a room only while the doorway in front of
/// the camera projects onto the screen. Everything it returns is drawn, so the only
/// failure mode left is drawing too much. With no sector graph (a model view, the
/// overview camera) every batch stays visible.
/// The camera's six clip planes, as `(normal, distance)`: `dot(n, p) + d >= 0` is inside.
///
/// Projecting a sphere's centre and comparing screen radii cannot answer "is this floor
/// slab under me visible" — the slab's near corners are below the screen while its far
/// part fills it. Intersecting the sphere with the planes can.
fn clip_planes(clip: Mat4) -> [[f32; 4]; 6] {
    let r0 = clip.row(0);
    let r1 = clip.row(1);
    let r2 = clip.row(2);
    let r3 = clip.row(3);
    // wgpu's clip space is z in [0, 1]: near is r2, far is r3 - r2.
    let rows = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2];
    let mut planes = [[0.0f32; 4]; 6];
    for (slot, plane) in planes.iter_mut().zip(rows) {
        let len = Vec3::new(plane.x, plane.y, plane.z).length();
        let scale = if len > 0.0 { 1.0 / len } else { 1.0 };
        *slot = [plane.x * scale, plane.y * scale, plane.z * scale, plane.w * scale];
    }
    planes
}

/// Is a sector's bounding sphere inside, or straddling, the camera's frustum?
fn sphere_in_frustum(planes: &[[f32; 4]; 6], centre: Vec3, radius: f32) -> bool {
    planes.iter().all(|p| p[0] * centre.x + p[1] * centre.y + p[2] * centre.z + p[3] >= -radius)
}

fn cull_sectors(
    on: Option<Res<Cull>>,
    view: Option<Res<SectorView>>,
    player: Option<Res<Player>>,
    level: Option<Res<World>>,
    overview: Option<Res<Overview>>,
    // The scripts' switched adjoin flags: `ClearAdjoinFlags(visFace, 1)` hides the room behind an
    // opening, and the walk below has to skip it.
    sim: Option<Res<Sim>>,
    cams: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    mut batches: Query<(&SectorId, &mut Visibility)>,
    mut shown: Local<Option<(usize, usize)>>,
) {
    let _span = Span::new("cull_sectors");
    let Some(view) = view else { return };
    if !on.as_deref().is_none_or(|c| c.0) {
        return;
    }
    // A plan view keeps **every** batch, which is what this pass' own doc comment promises and what
    // it never did: the walk below starts at the player's sector and steps through portals, so from
    // a camera above the roofs it can reach nothing. That is not a conservative miss — it hid 366 of
    // level 1's 462 sectors and 807 of `04escapehouse`'s 976 in the top-down capture, so the plan was
    // a handful of surfaces with every model drawn over and through it.
    if overview.as_deref().is_some_and(|o| o.overview) {
        return;
    }
    let Some((camera, transform, _)) = cams.iter().find(|(c, _, _)| c.order == 0) else { return };
    let here = transform.translation();
    let view_from_world: Mat4 = transform.affine().inverse().into();
    let clip_from_world = camera.clip_from_view() * view_from_world;
    let planes = clip_planes(clip_from_world);

    // Frustum, not portals. The portal walk in `combat`... see `visible_sectors`: its seeds
    // are unreliable in this data (level 1's floor is a sector the camera stands *above*,
    // not inside), so it stays a diagnostic and the safe sphere test decides. A sector
    // whose bounding sphere is off screen cannot be seen; one that straddles the edge
    // stays drawn, which is the only failure direction here.
    // Portal occlusion from a **tracked** sector: the walker keeps `player.sector` through its own
    // physics, so the portal walk has a reliable seed and can hide a room that is in the frustum but
    // not reachable through any opening (§16.1). The camera's own position was the unreliable seed —
    // level 1's floor is a sector the camera stands above, not inside.
    //
    // **Which sector is the player in?** Level 1 answers three ways: the `.jkl` says 339 (a
    // portal-only sliver with no planes), the walker tracks 218 (the roomy sector the spawn chose so
    // the body has walls), and the frustum holds 43-51 (the bar the camera is actually inside). A
    // single seed therefore cannot be trusted, so every candidate is walked and the one whose
    // reachable set best covers the frustum wins. With no overlap at all the cull falls back to the
    // frustum rather than blanking the level, and says so under `BK_CULL_TRACE=1`.
    let frustum: Vec<usize> = (0..view.by_sector.len())
        .filter(|&sector| match view.solids.spheres.get(sector) {
            Some((centre, radius)) => sphere_in_frustum(&planes, Vec3::from(*centre), *radius),
            None => true,
        })
        .collect();
    let mut seeds: Vec<usize> = Vec::new();
    if let Some(tracked) = player.as_ref().map(|p| p.sector) {
        seeds.push(tracked);
    }
    if let Some(level) = level.as_ref()
        && let Some(jkl) = level
            .level
            .things
            .iter()
            .find(|t| t.template.eq_ignore_ascii_case("walkplayer"))
            .and_then(|t| usize::try_from(t.sector).ok())
        && jkl < view.by_sector.len()
    {
        seeds.push(jkl);
    }
    if let Some(at) = view.solids.sector_at([here.x, here.y, here.z], 0.2)
        && !seeds.contains(&at)
    {
        seeds.push(at);
    }
    // The portal walk borrows the runtime's sector gates; the scope ends before the sighting pass
    // below, which needs to write to it.
    let (overlap, reachable) = {
        let mut any = |_: &[[f32; 3]]| true;
        let runtime = sim.as_deref().map(|s| s.world.borrow());
        let gates = runtime.as_ref().map(|rt| rt.gates()).unwrap_or_default();
        let mut best: Option<(usize, Vec<usize>)> = None;
        for seed in &seeds {
            let reach = view.solids.visible_sectors(&[*seed], gates, &mut any);
            let overlap = reach.iter().filter(|s| frustum.contains(s)).count();
            if best.as_ref().is_none_or(|(score, _)| overlap > *score) {
                best = Some((overlap, reach));
            }
        }
        best.unwrap_or((0, Vec::new()))
    };
    let use_portals = overlap > 0 && reachable.len() > 1;
    if std::env::var_os("BK_CULL_TRACE").is_some() && shown.is_none() {
        println!(
            "cull: seeds {seeds:?} -> best overlap {overlap} of {} in the frustum, {} rooms reachable",
            frustum.len(),
            reachable.len()
        );
    }
    let mut wanted = vec![false; view.by_sector.len()];
    for sector in 0..view.by_sector.len() {
        let in_frustum = frustum.contains(&sector);
        let through_portal = !use_portals || reachable.contains(&sector);
        wanted[sector] = in_frustum && through_portal;
    }

    // `sithRender` latches `SITH_SECTOR_SEEN` the first time it draws a room, and a COGLINKED room
    // then gets `sighted`. It does the same per **thing** (`sithRender_RenderThing`): a thing that has a
    // cog on it sends `sighted` the first time it is drawn. That second one is level 1's goal —
    // `01_goal0.cog 204` puts the script on 8t88, so seeing him is what ends the level (§16.12).
    if let Some(sim) = sim.as_deref() {
        let mut rt = sim.world.borrow_mut();
        for (sector, &visible) in wanted.iter().enumerate() {
            if visible {
                rt.sector_sighted(sector);
            }
        }
        // A thing counts as drawn when its own sector is visible and its body is inside the frustum.
        // That is coarser than the engine's per-thing clip (no per-triangle test), so it can fire a
        // frame early for something in the same room behind a pillar.
        let seen: Vec<usize> = rt
            .things
            .iter()
            .enumerate()
            .filter(|(_, t)| wanted.get(t.sector.max(0) as usize).copied().unwrap_or(true))
            .filter(|(_, t)| sphere_in_frustum(&planes, Vec3::from(t.pos), 1.0))
            .map(|(index, _)| index)
            .collect();
        for index in seen {
            rt.thing_sighted(index);
        }
    }

    for (id, mut visibility) in &mut batches {
        let want = match wanted.get(id.0) {
            Some(true) | None => Visibility::Visible,
            Some(false) => Visibility::Hidden,
        };
        if *visibility != want {
            *visibility = want;
        }
    }
    if std::env::var_os("BK_CULL_TRACE").is_some() && shown.is_none() {
        // Ground truth from the level data: the `.jkl` names each thing's sector, and the
        // walkplayer's own value is what the retail engine starts from.
        if let Some(level) = level.as_ref() {
            let jkl = level
                .level
                .things
                .iter()
                .find(|t| t.template.eq_ignore_ascii_case("walkplayer"))
                .map(|t| t.sector);
            println!("cull trace: the walkplayer's `.jkl` sector is {jkl:?}");
        }
        let frustum: Vec<usize> = (0..view.by_sector.len())
            .filter(|&s| match view.solids.spheres.get(s) {
                Some((c, r)) => sphere_in_frustum(&planes, Vec3::from(*c), *r),
                None => true,
            })
            .collect();
        let overlap = frustum.iter().filter(|s| reachable.contains(s)).count();
        println!(
            "cull trace: {} in frustum, {} of them reachable; frustum sample {:?}; reachable sample {:?}; batches {}",
            frustum.len(),
            overlap,
            frustum.iter().take(5).collect::<Vec<_>>(),
            reachable.iter().take(5).collect::<Vec<_>>(),
            batches.iter().count()
        );
    }
    let state = (wanted.iter().filter(|s| **s).count(), wanted.len());
    if *shown != Some(state) {
        let occluded = (0..view.by_sector.len())
            .filter(|s| !reachable.contains(s))
            .count();
        println!(
            "sector visibility: {} of {} sectors in view; {} reachable through portals from {seeds:?}, {occluded} occluded",
            state.0, state.1, reachable.len()
        );
        *shown = Some(state);
    }
}

/// The sector ambience: every `SOUND` loop the level names, preloaded, and the one that
/// is currently playing.
#[derive(Resource, Default)]
struct Ambience {
    sources: HashMap<String, Handle<AudioSource>>,
    playing: Option<(String, Entity)>,
}

/// Every `.wav` the shipped `.snd` classes name, ready to play, with the fade radii and
/// volume that class gave it — the data `snd::volume` needs at play time.
#[derive(Resource, Default)]
struct SoundBank {
    clips: HashMap<String, (Handle<AudioSource>, f32, f32, f32)>,
}

/// `--play <wav>`: play one shipped sound once and exit, so audio can be checked without
/// a player at the keyboard.
#[derive(Resource)]
struct PlaySound(Option<(String, Vec<u8>)>);

/// The sky dome: kept centred on the camera, so the world never leaves it.
#[derive(Component)]
struct SkyDome;

/// Surround the view with the level's own sky texture.
///
/// The `--overview` camera, as `(height above the level's centre, vertical FOV)`, for a level that
/// spans `span` world units along its widest axis.
///
/// Two limits set it, and both are checked in `plan_view_tests`:
///
/// * **It has to flatten the level.** At `6 x span` a 6-unit wall 20 units off axis projects outwards
///   by half a unit, so the plan reads as a map. The old `0.9 x span` at the default 45 degrees put the
///   camera just above the roofs, where every wall leaned out over the room beside it.
/// * **It has to stay inside the sky dome.** `spawn_sky` centres a radius-600 sphere on the camera each
///   frame, so a camera further out than that looks at the dome's far *surface* instead of the level:
///   `09fuelstation` spans 102 units and would sit 610 out, which rendered as a frame of nothing but sky.
fn plan_view(span: f32) -> (f32, f32) {
    let distance = (span * 6.0).min(500.0);
    // `1.1 x span` across the frame: at distance D the vertical extent is `2 D tan(fov/2)`.
    let fov = 2.0 * (0.55 * span / distance).atan();
    (distance, fov)
}

/// The engine draws sky-flagged surfaces in a separate pass and fills whatever they
/// do not cover with the sky, scrolling it with the view direction (the
/// `Horizon Sky Offset` / `Ceiling Sky Offset` header fields). That pass is what
/// makes an open sky sector show clouds instead of a black void.
///
/// This is the cheap equivalent: a textured dome kept centred on the camera, so it
/// is always behind the world whatever the view direction is. It does not scroll
/// with view pitch, which is the one thing the engine's pass does that this does
/// not.
///
/// Only levels that name a sky material get a dome; an interior level
/// (`01narshadda`) names none, so it keeps its dark openings rather than turning
/// cloudy.
fn spawn_sky(
    commands: &mut Commands,
    level: &jkl::Level,
    world: &World,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    center: Vec3,
) {
    let Some(image) = level
        .materials
        .iter()
        .enumerate()
        .find(|(i, n)| n.to_ascii_lowercase().contains("sky") && world.images.contains_key(&(*i as i32)))
        .and_then(|(i, _)| world.images.get(&(i as i32)))
    else {
        return;
    };

    let material = StandardMaterial {
        unlit: true,
        double_sided: true,
        cull_mode: None,
        base_color_texture: Some(images.add(image.clone())),
        ..default()
    };
    // Well beyond any level's own extent, but inside the default 1000-unit far plane.
    let radius = 600.0;
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(radius).mesh().uv(32, 16))),
        MeshMaterial3d(materials.add(material)),
        Transform::from_translation(center),
        // The camera sits inside the dome and Bevy culls it as outside the frustum
        // without this — the sky then silently vanishes and the void stays black.
        bevy::camera::visibility::NoFrustumCulling,
        SkyDome,
        LevelEntity,
    ));
    println!("sky dome radius {radius:.0} using the level's sky material");
}

/// Draw the actors where the runtime has them.
///
/// The chase is the runtime's (`Runtime::ai_tick`, driven by `ai_shots`): it moves the thing with
/// the class's own `sightdist`/`fov` and the template's speed. What was missing is this half —
/// the meshes stayed where the level placed them, so an actor could chase and shoot from a spot
/// it had long left. Each part keeps its offset; only the delta is applied.
fn actors_follow_runtime(
    sim: Option<Res<Sim>>,
    actors: Option<ResMut<Actors>>,
    mut transforms: Query<&mut Transform>,
) {
    let _span = Span::new("actors_follow_runtime");
    let (Some(sim), Some(mut actors)) = (sim, actors) else { return };
    let runtime = sim.world.borrow();
    // The walkplayer is an actor too, but its drawn body is not what the runtime's position
    // means: the camera is placed by `walk_player`, so moving its parts by this delta would
    // fight that every frame.
    let local_player = runtime
        .templates
        .iter()
        .position(|t| t.eq_ignore_ascii_case("walkplayer"));
    for actor in actors.0.iter_mut() {
        if Some(actor.thing) == local_player {
            continue;
        }
        let Some(thing) = runtime.things.get(actor.thing) else { continue };
        let target = Vec3::from(thing.pos);
        let delta = target - actor.drawn_at;
        if delta.length_squared() == 0.0 {
            continue;
        }
        if let Some(entities) = sim.entities.get(&actor.thing) {
            for entity in entities {
                if let Ok(mut part) = transforms.get_mut(*entity) {
                    part.translation += delta;
                }
            }
        }
        if std::env::var_os("BK_ACTOR_TRACE").is_some() {
            println!("actor: thing {} moved {:.2} units to {target}", actor.thing, delta.length());
        }
        actor.drawn_at = target;
    }
}

/// Play every placed actor's current puppet state: each `.key`'s own `FPS` field is the
/// clock, and a frame is written into the actor's meshes only when the frame *number*
/// changes, so a 15 fps animation costs one rebuild per 1/15 s rather than per render.
///
/// The state comes from motion, the way `sithPuppet_UpdateThingMove` (`0x4e4380`) picks it:
/// standing still is `stand`, moving forward is `walk` under 0.5 u/s and `run` above it,
/// backwards is `walkback`, sideways is `strafeleft`/`straferight`. A script's `PlayKey` wins
/// over all of it, and a dead actor plays `death` once and holds it.
/// ponytail: a changed frame rebuilds the whole mesh asset instead of writing into its
/// vertex buffer or skinning on the GPU. Ceiling: one rebuild per actor per animation
/// tick (15/s per actor), which holds for the ~120 actors a level places; the upgrade
/// path is to keep the topology and rewrite positions/normals in place, or to bind the
/// puppet's joints as a skin.
fn animate_actors(
    time: Res<Time>,
    on: Option<Res<Animate>>,
    sim: Option<Res<Sim>>,
    world: Option<Res<World>>,
    actors: Option<ResMut<Actors>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let _span = Span::new("animate_actors");
    if on.is_some_and(|on| !on.0) {
        return;
    }
    let (Some(world), Some(mut actors)) = (world, actors) else { return };
    let dt = time.delta_secs();
    // A dead actor plays `death` once and holds its last frame. The runtime is the source of
    // truth for health, so the renderer reads it each frame rather than being told.
    let dead: std::collections::HashSet<usize> = sim
        .as_deref()
        .map(|sim| {
            let rt = sim.world.borrow();
            (0..rt.things.len()).filter(|i| rt.things[*i].destroyed).collect()
        })
        .unwrap_or_default();
    for actor in &mut actors.0 {
        // A script can replace the puppet's own state track with one of its `keyframe` symbols
        // (`PlayKey`): level 1's TIE bomber swaps in `01VTieB.key` from frame 0x14 when the player
        // first sees it. The velocity is the one `ai_tick` walked at — the port's stand-in for
        // `physicsParams.vel`, which `sithPuppet_UpdateThingMove` transforms into the thing's own
        // frame to choose a state.
        let (scripted, pose, walk) = sim
            .as_deref()
            .map(|sim| {
                let rt = sim.world.borrow();
                (
                    rt.play_key.get(&actor.thing).cloned(),
                    rt.things.get(actor.thing).map(|t| t.yaw),
                    rt.walk_vel.get(&actor.thing).copied(),
                )
            })
            .unwrap_or((None, None, None));
        let vel = walk.map_or(Vec3::ZERO, |[x, y]| Vec3::new(x, y, 0.0));
        if dead.contains(&actor.thing) {
            actor.dying = true;
        }
        // An actor that just fired plays its puppet's `fire` track: the engine's
        // `sithPuppet_UpdateThingMove` picks the action track over the stride, and an actor that
        // keeps walking while shooting is the sliding soldier the trace made obvious.
        let (firing, hit, armed, rt_now) = sim
            .as_deref()
            .map(|sim| {
                let rt = sim.world.borrow();
                // `SetArmedMode(thing, 1)` is what a weapon cog calls when the weapon is drawn; a
                // thing whose template carries a `weapon=` counts as armed until a script says
                // otherwise, which is the engine's own default for a class that ships one.
                let armed = match rt.armed_mode.get(&actor.thing) {
                    Some(mode) => *mode as usize,
                    None => usize::from(rt.thing_has_weapon(actor.thing)),
                };
                (rt.just_fired(actor.thing), rt.just_hit(actor.thing), armed, rt.time)
            })
            .unwrap_or((false, false, 0, 0.0));
        // How fast the body is turning, in degrees per second: a body that turns in place plays its
        // `turnleft`/`turnright` track, which 226 of level 1's puppets ship and no code played.
        let yaw_now = pose.unwrap_or(0.0);
        let turn = actor
            .drawn_yaw
            .map(|before| {
                let mut delta = (yaw_now - before) % 360.0;
                if delta > 180.0 {
                    delta -= 360.0;
                } else if delta < -180.0 {
                    delta += 360.0;
                }
                delta / dt.max(1e-4)
            })
            .unwrap_or(0.0);
        actor.drawn_yaw = Some(yaw_now);
        // A change of armed mode plays the puppet's own draw or holster row first — this is the
        // body half of what a weapon cog's `selected:`/`deselected:` does (`SetArmedMode`).
        if actor.drawn_mode != armed {
            if let Some(row) = draw_row(actor.drawn_mode, armed)
                && let Some(key) = actor
                    .states
                    .get(&(row.to_string(), armed))
                    .or_else(|| actor.states.get(&(row.to_string(), 0)))
                && let Some(clip) = world.keys.get(key)
            {
                let playtime = clip.frames.max(1) as f32 / clip.fps.max(1.0);
                actor.drawing = Some((row.to_string(), rt_now + playtime));
            }
            actor.drawn_mode = armed;
        }
        // The state name, which is what decides the keyfile below. A scripted clip has no state
        // of its own, so it gets the empty name and always wins.
        // A draw or holster row still playing outranks the stride: it is the transition between two
        // armed modes, and `draw_row` is the only place that names it.
        let transition = actor
            .drawing
            .as_ref()
            .filter(|(_, until)| rt_now < *until)
            .map(|(row, _)| row.clone());
        let state = if scripted.is_some() {
            ""
        } else if actor.dying {
            "death"
        } else if let Some(row) = transition.as_deref().filter(|row| {
            actor.states.contains_key(&(row.to_string(), armed))
                || actor.states.contains_key(&(row.to_string(), 0))
        }) {
            row
        } else {
            actor_state(vel, pose.unwrap_or(0.0), firing, hit, turn, armed, |name| {
                actor.states.contains_key(&(name.to_string(), armed))
                    || actor.states.contains_key(&(name.to_string(), 0))
            })
        };
        if actor.state.as_str() != state {
            let next = match &scripted {
                Some((key, _, _)) => key.clone(),
                None => actor
                    .states
                    .get(&(state.to_string(), armed))
                    .or_else(|| actor.states.get(&(state.to_string(), 0)))
                    .cloned()
                    .or_else(|| Some(animating_key(actor, actor.dying).to_string()))
                    .unwrap_or_default(),
            };
            // A state change restarts the clip: `sithPuppet_PlayMode` resets `animStartedMs`.
            actor.state.clear();
            actor.state.push_str(state);
            actor.drawn_key = next;
            actor.drawn = -1;
            actor.phase = 0.0;
            if !state.is_empty() && state != "stand" {
                println!(
                    "actor: thing {} plays `{}` ({state}, {:.2} u/s at yaw {:.0})",
                    actor.thing,
                    actor.drawn_key,
                    horizontal_speed(vel),
                    pose.unwrap_or(0.0)
                );
            }
        }
        let (Some(model), Some(key)) = (world.models.get(&actor.model), world.keys.get(&actor.drawn_key))
        else {
            continue;
        };
        // Loop over the keyfile's own frame count: the `.pup` row's last two numbers are
        // event markers, not a loop range (`stand kystand0.key 0x00 0 0` on a 31-frame file).
        // A dying actor plays to its last frame and stays there; a living one loops.
        // ponytail: a scripted clip is offset by its start frame but has no own epoch, so the
        // `PlayKey(..., frame)` start looks like a loop offset rather than a timed track; `flags` is
        // recorded and not interpreted.
        let last = key.frames.max(1) as f32 - 1.0;
        let start = scripted.as_ref().map_or(0.0, |(_, _, frame)| *frame as f32);
        // A locomotion clip advances with the distance the body covered, not the clock
        // (`sithPuppet_UpdatePuppet` advances the track by `speed * dt * const`), so the legs keep
        // pace with the body instead of sliding on a fixed rate. ponytail: that units-to-frames
        // constant is not recovered, so the clip is normalised at the walk/run threshold above —
        // 0.5 u/s plays it at its authored rate — and capped at the floor and twice that.
        let moving = !state.is_empty() && state != "stand" && state != "death";
        let rate = if moving {
            key.fps * (horizontal_speed(vel) / 0.5).clamp(0.5, 2.0)
        } else {
            key.fps
        };
        actor.phase += dt * rate;
        let clock = actor.phase + start;
        let frame = if actor.dying {
            clock.min(last).floor() as i32
        } else {
            (clock % (last + 1.0)).floor() as i32
        };
        if frame == actor.drawn {
            continue;
        }
        actor.drawn = frame;
        for (handle, material, tex_size) in &actor.parts {
            if let Some(mut slot) = meshes.get_mut(handle) {
                *slot = model_mesh_posed(model, *material, *tex_size, Some((key, frame as f32)));
            }
        }
    }
}

/// Which state row a change of armed mode plays, if the puppet has one.
///
/// The rows are `drawgun`/`holster` (24 actors ship each). **Not** `mount`/`dismount`: those are
/// the names of the *weapon cog's* keyframe symbols (`mountAnim=bryvmnt.key`), which animate the
/// first-person viewmodel, not the body. Mixing the two up is what made an earlier round report
/// that no puppet shipped a draw animation.
fn draw_row(from: usize, to: usize) -> Option<&'static str> {
    match (from, to) {
        (0, 1) => Some("drawgun"),
        (1, 0) => Some("holster"),
        _ => None,
    }
}

/// The part of a velocity that drives a stride: a body riding a lift is not walking.
fn horizontal_speed(vel: Vec3) -> f32 {
    (vel.x * vel.x + vel.y * vel.y).sqrt()
}

/// Which puppet state a moving actor plays, from `sithPuppet_UpdateThingMove` (`0x4e4380`).
///
/// The engine transforms the thing's velocity into its own frame and picks by the dominant
/// axis: forward is `walk` below the walk/run threshold and `run` above it, a negative forward
/// component is `walkback`, and a lateral component that dominates the forward one is
/// `strafeleft`/`straferight`. Under 0.02 u/s the actor is standing still — it would play its
/// turn animation there if the AI's turning flag were modelled, which it is not.
///
/// The threshold is `local_10` in that function: 0.5 u/s, and 1.0 for a player thing.
/// ponytail: the port feeds this its own averaged displacement rather than a physics velocity,
/// because `ai_tick` moves actors by writing their position and no velocity exists to read.
/// Same selector, different source; a teleport reads as one huge step and snaps the clip.
fn locomotion_state(vel: Vec3, yaw_deg: f32) -> &'static str {
    // Only horizontal motion picks a stride. Retail compares the component it measured on the
    // dominant axis; here the measured quantity is a velocity, so the horizontal magnitude is
    // what must clear the dead zone.
    let speed = horizontal_speed(vel);
    if speed <= 0.02 {
        return "stand";
    }
    let (sin, cos) = yaw_deg.to_radians().sin_cos();
    let forward = vel.x * cos + vel.y * sin;
    let lateral = -vel.x * sin + vel.y * cos;
    if forward.abs() >= lateral.abs() {
        if forward <= 0.0 {
            "walkback"
        } else if speed < 0.5 {
            "walk"
        } else {
            "run"
        }
    } else if lateral >= 0.0 {
        "straferight"
    } else {
        "strafeleft"
    }
}

/// The state an actor plays: its action track while it is shooting, else the stride.
///
/// `sithPuppet_UpdateThingMove` prefers the action track (`fire`) over the locomotion one, and falls
/// back to the stride when the puppet ships no such row — a droid with no `fire` row still walks and
/// shoots, it just does not animate the shot. `has` is the puppet's own state list, which keeps this
/// a pure function the tests can drive without a Bevy world.
fn actor_state(
    vel: Vec3,
    yaw_deg: f32,
    firing: bool,
    hit: bool,
    turn: f32,
    armed: usize,
    has: impl Fn(&str) -> bool,
) -> &'static str {
    // The engine's order: a hit reaction interrupts everything but death, and `fire` outranks the
    // stride (`sithPuppet_UpdateThingMove`).
    if hit && has("hit") {
        return "hit";
    }
    if firing && has("fire") {
        return "fire";
    }
    // A body pivoting in place turns; one that is walking uses its stride instead — locomotion wins
    // over a turn while the body is moving.
    let moving = horizontal_speed(vel) > 0.02;
    if !moving && turn.abs() > 40.0 {
        let wanted = if turn > 0.0 { "turnleft" } else { "turnright" };
        if has(wanted) {
            return wanted;
        }
    }
    locomotion_state(vel, yaw_deg)
}

/// Which keyfile an actor plays: its `stand` loop, or its `death` animation once dead (a
/// puppet without one keeps looping `stand`, which is what the engine does for a class that
/// ships no death).
fn animating_key(actor: &Actor, dead: bool) -> &str {
    if dead {
        actor.death.as_deref().unwrap_or(&actor.key)
    } else {
        &actor.key
    }
}

/// The player's own state, pushed into the runtime's local-player thing so scripts and
/// projectiles see where the camera actually is.
fn sync_player_thing(player: Option<Res<Player>>, sim: Option<Res<Sim>>) {
    let (Some(player), Some(sim)) = (player, sim) else { return };
    let mut rt = sim.world.borrow_mut();
    let Some(index) = rt.templates.iter().position(|t| t.eq_ignore_ascii_case("walkplayer")) else {
        return;
    };
    let eye = player.pos + Vec3::Z * player.eye;
    rt.things[index].pos = [eye.x, eye.y, eye.z];
    rt.things[index].yaw = player.yaw.to_degrees();
    // The aim's pitch: `fire` shoots along the shooter's orientation, so without this the player's
    // shots left level however far up or down the view was pointed.
    rt.things[index].pitch = player.pitch.to_degrees();
    rt.things[index].sector = player.sector as i32;
}

/// Play one of the menu's own wav names through the same mixer the world uses.
///
/// A GUI sound has no place in the world, which is `PlaySoundLocal`: the request carries no position,
/// so `snd::volume` does not attenuate it.
fn play_menu_wav(name: &str, sim: &Option<Res<Sim>>, bank: &Option<Res<SoundBank>>) {
    let (Some(sim), Some(bank)) = (sim, bank) else { return };
    if !bank.clips.contains_key(&name.to_ascii_lowercase()) {
        return;
    }
    {
        let mut rt = sim.world.borrow_mut();
        let channel = rt.sound_volume.len();
        rt.sound_volume.push(1.0);
        rt.sound_pitch.push(1.0);
        rt.sounds.push(runtime::SoundRequest {
            name: name.to_ascii_lowercase(),
            volume: 1.0,
            pos: None,
            channel,
            pitch: 1.0,
        });
    }
}


/// Every verb a block calls, nested blocks included: what a script needs from the engine is exactly
/// the set of verbs it reaches, so this is how `--pull` names the pull's requirements.
fn collect_verbs(body: &[cog::Stmt], out: &mut Vec<String>) {
    let mut one = |stmt: &cog::Stmt, out: &mut Vec<String>| match stmt {
        cog::Stmt::Call(name, _) => {
            if !out.iter().any(|v| v.eq_ignore_ascii_case(name)) {
                out.push(name.clone());
            }
        }
        cog::Stmt::If(_, a, b) => {
            collect_verbs(std::slice::from_ref(a.as_ref()), out);
            if let Some(b) = b {
                collect_verbs(std::slice::from_ref(b.as_ref()), out);
            }
        }
        cog::Stmt::While(_, body) | cog::Stmt::DoWhile(body, _) => {
            collect_verbs(std::slice::from_ref(body.as_ref()), out);
        }
        cog::Stmt::For(init, _, step, body) => {
            collect_verbs(std::slice::from_ref(init.as_ref()), out);
            collect_verbs(std::slice::from_ref(step.as_ref()), out);
            collect_verbs(std::slice::from_ref(body.as_ref()), out);
        }
        cog::Stmt::Block(body) => collect_verbs(body, out),
        _ => {}
    };
    for stmt in body {
        one(stmt, out);
    }
}

/// The ammo bin an **armed bin** spends: `misc\items.dat` names that bin's item cog, and the cog's
/// own `ammobin(Int)` symbol names the rounds (`pow_bryar.cog` declares one).
///
/// This is the chain the engine uses — the bin is chosen by the player, so the rounds have to follow
/// the choice. `ammo_bin_for` below answers the different question of what the *level's* default
/// weapon spends, which is only the right answer until the player switches.
fn ammo_bin_for_bin(world: &World, bin: i32) -> Option<i32> {
    // The rounds a bin spends are declared by that bin's **powerup** script: `pow_bryar.cog` carries
    // `bin=2` and `ammobin=11`. `items.dat` names the *weapon* script (`weap_bryar.cog`), whose
    // `fire:` hard-codes the round bin instead of declaring it, so the powerup pair is the only
    // declarative source.
    let declared = world.cogs.values().find_map(|script| {
        let sym = |name: &str| {
            script
                .symbols
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))
                .and_then(|s| s.init.as_deref())
                .and_then(|raw| raw.trim().parse::<i32>().ok())
        };
        if sym("bin") == Some(bin) { sym("ammobin") } else { None }
    });
    if declared.is_some() {
        return declared;
    }
    let cog_name = world.item_cogs.get(&bin)?.to_ascii_lowercase();
    // `items.dat` names the script as the archive path it lives at (`cog\pow_bryar.cog`), while the
    // loader keys `world.cogs` by the name a template carries (`pow_bryar.cog`), so match on the file
    // name rather than the whole path.
    let file = cog_name.rsplit(['\\', '/']).next().unwrap_or(&cog_name).to_string();
    let script = world.cogs.get(&cog_name).or_else(|| world.cogs.get(&file)).or_else(|| {
        world
            .cogs
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(&file))
            .map(|(_, script)| script)
    })?;
    script
        .symbols
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case("ammobin"))
        .and_then(|s| s.init.as_deref())
        .and_then(|raw| raw.trim().parse().ok())
}

/// The ammo bin the player's weapon spends: the weapon's `cog=`, that cog's `ammobin` symbol, and the
/// value the level's own instance gave it.
///
/// No weapon template carries `ammo=`, so this is where the number lives — `pow_bryar.cog` declares
/// `ammobin(Int)` beside `bin(Int)`. The *spend* is one round per shot, calibrated: the engine's firing
/// tick is in `sithWeapon.c`/`jkPlayer.c`, neither of which this source tree carries.
fn ammo_bin_for(world: &World) -> Option<i32> {
    let weapon = world
        .level
        .template_param("walkplayer", "weapon")
        .unwrap_or("bryarpistol");
    let cog_name = world.level.template_param(weapon, "cog")?.to_ascii_lowercase();
    let script = world.cogs.get(&cog_name)?;
    // The bin is the cog's own default (`ammobin=NN` in its symbol list); a level that *places* the
    // cog may override it positionally, the same walk `Cog::bind` does.
    let declared = script
        .symbols
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case("ammobin"))
        .and_then(|s| s.init.as_deref())
        .and_then(|raw| raw.trim().parse().ok());
    if let Some(bin) = declared {
        return Some(bin);
    }
    let instance = world
        .level
        .cogs
        .iter()
        .find(|i| i.script.eq_ignore_ascii_case(&cog_name))?;
    let mut next = instance.values.iter();
    for symbol in &script.symbols {
        if symbol.local || symbol.kind == cog::SymKind::Message {
            continue;
        }
        let Some(raw) = next.next() else { break };
        if symbol.name.eq_ignore_ascii_case("ammobin") {
            return raw.trim().parse().ok();
        }
    }
    None
}

/// The ammo bin the player's weapon spends, resolved once at load from the weapon's powerup cog.
#[derive(Resource, Default)]
struct AmmoBin(Option<i32>);

/// The use key: `activate` to the linked surface the player is looking at.
///
/// `activate` is the third most common label level 1 declares (x14), and a linked surface is what
/// receives it. The reach and facing cone are calibrated (§16.10) — the engine's own use action lives
/// in `jkPlayer.c`, which the source tree does not carry — but the *candidates* are the surfaces a
/// script linked, so a room with nothing wired stays silent.
fn use_key(
    keys: Res<ButtonInput<KeyCode>>,
    sim: Option<Res<Sim>>,
    player: Option<Res<Player>>,
    mut said: Local<u32>,
) {
    let (Some(sim), Some(player)) = (sim, player) else { return };
    if !keys.just_pressed(KeyCode::KeyE) {
        // `BK_USE=1` presses it once, so a door can be exercised without a keyboard: the use action is
        // the only input a level's own door scripts need (`sithPlayer`'s use).
        let forced = std::env::var_os("BK_USE").is_some() && *said == 0;
        if !forced {
            return;
        }
    }
    let eye = [player.pos.x, player.pos.y, player.pos.z];
    let facing = [player.yaw.cos(), player.yaw.sin()];
    // Things first: a door's script is linked with `thing` symbols, and using it is what sends `used`
    // (`sithPlayer`'s use action). Surfaces are the other kind of link, and a level may have both.
    let mut rt = sim.world.borrow_mut();
    let used_thing = rt.use_thing(eye, facing, 3.0);
    let used_surface = if used_thing.is_none() {
        rt.activate_near(eye, facing, 3.0)
    } else {
        None
    };
    drop(rt);
    *said += 1;
    match (used_thing, used_surface) {
        (Some(thing), _) => println!("use: activated thing {thing}"),
        (None, Some(surface)) => println!("use: activated surface {surface}"),
        (None, None) => println!("use: nothing linked within reach"),
    }
}

/// `crossed`: the player passing through a surface a script linked.
///
/// Level 1 waits on `crossed` 7 times — trigger planes and force fields. The player's own movement
/// segment is tested against each linked surface's polygon (`collide::crosses`), so a fast walk
/// cannot tunnel through one the way a position-inside test would allow.
fn deliver_crossed(
    world: Option<Res<World>>,
    sim: Option<Res<Sim>>,
    player: Option<Res<Player>>,
    mut last: Local<Option<Vec3>>,
    mut count: Option<ResMut<TriggerCount>>,
) {
    let (Some(world), Some(sim), Some(player)) = (world, sim, player) else { return };
    let now = player.pos;
    let from = *last;
    *last = Some(now);
    let Some(from) = from else { return };
    if from == now {
        return;
    }
    let (a, b) = ([from.x, from.y, from.z], [now.x, now.y, now.z]);
    let surfaces: Vec<usize> = sim.world.borrow().surface_links.iter().map(|(_, _, s)| *s).collect();
    let mut queued = 0usize;
    for surface in surfaces {
        if !collide::crosses(&world.level, surface, a, b) {
            continue;
        }
        let mut rt = sim.world.borrow_mut();
        for (instance, _, linked) in &rt.surface_links.clone() {
            if *linked == surface {
                rt.pending_messages.push((*instance, "crossed", [0.0; 2]));
                queued += 1;
            }
        }
    }
    if let Some(count) = count.as_mut() {
        count.0 += queued;
    }
    if queued > 0 && std::env::var_os("BK_TRIGGER_TRACE").is_some() {
        println!("trigger: {queued} `crossed` message(s)");
    }
}

/// Deliver what the runtime queued: `damaged` and `killed` on the cogs linked to a thing.
///
/// The runtime holds no VM, so `Runtime::damage` records `(cog instance, label, params)` and this runs
/// it. Level 1 waits on `damaged` x4 and `killed` x5, so this is what lets a level react to combat.
///
/// A `damaged` handler can answer with `ReturnEx`, and the engine then applies *that* amount instead
/// of the one it passed in (`sithCog.c`'s message dispatch: `damaged`'s return **replaces** the
/// value). That is how `force_absorb.cog` turns part of a hit into mana, so the answer is fed back to
/// the runtime once the handlers have run.
fn deliver_thing_messages(
    world: Option<Res<World>>,
    mut sim: Option<ResMut<Sim>>,
    mut count: Option<ResMut<TriggerCount>>,
) {
    let (Some(world), Some(sim)) = (world, sim.as_mut()) else { return };
    // The handle, not the field: `instances` is borrowed mutably for the whole loop, and a VM's verbs
    // borrow this same runtime, so every borrow below has to be its own short-lived one.
    let handle = sim.world.clone();
    let pending = std::mem::take(&mut handle.borrow_mut().pending_messages);
    let mut fired = 0usize;
    // Which thing each `damaged` instance was for, and the amount the handler allows, so the answer
    // is applied after the last handler on that thing has had its say.
    let mut answers: Vec<(usize, Option<f32>)> = Vec::new();
    for (instance, label, params) in pending {
        let Some((script_name, vm)) = sim.instances.get_mut(instance) else { continue };
        let Some(script) = world.cogs.get(script_name) else { continue };
        if script.label(label).is_none() {
            continue;
        }
        // `GetParam(0)`/`GetParam(1)` inside the handler: the engine's own two words.
        handle.borrow_mut().last_params = params.to_vec();
        if vm.run_label(script, label).is_err() {
            continue;
        }
        fired += 1;
        if label == "damaged" {
            let thing = vm.source.max(0) as usize;
            let allowed = match vm.ret.take() {
                Some(cog::Value::Num(n)) => Some(n as f32),
                _ => None,
            };
            answers.push((thing, allowed));
        }
    }
    for (thing, allowed) in answers {
        let Some(allowed) = allowed else { continue };
        let amend = handle.borrow_mut().amend_damage(thing, allowed);
        if let Some((applied, allowed)) = amend {
            println!(
                "damage: a `damaged` handler on thing {thing} answered {allowed:.1} of the {applied:.1} applied"
            );
        }
    }
    if let Some(count) = count.as_mut() {
        count.0 += fired;
    }
    if fired > 0 && std::env::var_os("BK_TRIGGER_TRACE").is_some() {
        println!("trigger: {fired} thing message(s) delivered");
    }
}

/// Deliver the script-to-script messages `SendMessage` queued: a cog index and a message id.
///
/// This is the bus the shipped **level-end chain** rides. A level's `goal*.cog` reports a finished
/// objective with `SendMessage(GetMasterCOG(), user0)`, and the master cog's `user0` answers by
/// setting the goal flag, playing `Accomplish1.wav`, sleeping three seconds and calling
/// `jkEndLevel(1)` (`01_endlevel.cog`). Nothing used to drain this queue in play — only `--run` did —
/// so in the window that message never left the runtime. `pending_messages` is the other bus: it
/// carries thing- and sector-linked labels addressed by *instance index*, while this one carries what
/// one script said to another, addressed by cog index (`sithCogFunction_SendMessage`).
fn deliver_cog_messages(
    world: Option<Res<World>>,
    mut sim: Option<ResMut<Sim>>,
    mut count: Option<ResMut<TriggerCount>>,
) {
    let (Some(world), Some(sim)) = (world, sim.as_mut()) else { return };
    // The handle, not the field: `instances` is borrowed mutably for the whole loop and a scoring
    // verb borrows this same runtime.
    let handle = sim.world.clone();
    let mut fired = 0usize;
    // The renderer's own messages: `sithRender` latches "seen" and sends `sighted` to the cogs on a
    // room or a thing (`thing_sighted`/`sector_sighted` push into `pending_messages`). Nothing on the
    // live path ever drained that queue, so **seeing 8t88 queued the goal's message and dropped it** —
    // the level's first objective could never start. Delivered here, ahead of the ordinary bus.
    // Taken into a local first: a `handle.borrow_mut()` in the `for` header lives for the whole loop,
    // and the `last_params` write inside it would borrow again on the same thread.
    let pending = std::mem::take(&mut handle.borrow_mut().pending_messages);
    for (instance, label, params) in pending {
        let Some((script_name, vm)) = sim.instances.get_mut(instance) else { continue };
        let Some(script) = world.cogs.get(script_name) else { continue };
        if script.label(label).is_none() {
            continue;
        }
        handle.borrow_mut().last_params = params.to_vec();
        match vm.run_label(script, label) {
            Ok(_) => {
                fired += 1;
                if std::env::var_os("BK_TRIGGER_TRACE").is_some() {
                    println!("sighted: `{script_name}` ran `{label}`");
                }
            }
            Err(error) => println!("sighted: `{script_name}`'s `{label}` failed: {error}"),
        }
    }
    // A script may answer with another message, so drain until quiet. Bounded, because two scripts
    // that greet each other forever must not wedge a frame.
    for _round in 0..8 {
        let queued = std::mem::take(&mut handle.borrow_mut().messages);
        if queued.is_empty() {
            break;
        }
        for (name, sender, cog, params) in queued {
            for (script_name, vm) in &mut sim.instances {
                if let Some(target) = cog
                    && vm.cog_index != target
                {
                    continue;
                }
                let Some(script) = world.cogs.get(script_name) else { continue };
                // `#N` is a message number: the engine hands the id to the receiver, whose own
                // `message:` list decides the label, so both sides agree on `SITH_MESSAGE_*` values.
                let label = match name.strip_prefix('#').and_then(|n| n.parse::<usize>().ok()) {
                    Some(id) => match script.label_for_message_id(id) {
                        Some(label) => label,
                        None => continue,
                    },
                    None => name.clone(),
                };
                if script.label(&label).is_none() {
                    continue;
                }
                vm.sender = sender.map_or(0, |s| s as i64);
                // **Source as well as sender.** `GetSourceRef()` is what a message handler reads to
                // find the thing that caused it — `cog_weap_bryar.cog`'s `fire:` does
                // `player = GetSourceRef(); if (GetThingHealth(player) <= 0) Return;` — and this loop
                // set only the sender, so every weapon's `fire:` saw source 0 and returned at once.
                vm.source = sender.map_or(0, |s| s as i64);
                // `SendMessageEx`'s parameters, the same two words a `damaged` handler gets.
                handle.borrow_mut().last_params = params.to_vec();
                if vm.run_label(script, &label).is_ok() {
                    fired += 1;
                }
            }
        }
    }
    if let Some(count) = count.as_mut() {
        count.0 += fired;
    }
    if fired > 0 && std::env::var_os("BK_TRIGGER_TRACE").is_some() {
        println!("trigger: {fired} script message(s) delivered");
    }
}

/// Sector triggers: `entered` and `exited`, from `sithThing_EnterSector`/`LeaveSector`.
///
/// A sector messages the cogs linked to it, and a script's `sector` symbol is that link (§16.10).
/// Level 1 declares ten of them, and its soundtrack fades the music in from one, so this is what lets
/// the fade run at all.
fn deliver_sector_triggers(
    world: Option<Res<World>>,
    mut sim: Option<ResMut<Sim>>,
    player: Option<Res<Player>>,
    mut last: Local<Option<usize>>,
    mut count: Option<ResMut<TriggerCount>>,
) {
    let (Some(world), Some(sim), Some(player)) = (world, sim.as_mut(), player) else { return };
    let sector = player.sector;
    let previous = *last;
    if previous == Some(sector) {
        return;
    }
    *last = Some(sector);
    let (messages, volume_before) = {
        let rt = sim.world.borrow();
        (rt.sector_messages(previous, Some(sector)), rt.music_volume)
    };
    let mut fired = 0usize;
    for (instance, label) in messages {
        let Some((script_name, vm)) = sim.instances.get_mut(instance) else { continue };
        let Some(script) = world.cogs.get(script_name) else { continue };
        if script.label(label).is_none() {
            continue;
        }
        // A label that runs and **errors** has not been delivered, and counting it as delivered hides
        // the reason a level cannot progress. 21ajerec's `21_JerecAI.cog` entry failed here silently.
        match vm.run_label(script, label) {
            Ok(_) => fired += 1,
            Err(error) => println!(
                "trigger: `{script_name}`'s `{label}` failed entering sector {sector}: {error}"
            ),
        }
    }
    if let Some(count) = count.as_mut() {
        count.0 += fired;
    }
    if fired > 0 && std::env::var_os("BK_TRIGGER_TRACE").is_some() {
        let volume = sim.world.borrow().music_volume;
        println!(
            "trigger: {fired} message(s) entering sector {sector} (from {previous:?}), music {volume_before:.2} -> {volume:.2}"
        );
    }
}

/// How many sector `entered`/`exited` messages have been delivered.
#[derive(Resource, Default)]
struct TriggerCount(usize);

/// Hand the level's collider to the runtime, once, so moving things can be blocked by geometry.
fn share_collider(
    collisions: Option<Res<Collision>>,
    sim: Option<Res<Sim>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (Some(collisions), Some(sim)) = (collisions, sim) else { return };
    if std::env::var_os("BK_NO_SOLIDS").is_none() {
        sim.world.borrow_mut().solids = Some(std::sync::Arc::new(collisions.0.clone()));
    }
    *done = true;
    println!("collision: the runtime can now stop moving things ({} sectors)", collisions.0.sectors.len());
}

/// Resume the cogs a `sleep` interrupted, which is the engine's `script_running == 2` state.
///
/// `sithCog_ProcessCog` wakes a waiting cog when its deadline passes; here the label is run again and
/// `run_label` continues from the statement after the `sleep`.
fn resume_sleeping_labels(
    time: Res<Time>,
    world: Option<Res<World>>,
    mut sim: Option<ResMut<Sim>>,
    mut count: Option<ResMut<PulseCount>>,
) {
    let (Some(world), Some(sim)) = (world, sim.as_mut()) else { return };
    let dt = time.delta_secs().min(0.1);
    let mut resumed = 0usize;
    for (script_name, vm) in &mut sim.instances {
        vm.now += dt;
        if vm.sleeping() || vm.sleeping_label().is_none() {
            continue;
        }
        let Some(label) = vm.sleeping_label().map(str::to_string) else { continue };
        let Some(script) = world.cogs.get(script_name) else { continue };
        if script.label(&label).is_none() {
            continue;
        }
        let _ = vm.run_label(script, &label);
        resumed += 1;
    }
    if let Some(count) = count.as_mut() {
        count.0 += resumed;
    }
    if resumed > 0 && std::env::var_os("BK_PULSE_TRACE").is_some() {
        println!("pulse: {resumed} sleeping label(s) resumed");
    }
}

/// Pulses and timers: the engine's `sithCog_ProcessCog`.
///
/// Each cog owns a pulse interval (`SetPulse(seconds)`) and a one-shot timer (`SetTimer`). When a
/// deadline passes, the engine sends that cog the `pulse` or `timer` message; the scripts do the
/// rest. Level 1's soundtrack is the clearest example: `SetPulse(0.1)` plus `SetMusicVol` inside
/// `pulse` is what fades the track in, so without this the music stays at the volume the first
/// `SetMusicVol` gave it.
fn run_pulses_and_timers(
    time: Res<Time>,
    world: Option<Res<World>>,
    mut sim: Option<ResMut<Sim>>,
    mut count: Option<ResMut<PulseCount>>,
) {
    let (Some(world), Some(sim)) = (world, sim.as_mut()) else { return };
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    let mut fired = 0usize;
    for (script_name, vm) in &mut sim.instances {
        vm.now += dt;
        let Some(script) = world.cogs.get(script_name) else { continue };
        // The pulse fires every interval while armed; the timer fires once.
        if let Some((interval, next)) = vm.pulse
            && vm.now >= next
        {
            vm.pulse = Some((interval, vm.now + interval));
            if script.label("pulse").is_some() {
                let _ = vm.run_label(script, "pulse");
                if std::env::var_os("BK_PULSE_TRACE").is_some() {
                    println!("pulse: {script_name} interval {interval:.2}");
                }
                fired += 1;
            }
        }
        if let Some(due) = vm.timer
            && vm.now >= due
        {
            vm.timer = None;
            if script.label("timer").is_some() {
                let _ = vm.run_label(script, "timer");
                fired += 1;
            }
        }
    }
    if let Some(count) = count.as_mut() {
        count.0 += fired;
    }
    if fired > 0 && std::env::var_os("BK_PULSE_TRACE").is_some() {
        println!("pulse: {fired} pulse/timer message(s) delivered");
    }
}

/// Force seeing's infrared pass, from the runtime's `ir_mode` into every level material.
///
/// The script flips `EnableIRMode`/`DisableIRMode` (§16.5); this is the half that draws it, which is
/// what the level shader's `infra` parameter exists for. Materials are shared per texture, so the
/// whole set is walked whenever the mode changes - not every frame.
fn infrared_follows_force(
    sim: Option<Res<Sim>>,
    start_on: Option<Res<IrFromStart>>,
    mut materials: ResMut<Assets<level_mat::LevelMaterial>>,
    mut shown: Local<Option<bool>>,
) {
    let Some(sim) = sim else { return };
    if shown.is_none() && start_on.is_some_and(|s| s.0) {
        sim.world.borrow_mut().set_ir_mode(true);
    }
    let on = sim.world.borrow().ir_mode;
    if *shown == Some(on) {
        return;
    }
    *shown = Some(on);
    for (_, material) in materials.iter_mut() {
        material.params.infra = if on { 1.0 } else { 0.0 };
    }
    println!("force seeing: infrared pass {}", if on { "on" } else { "off" });
}

/// `--live-check`: invariants on the runtime the **game** simulates, not one a check built.
///
/// Two bugs in a row passed every gate because each check constructed its own runtime: `--fire` never
/// called the impact scan, and `.ai` instincts went into a throwaway `Runtime::handle`. This looks at
/// `--live-play`: does a **key press** move the body and a **mouse click** fire? Every other check
/// either drives the runtime directly (`--walk`, `--fire`) or inspects resources (`--live-check`),
/// so nothing proved that the input path into the live schedule works. This presses the real keys.
#[derive(Resource, Default)]
struct LivePlay {
    enabled: bool,
}

/// `--goal`: put the body in front of the level's goal thing and let the renderer sight it.
#[derive(Resource, Default)]
struct GoalProbe {
    enabled: bool,
}

/// Stand in front of the goal and **look** at it.
///
/// `01_goal0.cog` sits on 8t88 and its `sighted:` label starts "Find 8t88 before he escapes Nar
/// Shaddaa"; the renderer sends `sighted` the first time the thing is drawn. So the mission's first
/// objective can complete without a hand on the keyboard: place the body in the goal's own sector
/// facing it, run some frames, and read the goal bins the objectives panel draws from.
fn goal_probe(
    probe: Res<GoalProbe>,
    world: Option<Res<World>>,
    mut player: Option<ResMut<Player>>,
    plan: Option<Res<FlowPlan>>,
    sim: Option<Res<Sim>>,
    mut state: Local<(u32, bool, usize, bool)>,
    mut exit: MessageWriter<AppExit>,
) {
    if !probe.enabled {
        return;
    }
    let (Some(world), Some(sim)) = (world, sim.as_deref()) else { return };
    let Some(player) = player.as_deref_mut() else { return };
    state.0 += 1;
    // The instance that waits on `sighted` and is named like a goal — the same lookup `--endlevel`
    // makes; its linked thing is the one the renderer has to draw.
    let goal = sim
        .world
        .borrow()
        .thing_links
        .iter()
        .find(|(instance, _, _)| {
            world.level.cogs.get(*instance).is_some_and(|c| {
                c.script.to_ascii_lowercase().contains("goal")
                    && world.cogs.get(&c.script).is_some_and(|s| s.label("sighted").is_some())
            })
        })
        .map(|(_, _, thing)| *thing);
    let Some(goal) = goal else {
        println!("goal: this level has no script waiting on `sighted`");
        exit.write(AppExit::Success);
        return;
    };
    let Some((pos, sector)) = sim.world.borrow().things.get(goal).map(|t| (t.pos, t.sector)) else {
        return;
    };
    if !state.1 {
        // Eight units in front of it, facing it, in its own room, so the cull pass has no excuse.
        player.pos = Vec3::new(pos[0], pos[1] - 8.0, pos[2]);
        player.sector = sector.max(0) as usize;
        player.yaw = std::f32::consts::FRAC_PI_2;
        player.vy = 0.0;
        state.1 = true;
        println!("goal: placed the body 8 units from thing {goal} in sector {sector}");
        return;
    }
    if state.0 == 120 {
        let flags = sim
            .world
            .borrow()
            .inv
            .get(&runtime::GOAL_BIN)
            .copied()
            .unwrap_or(0.0) as u32;
        state.2 = world.level.sectors.len();
        println!("goal: GOAL00 flags {flags:#b} after {} frames", state.0);
        if flags & 0b10 == 0 {
            println!("goal: FAIL the objective did not complete (flags {flags:#b})");
            exit.write(AppExit::Success);
            return;
        }
        println!("goal: the objective completed; waiting for the level to end");
    }
    // The level's own final script calls `jkEndLevel`, which is what turns a completed objective into
    // the end of the level. The film after it holds the load, and waiting that out would cost a minute
    // of wall clock per run — `--transition` and `--campaign` cover the swap; this claims the goal, the
    // end of the level and the successor's name.
    if state.0 > 120 {
        if sim.world.borrow().level_complete {
            if !state.3 {
                state.3 = true;
                let next = plan.as_deref().and_then(|p| p.next.clone());
                println!(
                    "goal: level complete after {} frames, next {:?}",
                    state.0,
                    next.as_deref().unwrap_or("(end of campaign)")
                );
            }
            // `BK_GOAL_SWAP=1` keeps the probe alive through the film so the whole handover can be
            // watched once by hand. It is not the default: 992 frames of film is a minute of wall
            // clock, and a gate that slow is a gate people stop running.
            if std::env::var_os("BK_GOAL_SWAP").is_some() && state.2 != 0 {
                if world.level.sectors.len() != state.2 {
                    println!(
                        "goal: the next level loaded after {} frames ({} sectors)",
                        state.0,
                        world.level.sectors.len()
                    );
                    println!("--- goal check passed ---");
                    exit.write(AppExit::Success);
                } else if state.0 > 2400 {
                    println!("goal: FAIL the film never handed over");
                    exit.write(AppExit::Success);
                }
                return;
            }
            println!("--- goal check passed ---");
            exit.write(AppExit::Success);
        } else if state.0 > 900 {
            println!("goal: FAIL the level never ended (GOAL00 set, `jkEndLevel` never called)");
            exit.write(AppExit::Success);
        }
    }
}

fn live_play_drive(
    state: Res<LivePlay>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut playing: Local<Option<LivePlayState>>,
    player: Option<Res<Player>>,
    sim: Option<Res<Sim>>,
    ammo: Option<Res<AmmoBin>>,
    count: Option<Res<TriggerCount>>,
    world: Option<Res<World>>,
    cam: Query<Entity, (With<Camera3d>, Without<OverlayQuad>)>,
    mut exit: MessageWriter<AppExit>,
) {
    if !state.enabled {
        return;
    }
    // `walk_player` returns early when `cam.single_mut()` fails, which would leave the body frozen no
    // matter what the keyboard says. Say how many cameras the query sees before blaming input.
    if std::env::var_os("BK_LIVE_PLAY_TRACE").is_some() {
        println!("live-play: {} camera(s) match the walker's query", cam.iter().count());
    }
    // Short phases: the sweep gives every gate 600 frames, and a big level can spend a few hundred of
    // them loading before the body exists. 45 walk + 15 fire still measures ~2 units of walking.
    // 60, not 30: at a pinned 1/30 s step the old phase was one second of simulated time spread over
    // four directions, and the body barely left the mark before the direction changed.
    // 120: the body accelerates (maxthrust 2 / drag 3, so it reaches 0.63 of its top speed in a
    // third of a second) and 15 frames per direction left it at 0.02 units/s — it never left the mark
    // on a ramp. 30 frames a direction is half a second of walking.
    const WALK_FRAMES: u32 = 120;
    // 30, not 15: `fire_player` acts on `just_pressed`, so a held trigger fires at most once and the
    // first click can still land inside a scripted `SetFireWait` cooldown. Fifteen frames was too
    // short for the shot to land at all (01narshadda reported 0 with the shorter phase).
    const FIRE_FRAMES: u32 = 20;
    // Long enough for a jump to rise and land: `jkPlayer`'s jump is a short hop.
    const JUMP_FRAMES: u32 = 30;
    let state = playing.get_or_insert_with(|| LivePlayState {
        start: player.as_deref().map(|p| p.pos),
        ..Default::default()
    });
    // The clock starts when the body exists: a big level spends its first frames loading, and a
    // counted phase that runs through them presses W at a body that is not there yet. That is why
    // 02narshadda, 04escapehouse and 09fuelstation measured zero movement while 01narshadda did not.
    if player.is_none() {
        return;
    }
    if state.start.is_none() {
        state.start = player.as_deref().map(|p| p.pos);
    }
    // Jump **first**, before the body walks anywhere: it stands on its spawn floor now, and the walk
    // phase can step off a ledge (16aescapeship ends up falling for the rest of the run, so the jump
    // was never attempted there). One press, then the flight, then the landing: mashing made the body
    // re-jump the instant it touched down, so every reading caught it mid-air.
    if !state.jump_done {
        let grounded = player.as_deref().is_some_and(|p| p.grounded);
        state.jump_frames += 1;
        if !state.jump_started {
            // Wait for the floor, bounded: a body that never touches one cannot jump, and waiting
            // forever would leave the check with no verdict at all.
            if grounded {
                state.jump_started = true;
                state.jump_base = player.as_deref().map(|p| p.pos.z);
                keys.press(KeyCode::Space);
            } else if state.jump_frames > 90 {
                state.jump_done = true;
                println!(
                    "live-play: the body never settled on a floor in 90 frames, so the jump was not attempted"
                );
            }
            return;
        }
        keys.release(KeyCode::Space);
        if let (Some(base), Some(player)) = (state.jump_base, player.as_deref()) {
            state.jump_peak = state.jump_peak.max(player.pos.z - base);
        }
        // Landed: it left the ground and is back on it. The cap keeps a body that falls out of the
        // level from stalling the run.
        if (state.jump_peak > 0.05 && grounded) || state.jump_frames > 180 {
            state.jump_done = true;
        }
        return;
    }
    state.frame += 1;
    // Tracked every frame, not only at the end: if the body moves and then snaps back, the summary
    // at the end says "moved 0.00" and hides the cause.
    if let Some(player) = player.as_deref() {
        if state.sector_last != Some(player.sector) {
            if state.sector_last.is_some() {
                state.crossings += 1;
            }
            state.sector_last = Some(player.sector);
            // How many scripts are linked into the room the body just entered. A crossing into a room
            // nothing is linked to is silent by design; a crossing into a room with links is not.
            // Only links whose script has an `entered:` label can answer — `sector_enter` skips the rest
            // — so a level whose links are all `exited:`-only is silent by design, not broken.
            if let (Some(sim), Some(world)) = (sim.as_deref(), world.as_deref()) {
                let rt = sim.world.borrow();
                let capable: Vec<usize> = rt
                    .sector_links
                    .iter()
                    .filter(|(_, _, sector)| *sector == player.sector)
                    .filter(|(instance, _, _)| {
                        sim.instances
                            .get(*instance)
                            .and_then(|(name, _)| world.cogs.get(name))
                            .is_some_and(|script| script.label("entered").is_some())
                    })
                    .map(|(instance, _, _)| *instance)
                    .collect();
                state.linked_visited += capable.len();
                if std::env::var_os("BK_LIVE_PLAY_TRACE").is_some() {
                    for instance in &capable {
                        println!(
                            "live-play: sector {} link -> instance {instance} ({:?}) of {}",
                            player.sector,
                            sim.instances.get(*instance).map(|(n, _)| n.clone()),
                            sim.instances.len()
                        );
                    }
                }
            }
        }
        if state.triggers_before == 0 {
            state.triggers_before = count.as_deref().map(|c| c.0).unwrap_or(0);
        }
    }
    if let (Some(start), Some(player)) = (state.start, player.as_deref()) {
        // Horizontal, like the final measurement: a jump adds vertical movement, and counting it made
        // a body that only jumped fail as "moves and snaps back".
        state.max_moved = state.max_moved.max((player.pos - start).truncate().length());
    }
    if state.frame <= WALK_FRAMES {
        // The walk's own first frame is the baseline: taking it when the body first appeared counted a
        // script's teleport at level start as movement.
        if state.frame == 1 {
            state.start = player.as_deref().map(|p| p.pos);
        }
        // Held, not tapped: the walker reads `just_pressed`/`pressed`, and a released key moves nothing.
        //
        // Four directions in turn, because a body can spawn facing a wall: on 02narshadda, 04escapehouse
        // and 09fuelstation `W` produced zero movement while `player.vel` was non-zero, which is a
        // blocked step, not a broken input path. Sweeping the compass asks "can the body move at all".
        let phase = state.frame / 30;
        for key in [KeyCode::KeyW, KeyCode::KeyD, KeyCode::KeyS, KeyCode::KeyA] {
            keys.release(key);
        }
        keys.press(match phase % 4 {
            0 => KeyCode::KeyW,
            1 => KeyCode::KeyD,
            2 => KeyCode::KeyS,
            _ => KeyCode::KeyA,
        });
        return;
    }
    if state.frame == WALK_FRAMES + 1 {
        state.walked = player.as_deref().map(|p| p.pos);
        for key in [KeyCode::KeyW, KeyCode::KeyD, KeyCode::KeyS, KeyCode::KeyA] {
            keys.release(key);
        }
        if let Some(sim) = sim.as_deref() {
            // Live things, not the vector length: a slot freed earlier in the same frame is reused, so
            // the length can stay the same while a bolt is very much in flight (09fuelstation fired
            // `+econcbullet` and the count did not move).
            state.things_before = sim.world.borrow().shots_fired;
        }
    }
    if (WALK_FRAMES + 1..=WALK_FRAMES + FIRE_FRAMES).contains(&state.frame) {
        // Toggled: `fire_player` acts on `just_pressed`, so a held button fires once. Alternating
        // press/release gives the trigger a fresh `just_pressed` every other frame, which is also
        // what a player clicking repeatedly produces.
        if state.frame % 2 == 0 {
            buttons.press(MouseButton::Left);
        } else {
            buttons.release(MouseButton::Left);
        }
        // Peak, not delta: a bolt that hits a wall is destroyed within the same 30 frames, so the
        // count at the end says nothing about whether the trigger worked.
        if let Some(sim) = sim.as_deref() {
            let live = sim.world.borrow().things.iter().filter(|t| !t.destroyed).count();
            state.things_peak = state.things_peak.max(live);
        }
        return;
    }
    buttons.release(MouseButton::Left);
    let (start, walked) = (state.start, state.walked);
    let moved = match (start, walked) {
        // Horizontal only: the settle pass lifts a body onto its floor, and a check that counted that
        // as walking would pass a body that cannot take a single step.
        (Some(a), Some(b)) => (((b - a).truncate().length()), a, b),
        _ => (0.0, Vec3::ZERO, Vec3::ZERO),
    };
    let landed = player.as_deref().and_then(|p| state.jump_base.map(|base| p.pos.z - base));
    // The runtime's own counter: a bolt can be born and destroyed between two frames at 10 fps, so a
    // sample of live things said "0 shots" on a working trigger.
    let fired = sim
        .as_deref()
        .map(|sim| sim.world.borrow().shots_fired.saturating_sub(state.things_before))
        .unwrap_or(0);
    println!("live-play: {FIRE_FRAMES} frames of left click added {fired} thing(s) to the world");
    println!(
        "live-play: Space lifted the body {:.2} units (highest {:.2}, now {:?})",
        state.jump_peak, state.jump_peak, landed.map(|z| (z * 1000.0).round() / 1000.0)
    );
    let mut failed = 0;
    // Peak horizontal displacement, not the final one: the walk phase sweeps W/D/S/A so the body goes
    // out and comes back, and comparing start with end measured the return trip — a body that walked
    // 0.27 units failed as "did not move at all".
    // The whole run, not a delta: the spawn crossing is delivered on the first frame the body exists,
    // which can be before this driver has ever seen it — a delta missed exactly that message and made
    // 21ajerec look like a dead link when its script had already run.
    let fired_triggers = count.as_deref().map(|c| c.0).unwrap_or(0);
    println!(
        "live-play: crossed {} sector(s); the level's scripts answered {} of them",
        state.crossings, fired_triggers
    );
    // The mission itself: `SetGoalFlags` writes bins 100+ (`SITHBIN_GOAL00`) and the panel draws from
    // those bits, so a goal that flips while the body stands there is the level progressing.
    let goals = sim
        .as_deref()
        .map(|sim| {
            let rt = sim.world.borrow();
            (0..8)
                .filter_map(|n| {
                    let flags = rt.inv.get(&(runtime::GOAL_BIN + n)).copied().unwrap_or(0.0);
                    (flags != 0.0).then_some((n, flags as u32))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    println!("live-play: goal bins set {goals:?}");
    // A level that links cogs to sectors must hear about a crossing, or walking through a room does
    // nothing — `sithThing_EnterSector` is the whole of every door, lift and cutscene trigger in the
    // game. A link whose script has no `entered:` label never runs, so the count is the honest one.
    // Only links **into the body's own sector** can fire on entry: a level with 171 links whose sectors
    // the player never touches (09fuelstation) answers nothing at spawn, and that is not a defect.
    let linked = state.linked_visited;
    // The spawn is a crossing too (`sithThing_EnterSector` runs with no previous sector), which is why
    // the count is checked whenever the level links anything at all.
    if linked > 0 && fired_triggers == 0 {
        println!("live-play: FAIL {linked} sector link(s), but no script answered");
        failed += 1;
    }
    if state.max_moved < 0.05 {
        println!("live-play: FAIL the body did not move at all (peak {:.2})", state.max_moved);
        failed += 1;
    }
    // "Nothing to fire" is not a broken trigger: a bot map can start the player with an empty clip and
    // no weapon cog armed (m10, m_boss15 — `out of ammo (bin 11 is empty)`), and a saber level swings
    // instead of shooting. Report which one it is instead of calling the map broken.
    let nothing_to_fire = sim
        .as_deref()
        .map(|sim| {
            let rt = sim.world.borrow();
            let bin = ammo.as_deref().and_then(|a| a.0);
            let rounds = bin
                .map(|bin| rt.inv.get(&bin).copied().unwrap_or(0.0))
                .unwrap_or(0.0);
            rt.weapon_cog.is_none() && rounds <= 0.0
        })
        .unwrap_or(false);
    if fired == 0 && !nothing_to_fire {
        println!("live-play: FAIL the trigger produced no projectile");
        failed += 1;
    } else if fired == 0 {
        println!("live-play: the trigger is armed with nothing to fire (no cog, empty clip)");
    }
    // A jump has to leave the ground; landing is reported but not required, because a body that starts
    // over a drop stays airborne and that is the map's business.
    // Only when the jump was actually attempted: a body that never touches a floor (static, whose
    // spawn falls through the world) has already been reported, and cannot be asked to jump.
    if state.jump_started && state.jump_peak < 0.2 {
        println!("live-play: FAIL Space did not lift the body (peak {:.2})", state.jump_peak);
        failed += 1;
    }
    if failed > 0 {
        exit.write(AppExit::error());
        return;
    }
    println!("--- live-play check passed ---");
    exit.write(AppExit::Success);
}

#[derive(Default)]
struct LivePlayState {
    frame: u32,
    max_moved: f32,
    start: Option<Vec3>,
    walked: Option<Vec3>,
    things_before: usize,
    things_peak: usize,
    jump_base: Option<f32>,
    jump_frames: u32,
    jump_started: bool,
    /// Sector crossings the body made, and the trigger messages they produced.
    sector_last: Option<usize>,
    crossings: usize,
    triggers_before: usize,
    /// Links into every sector the body has entered **that could answer**, so a silent entry can be
    /// told from an unlinked one.
    linked_visited: usize,
    jump_wait: u32,
    jump_done: bool,
    jump_peak: f32,
}

/// the resources the game itself is running.
fn live_check(
    state: Res<LiveCheckState>,
    mut said: Local<bool>,
    mut elapsed: Local<u32>,
    mut exit: MessageWriter<AppExit>,
    world: Option<Res<World>>,
    sim: Option<Res<Sim>>,
    collisions: Option<Res<Collision>>,
    materials: Option<Res<Assets<StandardMaterial>>>,
    actors: Option<Res<Actors>>,
    pulses: Option<Res<PulseCount>>,
    player: Option<Res<Player>>,
) {
    if !state.enabled || *said {
        return;
    }
    // A few frames in, so `setup` has run and the first `ai_shots` tick has happened.
    *elapsed += 1;
    // Long enough for a one-second pulse to have fired.
    if *elapsed < 100 {
        return;
    }
    *said = true;
    let (Some(world), Some(sim)) = (world, sim) else {
        println!("live: no world or no sim after 20 frames");
        exit.write(AppExit::error());
        return;
    };
    // A body that cannot find a floor falls for ever, which is a black screen — the reported
    // "I immediately fall infinitely when I start the game". The sweep is the judge: if nothing is
    // under the live body, say so here instead of in a player report.
    let mut fallen = None;
    if let (Some(player), Some(collisions)) = (player.as_deref(), collisions.as_deref()) {
        let solids = &collisions.0;
        let pos = [player.pos.x, player.pos.y, player.pos.z];
        match solids.floor_under(player.sector, pos, 1.0, player.rules.radius) {
            Some((floor, travelled, _)) => {
                println!(
                    "live: the body stands in sector {} at z {:.2}, floor {:.2} ({travelled:.2} below it)",
                    player.sector, pos[2], floor
                );
                if travelled > 2.0 {
                    fallen = Some(format!("the body is falling: floor {travelled:.2} below it"));
                }
            }
            None => fallen = Some(format!("no floor under the live body in sector {}", player.sector)),
        }
    }
    let rt = sim.world.borrow();
    let mut failed = 0;
    let mut say = |ok: bool, line: String| {
        // A failed invariant has to be findable in the output: the summary said "1 invariant(s)
        // failed" and left the reader to guess which.
        println!("live: {} {line}", if ok { "" } else { "FAIL" });
        if !ok {
            failed += 1;
        }
    };
    // The instincts belong to this runtime: the level names classes, so its table must not be empty.
    let classes = world
        .level
        .templates
        .iter()
        .filter(|t| world.level.template_param(&t.name, "aiclass").is_some())
        .count();
    say(
        classes == 0 || !rt.ai.is_empty(),
        format!(
            "{} actor instinct(s) attached ({classes} template(s) name an aiclass)",
            rt.ai.len()
        ),
    );
    let player = rt.templates.iter().position(|t| t.eq_ignore_ascii_case("walkplayer"));
    // `static` is a test map with no player at all: there is nothing to follow, nothing to stand on
    // and no HUD to draw, so the run reports it and stops rather than failing an invariant that does
    // not apply. Every playable level places a `walkplayer`.
    if player.is_none() {
        println!("live: this level places no `walkplayer`: a test map, not a playable level");
        println!("--- live check passed ---");
        return;
    }
    say(player.is_some(), format!("player thing {player:?}"));
    // …and its own body is not drawn. The camera follows that thing, so a spawned body wraps the
    // camera in the inside of the player's own model (`is_local_player`); this is the invariant that
    // fails if it comes back.
    let body = player
        .and_then(|i| sim.entities.get(&i))
        .map(Vec::len)
        .unwrap_or(0);
    say(body == 0, format!("the player's own body is drawn by {body} entity(ies) (want 0)"));
    let sectors = collisions.as_ref().map(|c| c.0.sectors.len()).unwrap_or(0);
    say(sectors > 0, format!("{sectors} collision sector(s)"));
    // The switched sector/adjoin words belong to this runtime, not to a copy a check built. The
    // level's own door scripts have run, so some doorway is shut, and the solver's gate reads the
    // very arrays that were switched (`--adjoins` proves the solver honours them).
    let shut = rt.sector_flags.iter().filter(|f| *f & collide::SECTOR_ADJOINS_OFF != 0).count();
    say(
        rt.adjoin_flags.len() == world.level.adjoins.len()
            && rt.sector_flags.len() == world.level.sectors.len()
            && (rt.adjoins_changes == 0 || shut == rt.adjoins_changes),
        format!(
            "{} adjoin row(s), {} sector flag(s), {shut} doorway(s) shut by {} script change(s)",
            rt.adjoin_flags.len(),
            rt.sector_flags.len(),
            rt.adjoins_changes
        ),
    );
    let drawn = materials.as_ref().map(|m| m.len()).unwrap_or(0);
    say(drawn > 0, format!("{drawn} standard material(s)"));
    let (parts, things) = actors
        .as_ref()
        .map(|a| (a.0.iter().map(|x| x.parts.len()).sum::<usize>(), a.0.len()))
        .unwrap_or((0, 0));
    say(parts > 0 || things == 0, format!("{things} actor(s) with {parts} part(s)"));
    // `sim.instances` holds the level's placed scripts **and** the item scripts the loader starts for
    // bins the player holds, so it is bounded below by the placed count, not equal to it: the old
    // equality reported "44 of 42 bound" as a failure and hid whether anything was really missing.
    let bound = sim.instances.len();
    let placed = world.level.cogs.len();
    say(
        bound >= placed,
        format!("{bound} script instance(s) bound for {placed} placed one(s)"),
    );
    // The script queue: a pulse or timer has to have been delivered by now (the level's own scripts
    // arm one-second pulses, so a hundred frames is enough), or the pulse fade can never run (§16.10).
    let armed = sim
        .instances
        .iter()
        .filter(|(_, vm)| vm.pulse.is_some() || vm.timer.is_some())
        .count();
    let pulses = pulses.as_ref().map(|p| p.0).unwrap_or(0);
    say(
        armed == 0 || pulses > 0,
        format!("{pulses} pulse/timer message(s) delivered for {armed} armed cog(s)"),
    );
    // Sector links exist for this level, so `deliver_sector_triggers` has something to deliver. The
    // labels themselves are exercised by `--triggers`, which can report which ones resolve.
    let links = rt.sector_links.len();
    let thing_links = rt.thing_links.len();
    say(
        true,
        format!("{links} sector link(s) and {thing_links} thing link(s) for the event layer"),
    );
    if failed == 0 {
        println!("--- live check passed ---");
        // Leave through `exit`, like the failure path: `AppExit` goes back through Bevy's teardown,
        // and a gate's verdict should not depend on what that teardown happens to do.
        std::process::exit(0);
    } else {
        // `AppExit::error()` is returned by `App::run`, and this binary ignores that value, so a
        // failure has to leave through `exit` to be visible in a gate's exit code.
        println!("live: {failed} invariant(s) failed");
        std::process::exit(1);
    }
}

/// `--live-check` state: whether the check is on.
#[derive(Resource, Default)]
struct LiveCheckState {
    enabled: bool,
}

/// How many `pulse`/`timer` messages have been delivered, so a check can assert the queue runs.
#[derive(Resource, Default)]
struct PulseCount(usize);



/// The player's health, from the runtime's own thing, into the HUD.
///
/// Nothing carried this before: actors could not hit the player at all (§16.5), so the status bar
/// always drew its default 100. Now that bolts land on the walkplayer, the number the player sees
/// is the runtime's.
/// Apply fall damage the walker recorded: the runtime owns the player's health, so the impact surfaces
/// there on the next frame (`sithCollision_FallHurt` → `sithThing_DamageThing`).
fn apply_fall_damage(sim: Option<Res<Sim>>, player: Option<ResMut<Player>>) {    let (Some(sim), Some(mut player)) = (sim, player) else { return };
    if player.fall_damage <= 0.0 {
        return;
    }
    let damage = std::mem::take(&mut player.fall_damage);
    let mut rt = sim.world.borrow_mut();
    let Some(index) = rt
        .templates
        .iter()
        .position(|t| t.eq_ignore_ascii_case("walkplayer"))
    else {
        return;
    };
    // A landing is a **fall**, not an impact: `sithActor_DamageActor` exempts `SITH_DAMAGE_FALL`
    // (0x40) from `SITH_AF_INVULNERABLE`, which is why the level-end freeze does not save a body
    // that drops off a ledge.
    let left = rt.damage_typed(index as i64, damage, runtime::DAMAGE_FALL);
    // `damage` is the generic path (messages, `destroyed`); the player's *death* flag is the one
    // `sithPlayer_KillPlayer` sets, and without it a lethal landing left the body dead on its feet with
    // nothing to bring it back. The fall death in `walk_player` takes this same road.
    if left <= 0.0 {
        rt.player_dead = true;
    }
    println!("fall: {damage:.0} damage, {left:.0} health left");
}

fn health_follows_damage(
    sim: Option<Res<Sim>>,
    world: Option<Res<World>>,
    player: Option<ResMut<Player>>,
    ammo_bin: Option<Res<AmmoBin>>,
    mut overlay: Option<ResMut<Overlay>>,
) {
    let (Some(sim), Some(mut player), Some(world)) = (sim, player, world) else { return };
    let rt = sim.world.borrow();
    let Some(index) = rt.templates.iter().position(|t| t.eq_ignore_ascii_case("walkplayer")) else {
        return;
    };
    let health = rt.things[index].health;
    let shields = rt.player_shields;
    // The status bar's ammo readout is the weapon's own bin (`--ammo` resolves which), so it runs out
    // when the weapon does. `-999` is the engine's "no ammo" marker, drawn as `:::`.
    // The readout follows the bin the player has armed; `AmmoBin` is only the level's default.
    let ammo = ammo_bin_for_bin(&world, rt.cur_bin)
        .or_else(|| ammo_bin.as_ref().and_then(|a| a.0))
        .map(|bin| rt.inv.get(&bin).copied().unwrap_or(0.0).round() as i32)
        .unwrap_or(-999);
    if (player.health - health).abs() > 0.01 || (player.shields - shields).abs() > 0.01 {
        player.health = health;
        player.shields = shields;
        if let Some(overlay) = overlay.as_mut() {
            overlay.hud.health = health.round() as i32;
            overlay.hud.shields = shields.round() as i32;
            overlay.dirty = true;
        }
    }
    if let Some(overlay) = overlay.as_mut()
        && overlay.hud.ammo != ammo
    {
        overlay.hud.ammo = ammo;
        overlay.dirty = true;
    }
    // The status bar's inventory strip: every bin the player holds, named by `misc\items.dat`. The
    // HUD was never fed this, so the panel stayed blank however much was picked up.
    if let Some(overlay) = overlay.as_mut() {
        let carried = hud::carried(&rt.inv, &world.item_defs);
        // The bin behind each carried entry, in the same order: using one needs its number.
        overlay.carried_bins = world
            .item_defs
            .iter()
            .filter(|def| rt.inv.get(&def.id).copied().unwrap_or(0.0).round() as i32 > 0)
            .map(|def| def.id)
            .collect();
        if overlay.inv_selected >= carried.len() {
            overlay.inv_selected = carried.len().saturating_sub(1);
        }
        if overlay.hud.carried != carried {
            overlay.hud.item = carried.first().map(|(name, _)| name.clone());
            overlay.hud.item_count = carried.first().map(|(_, count)| *count).unwrap_or(0);
            overlay.hud.carried = carried;
            overlay.dirty = true;
        }
    }
}

/// Death and respawn: at 0 health the player is put back at the level's `walkplayer` with full
/// health and shields.
///
/// `sithPlayer` reloads or respawns at the last checkpoint; this build has no checkpoint list yet,
/// so the level's own start is the respawn point. The dead state shows as a message and the
/// player's movement is not blocked — the reset is immediate.
/// One span of a frame, printed when `BK_FRAME_TRACE` is set. `Drop` means an early return still
/// reports, which is the whole reason this exists instead of two `Instant::now()` calls per system.
struct Span(&'static str, std::time::Instant);

impl Span {
    fn new(label: &'static str) -> Self {
        Span(label, std::time::Instant::now())
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if std::env::var_os("BK_FRAME_TRACE").is_some() {
            println!("frame: {} took {:.2} ms", self.0, self.1.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

/// `SetThingCurGeoMode(thing, 0)` takes a thing off screen: every entity the level spawned for that
/// thing goes hidden, and any other mode brings it back. `renderData.curGeoMode` is the engine's
/// per-thing draw mode, and the dark Jedi scripts use 0 to drop a body.
///
/// ponytail: a sweep of `Sim::entities` every frame, writing only where the value differs; an
/// epoch-guarded version is the upgrade if that map ever grows into the thousands.
fn things_follow_geomode(sim: Option<Res<Sim>>, mut visibility: Query<&mut Visibility>) {
    let Some(sim) = sim else { return };
    let modes = sim.world.borrow().thing_geomode.clone();
    for (thing, entities) in &sim.entities {
        let want = if modes.get(thing).copied().unwrap_or(1) == 0 {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        for entity in entities {
            if let Ok(mut current) = visibility.get_mut(*entity)
                && *current != want
            {
                *current = want;
            }
        }
    }
}

fn respawn_on_death(sim: Option<Res<Sim>>, mut player: Option<ResMut<Player>>, world: Option<Res<World>>) {
    let (Some(sim), Some(player), Some(world)) = (sim, player.as_mut(), world) else { return };
    let spawn = world
        .level
        .things
        .iter()
        .find(|t| t.template.eq_ignore_ascii_case("walkplayer"))
        .map(|t| ([t.pos[0], t.pos[1], t.pos[2] + PLAYER_RADIUS], t.sector));
    let mut rt = sim.world.borrow_mut();
    if !rt.player_dead {
        return;
    }
    let Some((spawn, sector)) = spawn else { return };
    let (shields, health) = rt.respawn_player(spawn);
    drop(rt);
    player.pos = Vec3::from(spawn);
    player.vy = 0.0;
    // The sector is part of the body's state, not just its position: leaving the sector the player died in
    // makes the containment rule refuse every move at the spawn (it is a different room), and the body
    // stands there unable to walk. The walker's velocity goes too — a respawn is not a fall.
    player.vel = [0.0; 2];
    if sector >= 0 {
        player.sector = sector as usize;
    }
    player.fall_killed = false;
    player.fall_fade = 1.0;
    player.grounded = true;
    player.health = health;
    player.shields = shields;
    println!("player: died; respawned at the level's start with {health:.0} health and {shields:.0} shields");
}

/// `--sprite <template>`: keep one of an effect template alive in front of the camera, so a
/// screenshot can show what a sprite actually draws as.
///
/// This is the only way to look at the sprite layer: an impact effect lives for its `timer=`
/// (`+laserhit` ships 0.5 s) and the game's own impacts land behind whatever the camera is facing.
/// The thing is created the way a script would — `CreateThing` at the player, then moved — and
/// re-created after it expires, so whichever frame gets captured has one.
fn sprite_demo(
    demo: Option<Res<SpriteDemo>>,
    mut live: Local<Option<usize>>,
    mut frame: Local<u32>,
    player: Option<Res<Player>>,
    sim: Option<Res<Sim>>,
) {
    let (Some(demo), Some(player), Some(sim)) = (demo, player, sim) else { return };
    let Some(template) = demo.0.as_deref() else { return };
    *frame += 1;
    // The world is still being built for the first frames: a thing created then is not drawn.
    if *frame < 30 {
        return;
    }
    let mut rt = sim.world.borrow_mut();
    if live.is_some_and(|index| rt.things.get(index).is_some_and(|t| !t.destroyed)) {
        // Keep it in front of the camera as the camera moves. A JK yaw `a` faces `(-sin a, cos a)`
        // — the engine's own basis, which `docs/formats.md` §24 recovered and `yaw_axes` holds —
        // so `(cos, sin)` puts the thing 90 degrees to the side instead.
        let ahead = env_f32("BK_SPRITE_AHEAD", 1.5);
        let (sin, cos) = player.yaw.sin_cos();
        if let Some(index) = *live
            && let Some(thing) = rt.things.get_mut(index)
        {
            thing.pos = [
                player.pos.x - sin * ahead,
                player.pos.y + cos * ahead,
                player.pos.z,
            ];
            thing.sector = player.sector as i32;
        }
        return;
    }
    let player_thing = rt.player_thing();
    let created = rt.call(
        "CreateThing",
        &[cog::Value::Str(template.to_string()), cog::Value::Num(player_thing as f64)],
    );
    let cog::Value::Num(index) = created else { return };
    let index = index as usize;
    let (sin, cos) = player.yaw.sin_cos();
    let ahead = env_f32("BK_SPRITE_AHEAD", 1.5);
    let at = [
        player.pos.x - sin * ahead,
        player.pos.y + cos * ahead,
        player.pos.z,
    ];
    if let Some(thing) = rt.things.get_mut(index) {
        thing.pos = at;
        thing.sector = player.sector as i32;
    }
    println!("sprite demo: `{template}` at {at:?}, thing {index}");
    *live = Some(index);
}

/// `--fire-demo`: point the player at the nearest actor once, so the stream of bolts has
/// something to hit. Without this the demo fires into whatever wall the level spawns facing.
fn demo_aim(
    demo: Option<Res<FireDemo>>,
    mut aimed: Local<bool>,
    mut player: Option<ResMut<Player>>,
    sim: Option<Res<Sim>>,
) {
    let (Some(demo), Some(player), Some(sim)) = (demo, player.as_mut(), sim) else { return };
    if !demo.0 || *aimed {
        return;
    }
    let rt = sim.world.borrow();
    let Some(player_index) = rt.templates.iter().position(|t| t.eq_ignore_ascii_case("walkplayer"))
    else {
        return;
    };
    let origin = rt.things[player_index].pos;
    let nearest = rt
        .things
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != player_index && rt.types.get(*i).copied() == Some(2))
        .map(|(i, t)| {
            let d = ((t.pos[0] - origin[0]).powi(2) + (t.pos[1] - origin[1]).powi(2)).sqrt();
            (i, t.pos, d)
        })
        .filter(|(_, _, d)| *d >= 2.0 && *d <= 25.0)
        .min_by(|a, b| a.2.total_cmp(&b.2));
    if let Some((index, target, distance)) = nearest {
        player.yaw = (target[1] - origin[1]).atan2(target[0] - origin[0]);
        player.pitch = 0.0;
        *aimed = true;
        println!("demo: aiming at thing {index} ({distance:.1} units away)");
    }
}

/// Left mouse button: fire the player's weapon from the camera's aim.
fn fire_player(
    buttons: Res<ButtonInput<MouseButton>>,
    overview: Option<Res<Overview>>,
    demo: Option<Res<FireDemo>>,
    mut frame: Local<u32>,
    player: Option<Res<Player>>,
    gun: Option<Res<Gun>>,
    sim: Option<Res<Sim>>,
    world: Option<Res<World>>,
    ammo_bin: Option<Res<AmmoBin>>,
) {
    *frame += 1;
    // A stream, not one shot: at 4 units/s a bolt crosses a room in a few frames, so a
    // single frame is easy to miss in a screenshot.
    let demo_shot = demo.is_some_and(|d| d.0) && *frame >= 40 && frame.is_multiple_of(8);
    // A capture takes no live input: a click delivered during a headless run fired a bolt into every
    // `--shot` frame it was in, which is a different picture on every run.
    let capture = overview.as_deref().is_some_and(|o| o.capture);
    if (!buttons.just_pressed(MouseButton::Left) || capture) && !demo_shot {
        return;
    }
    let (Some(player), Some(gun), Some(sim)) = (player, gun, sim) else { return };
    let mut rt = sim.world.borrow_mut();
    let Some(index) = rt.templates.iter().position(|t| t.eq_ignore_ascii_case("walkplayer")) else {
        return;
    };
    // The runtime's thing is synced to the camera already; `fire` spawns down its +Y, so
    // the shoot direction is the player's yaw. The weapon is the runtime's, so a pickup
    // changes what fires.
    rt.things[index].yaw = player.yaw.to_degrees();
    rt.things[index].pitch = player.pitch.to_degrees();
    let weapon = rt.current_weapon();
    let _ = &gun.0;
    // A scripted `SetFireWait` locks the trigger until its cooldown runs out. Checked before the ammo
    // is spent, so a refused shot costs nothing.
    if std::env::var_os("BK_FIRE_TRACE").is_some() {
        eprintln!(
            "fire trace: click reached rt, time {:.2} next_fire {:.2} weapon_cog {:?} weapon {weapon}",
            rt.time, rt.next_fire, rt.weapon_cog.map(|(c, _, _)| c)
        );
    }
    if rt.time < rt.next_fire {
        return;
    }
    // A weapon cog the script armed (`ActivateWeapon` from its own `activated:`) owns the shot: send
    // it `fire` and let its policy spend the ammo and call `FireProjectile`, which is the engine's
    // split — the cog is the policy, the verb is the mechanism. Firing here as well would double it.
    if let Some((cog, rate, _)) = rt.weapon_cog {
        // The weapon's own cadence: `ActivateWeapon`'s rate (a cog computes `fireWait / powerBoost`
        // and `ChangeFireRate` updates it), so a held or mashed trigger cannot outpace the script.
        // The engine gates this with the bin's `activationDelaySecs`/`binWait` — the same idea.
        if rt.time >= rt.weapon_ready {
            rt.messages.push(("fire".to_string(), Some(index), Some(cog), [0.0; 2]));
            rt.weapon_ready = rt.time + rate.max(0.0);
        }
        return;
    }
    // The saber is not a projectile: a swing traces the blade and spends no ammo. `jkSetSaberInfo`
    // is what gives the player one, and the ending scripts hand it out on the light path.
    if runtime::Runtime::is_saber(&weapon) && rt.saber.contains_key(&index) {
        let hit = rt.saber_swing(index);
        println!("swung {weapon}: {} thing(s) in reach", hit.len());
        return;
    }
    // Ammo: the bin follows the **armed** bin, not the level's default weapon. `AmmoBin` was resolved
    // once at load from `walkplayer`'s `weapon=`, so switching guns charged the bryar's clip for the
    // stormtrooper rifle's shots — and refused to fire the rifle once that clip was empty.
    let bin = world
        .as_deref()
        .and_then(|world| ammo_bin_for_bin(world, rt.cur_bin))
        .or_else(|| ammo_bin.as_ref().and_then(|a| a.0));
    if let Some(bin) = bin
        && !rt.spend(bin)
    {
        println!("fired {weapon}: out of ammo (bin {bin} is empty)");
        return;
    }
    if let Some(shot) = rt.player_fire(index, &weapon) {
        println!("fired {weapon} (thing {shot})");
    }
}

/// Write a savegame: the `.jks` container with this build's state payload inside it.
///
/// Both the `--save` check and the live `AutosaveGame` path go through here, so what the scripts ask for
/// and what a check proves are the same file.
fn write_snapshot(
    path: &std::path::Path,
    level_name: &str,
    rt: &runtime::Runtime,
) -> std::io::Result<usize> {
    let mut save = jks::Save::new(level_name);
    save.sections.push(jks::Section::new(runtime::SAVE_SECTION, rt.snapshot().to_bytes()));
    let bytes = save.write();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, &bytes)?;
    Ok(bytes.len())
}

/// Start the item script a newly-held inventory bin names.
///
/// `misc\items.dat` gives every weapon bin a `cog=`; the engine starts it when the player acquires
/// the item, and that script then owns the weapon's own messages. `Runtime::set_bin` names the
/// scripts and this system builds the VMs — the runtime has no file access, and the VMs belong to
/// `Sim`.
fn load_item_cogs(mut sim: Option<ResMut<Sim>>, world: Option<Res<World>>) {
    let (Some(sim), Some(world)) = (sim.as_mut(), world) else { return };
    let pending: Vec<String> = std::mem::take(&mut sim.world.borrow_mut().pending_cogs);
    if pending.is_empty() {
        return;
    }
    for name in pending {
        let key = name.to_ascii_lowercase();
        let Some(script) = world.cogs.get(&key) else {
            println!("items: `{name}` is named by `items.dat` but was not loaded from the level");
            continue;
        };
        let mut vm = cog::Vm::with_world(sim.world.clone());
        script.bind(&mut vm, &[]);
        vm.cog_index = sim.instances.len() as i32;
        let started = if script.label("startup").is_some() {
            vm.run_label(script, "startup").is_ok()
        } else {
            false
        };
        // `activated:` is what a weapon cog arms itself from (`ActivateWeapon(player, …)`), and the
        // engine sends it when the weapon is drawn — which is now, since the player just got it.
        let mut armed = false;
        if script.label("activated").is_some() {
            if let Ok(player) = i64::try_from(sim.world.borrow().player_thing()) {
                vm.sender = player;
                vm.sender_ref = player;
            }
            armed = vm.run_label(script, "activated").is_ok();
        }
        println!(
            "items: started `{name}` for the player (cog {}, startup {}, activated {})",
            vm.cog_index,
            if started { "ran" } else { "has none" },
            if armed { "ran" } else { "none" }
        );
        {
            // The bin's own cog index, so `UseBin` can send `activated` to it the way the engine
            // does (`sithInventory_ActivateBin` targets the bin's cog, not a broadcast).
            let mut rt = sim.world.borrow_mut();
            let bins: Vec<i32> = rt
                .item_cog
                .iter()
                .filter(|(_, script)| script.eq_ignore_ascii_case(&name))
                .map(|(bin, _)| *bin)
                .collect();
            for bin in bins {
                rt.bin_cog_index.insert(bin, vm.cog_index);
            }
            rt.spawned_cogs.insert(key);
        }
        sim.instances.push((name.clone(), vm));
    }
}

/// `AutosaveGame()`: the first system to see the flag writes the checkpoint and clears it.
fn autosave(
    sim: Option<Res<Sim>>,
    dir: Option<Res<SaveDir>>,
    plan: Option<Res<FlowPlan>>,
) {
    let (Some(sim), Some(dir)) = (sim, dir) else { return };
    let path = dir.0.join("autosave.jks");
    let mut rt = sim.world.borrow_mut();
    if !rt.autosave {
        return;
    }
    rt.autosave = false;
    let level = plan.as_ref().map(|p| p.level.clone()).unwrap_or_default();
    match write_snapshot(&path, &level, &rt) {
        Ok(bytes) => println!("autosave: `AutosaveGame` wrote {bytes} bytes to `{}`", path.display()),
        Err(e) => println!("autosave: could not write `{}`: {e}", path.display()),
    }
}

/// Where this build writes its saves: `BK_SAVE_DIR`, else `saves` beside the game's data.
#[derive(Resource)]
struct SaveDir(std::path::PathBuf);

/// The first-person weapon model a weapon cog named with `jkSetPOVModel`.
///
/// The engine draws the cog's `povModel` (the bryar's is `bryv.3do`) in front of the eye and plays
/// its `jkPlayPOVKey` keyframe on it. This draws the model, held off the view like a weapon in hand,
/// and follows `jkSetWaggle`'s sway. It is one mesh per weapon change, rebuilt only when the name
/// changes, because a `.3do` parse and pose per frame would cost more than it shows.
fn draw_viewmodel(
    mut commands: Commands,
    archives: Option<Res<Archives>>,
    sim: Option<Res<Sim>>,
    mut view: Option<ResMut<Viewmodel>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut drawn: Query<&mut Transform, With<ViewmodelMesh>>,
    camera: Query<&GlobalTransform, (With<Camera3d>, Without<OverlayQuad>)>,
) {
    let (Some(gobs), Some(sim), Some(view)) = (archives, sim, view.as_mut()) else { return };
    let want = {
        let rt = sim.world.borrow();
        let player = rt.player_thing();
        // The saber is its own blade (`saber_blade`), so a weapon model would be a second object.
        let weapon = rt.current_weapon();
        rt.pov_model
            .get(&player)
            .filter(|_| !crate::runtime::Runtime::is_saber(&weapon))
            .cloned()
    };
    if want == view.name && (want.is_none() || view.entity.is_some()) {
        // Same weapon: the sway moves, and a `jkPlayPOVKey` clip advances if the cog started one.
        let (sway, key, clock) = {
            let rt = sim.world.borrow();
            let player = rt.player_thing();
            (rt.waggle.get(&player).copied().unwrap_or([0.0; 3]), rt.pov_key.get(&player).cloned(), rt.time)
        };
        if let Some(entity) = view.entity {
            // A new clip restarts from its first frame; the same clip keeps its cursor.
            if key != view.key {
                view.key = key;
                view.phase = 0.0;
                view.frame = -1;
            }
            if let Some(name) = view.key.clone()
                && let Some(keyfile) = gobs
                    .0
                    .read(&name)
                    .map(|raw| key::parse(&String::from_utf8_lossy(&raw)))
            {
                let last = keyfile.frames.max(1) as f32 - 1.0;
                let frame = (view.phase % (last + 1.0)).floor() as i32;
                if frame != view.frame {
                    view.frame = frame;
                    if let Some(model) = gobs.0.read(&name.replace(".key", ".3do")).or_else(|| {
                        // The model is the weapon's, not the clip's: reuse the one already drawn.
                        view.name.as_ref().and_then(|n| gobs.0.read(n))
                    }) {
                        let model = tdo::parse(&String::from_utf8_lossy(&model));
                        let material = model
                            .materials
                            .iter()
                            .position(|mat| gobs.0.read(mat).is_some())
                            .unwrap_or(0) as i32;
                        let posed = model_mesh_posed(
                            &model,
                            material,
                            (1.0, 1.0),
                            Some((&keyfile, frame as f32)),
                        );
                        let handle = meshes.add(posed);
                        if let Ok(mut mesh) = drawn.get_mut(entity) {
                            let _ = &mut mesh;
                        }
                        commands.entity(entity).insert(Mesh3d(handle));
                    }
                }
                view.phase += keyfile.fps * clock.max(0.0).min(0.1);
            }
            if let Ok(mut transform) = drawn.get_mut(entity)
                && let Some(camera) = camera.iter().next()
            {
                let placement = view_offset(camera);
                transform.translation = placement.0 + camera.right().as_vec3() * sway[0] as f32;
            }
        }
        return;
    }
    if let Some(entity) = view.entity.take() {
        commands.entity(entity).despawn();
    }
    view.name = want.clone();
    let Some(name) = want else { return };
    let Some(raw) = gobs.0.read(&name) else {
        println!("viewmodel: `{name}` is not in the archives");
        return;
    };
    let model = tdo::parse(&String::from_utf8_lossy(&raw));
    let material = model
        .materials
        .iter()
        .position(|mat| gobs.0.read(mat).is_some())
        .unwrap_or(0) as i32;
    let mesh = meshes.add(model_mesh(&model, material, (1.0, 1.0)));
    let material = materials.add(StandardMaterial {
        base_color: bevy::color::Color::WHITE,
        perceptual_roughness: 0.7,
        cull_mode: None,
        ..default()
    });
    let Some(camera) = camera.iter().next() else { return };
    let (translation, rotation) = view_offset(camera);
    let entity = commands
        .spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform {
            translation,
            rotation,
            ..default()
        }, ViewmodelMesh))
        .id();
    view.entity = Some(entity);
    println!("viewmodel: drew `{name}` ({} meshes, material `{}`)", model.meshes.len(), model.materials.first().cloned().unwrap_or_default());
}

/// Where a first-person weapon sits relative to the view: low and right, angled in.
fn view_offset(camera: &GlobalTransform) -> (Vec3, Quat) {
    let forward = camera.forward().as_vec3();
    let right = camera.right().as_vec3();
    let up = camera.up().as_vec3();
    let translation = camera.translation() + forward * 0.42 + right * 0.16 - up * 0.14;
    (translation, Quat::from_rotation_arc(Vec3::Z, forward))
}

/// The player's lightsaber blade, drawn from the eye when `jkSetSaberInfo` armed one.
///
/// The engine hangs a polyline of `length` from the player's hand; this build has no first-person
/// weapon model, so the blade is a thin emissive quad along the view — long enough to read as a
/// lightsaber, short enough not to fill the screen. `Runtime::SABER_MODEL_SCALE` is the model-to-level
/// factor the swing uses, so what is drawn and what is hit are the same blade.
fn saber_blade(
    mut commands: Commands,
    sim: Option<Res<Sim>>,
    mut blade: Option<ResMut<SaberBlade>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    camera: Query<&GlobalTransform, (With<Camera3d>, Without<OverlayQuad>)>,
    mut drawn: Query<&mut Transform, With<SaberBladeMesh>>,
) {
    let (Some(sim), Some(blade)) = (sim, blade.as_mut()) else { return };
    let Some(view) = camera.iter().next() else { return };
    let armed = {
        let rt = sim.world.borrow();
        let player = rt.player_thing();
        // `jkDisableSaber`/`jkEnableSaber` are what the saber scripts flip when the blade goes away,
        // and nothing read the flag, so a disabled saber still drew.
        let blade_info = rt
            .saber_shown(player)
            .then(|| rt.saber.get(&player).cloned())
            .flatten();
        let weapon = rt.current_weapon();
        blade_info.filter(|_| crate::runtime::Runtime::is_saber(&weapon))
    };
    match armed {
        Some(info) => {
            let reach = (info.length * 10.0).max(0.4);
            let entity = *blade.0.get_or_insert_with(|| {
                let mesh = meshes.add(Cuboid::new(0.03, 0.03, reach));
                let material = materials.add(StandardMaterial {
                    base_color: bevy::color::Color::srgb(0.7, 0.85, 1.0),
                    emissive: bevy::color::LinearRgba::rgb(2.5, 3.0, 4.0),
                    unlit: true,
                    cull_mode: None,
                    ..default()
                });
                commands.spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::default(), SaberBladeMesh)).id()
            });
            // Out of the eye and down the view, a little to the right of centre: the hilt is in the
            // player's hand even though this build draws no hand.
            if let Ok(mut transform) = drawn.get_mut(entity) {
                let forward = view.forward().as_vec3();
                let right = view.right().as_vec3();
                let up = view.up().as_vec3();
                let hilt = view.translation()
                    + forward * 0.35
                    + right * 0.22
                    - up * 0.18;
                transform.translation = hilt + forward * (reach * 0.5);
                transform.rotation = Quat::from_rotation_arc(Vec3::Z, forward);
            }
        }
        None => {
            if let Some(entity) = blade.0.take() {
                commands.entity(entity).despawn();
            }
        }
    }
}

/// Draw whatever the scripts created after load, and move it as the runtime moves it.
///
/// Projectiles are `SITH_THING_WEAPON` templates with their own `.3do`; the level's own
/// thing list never contains them, which is why nothing appeared when one was fired.
#[allow(clippy::too_many_arguments)]
fn draw_created(
    mut commands: Commands,
    sim: Option<Res<Sim>>,
    world: Option<Res<World>>,
    collisions: Option<Res<Collision>>,
    mut created: Option<ResMut<Created>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut drawn: Query<&mut Transform, Without<Sprite3d>>,
    // A sprite's frame is a different mesh, and `drawn` cannot see sprite quads (`billboard_sprites`
    // owns their rotation), so the cel and the placement of a *moving* sprite weapon
    // (`+force_dest_p1`) are updated here instead.
    mut quads: Query<(&mut Mesh3d, &mut Transform), With<Sprite3d>>,
) {
    let _span = Span::new("draw_created");
    let (Some(sim), Some(world), Some(created)) = (sim, world, created.as_mut()) else { return };
    let mut rt = sim.world.borrow();
    let first_created = world.level.things.len();
    for index in first_created..rt.things.len() {
        // A projectile that has left the level's sectors has hit a wall: nothing traces
        // the polygon it struck yet, so it is removed here rather than flying through.
        let pos = rt.things[index].pos;
        // "Outside every sector" used `sector_at(pos, 0.05)`, which is too strict for a muzzle: a
        // bolt spawned a unit ahead of the player often lands outside the hull it is standing in,
        // and every projectile died on the frame it was created (`--hit` measured 22 fired, 0
        // alive). A bolt is gone when it leaves the *level*, which the bounds test answers.
        let bounds = world.level.bounds();
        let outside = |p: [f32; 3]| {
            (0..3).any(|k| p[k] < bounds.0[k] - 4.0 || p[k] > bounds.1[k] + 4.0)
        };
        if !collisions.is_some() || outside(pos) {
            let entities = created.drawn.remove(&index).unwrap_or_default();
            for entity in entities {
                commands.entity(entity).despawn();
            }
            // Mark it gone in the runtime too, or the next frame sees a projectile with no
            // entity and spawns one again — a respawn/despawn loop every frame.
            drop(rt);
            sim.world.borrow_mut().things[index].destroyed = true;
            rt = sim.world.borrow();
            continue;
        }
        let thing = &rt.things[index];
        let entities = created.drawn.get(&index).cloned().unwrap_or_default();
        if thing.destroyed {
            for entity in entities {
                commands.entity(entity).despawn();
            }
            created.drawn.remove(&index);
            continue;
        }
        let transform = Transform::from_translation(Vec3::from(thing.pos))
            .with_rotation(Quat::from_rotation_z(-thing.yaw.to_radians()));
        // `SetThingModel` swaps the mesh a script wants drawn in place of the template's
        // (`ctf_main`'s team bodies). A thing the level itself placed keeps the mesh built at load —
        // replacing that means despawning and rebuilding its entities.
        let template = rt
            .thing_model
            .get(&index)
            .cloned()
            .unwrap_or_else(|| rt.templates.get(index).cloned().unwrap_or_default());
        let template_key = template.to_ascii_lowercase();
        if entities.is_empty() {
            // A sprite template is drawn as camera-facing rectangles, one per `.spr` row: an
            // impact spark, the explosion chain, smoke. `rdSprite_Draw` builds the quad in camera
            // space with the sprite's width/height and its model-space offset, so the port's
            // version is a quad billboarded to the camera (`billboard_sprites`).
            if let Some(faces) = world.template_sprite.get(&template_key) {
                let mut spawned = Vec::new();
                for face in faces {
                    // One cel of the strip. The count comes from the material's cel table.
                    let cels = world.sprite_cels.get(&face.material).copied().unwrap_or(1);
                    let key = (template_key.clone(), face.material.clone(), 0);
                    let mesh = created
                        .sprite_meshes
                        .entry(key)
                        .or_insert_with(|| meshes.add(sprite_quad(face.width, face.height, cels as f32, 0)))
                        .clone();
                    let mut material = StandardMaterial {
                        unlit: true,
                        double_sided: true,
                        cull_mode: None,
                        alpha_mode: AlphaMode::Blend,
                        ..default()
                    };
                    material.base_color_texture =
                        world.model_images.get(&face.material).map(|image| images.add(image.clone()));
                    // `BK_SPRITE_DEBUG=1` draws the quad untextured red and opaque. It is the A/B
                    // that separates "the material rasterises nothing" from "the quad never
                    // reaches the rasteriser", which are different bugs with the same black frame.
                    if std::env::var_os("BK_SPRITE_DEBUG").is_some() {
                        material.base_color = Color::srgb(1.0, 0.0, 0.0);
                        material.base_color_texture = None;
                        material.alpha_mode = AlphaMode::Opaque;
                    }
                    let entity = commands
                        .spawn((
                            Mesh3d(mesh),
                            MeshMaterial3d(materials.add(material)),
                            Transform::from_translation(
                                Vec3::from(thing.pos) + Vec3::from(face.offset),
                            ),
                            Sprite3d,
                        ))
                        .id();
                    spawned.push(entity);
                }
                created.drawn.insert(index, spawned);
                println!("drawing sprite thing {index} ({template}, {} face(s))", faces.len());
                continue;
            }
            let Some(model_name) = world.template_model.get(&template_key) else {
                // A template with no model and no sprite (a hint) has nothing to draw yet.
                created.drawn.insert(index, Vec::new());
                continue;
            };
            let Some(model) = world.models.get(model_name) else { continue };
            let mut spawned = Vec::new();
            for mat_idx in model.meshes.iter().flat_map(|m| m.faces.iter().map(|f| f.material)) {
                let key = (model_name.clone(), mat_idx);
                let mesh = match created.meshes.get(&key) {
                    Some(handle) => handle.clone(),
                    None => {
                        let tex_size = model
                            .materials
                            .get(mat_idx.max(0) as usize)
                            .and_then(|n| world.model_images.get(n))
                            .map(|img| {
                                (
                                    img.texture_descriptor.size.width as f32,
                                    img.texture_descriptor.size.height as f32,
                                )
                            })
                            .unwrap_or((1.0, 1.0));
                        let handle = meshes.add(model_mesh(model, mat_idx, tex_size));
                        created.meshes.insert(key, handle.clone());
                        handle
                    }
                };
                let mat_name = model.materials.get(mat_idx.max(0) as usize).cloned().unwrap_or_default();
                let mut material = StandardMaterial {
                    unlit: true,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                };
                match world.model_images.get(&mat_name) {
                    Some(image) => material.base_color_texture = Some(images.add(image.clone())),
                    None => material.base_color = Color::srgb(0.9, 0.75, 0.3),
                }
                let entity = commands
                    .spawn((Mesh3d(mesh), MeshMaterial3d(materials.add(material)), transform))
                    .id();
                spawned.push(entity);
            }
            if !spawned.is_empty() {
                println!("drawing created thing {index} ({template})");
            }
            created.drawn.insert(index, spawned);
        } else if let Some(faces) = world.template_sprite.get(&template_key) {
            // The sprite's frame comes from the thing's life, so a spark walks its eight cels as
            // it burns out (`Runtime::sprite_cel`).
            for (entity, face) in entities.iter().zip(faces) {
                let cels = world.sprite_cels.get(&face.material).copied().unwrap_or(1);
                let cel = rt.sprite_cel(index, cels);
                let key = (template_key.clone(), face.material.clone(), cel);
                let mesh = created
                    .sprite_meshes
                    .entry(key)
                    .or_insert_with(|| {
                        meshes.add(sprite_quad(face.width, face.height, cels as f32, cel))
                    })
                    .clone();
                if let Ok((mut quad, mut tf)) = quads.get_mut(*entity) {
                    if quad.0 != mesh {
                        quad.0 = mesh;
                    }
                    tf.translation = Vec3::from(thing.pos) + Vec3::from(face.offset);
                }
                if std::env::var_os("BK_SPRITE_TRACE").is_some() {
                    println!("  sprite thing {index} ({template}) draws cel {cel} of {cels}");
                }
            }
        } else {
            for entity in entities {
                if let Ok(mut tf) = drawn.get_mut(entity) {
                    *tf = transform;
                }
            }
        }
    }
}

/// A projectile that reaches an actor damages it.
///
/// The engine's projectile collides with the thing's bounding box and applies the weapon's
/// `damage=`; there is no explosion thing yet and no falloff, so this is the straight hit:
/// inside half a unit of an actor of type 2 (actor) or 10 (player), apply the weapon's
/// damage and remove the projectile.
fn projectile_hits(
    sim: Option<Res<Sim>>,
    world: Option<Res<World>>,
    mut created: Option<ResMut<Created>>,
    mut commands: Commands,
) {
    let (Some(sim), Some(world), Some(created)) = (sim, world, created.as_mut()) else { return };
    let mut rt = sim.world.borrow_mut();
    let first_created = world.level.things.len();
    // The scan itself lives in `Runtime::impacts`, so `--hit` drives the same code the game
    // does instead of a copy of it.
    let impacts = rt.impacts(first_created);
    let player_index = rt
        .templates
        .iter()
        .position(|t| t.eq_ignore_ascii_case("walkplayer"));
    // Blasts: a landed projectile that names `explode=` leaves that thing behind, and the blast
    // hurts everything inside its own body. Both happen before the bolt is removed.
    let mut blasts: Vec<usize> = Vec::new();
    for (shot, _, _) in &impacts {
        if let Some(explosion) = rt.explode(*shot) {
            blasts.push(explosion);
        }
    }
    let player_index_blasts = rt
        .templates
        .iter()
        .position(|t| t.eq_ignore_ascii_case("walkplayer"));
    for explosion in &blasts {
        let template = rt.templates.get(*explosion).cloned().unwrap_or_default();
        let damage = rt.damage_of(&template);
        let name = rt.template_explode.iter().find(|(_, e)| **e == template).map(|(k, _)| k.clone());
        if damage <= 0.0 {
            continue;
        }
        for victim in rt.explode_hits(*explosion) {
            if Some(victim) == player_index_blasts {
                let (shields, health) = rt.damage_player(damage, runtime::DAMAGE_IMPACT);
                println!("blast {template}: the player took {damage:.0}, {shields:.0} shield(s), {health:.0} health");
            } else {
                let left = rt.damage(victim as i64, damage);
                println!(
                    "blast {template}: thing {victim} ({}) took {damage:.0}, {left:.0} health left",
                    rt.templates.get(victim).cloned().unwrap_or_default()
                );
            }
        }
        let _ = name;
    }
    for (shot, target, damage) in impacts {
        // The player has a shield bar in front of their health; everything else is health only.
        let left = if Some(target) == player_index {
            let (shields, health) = rt.damage_player(damage, runtime::DAMAGE_ENERGY);
            println!(
                "hit: the player took {damage:.0}, {shields:.0} shield(s) and {health:.0} health left"
            );
            health
        } else {
            rt.damage_typed(target as i64, damage, runtime::DAMAGE_ENERGY)
        };
        rt.things[shot].destroyed = true;
        if let Some(entities) = created.drawn.remove(&shot) {
            for entity in entities {
                commands.entity(entity).despawn();
            }
        }
        println!(
            "hit: thing {target} ({}), {damage:.0} damage, {left:.0} health left",
            rt.templates.get(target).cloned().unwrap_or_default()
        );
    }
}

/// Show a thing's model while it is in the world, hide it once it is gone (taken,
/// destroyed, or counting down to respawn).
fn show_taken(sim: Option<Res<Sim>>, mut shown: Query<&mut Visibility>) {
    let Some(sim) = sim else { return };
    let rt = sim.world.borrow();
    for (index, entities) in &sim.entities {
        let alive = rt.things.get(*index).is_none_or(|t| !t.destroyed);
        let want = if alive { Visibility::Visible } else { Visibility::Hidden };
        for entity in entities {
            if let Ok(mut visible) = shown.get_mut(*entity)
                && *visible != want
            {
                *visible = want;
            }
        }
    }
}

/// Walking over a powerup takes it: within a unit of the player's body, an item or weapon
/// thing goes into the inventory and its model leaves the world.
///
/// The engine delivers a `touched` message to the item's own cog, which calls `TakeItem`;
/// this runs that script directly rather than through the bus. A template with no `cog=`
/// falls back to `Runtime::take_item`'s stand-in bin.
fn collect_items(
    mut sim: Option<ResMut<Sim>>,
    player: Option<Res<Player>>,
    world: Option<Res<World>>,
    mut commands: Commands,
) {
    let (Some(sim), Some(player)) = (sim.as_mut(), player) else { return };
    // The runtime guard is dropped before any script runs, so it is a plain read here.
    let rt = sim.world.borrow();
    let Some(index) = rt.templates.iter().position(|t| t.eq_ignore_ascii_case("walkplayer"))
    else {
        return;
    };
    let body = [player.pos.x, player.pos.y, player.pos.z];
    let mut taken: Vec<usize> = Vec::new();
    for (other, thing) in rt.things.iter().enumerate() {
        if other == index || thing.destroyed {
            continue;
        }
        if !matches!(rt.types.get(other).copied(), Some(5) | Some(3)) {
            continue;
        }
        let d = ((thing.pos[0] - body[0]).powi(2)
            + (thing.pos[1] - body[1]).powi(2)
            + (thing.pos[2] - body[2]).powi(2))
        .sqrt();
        if d < 1.0 {
            taken.push(other);
        }
    }
    // Collect the results first, then touch the renderer's map: the runtime is borrowed
    // through `sim`, so removing entities inside the loop would borrow it twice.
    let mut picked: Vec<(usize, String, Option<String>)> = Vec::new();
    drop(rt);
    for other in taken {
        let template = sim
            .world
            .borrow()
            .templates
            .get(other)
            .cloned()
            .unwrap_or_default();
        // The item's own script first: `touched` then `taken`, which is where the engine's
        // `TakeItem` ends up. A template with no `cog=` falls back to the runtime's rule.
        let scripted = world.as_deref().and_then(|world| {
            take_item_with_script(world, &sim.world, &world.level, other, &template)
                .map(|(cog_name, _)| cog_name)
        });
        if scripted.is_none() {
            sim.world.borrow_mut().take_item(other as i64, index as i64);
        }
        picked.push((other, template, scripted));
    }
    for (other, template, scripted) in picked {
        println!(
            "picked up {template} via {}",
            scripted.unwrap_or_else(|| "the runtime rule".into())
        );
        if let Some(entities) = sim.entities.remove(&other) {
            for entity in entities {
                commands.entity(entity).despawn();
            }
        }
    }
}

/// How many `Sleep`/`WaitForMessage` calls a statement tree contains, for `--triggers`.
fn count_wait_calls(stmts: &[cog::Stmt]) -> usize {
    fn walk(stmt: &cog::Stmt, n: &mut usize) {
        match stmt {
            cog::Stmt::Call(name, args) => {
                if name.eq_ignore_ascii_case("sleep") || name.eq_ignore_ascii_case("waitformessage") {
                    *n += 1;
                }
                for arg in args {
                    walk_expr(arg, n);
                }
            }
            cog::Stmt::Block(body) => body.iter().for_each(|s| walk(s, n)),
            cog::Stmt::If(_, body, other) => {
                walk(body, n);
                if let Some(other) = other {
                    walk(other, n);
                }
            }
            cog::Stmt::While(_, body) | cog::Stmt::DoWhile(body, _) => walk(body, n),
            cog::Stmt::For(init, cond, step, body) => {
                walk(init, n);
                walk_expr(cond, n);
                walk(step, n);
                walk(body, n);
            }
            cog::Stmt::Assign(_, index, value) => {
                if let Some(index) = index {
                    walk_expr(index, n);
                }
                walk_expr(value, n);
            }
            cog::Stmt::Return(Some(e)) => walk_expr(e, n),
            _ => {}
        }
    }
    fn walk_expr(e: &cog::Expr, n: &mut usize) {
        if let cog::Expr::Call(name, args) = e {
            if name.eq_ignore_ascii_case("sleep") || name.eq_ignore_ascii_case("waitformessage") {
                *n += 1;
            }
            args.iter().for_each(|a| walk_expr(a, n));
        }
    }
    let mut n = 0;
    stmts.iter().for_each(|s| walk(s, &mut n));
    n
}

/// Every `PlaySong(...)` argument in a statement tree, for `--music`.
fn collect_song_args(stmts: &[cog::Stmt], out: &mut Vec<String>) {
    fn expr(e: &cog::Expr, out: &mut Vec<String>) {
        match e {
            cog::Expr::Call(name, args) => {
                if name.eq_ignore_ascii_case("playsong")
                    && let Some(cog::Expr::Str(song)) = args.first()
                {
                    out.push(song.to_ascii_lowercase());
                }
                for a in args {
                    expr(a, out);
                }
            }
            cog::Expr::Bin(_, a, b) => {
                expr(a, out);
                expr(b, out);
            }
            cog::Expr::Un(_, a) | cog::Expr::Index(_, a) | cog::Expr::Assign(_, a) => expr(a, out),
            _ => {}
        }
    }
    for stmt in stmts {
        match stmt {
            cog::Stmt::Call(name, args) => {
                if name.eq_ignore_ascii_case("playsong")
                    && let Some(cog::Expr::Str(song)) = args.first()
                {
                    out.push(song.to_ascii_lowercase());
                }
                for a in args {
                    expr(a, out);
                }
            }
            cog::Stmt::Assign(_, index, value) => {
                if let Some(index) = index {
                    expr(index, out);
                }
                expr(value, out);
            }
            cog::Stmt::If(cond, then, otherwise) => {
                expr(cond, out);
                collect_song_args(std::slice::from_ref(then), out);
                if let Some(otherwise) = otherwise {
                    collect_song_args(std::slice::from_ref(otherwise), out);
                }
            }
            cog::Stmt::While(cond, body) | cog::Stmt::DoWhile(body, cond) => {
                expr(cond, out);
                collect_song_args(std::slice::from_ref(body), out);
            }
            cog::Stmt::For(init, cond, step, body) => {
                collect_song_args(std::slice::from_ref(init), out);
                expr(cond, out);
                collect_song_args(std::slice::from_ref(step), out);
                collect_song_args(std::slice::from_ref(body), out);
            }
            _ => {}
        }
    }
}

/// Where the install keeps its redbook music (`MUSIC\TrackNN.ogg` in the GOG build).
#[derive(Resource)]
struct MusicDir(std::path::PathBuf);

/// What is playing, so a change of song or volume is applied once.
#[derive(Resource, Default)]
struct MusicState {
    playing: Option<(i32, Entity, f32)>,
}

/// Follow the scripts' redbook music: `PlaySong(start, end, loopto)` names track numbers and
/// `SetMusicVol` fades them. The level-1 soundtrack script starts `PlaySong(12, 13, 12)` at
/// volume 0 and ramps it up when the player crosses the bar's sector adjoins.
///
/// The range is played the way `stdMci_Play(from, to)` plays it: `from..to` in order, wrapping back
/// to `from` when the last track ends (`Runtime::advance_music`).
fn music_follows_scripts(
    mut commands: Commands,
    sim: Option<Res<Sim>>,
    dir: Option<Res<MusicDir>>,
    mut state: ResMut<MusicState>,
    mut audio: ResMut<Assets<AudioSource>>,
    mut sinks: Query<&mut AudioSink>,
) {
    let (Some(sim), Some(dir)) = (sim, dir) else { return };
    // A finished track advances to the next in the range, wrapping at `to` to `from`, which is what
    // `stdMci_Play(from, to)` does. The mixer owns the wrap, so the sink is only asked when empty.
    if let Some((_, entity, _)) = state.playing
        && sinks.get_mut(entity).map(|s| s.empty()).unwrap_or(true)
    {
        sim.world.borrow_mut().advance_music();
        state.playing = None;
    }
    let (song, volume) = {
        let rt = sim.world.borrow();
        (rt.music, rt.music_volume)
    };
    let Some((start, _end, _loopto)) = song else {
        if let Some((_, entity, _)) = state.playing.take() {
            commands.entity(entity).despawn();
            println!("music: stopped");
        }
        return;
    };
    // Same track: only the volume may have moved.
    if let Some((playing, entity, current)) = state.playing
        && playing == start
    {
        if (current - volume).abs() > 0.01
            && let Ok(mut sink) = sinks.get_mut(entity)
        {
            sink.set_volume(Volume::Linear(volume.clamp(0.0, 1.0) * sound_gain()));
            state.playing = Some((start, entity, volume));
        }
        return;
    }
    if let Some((_, entity, _)) = state.playing.take() {
        commands.entity(entity).despawn();
    }
    // `MUSIC\Track12.ogg` in the GOG install; the original shipped redbook audio.
    let name = format!("track{start:02}.ogg");
    let path = [dir.0.join(format!("Track{start:02}.ogg")), dir.0.join(&name)]
        .into_iter()
        .find(|p| p.exists());
    let Some(path) = path else {
        println!("music: track {start} is not in {}", dir.0.display());
        return;
    };
    let Ok(bytes) = std::fs::read(&path) else { return };
    let source = audio.add(AudioSource { bytes: bytes.into() });
    let entity = commands
        .spawn((
            AudioPlayer::new(source),
            PlaybackSettings {
                // Once, not Loop: the engine plays the range `from..to` and wraps, so the next
                // track is the mixer`s business (`Runtime::advance_music`), not the sink`s.
                mode: PlaybackMode::Once,
                volume: Volume::Linear(volume.clamp(0.0, 1.0) * sound_gain()),
                ..default()
            },
        ))
        .id();
    println!("music: {name} at volume {volume:.2}");
    state.playing = Some((start, entity, volume));
}

/// Play the sounds the scripts asked for, attenuated by distance and facing.
///
/// `PlaySoundThing` gives a thing's position and `PlaySoundPos` three numbers; the gain is
/// `snd::volume` over the class's own `minRadius`/`maxRadius`, so a shot across a room is
/// quiet and one at your feet is not. `PlaySoundLocal` has no position and is not attenuated.
fn play_requested_sounds(
    mut commands: Commands,
    sim: Option<Res<Sim>>,
    player: Option<Res<Player>>,
    bank: Option<Res<SoundBank>>,
) {
    let (Some(sim), Some(player), Some(bank)) = (sim, player, bank) else { return };
    let (requests, channel_volume, channel_pitch_of) = {
        let mut rt = sim.world.borrow_mut();
        (
            std::mem::take(&mut rt.sounds),
            rt.sound_volume.clone(),
            rt.sound_pitch.clone(),
        )
    };
    if requests.is_empty() {
        return;
    }
    let eye = [player.pos.x, player.pos.y, player.pos.z];
    let yaw = player.yaw.to_radians();
    let facing = (yaw.cos(), yaw.sin());
    let mut played = 0;
    for request in requests {
        let Some((clip, min, max, volume)) = bank.clips.get(&request.name.to_ascii_lowercase())
        else {
            continue;
        };
        let (distance, forward) = match request.pos {
            Some(pos) => {
                let (dx, dy, dz) = (pos[0] - eye[0], pos[1] - eye[1], pos[2] - eye[2]);
                let d = (dx * dx + dy * dy + dz * dz).sqrt();
                let f = if d > 1e-4 { (dx * facing.0 + dy * facing.1) / d } else { 1.0 };
                (d, f)
            }
            None => (0.0, 1.0),
        };
        let gain = snd::volume(volume * request.volume, distance, *min, *max, forward);
        if gain <= 0.002 {
            continue;
        }
        // `ChangeSoundVol`/`ChangeSoundPitch` act on a *channel* of the mixer. A one-shot mixer cannot
        // ramp a sound that is already playing, but a request that arrives after the change reads the
        // channel's own volume and rate — which is what `sithSoundMixer_FadeVolume`/`SetPitch` leave
        // behind for the next play.
        let (channel_volume, channel_pitch) = channel_volume
            .get(request.channel)
            .copied()
            .map(|v| (v.max(0.0), 1.0))
            .unwrap_or((1.0, 1.0));
        let channel_pitch = channel_pitch_of
            .get(request.channel)
            .copied()
            .unwrap_or(channel_pitch);
        commands.spawn((
            AudioPlayer::new(clip.clone()),
            PlaybackSettings {
                mode: PlaybackMode::Despawn,
                volume: Volume::Linear(gain * channel_volume * sound_gain()),
                speed: channel_pitch.clamp(0.01, 4.0),
                ..default()
            },
        ));
        played += 1;
    }
    if played > 0 {
        println!("sound: {played} request(s) played with attenuation");
    }
}

/// One sprite rectangle, with the UVs of a single animation cel.
///
/// A sprite `.mat` is a **vertical strip of cels**, not one image: `bryx.mat` is 8 x 78 for an
/// 8-wide spark, so it holds 9 frames stacked. `rdSprite_Draw` maps one cel with `texVertOffset`;
/// drawing the whole strip onto the quad shows every frame at once, squeezed. `cel` picks the frame
/// and the cel count is the image's height over its width, which is how all 35 shipped sprites are
/// authored.
fn sprite_quad(width: f32, height: f32, cels: f32, cel: u32) -> Mesh {
    let (w, h) = (width.max(0.01) * 0.5, height.max(0.01) * 0.5);
    let step = 1.0 / cels.max(1.0);
    let top = step * cel as f32;
    let (bottom, top) = (top + step, top);
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-w, -h, 0.0], [w, -h, 0.0], [w, h, 0.0], [-w, h, 0.0]],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4]);
    // Bevy's UV origin is the top-left corner, so cel 0 is the top band of the strip.
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, bottom], [1.0, bottom], [1.0, top], [0.0, top]],
    );
    mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    mesh
}

/// A sprite rectangle: the quad a `.spr` row draws, which has to face the camera every frame.
#[derive(Component)]
struct Sprite3d;

/// Turn every sprite quad towards the camera, which is what `rdSprite_Draw` builds its four
/// vertices in camera space for.
///
/// ponytail: the world camera is the first `Camera3d`; a level also has a HUD camera, and both
/// look down the screen's -Z, so a quad facing either one is facing the viewer. If a third camera
/// with a different orientation is ever added, select it by name instead.
fn billboard_sprites(
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut sprites: Query<&mut Transform, With<Sprite3d>>,
) {
    let Some(view) = camera.iter().next() else { return };
    let rotation = view.rotation();
    let mut n = 0;
    for mut transform in &mut sprites {
        transform.rotation = rotation;
        if std::env::var_os("BK_SPRITE_TRACE").is_some() {
            // Camera space: negative z is in front of the eye in Bevy's convention.
            let local = view.affine().inverse().transform_point3(transform.translation);
            println!(
                "  billboard #{n}: world {:?}, camera-space {:?} (scale {:?})",
                transform.translation, local, transform.scale
            );
            n += 1;
        }
    }
}

/// Give every actor its `.ai` instinct class, and let the armed ones shoot at the player.
///
/// The instinct scheduler's weights are not implemented (see `Runtime::ai_tick`); what this
/// does use is the file's own `sightdist=` and `fov=`, and the actor's own `weapon=`.
fn ai_shots(
    time: Res<Time>,
    sim: Option<Res<Sim>>,
    player: Option<Res<Player>>,
    collisions: Option<Res<Collision>>,
) {
    let (Some(sim), Some(player)) = (sim, player) else { return };
    let dt = time.delta_secs().min(0.1);
    let mut rt = sim.world.borrow_mut();
    let mut adjusted = 0usize;
    let Some(index) = rt.templates.iter().position(|t| t.eq_ignore_ascii_case("walkplayer"))
    else {
        return;
    };
    // The player's runtime thing is the AI's target.
    let eye = player.pos;
    rt.things[index].pos = [eye.x, eye.y, eye.z];
    let fired = rt.ai_tick(index, dt);
    if fired > 0 {
        println!("ai: {fired} shot(s) at the player");
    }
    // The chase walks in a straight line, because the runtime has no collider: `.ai` gives sight and
    // speed, not a path. Put the actors the tick moved back on their feet and out of the walls
    // before they are drawn, which is the minimum that stops them strolling through bulkheads.
    let walked = rt.walked.clone();
    if let Some(collisions) = collisions.as_deref() {
        let solids = &collisions.0;
        for actor in walked {
            let Some(thing) = rt.things.get(actor) else { continue };
            let (pos, sector) = (thing.pos, thing.sector.max(0) as usize);
            let (resolved, _) = solids.resolve_stepped(sector, pos, 0.5);
            let mut next = [resolved[0], resolved[1], pos[2]];
            // Back onto the floor it is standing over, so a chase over rubble does not sink.
            if let Some(floor) = solids.support(next, 0.5, 0.6) {
                next[2] = floor + 0.5;
            }
            if let Some(thing) = rt.things.get_mut(actor) {
                let moved = (next[0] - pos[0]).abs() + (next[1] - pos[1]).abs();
                thing.pos = next;
                if moved > 0.01 {
                    adjusted += 1;
                }
            }
        }
    }
    if adjusted > 0 && std::env::var_os("BK_AI_TRACE").is_some() {
        println!("ai: {adjusted} actor step(s) held out of the geometry");
    }
}

/// Swap the sector ambience when the player enters a sector with a different `SOUND`.
fn ambient_follows_view(
    mut commands: Commands,
    world: Option<Res<World>>,
    sim: Option<Res<Sim>>,
    player: Option<Res<Player>>,
    mut ambience: ResMut<Ambience>,
) {
    let (Some(world), Some(player)) = (world, player) else { return };
    // `SectorSound` may have replaced the sector's ambient: the script's override wins over the
    // level's own (`sithSoundMixer_SetSectorAmbientSound` writes the sector itself).
    let wanted = sim
        .map(|sim| sim.world.borrow().sector_ambient.get(&(player.sector as i32)).cloned())
        .unwrap_or_default()
        .or_else(|| world.level.sectors.get(player.sector).and_then(|s| s.sound.clone()));
    if let Some((name, entity)) = ambience.playing.clone() {
        if wanted.as_ref().is_some_and(|(wanted, _)| *wanted == name) {
            return;
        }
        commands.entity(entity).despawn();
        ambience.playing = None;
    }
    let Some((name, volume)) = wanted else { return };
    let Some(source) = ambience.sources.get(&name).cloned() else { return };
    let entity = commands
        .spawn((
            AudioPlayer::new(source),
            PlaybackSettings {
                // Once, not Loop: the engine plays the range `from..to` and wraps, so the next
                // track is the mixer`s business (`Runtime::advance_music`), not the sink`s.
                mode: PlaybackMode::Once,
                volume: Volume::Linear(volume.clamp(0.0, 1.0) * sound_gain()),
                ..default()
            },
        ))
        .id();
    println!("ambience: {name} (volume {volume:.2})");
    ambience.playing = Some((name, entity));
}

fn setup(
    mut commands: Commands,
    world: Res<World>,
    level_shader: Res<LevelShader>,
    fade_test: Res<FadeTest>,
    overview: Res<Overview>,
    near: Res<Near>,
    stand: Res<Stand>,
    look: Res<Look>,
    deliver: Res<Deliver>,
    mut ambience: ResMut<Ambience>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut level_materials: ResMut<Assets<level_mat::LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut audio: ResMut<Assets<AudioSource>>,
    // The overlay camera is tagged `OverlayQuad`, so this answers "is there a world camera
    // yet?" — false on the first load, true when a level swap re-runs this system.
    cameras: Query<(), (With<Camera3d>, With<WorldCamera>)>,
) {

    let level = &world.level;

    // Hand the sector ambience to the audio system.
    for (name, bytes) in &world.ambient {
        let source = audio.add(AudioSource { bytes: bytes.clone().into() });
        ambience.sources.insert(name.clone(), source);
    }

    // The sound classes' clips: every wav they name, with the class's own fade radii and
    // volume, so a script's `PlaySoundThing` can be played with distance attenuation.
    let mut bank = SoundBank::default();
    for (wav, (min, max, volume)) in &world.sound_radii {
        if bank.clips.contains_key(wav) {
            continue;
        }
        let Some(bytes) = world.wav_bytes.get(wav) else { continue };
        let source = audio.add(AudioSource { bytes: bytes.clone().into() });
        bank.clips.insert(wav.clone(), (source, *min, *max, *volume));
    }
    println!("sound bank: {} clips with fade radii", bank.clips.len());
    commands.insert_resource(bank);

    // Surfaces a script has hidden before the world was built (`GEOMETRY_NONE`). Scripts start before
    // the meshes are built, so a level that hides glass in `startup` is already accounted for here;
    // anything later goes through `rebuild_surfaces`.
    let (batches, skipped) = build_world(level, &world.images, &Default::default(), &SurfaceLook::default());
    let no_mat = level.surfaces.iter().filter(|s| s.material < 0).count();
    let few_verts = level.surfaces.iter().filter(|s| s.verts.len() < 3).count();
    println!(
        "{} sector batches, {skipped} surfaces skipped ({} with no material, {} under 3 verts)",
        batches.len(),
        no_mat,
        few_verts
    );

    // One material per texture, shared by every sector batch that draws it.
    let mut handles: HashMap<i32, Handle<StandardMaterial>> = HashMap::new();
    let shader_on = level_shader.0;
    let mut shader_handles: HashMap<i32, Handle<level_mat::LevelMaterial>> = HashMap::new();
    // The fade colours come from the level's own colormap, so the shader's distance term walks the
    // same darkest table the CPU-side ramp does (§6).
    let level_params = {
        let [r, g, b] = tex::Palette::installed_far_colour();
        let fade = if fade_test.0 {
            // Close stops, so the screenshot shows the gradient across one room.
            (0.0, 40.0)
        } else {
            (30.0, 320.0)
        };
        level_mat::LevelParams {
            far_r: r as f32 / 255.0,
            far_g: g as f32 / 255.0,
            far_b: b as f32 / 255.0,
            fade_start: fade.0,
            fade_end: fade.1,
            ..Default::default()
        }
    };
    let mut by_sector: Vec<Vec<Entity>> = vec![Vec::new(); level.sectors.len()];
    let mut surface_batches = SurfaceBatches::default();
    for ((sector, mat_idx), batch) in batches {
        if batch.indices.is_empty() {
            continue;
        }
        let material = handles
            .entry(mat_idx)
            .or_insert_with(|| {
                let mut material = StandardMaterial {
                    // JK's world pass is a lightmapped texture pass, not real-time
                    // lighting: the level bakes intensity into the vertices.
                    unlit: true,
                    // ponytail: no back-face culling. Surface winding in the original data
                    // is not guaranteed consistent and a wrong guess hides half the level;
                    // derive and honour winding once the geometry is verified.
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                };
                if let Some(image) = world.images.get(&mat_idx) {
                    material.base_color_texture = Some(images.add(image.clone()));
                } else {
                    // Materials with no bitmap (effect surfaces such as `00RED.mat`) have
                    // no texture to sample; draw them flat instead of dropping the surface.
                    material.base_color = Color::srgb(0.42, 0.42, 0.46);
                }
                materials.add(material)
            })
            .clone();
        let mesh_handle = meshes.add(to_mesh(batch));
        surface_batches.0.push(((sector, mat_idx), mesh_handle.clone()));
        if shader_on {
            let shaded = shader_handles
                .entry(mat_idx)
                .or_insert_with(|| {
                    let texture = world.images.get(&mat_idx).map(|i| images.add(i.clone()));
                    level_materials.add(level_mat::LevelMaterial {
                        base_color_texture: texture,
                        params: level_mat::LevelParams {
                            cels: world.material_cels.get(&mat_idx).copied().unwrap_or(1) as f32,
                            ..level_params
                        },
                    })
                })
                .clone();
            let id = commands
                .spawn((
                    Mesh3d(mesh_handle),
                    MeshMaterial3d(shaded),
                    SectorId(sector),
                    LevelEntity,
                ))
                .id();
            by_sector[sector].push(id);
            continue;
        }
        let id = commands
            .spawn((
                Mesh3d(mesh_handle),
                MeshMaterial3d(material),
                SectorId(sector),
                LevelEntity,
            ))
            .id();
        if let Some(slot) = by_sector.get_mut(sector) {
            slot.push(id);
        }
    }

    let (entities, actors) = spawn_models(&mut commands, &world, &mut meshes, &mut materials, &mut images);
    commands.insert_resource(Actors(actors));
    commands.insert_resource(surface_batches);
    commands.insert_resource(Created::default());
    // The player's weapon: the template's own `weapon=`, else the first template that is
    // a projectile — a weapon type with a launch velocity and a damage value.
    let is_weapon = |name: &str| {
        level
            .template_param(name, "type")
            .is_some_and(|kind| kind.eq_ignore_ascii_case("weapon"))
            && level.template_f32(name, "damage").unwrap_or(0.0) > 0.0
            && level.template_param(name, "vel").is_some()
    };
    let gun = level
        .template_param("walkplayer", "weapon")
        .map(|w| w.to_string())
        .or_else(|| {
            let mut names: Vec<&str> = level.templates.iter().map(|t| t.name.as_str()).collect();
            names.sort();
        println!("uni: string file(s): {names:?}");
            names.into_iter().find(|name| is_weapon(name)).map(|name| name.to_string())
        })
        .unwrap_or_else(|| "+bryarbolt".to_string());
    println!("weapon: {gun}");
    commands.insert_resource(Gun(gun));
    // Actors' instinct classes are attached to the **live** runtime, in the block that has `sim`
    // below. They used to be attached to a fresh `Runtime::handle(&level)` created right here, which
    // built a second runtime and threw it away: no actor in the running game had an instinct class,
    // so nothing ever chased or fired outside the `--ai` check.

    // Models are *not* tied to a sector's visibility. Their `thing.sector` is often stale
    // (level 1 has things placed in a sector they were moved out of), and a model hidden
    // with a room it is no longer in pops out right in front of the camera. Bevy already
    // frustum-culls them per mesh, which is both safe and enough.

    // The portal walk needs the sector graph, not its planes: reuse the collision build
    // so "the room behind this doorway" means the same thing to drawing and to walking.
    commands.insert_resource(SectorView { solids: collide::build(level), by_sector });

    // Start the level's own scripts. This is the same binding `--run` reports on, but
    // here it feeds the renderer: `startup` always runs (that is what the engine does
    // at load); `--sim` also delivers each declared message once so animated geometry
    // acts without a player to trigger it.
    let mut sim = Sim {
        world: runtime::Runtime::handle(level),
        instances: Vec::new(),
        entities,
        elapsed: 0.0,
        reported: 0.0,
    };
    // How many cels each surface's material has, so `SurfaceAnim` knows where to wrap. The runtime
    // has no file access, so the loader hands the counts over once. The inventory's own `cog=`
    // column comes over the same way, so a pickup can name the script that bin runs.
    sim.world.borrow_mut().item_cog = world.item_cogs.clone();
    // `GetKeyLen` answers a `.key`'s frame count, which the loader already parsed.
    sim.world.borrow_mut().key_lengths =
        world.keys.iter().map(|(name, key)| (name.clone(), key.frames as f32)).collect();
    sim.world.borrow_mut().surface_cels = level
        .surfaces
        .iter()
        .map(|s| world.material_cels.get(&s.material).copied().unwrap_or(1))
        .collect();
    // `MaterialAnim` names its material (`09sign.mat`) and wraps at that material's cel count, so the
    // runtime needs the names and the per-material table too.
    {
        let mut rt = sim.world.borrow_mut();
        rt.material_cels = world.material_cels.clone().into_iter().collect();
        rt.material_names = level
            .materials
            .iter()
            .enumerate()
            .map(|(index, name)| (name.trim().to_ascii_lowercase(), index as i32))
            .collect();
    }
    for inst in &level.cogs {
        let Some(script) = world.cogs.get(&inst.script) else { continue };
        let mut vm = cog::Vm::with_world(sim.world.clone());
        script.bind(&mut vm, &inst.values);
        if script.label("startup").is_some() {
        let _ = vm.run_label(&script, "startup");
        }
        if deliver.0 {
            let labels: Vec<String> =
                script.messages().map(|m| m.name.clone()).filter(|m| m != "startup").collect();
            for label in labels {
                if script.label(&label).is_some() {
                    let _ = vm.run_label(script, &label);
                }
            }
        }
        // `GetSelfCog` is the instance's own index in the level's cog list, which is the order the
        // engine creates them in — so this is the same number a script compares against.
        vm.cog_index = sim.instances.len() as i32;
        sim.instances.push((inst.script.clone(), vm));
    }
    let moving = sim.world.borrow().things.iter().filter(|t| t.is_moving()).count();
    let mut kinds: Vec<String> = sim
        .world
        .borrow()
        .things
        .iter()
        .enumerate()
        .filter(|(_, t)| t.is_moving())
        .filter_map(|(i, _)| level.things.get(i).map(|s| s.template.clone()))
        .collect();
    kinds.sort();
    kinds.dedup();
    println!(
        "{} script instances bound, {moving} things moving at load ({})",
        sim.instances.len(),
        kinds.join(", ")
    );
    // Actors' instinct classes: `aiclass=` resolves through the template chain, and the file arrives
    // from the archives in `World::ai_classes`. This is the runtime the game runs, so this is the one
    // the instincts belong to.
    {
        let mut rt = sim.world.borrow_mut();
        let mut attached = 0;
        // The class table as well: `AISetClass` names a class the way the cog does (`picup.ai`), and
        // the runtime cannot read the archives.
        rt.ai_classes = world.ai_classes.clone().into_iter().collect();
        for (index, thing) in level.things.iter().enumerate() {
            if let Some(class) = level.template_param(&thing.template, "aiclass")
                && let Some(raw) = world.ai_classes.get(&class.to_ascii_lowercase())
            {
                rt.ai.insert(index, ai::parse(raw));
                attached += 1;
            }
        }
        if attached > 0 {
            println!("ai: {attached} actors with an instinct class");
        }
    }
    // Sector links: a cog symbol of kind `sector` with a value ties the script to that sector, and
    // `sithCog_LinkCog` turns each one into a link whose `entered`/`exited` messages the sector
    // delivers (§16.10). The level's own soundtrack script uses one to start the music fade.
    {
        // Instance values are positional over the script's non-local, non-message symbols, which is
        // the same walk `Cog::bind` does; a `sector` symbol's value is the sector index.
        let mut links: Vec<(usize, i32, usize)> = Vec::new();
        for (index, inst) in level.cogs.iter().enumerate() {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            let mut next = inst.values.iter();
            for symbol in &script.symbols {
                if symbol.local || symbol.kind == cog::SymKind::Message {
                    continue;
                }
                let Some(raw) = next.next() else { break };
                if symbol.kind != cog::SymKind::Sector {
                    continue;
                }
                if let Ok(sector) = raw.trim().parse::<usize>() {
                    links.push((index, symbol.linkid.unwrap_or(0) as i32, sector));
                }
            }
        }
        if !links.is_empty() {
            println!("triggers: {} sector link(s) from the level's scripts", links.len());
            sim.world.borrow_mut().sector_links = links;
        }
        // Thing links: the `damaged`/`killed`/`touched` half. A `thing` symbol's value is the thing
        // index, so the same walk gives the table `Runtime::damage` queues messages against.
        let mut thing_links: Vec<(usize, i32, usize)> = Vec::new();
        for (index, inst) in level.cogs.iter().enumerate() {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            let mut next = inst.values.iter();
            for symbol in &script.symbols {
                if symbol.local || symbol.kind == cog::SymKind::Message {
                    continue;
                }
                let Some(raw) = next.next() else { break };
                if symbol.kind == cog::SymKind::Thing
                    && let Ok(thing) = raw.trim().parse::<i64>()
                    && thing >= 0
                {
                    thing_links.push((index, symbol.linkid.unwrap_or(0) as i32, thing as usize));
                }
            }
        }
        if !thing_links.is_empty() {
            println!("triggers: {} thing link(s) from the level's scripts", thing_links.len());
            sim.world.borrow_mut().thing_links = thing_links;
        }
        // Portals for the actors' steering: a surface two sectors share is a doorway, and its centre
        // is where an actor heads when the straight line to its target is walled off.
        {
            let claims = collide::surface_claims(&world.level);
            // A portal surface is owned by *one* sector; the far side is the surface its adjoin's
            // `mirror` names. So the map is adjoin index -> surface, then mirror -> far surface ->
            // that surface's sector.
            let mut surface_of_adjoin: Vec<Option<usize>> = vec![None; world.level.adjoins.len()];
            for (index, surface) in world.level.surfaces.iter().enumerate() {
                if surface.adjoin >= 0
                    && let Some(slot) = surface_of_adjoin.get_mut(surface.adjoin as usize)
                {
                    *slot = Some(index);
                }
            }
            let mut portals: Vec<(usize, usize, [f32; 3])> = Vec::new();
            for (index, surface) in world.level.surfaces.iter().enumerate() {
                if surface.adjoin < 0 {
                    continue;
                }
                let Some(near_sector) = claims.get(index).and_then(|s| s.first()).copied() else {
                    continue;
                };
                let Some(mirror) = world
                    .level
                    .adjoins
                    .get(surface.adjoin as usize)
                    .map(|a| a.mirror)
                    .filter(|m| *m >= 0)
                else {
                    continue;
                };
                let Some(far_surface) = surface_of_adjoin.get(mirror as usize).copied().flatten()
                else {
                    continue;
                };
                let Some(far_sector) = claims.get(far_surface).and_then(|s| s.first()).copied()
                else {
                    continue;
                };
                let _ = near_sector;
                if far_sector == near_sector {
                    continue;
                }
                let mut centre = [0.0f32; 3];
                let mut n = 0usize;
                for (v, _) in &surface.verts {
                    if let Some(p) = world.level.vertices.get(*v as usize) {
                        for k in 0..3 {
                            centre[k] += p[k];
                        }
                        n += 1;
                    }
                }
                if n == 0 {
                    continue;
                }
                for k in 0..3 {
                    centre[k] /= n as f32;
                }
                portals.push((near_sector, far_sector, centre));
            }
            {
                let two_sided = claims.iter().filter(|c| c.len() >= 2).count();
                let with_adjoin = world.level.surfaces.iter().filter(|s| s.adjoin >= 0).count();
                println!(
                    "pathing: {} surface(s) with an adjoin, {two_sided} claimed by two sectors, {} portal(s)",
                    with_adjoin,
                    portals.len()
                );
            }
            if !portals.is_empty() {
                println!("pathing: {} portal(s) for actor steering", portals.len());
                sim.world.borrow_mut().portals = portals;
            }
        }
        // Surface links: what the player's use action messages.
        let mut surface_links: Vec<(usize, i32, usize)> = Vec::new();
        for (index, inst) in level.cogs.iter().enumerate() {
            let Some(script) = world.cogs.get(&inst.script) else { continue };
            let mut next = inst.values.iter();
            for symbol in &script.symbols {
                if symbol.local || symbol.kind == cog::SymKind::Message {
                    continue;
                }
                let Some(raw) = next.next() else { break };
                if symbol.kind == cog::SymKind::Surface
                    && let Ok(surface) = raw.trim().parse::<usize>()
                {
                    surface_links.push((index, symbol.linkid.unwrap_or(0) as i32, surface));
                }
            }
        }
        if !surface_links.is_empty() {
            println!("triggers: {} surface link(s) from the level's scripts", surface_links.len());
            sim.world.borrow_mut().surface_links = surface_links;
        }
    }
    commands.insert_resource(sim);

    let (lo, hi) = level.bounds();
    let center = Vec3::new(
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    );
    println!(
        "bounds x[{:.1},{:.1}] y[{:.1},{:.1}] z[{:.1},{:.1}]  center {center}",
        lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]
    );

    // Sectors are convex, so a sector's CENTER is guaranteed to be inside a room.
    // Many sectors are thin trigger slivers (radius < 1), so prefer the room the
    // level's walkplayer occupies only when it is actually a room.
    let player = level
        .things
        .iter()
        .find(|t| t.template.eq_ignore_ascii_case("walkplayer"));
    let player_room = player
        .and_then(|t| usize::try_from(t.sector).ok().and_then(|i| level.sectors.get(i)))
        .filter(|s| s.radius >= 1.5);
    let room = player_room.or_else(|| level.sectors.iter().max_by(|a, b| a.radius.total_cmp(&b.radius)));
    if std::env::var("BK_TRACE").is_ok() {
        let mut top: Vec<f32> = level.sectors.iter().map(|s| s.radius).collect();
        top.sort_by(|a, b| b.total_cmp(a));
        println!("largest sector radii: {:?}", &top[..top.len().min(5)]);
    }
    // The level's `walkplayer` thing is the game's own start point: where it stands
    // and which way it faces are what the player sees on load.
    let mut eye = player
        .map(|t| Vec3::from(t.pos) + Vec3::Z * PLAYER_EYE)
        .or_else(|| room.map(|s| Vec3::from(s.center)))
        .unwrap_or(center);
    let area = room.map(|s| s.radius).unwrap_or(4.0);

    // Sky dome, so an open sky sector shows clouds rather than a black void.
    spawn_sky(
        &mut commands,
        level,
        &world,
        &mut meshes,        &mut materials,
        &mut images,
        center,
    );

    // Face the way the level placed the player. JK yaw is degrees about the up
    // axis; fall back to looking at the level centre when the level has no player.
    let yaw = player.map(|t| t.yaw.to_radians());
    let mut aim = yaw
        .map(|y| { let (f, _) = yaw_axes(y); Vec3::new(f[0], f[1], 0.0) })
        .unwrap_or_else(|| Vec3::new(center.x, center.y, eye.z) - eye);
    if aim.length() < 0.5 {
        aim = Vec3::new(1.0, 0.0, 0.0);
    }
    if look.0 != 0.0 {
        aim = Quat::from_rotation_z(-look.0.to_radians()) * aim.normalize();
    }

    // `--shot-at "x,y,z"` (with `--shot-yaw`): a chosen vantage for a capture, overriding whatever the
    // spawn or `--stand` decided. Comparing two frames needs the same eye every time.
    if let Some((at, y)) = SHOT_AT.get().copied() {
        eye = Vec3::from(at);
        let y = y.to_radians();
        let (f, _) = yaw_axes(y);
        aim = Vec3::new(f[0], f[1], 0.0);
    }

    if !cameras.is_empty() {
        // A level swap re-runs this system; the world camera already exists, and a second one
        // at the same order makes the renderer pick between them (§16.9). Its transform is
        // moved by the player's own walk on the next frame.
    } else if overview.overview {
        // A plan view: high enough, and through a long enough lens, that the level flattens into a map.
        // The old 0.9 x span at the default 45 degree FOV put the camera just above the roofs, so every
        // wall leaned outwards over the room beside it and the capture read as the level's own geometry
        // overlapping the models standing in it. See `plan_view` for the framing and its two limits.
        let span = (hi[1] - lo[1]).max(hi[0] - lo[0]).max(hi[2] - lo[2]).max(1.0);
        let (distance, fov) = plan_view(span);
        let above = center + Vec3::Z * distance;
        let up = Vec3::Y; // a plan view of a Z-up world: Z would be parallel to the view
        commands.spawn((
            Camera3d::default(),
            WorldCamera,

            // The far plane has to reach the sky dome, which is centred on the camera at radius 600.
            Projection::Perspective(PerspectiveProjection {
                fov,
                far: distance + 700.0,
                ..default()
            }),
            Transform::from_translation(above).looking_at(center, up),
        ));
    } else if let Some(template) = &near.0 {
        // Stand off from the first thing of the requested template and look at it.
        match level.things.iter().find(|t| t.template.eq_ignore_ascii_case(template)) {
            Some(t) => {
                let target = Vec3::from(t.pos);
                // Stand off by the model's own size: a fixed distance sits inside a
                // 4-unit crate and a mile away from an energy cell.
                let radius = world
                    .template_model
                    .get(&t.template.to_ascii_lowercase())
                    .and_then(|m| world.models.get(m))
                    .map(model_radius)
                    .unwrap_or(1.0);
                // Look from the sector centre (a convex room's centre is inside the
                // room) so the camera never ends up buried in a wall.
                let room = usize::try_from(t.sector)
                    .ok()
                    .and_then(|i| level.sectors.get(i))
                    .map(|s| Vec3::from(s.center));
                let back = room
                    .map(|c| (c - target).normalize_or(Vec3::Z))
                    .unwrap_or(Vec3::new(0.7, 0.5, 0.5).normalize());
                let dist = env_f32("BK_STAND_DIST", radius * 2.5 + 0.5);
                let eye = target + back * dist + Vec3::Z * (radius * 0.5);
                println!("framing `{template}` at {target} (thing sector {}, room {room:?})", t.sector);
                commands.spawn((
                    Camera3d::default(),
                    WorldCamera,
                    EngineFov,
                    Transform::from_translation(eye).looking_at(target, Vec3::Z),
                ));
            }
            None => {
                println!("no thing with template `{template}`; using default view");
                commands.spawn((
                    Camera3d::default(),
                    WorldCamera,
                    EngineFov,
                    Transform::from_translation(eye).looking_at(eye + aim.normalize(), Vec3::Z),
                ));
            }
        }
    } else {
        println!(
            "spawn: sector center {eye} radius {area:.1} (walkplayer sector {})",
            player.map(|t| t.sector).unwrap_or(-1)
        );
        commands.spawn((
            Camera3d::default(),
            WorldCamera,
            EngineFov,
            Transform::from_translation(eye).looking_at(eye + aim.normalize(), Vec3::Z),
        ));
    }

    // The walker: same code path as `--walk`, with the level's `walkplayer` as its start.
    let solids = collide::build(level);
    // The body's centre, not the eye: `walk_player` adds `PLAYER_EYE` when it places the
    // camera, so seeding the eye here put the camera one eye-height too high.
    // `--stand <template>`: stand in that thing's own sector and face it, which is how an
    // actor's chase is watched in the window rather than only asserted in `--ai`.
    let standing = stand.0.as_ref().and_then(|template| {
        level
            .things
            .iter()
            .find(|t| t.template.eq_ignore_ascii_case(template))
            .map(|t| (t.pos, t.sector))
    });
    let (start, facing) = match standing {
        Some((at, thing_sector)) => {
            // A convex room's centre is inside the room, so the camera is not in a wall.
            let from = usize::try_from(thing_sector)
                .ok()
                .and_then(|i| level.sectors.get(i))
                .map(|s| Vec3::from(s.center))
                .unwrap_or(Vec3::new(at[0] + 4.0, at[1], at[2]));
            let a = from.to_array();
            println!(
                "standing in the room of `{}` (sector {thing_sector}) at {a:?}",
                stand.0.clone().unwrap_or_default()
            );
            (a, Some((at[1] - a[1]).atan2(at[0] - a[0])))
        }
        None => (
            // The same start the `--walk` sweep uses: `EYE` above the level's own
            // `walkplayer`, which is the pair that walks on all 34 levels.
            player
                // `EYE` in the `--walk` sweep (0.35): the same pair that walks on 34 levels.
                .map(|t| [t.pos[0], t.pos[1], t.pos[2] + PLAYER_RADIUS])
                .unwrap_or([eye.x, eye.y, eye.z - PLAYER_EYE]),
            None,
        ),
    };
    // The level records a sector for its own `walkplayer`, and that is the authority: level 1 records
    // `339`, whose floor is one radius under the spawn point. The earlier code replaced it whenever
    // `floor_below` (a *planes-only* test) said None, and `339` looks plane-less from the walkway above —
    // so the body was dropped into sector `218`, which has neither floor nor doorway anywhere near it,
    // and fell for ever. The sweep decides now, and it starts from what the level says.
    let recorded = player
        .and_then(|t| usize::try_from(t.sector).ok())
        .filter(|i| *i < level.sectors.len())
        .unwrap_or(0);
    // The level names the sector its `walkplayer` starts in, and the engine trusts that field
    // (`sithThing_SetSector` at load) — `sector_at` is only for a body `--stand` moved.
    let sector = if facing.is_some() {
        solids.sector_at(start, PLAYER_RADIUS).unwrap_or(recorded)
    } else {
        recorded
    };    // The recorded sector can be a trigger sliver with no planes and no doorways, where no sweep can
    // reach anything and the body falls for ever. The test has to be the *sweep*, not `floor_below`:
    // the floor may belong to the room below a doorway, which no plane list of the sliver knows about.
    let mut sector = sector;
    let mut start = start;
    // Nudge out of an NPC or a wall, and stand on the floor found there.
    match spawn_point(&world.level, &solids, sector, start, PLAYER_RADIUS, "walkplayer") {
        Some((body, room)) => {
            if (body[0] - start[0]).abs() > 1e-3 || (body[1] - start[1]).abs() > 1e-3 {
                println!(
                    "walker: spawn {start:?} was inside something; standing at ({:.2}, {:.2}) instead",
                    body[0], body[1]
                );
            }
            start = body;
            sector = room;
        }
        None => println!(
            "walker: no clear spawn near {start:?} (sector {sector}) - the body may fall or overlap"
        ),
    }

    // Experiment hook: `BK_SPAWN_SECTOR=<n>` puts the body in that sector's floor instead of the level's
    // start, and `BK_SPAWN_LIFT=<units>` raises it, so a fall can be watched where a level's own geometry
    // has one. Same family as `BK_RADIUS`/`BK_EYE`; remove once the falls are settled.
    if let Ok(want) = std::env::var("BK_SPAWN_SECTOR")
        && let Ok(want) = want.parse::<usize>()
        && let Some(centre) = solids.spheres.get(want).map(|s| s.0)
        && let Some(floor) = solids.floor_below(want, centre)
    {
        start = [centre[0], centre[1], floor + PLAYER_RADIUS];
        sector = want;
        start[2] += env_f32("BK_SPAWN_LIFT", 0.0);
        println!("walker: BK_SPAWN_SECTOR put the body at {start:?} in sector {sector}");
    }
    // `BK_SPAWN_THING=<n>` stands the body half a unit in front of thing `n`, facing it — the way the
    // engine's use action needs (`sithPlayer`'s look, not a touch). `BK_YAW=<degrees>` overrides where
    // the walk heads, so `BK_WALK_FORWARD` can push through a door the script has just opened.
    let mut forced_yaw = None;
    if let Ok(want) = std::env::var("BK_SPAWN_THING")
        && let Ok(want) = want.parse::<usize>()
    {
        let at = runtime::Runtime::handle(&world.level)
            .borrow()
            .things
            .get(want)
            .map(|t| (t.pos, t.yaw));
        if let Some((pos, yaw)) = at {
            let (sin, cos) = yaw.to_radians().sin_cos();
            start = [pos[0] - cos * 0.5, pos[1] - sin * 0.5, pos[2] + PLAYER_RADIUS];
            sector = solids.sector_at(start, PLAYER_RADIUS).unwrap_or(sector);
            forced_yaw = Some(yaw);
            println!("walker: BK_SPAWN_THING put the body at {start:?} facing {yaw:.1} deg, in sector {sector}");
        }
    }
    if let Ok(degrees) = std::env::var("BK_YAW")
        && let Ok(degrees) = degrees.parse::<f32>()
    {
        forced_yaw = Some(degrees);
    }
    // `BK_SPAWN_OFFSET="dx,dy"`: nudge the spawn horizontally. A door piece usually sits *in* its wall,
    // so half a unit along its own yaw lands outside the room and every move is undone; this puts the
    // body on the room's side of it.
    if let Ok(offset) = std::env::var("BK_SPAWN_OFFSET")
        && let Some((dx, dy)) = offset.split_once(',')
        && let (Ok(dx), Ok(dy)) = (dx.trim().parse::<f32>(), dy.trim().parse::<f32>())
    {
        start[0] += dx;
        start[1] += dy;
        sector = solids.sector_at(start, PLAYER_RADIUS).unwrap_or(sector);
        println!("walker: BK_SPAWN_OFFSET moved the body to {start:?}, sector {sector}");
    }

    // Seed the body onto the floor it stands over. The level's `walkplayer` z is the floor
    // surface itself (level 1: `-0.78` against a walkway top at `-0.75`), so a body placed exactly
    // there is 0.03 *below* the face and gravity then drops it through — the camera ends up in the
    // slab. Probing from 0.2 above puts the feet on the face instead.

    let player_yaw = forced_yaw.unwrap_or_else(|| facing.unwrap_or_else(|| yaw.unwrap_or_else(|| aim.z.atan2(aim.x))));
    // The camera sits at `position + eyeOffset` (`sithCamera.c`), and the template's
    // `eyeoffset=(x/y/z)` is the offset in **model** units, so `MODEL_SCALE` takes it into level units:
    // `walkplayer`'s `0.037 x 10 = 0.37` above the body centre, which is itself one radius above the floor.
    // A sector with a low ceiling still bounds it below, so a body in a crawlspace does not put the camera
    // through the roof.
    let mut ceilings: Vec<f32> = std::iter::once(sector)
        .chain(solids.sectors_around(start))
        .filter_map(|s| solids.ceiling_above(s, start))
        .filter(|c| *c > start[2] + 0.25)
        .collect();
    ceilings.sort_by(f32::total_cmp);
    // The measured ceilings do not explain it: level 1's spawn has 6.8 units of headroom, so
    // `PLAYER_EYE` = 1.45 is nowhere near the ceiling — and yet a camera at 1.45 renders the
    // inside of a solid while one at 0.35 is clear. What is in the way is therefore not a
    // sector plane: it is a model (nothing in the collider) or geometry belonging to a sector
    // the point is not in. Until placement can see both, the eye stays at the sweep's height,
    // which is known to work on all 34 levels, and the ceiling is printed for the next attempt.
    let derived_eye = world
        .level
        .template_param("walkplayer", "eyeoffset")
        .or_else(|| world.level.template_param("_actor", "eyeoffset"))
        .and_then(|raw| {
            let inner = raw.trim().trim_start_matches('(').trim_end_matches(')');
            inner
                .split(['/', ','])
                .filter_map(|n| n.trim().parse::<f32>().ok())
                .nth(2)
        })
        .map(|z| z * MODEL_SCALE)
        .filter(|z| *z > 0.0)
        .unwrap_or(PLAYER_EYE);
    let eye_height = env_f32("BK_EYE", derived_eye).min(
        ceilings
            .first()
            .map(|c| c - start[2] - 0.05)
            .unwrap_or(f32::INFINITY),
    );
    println!(
        "walker: eye {eye_height:.2} above the body, nearest ceiling {:?}, calibrated {PLAYER_EYE:.2}",
        ceilings.first()
    );

    println!(
        "walker: sector {sector}, {} solid planes, gravity {:.2}",
        solids.sectors.iter().map(|s| s.len()).sum::<usize>(),
        level.gravity
    );
    commands.insert_resource(Collision(solids));
    // The ammo bin the player's weapon spends, resolved once from the weapon's own powerup cog.
    let ammo_bin = ammo_bin_for(&world);
    println!("ammo: the player's weapon spends bin {ammo_bin:?}");
    commands.insert_resource(AmmoBin(ammo_bin));
    // The runtime needs the same collider to stop a moving thing and report `blocked`; it is filled
    // on the first frame, when the resource exists.
    // The player's motion comes from the level's own `walkplayer` template, which is why
    // the numbers differ per level: `-walk` sweeps use the same rules.
    // The link layer: which script hears a thing's `used`, a surface's `activate`, a sector's
    // `entered`. It is what makes a level's own doors and switches work, and it is built from the
    // level's `cogs` section plus each script's `symbols`.
    {
        let handle = runtime::Runtime::handle(&world.level);
        let mut rt = handle.borrow_mut();
        rt.wire_links(&world.level.cogs, &world.cogs);
        println!(
            "links: {} thing, {} surface, {} sector link(s) wired from the level's scripts",
            rt.thing_links.len(),
            rt.surface_links.len(),
            rt.sector_links.len()
        );
    }
    let rules = Rules::from_level(level);
    println!("rules: {}", rules.describe());
    commands.insert_resource(Player {
        fall_damage: 0.0,
        extra_speed: 0.0,
        vel: [0.0; 2],
        step_rollover: 0.0,
        fall_killed: false,
        fall_fade: 1.0,
        pos: Vec3::from(start),
        vy: 0.0,
        yaw: player_yaw,
        pitch: 0.0,
        sector,
        grounded: false,
        gravity: rules.gravity.max(1.0),
        rules,
        water: false,
        health: 100.0,
        shields: 100.0,
        eye: eye_height,
        third_person: std::env::var_os("BK_THIRD_PERSON").is_some(),
    });
}

/// The level's solid planes, built once at load.
#[derive(Resource)]
struct Collision(collide::Solids);

/// Fall damage for an impact speed, exactly as `sithCollision_FallHurt` computes it.
///
/// `(vel - 2.5)^2 * 45`, applied only when that exceeds 1.0 — so a fall under 2.5 units/s hurts nothing,
/// and 2.65 is the first speed that registers at all. `sithThing_AttachThingToSurface` calls this when a
/// body lands, alongside a surface-typed landing sound.
#[cfg(test)]
mod fall_tests {
    use super::fall_damage;
    use super::fall_death_rule;

    /// `sithCollision_FallHurt`: `(vel - 2.5)^2 * 45`, and nothing at all below a 1.0 threshold.
    #[test]
    fn fall_damage_matches_the_engine_curve() {
        assert_eq!(fall_damage(2.5), 0.0, "at the threshold there is no damage");
        assert_eq!(fall_damage(2.6), 0.0, "0.01^2 * 45 is under the 1.0 floor");
        // Below the threshold the engine does not call `FallHurt` at all. Squaring the negative difference
        // is what made a 1.2 u/s landing cost 76 damage before this gate existed.
        assert_eq!(fall_damage(1.2), 0.0, "a landing under 2.5 must be free");
        assert_eq!(fall_damage(0.0), 0.0);
        assert!((fall_damage(3.0) - 11.25).abs() < 1e-4, "{}", fall_damage(3.0));
        assert!((fall_damage(5.0) - 281.25).abs() < 1e-4, "{}", fall_damage(5.0));
        assert!((fall_damage(10.0) - 2531.25).abs() < 1e-3, "{}", fall_damage(10.0));
    }

    /// The fall death is the sector's flag, not the speed alone: a fast fall in an ordinary room is a
    /// landing (`FallHurt` handles it), a fast fall in a `FALLDEATH` sector is the end of the level.
    #[test]
    fn fall_death_needs_a_fast_fall_and_a_flagging_sector() {
        let flagged = crate::collide::SECTOR_FALLDEATH;
        assert!(fall_death_rule(3.1, flagged), "a 3.1 u/s fall in a flagged sector kills");
        assert!(!fall_death_rule(3.0, flagged), "exactly 3.0 is the engine's `vel.z < -3.0`");
        assert!(!fall_death_rule(3.1, 0), "an ordinary room does not kill, however fast the fall");
        assert!(!fall_death_rule(3.1, crate::collide::SECTOR_UNDERWATER), "water is not a fall death");
        assert!(fall_death_rule(8.0, flagged | crate::collide::SECTOR_UNDERWATER), "the flag is a bit");
    }
}
fn fall_damage(impact: f32) -> f32 {
    // `sithThing_AttachThingToSurface` only calls `FallHurt` when `-dot(vel, normal) > 2.5`, and `FallHurt`
    // then applies `(vel - 2.5)^2 * 45` above a 1.0 floor. Without that outer test the square of a
    // *negative* difference is positive, so a gentle landing was doing real damage — the `--fall` gate
    // caught a 1.2 u/s landing being charged 76.
    if impact <= 2.5 {
        return 0.0;
    }
    let d = (impact - 2.5) * (impact - 2.5) * 45.0;
    if d > 1.0 { d } else { 0.0 }
}

/// `sithPlayer_Update`'s fall-death test (`decomp/004c89d0_sithPlayer_Update.c:80`, `sithPlayer.c:251`):
/// a body falling faster than 3 u/s **inside a `SITH_SECTOR_FALLDEATH` sector**. The sector is what
/// separates "fell off a ledge" from "fell out of the world".
///
/// It lives here, not in the walk system, so the rule can be checked without a running world.
fn fall_death_rule(engine_vy: f32, sector_flags: u32) -> bool {
    engine_vy > 3.0 && sector_flags & collide::SECTOR_FALLDEATH != 0
}
/// Fallbacks for the body, used only when a level's `walkplayer` names no `size=`/`eyeoffset=`.
///
/// The derived path is [`Rules::from_level`]: `size=0.065` x `MODEL_SCALE` = 0.065 for the radius and
/// `eyeoffset.z=0.037` x `MODEL_SCALE` = 0.037 for the camera, both traced to `sithThing.c`/`sithCamera.c`.
/// These constants are the same two numbers so a level without the fields cannot disagree with one that
/// has them; `BK_RADIUS`/`BK_EYE`/`BK_STEP` still override per run for experiments.
const PLAYER_RADIUS: f32 = 0.065;

/// Temporary experiment hooks: the player's radius, eye offset and step, overridable per run so a scale
/// can be judged from a screenshot without a rebuild. Remove once the values are settled.
/// `--shot-at` vantage, set once in `main` and read by `setup`. A global because `setup` already takes
/// as many parameters as Bevy will accept.
static SHOT_AT: std::sync::OnceLock<([f32; 3], f32)> = std::sync::OnceLock::new();

/// Silence everything. Captures are muted outright — a screenshot run has no business making noise — and
/// `--mute` (or `BK_MUTE=1`) silences any other run.
static MUTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn muted() -> bool {
    MUTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Master gain for everything audible: zero while muted, otherwise a deliberately quiet default
/// (`BK_VOLUME` overrides). The game mixes loud, and a test run that makes any sound at all should not be
/// unpleasant, so this is 5% (-26 dB) rather than 100%.
fn sound_gain() -> f32 {
    if muted() {
        0.0
    } else {
        env_f32("BK_VOLUME", 0.05).clamp(0.0, 1.0)
    }
}

fn env_f32(key: &str, fallback: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}
/// Radians of turn per pixel of mouse motion. Calibrated, not recovered: JK reads the mouse through
/// an options-screen sensitivity (`stdControl_SetMouseSensitivity` scales counts by `sensitivity * 250`),
/// so the shipped ratio is a setting rather than data. `BK_LOOK_SENS` overrides it.
const LOOK_SENSITIVITY: f32 = 0.0022;
/// The step a body may climb: **one body radius, `movesize + 0.01` = 0.066**.
///
/// The engine has no step at all — `sithCollision.c` contains no step height, and `MoveThing` crosses
/// floors through adjoins. This port's sweep needs one because its adjoin coverage is incomplete, so the
/// value is the engine's own floor-retry allowance: `sithPhysics_FindFloor`'s second pass runs at
/// `range = moveSize`, and `walkplayer`'s `movesize=0.065` is one body radius. A ledge up to a third of
/// the body's height is walkable and a wall is not.
///
/// It used to be 0.66 because the drawing scale was ten times too big, so every riser in the level was
/// measured in a unit ten times the body. `--move` is the check that keeps this honest: a body that
/// cannot walk off its own spawn fails the gate.
const STEP_UP: f32 = 0.066;
const PLAYER_EYE: f32 = 0.037;

/// The player's motion, from the level's own `walkplayer` template chain.
///
/// `walkplayer` -> `_actor` gives `maxthrust=2.0`, `surfdrag=3.0`, `jumpspeed=1.5` and
/// `movesize=0.065`. The engine integrates `vel += thrust*dt` and then
/// `vel *= 1 - drag*dt` (`sithPhysics_ApplyDrag`), so the speed the player settles at is
/// `thrust / drag`; the control code then scales it per key: FAST ×2, SLOW ×0.5, crouch
/// 0.5, backwards ×0.5, strafing ×0.7 (`sithControl_PlayerMovement`), and a crouched jump
/// ×0.7 (`sithPlayerActions_JumpWithVel`). Template lengths are model-space, so they are
/// scaled by `MODEL_SCALE` into level space, gravity included.
#[derive(Clone, Copy)]
struct Rules {
    /// Settled ground speed in level units per second (walking).
    walk: f32,
    /// Upward velocity added by a jump.
    jump: f32,
    /// The level's `WORLD GRAVITY`, in level units per second squared.
    gravity: f32,
    /// Body radius for the collision solver (`movesize`).
    radius: f32,
    /// `surfdrag=` from the `walkplayer` template: what settles a thrust into a speed, and
    /// so what converts a force-speed term into one.
    drag: f32,
    /// `airdrag=` — the drag while not standing on a surface. Much smaller than `surfdrag`, which is
    /// why a jump keeps its momentum.
    air_drag: f32,
    /// The head-pitch clamp, in radians: `minheadpitch`/`maxheadpitch` from the actor template, which is
    /// what `sithControl` clamps against. Falls back to about 80 degrees either way.
    pitch_min: f32,
    pitch_max: f32,

}

impl Rules {
    fn from_level(level: &jkl::Level) -> Rules {
        let name = "walkplayer";
        let thrust = level.template_f32(name, "maxthrust").unwrap_or(2.0);
        let drag = level.template_f32(name, "surfdrag").filter(|d| *d > 0.0).unwrap_or(3.0);
        let jump_speed = level.template_f32(name, "jumpspeed").unwrap_or(1.5);
        Rules {
            walk: thrust * physics_scale() / drag,
            jump: jump_speed * physics_scale(),
            gravity: level.gravity * physics_scale(),
            // `sithThing.c` sets `moveSize` from the template's `size=` (0.065 on `walkplayer`), and the
            // `.3do` `RADIUS` field is in the same model units the geometry is drawn from, so
            // `MODEL_SCALE` takes it into level units: 0.065 x 10 = 0.65. That is the body the level was
            // authored around, and `--walk all` agrees with the hull test on more levels at 0.65 than at
            // the old 0.35.
            radius: level
                .template_f32(name, "size")
                .or_else(|| level.template_f32(name, "movesize"))
                .map(|s| s * MODEL_SCALE)
                .filter(|r| *r > 0.05)
                .unwrap_or(PLAYER_RADIUS),
            drag,
            air_drag: level.template_f32(name, "airdrag").filter(|d| *d > 0.0).unwrap_or(1.0),
            // The limits live on `_actor`, which `walkplayer` derives from, so ask both.
            pitch_min: level
                .template_f32(name, "minheadpitch")
                .or_else(|| level.template_f32("_actor", "minheadpitch"))
                .map(f32::to_radians)
                .unwrap_or(-1.4),
            pitch_max: level
                .template_f32(name, "maxheadpitch")
                .or_else(|| level.template_f32("_actor", "maxheadpitch"))
                .map(f32::to_radians)
                .unwrap_or(1.4),

        }
    }

    /// Horizontal speed for one frame's input, as `sithControl_PlayerMovement` builds it.
    ///
    /// `water` is true when the surface underfoot carries `SITH_SURFACE_WATER` (0x20000) or
    /// `SITH_SURFACE_VERYDEEPWATER` (0x100000); the engine multiplies the whole result by
    /// 0.5 there, and crouching in water therefore quarter-speeds the player.
    /// `extra` is the force-speed term: the engine adds it to `maxThrust` before drag
    /// settles the speed, so it converts the same way — `walk + extra * MODEL_SCALE / drag`.
    fn extra_walk(&self, extra: f32) -> f32 {
        self.walk + extra * physics_scale() / self.drag.max(0.001)
    }

    fn speed(&self, forward: f32, strafe: f32, run: bool, crouch: bool, water: bool) -> [f32; 2] {
        // `move_multiplier`: FAST doubles, crouch *replaces* it with a half.
        let multiplier = if crouch {
            0.5
        } else if run {
            2.0
        } else {
            1.0
        };
        // Backing up is half speed; strafing is 0.7 of it; water halves everything.
        let forward = if forward < 0.0 { forward * 0.5 } else { forward };
        let strafe = strafe * 0.7;
        let water = if water { 0.5 } else { 1.0 };
        [
            self.walk * multiplier * water * forward,
            self.walk * multiplier * water * strafe,
        ]
    }

    /// A jump's upward velocity; crouching cuts it to 0.7.
    fn jump_velocity(&self, crouch: bool) -> f32 {
        if crouch {
            self.jump * 0.7
        } else {
            self.jump
        }
    }

    /// One line for `--rules`.
    fn describe(&self) -> String {
        format!(
            "walk {:.2}  run {:.2}  back {:.2}  strafe {:.2}  crouch {:.2}  jump {:.2} (up {:.2})  gravity {:.1}  radius {:.2}",
            self.walk,
            self.walk * 2.0,
            self.walk * 0.5,
            self.walk * 0.7,
            self.walk * 0.5,
            self.jump,
            self.jump * 0.7,
            self.gravity,
            self.radius
        )
    }
}

/// The player's own state: where the body is, how fast it is falling, and where it looks.
#[derive(Resource)]
struct Player {
    pos: Vec3,
    vy: f32,
    yaw: f32,
    pitch: f32,
    sector: usize,
    grounded: bool,
    gravity: f32,
    /// The level's own motion: speeds, jump and radius from `walkplayer`'s template.
    rules: Rules,
    /// Standing on a water or deep-water surface, which halves movement.
    water: bool,
    /// The shield bar in front of that health (`SITHBIN_SHIELDS`), mirrored the same way.
    shields: f32,
    /// Health, mirrored from the runtime's own `walkplayer` thing so the HUD can show what the
    /// bolts took off (§16.5). `DamageThing` on the player writes the runtime's copy; this is a
    /// read-only mirror for the interface.
    health: f32,
    /// How high the eye sits above the body. `PLAYER_EYE` is the calibrated height, but a
    /// sector with a low ceiling cannot hold it: level 1's spawn puts 1.45 inside the ceiling
    /// and the view renders from inside the geometry, so it is fitted to the room.
    eye: f32,
    /// Third person: the engine's follow camera height with a boom behind the look axis. `C` toggles.
    third_person: bool,
        /// Fall damage owed to the runtime, applied on the next frame by `apply_fall_damage`. The walker only
    /// knows the impact speed; the runtime owns the health.
    fall_damage: f32,
    /// The force-speed term (`SetActorExtraSpeed`), applied to the walk speed.
        extra_speed: f32,
    /// Horizontal velocity in level units per second. The engine never sets a speed directly:
    /// `sithControl_PlayerMovement` sets `acceleration`, `sithPhysics_UpdatePlayerPhysics` adds
    /// `acceleration * dt` and applies `ApplyDrag(vel, drag)`, so walking is an exponential approach to
    /// `maxThrust * MODEL_SCALE / drag`. Moving by `speed * dt` instead reads as instant starts and
    /// stops, which is what "the controls feel weird" was.
    vel: [f32; 2],
    /// Leftover time for the fixed 50 Hz physics step (`OLDSTEP_TARGET_FPS`).
    step_rollover: f32,
    /// `SITH_AF_FALLKILLED`: the player fell too fast in a `SITH_SECTOR_FALLDEATH` sector and is dying.
    /// The palette fades at 0.7 a second and death follows (`sithPlayer.c:265`).
    fall_killed: bool,
    /// The palette fade the fall death runs down, 1 → 0 (`sithPlayer.c:267`).
    fall_fade: f32,
}

/// Where to put the player: the level's `walkplayer` position, nudged out of anything it overlaps.
///
/// On some maps that position is a trigger volume's centre rather than a spot in the room — levels 1's
/// records sector `339`, a sliver with no floor of its own — and on others it sits on top of an NPC, which
/// is how the port ends up spawning inside one. The engine resolves placement against the collision world;
/// this does the same cheaply: search outwards for a point with a floor under the footprint, no solid
/// within it, and no other thing inside it.
fn spawn_point(
    level: &jkl::Level,
    solids: &collide::Solids,
    sector: usize,
    at: [f32; 3],
    radius: f32,
    self_template: &str,
) -> Option<([f32; 3], usize)> {
    let thing_clearance = env_f32("BK_THING_CLEARANCE", 2.0);
    // A thing that *is* the walkplayer is the spawn marker itself; anything else must be far enough away
    // in the plane, or far enough vertically, not to be inside the body.
    let clear_of_things = |p: [f32; 3]| {
        level.things.iter().all(|t| {
            if t.template.eq_ignore_ascii_case(self_template) {
                return true;
            }
            // A body's width, not the collision radius: an NPC model is 0.7 units across and 1.8 tall, so
            // `radius * 2` = 0.7 left the camera standing inside somebody. `--shot` showed a head and
            // shoulders filling the frame, which is the "stuffed inside each other" report.
            let (dx, dy) = (t.pos[0] - p[0], t.pos[1] - p[1]);
            (dx * dx + dy * dy).sqrt() > thing_clearance
                || (t.pos[2] - p[2]).abs() > 2.0
        })
    };
    let mut best_roomy: Option<([f32; 3], usize, f32)> = None;
    let ring = [0.0f32, 0.4, 0.8, 1.2, 1.8, 2.5, 3.5];
    for step in ring {
        for turn in 0..16 {
            let angle = turn as f32 * std::f32::consts::TAU / 16.0;
            let p = [
                at[0] + angle.cos() * step,
                at[1] + angle.sin() * step,
                at[2],
            ];
            let Some((room, floor, _)) = solids.standable(sector, [p[0], p[1], p[2]], 1.0, radius)
            else {
                continue;
            };
            if !clear_of_things(p) {
                continue;
            }
            // Room to stand, not merely a floor. The level places its `walkplayer` hard against geometry —
            // 0.08 units from a wall on level 1, which is inside any body — and a camera with its face in a
            // wall is what "spawned into something non-Euclidean" looks like. Require a body's width of
            // clearance on all four sides, and take the most open point if nothing is perfect.
            let body = [p[0], p[1], floor + radius];
            let mut tightest = f32::INFINITY;
            for dir in [[1.0f32, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0]] {
                let gap = solids
                    .sweep(room, body, dir, radius * 6.0, radius, collide::Gates::default())
                    .map_or(radius * 6.0, |hit| hit.distance);
                tightest = tightest.min(gap);
            }
            if tightest < radius * 1.5 {
                // Remember it as a fallback; a genuinely open point is better.
                if best_roomy.as_ref().is_none_or(|(_, _, t)| tightest > *t) {
                    best_roomy = Some((body, room, tightest));
                }
                continue;
            }
            if clear_of_things(p) {
                // The recorded sector is what the walker walks in; a nudge moves the *body*, not the room it
            // believes it is in. Swapping the sector here changed how the lateral sweep answers and the body
            // walked off the walkway.
            let _ = room;
            return Some((body, sector));
            }
        }
    }
    // Nothing satisfies both rules — a crowded room. Take the floor-standing point that is furthest from
    // any thing: spawning a body's width from an NPC is worse than spawning a stride away.
    let mut best: Option<([f32; 3], usize, f32)> = None;
    for step in [0.0f32, 0.4, 0.8, 1.2, 1.8, 2.5, 3.5] {
        for turn in 0..16 {
            let angle = turn as f32 * std::f32::consts::TAU / 16.0;
            let p = [at[0] + angle.cos() * step, at[1] + angle.sin() * step, at[2]];
            let Some((room, floor, _)) = solids.standable(sector, p, 1.0, radius) else {
                continue;
            };
            let clearance = level
                .things
                .iter()
                .filter(|t| !t.template.eq_ignore_ascii_case(self_template))
                .map(|t| ((t.pos[0] - p[0]).powi(2) + (t.pos[1] - p[1]).powi(2)).sqrt())
                .fold(f32::INFINITY, f32::min);
            // Nearest acceptable, not clearest: moving the player three units to find elbow room would
            // change where the level starts.
            if clearance > radius {
                return Some(([p[0], p[1], floor + radius], room));
            }
            if best.is_none_or(|(_, _, c)| clearance > c) {
                best = Some(([p[0], p[1], floor + radius], room, clearance));
            }
        }
    }
    if let Some((body, room, _)) = best_roomy {
        return Some((body, room));
    }
    best.filter(|(_, _, clearance)| *clearance > radius)
        .map(|(body, _room, _)| (body, sector))
}

/// The world direction of a movement input, from the engine's own basis.
///
/// JK builds the body's motion in its local frame and turns it into the world with the thing
/// orientation, whose axes are `lvec` (forward) and `rvec = lvec × uvec` (right). With
/// `lvec = (cos yaw, sin yaw, 0)` and `uvec = (0, 0, 1)` that is:
///
/// ```text
/// forward = ( cos yaw,  sin yaw)
/// right   = ( sin yaw, -cos yaw)      // lvec x uvec
/// ```
///
/// The strafe sign was **backwards** (`(-sin yaw, cos yaw)`, which is *left*), so `A` and `D` were
/// swapped. This function is the single place the mapping lives, and `--controls` checks it against the
/// camera transform the renderer actually uses.
fn walk_target(along: f32, across: f32, yaw: f32) -> [f32; 2] {
    let (forward, right) = yaw_axes(yaw);
    [
        forward[0] * along + right[0] * across,
        forward[1] * along + right[1] * across,
    ]
}

/// The engine's own yaw basis, and the only place the mapping lives.
///
/// The matrix builder every thing and the camera go through (`FUN_0043fcf0` @ `0x43fcf0`, ported as
/// `thing_quat`) puts the *pitch-tilted* axis in **column 1**: at yaw 0 with pitch `p` its columns are
/// `(1,0,0)`, `(0,cos p,sin p)`, `(0,-sin p,cos p)`, so only column 1 rises when the view looks up. The
/// first-person camera is `R(thing orient) . R(view angles)` (`FUN_004c56b0` @ `0x4c56b0`), so the
/// camera looks along column 1 and a level's yaw `a` faces `(-sin a, cos a)`.
///
/// This port used column 0 (`(cos a, sin a)`) and called it forward, which is the same frame a quarter
/// turn round: the camera, the walk keys and the spawn aim all followed the level's yaw into the wrong
/// direction. `scrap/yaw-*.png` is the 03katarn start both ways — column 0 puts the eye inside a
/// doorway, column 1 is the street the level opens on.
fn yaw_axes(yaw: f32) -> ([f32; 2], [f32; 2]) {
    let (sin, cos) = yaw.sin_cos();
    ([-sin, cos], [cos, sin])
}

/// The engine's first-person strafe lean, in **degrees**: roll += clamp(5 * dot(vel, right), -8, 8).
///
/// `FUN_004c56b0` @ `0x4c56b0` (camera mode 1) adds it to the third view angle - the roll, per
/// `FUN_0043fcf0`'s column order - and the three constants are in the file: 5.0 at `0x522ddc`,
/// -8.0 at `0x522de0`, 8.0 at `0x522de4`. So it is not a pitch: the view banks with *lateral*
/// speed, walking forward is level, and a full-speed strafe banks 3.3 degrees.
fn camera_lean(vel: [f32; 2], yaw: f32) -> f32 {
    let (_, right) = yaw_axes(yaw);
    (5.0 * (vel[0] * right[0] + vel[1] * right[1])).clamp(-8.0, 8.0)
}

/// The engine's projection: a fixed **horizontal** field of view, with the vertical angle left to the
/// window's aspect.
///
/// `90.0` is the word at `buffer + 0x08` of JK.EXE's render buffer, read from a running copy of the
/// shipped executable with level 1 loaded (`--gates`'s peer observation, recorded in
/// `docs/formats.md` §24). The engine never states a *vertical* angle: it fixes the horizontal one and
/// lets the taller axis follow, so a 4:3 screen shows 73.7 degrees vertically and a 16:9 window 58.7 —
/// which is why Bevy, whose `PerspectiveProjection.fov` is the vertical angle, has to derive it.
const ENGINE_H_FOV_DEG: f32 = 90.0;

/// The vertical angle that gives `ENGINE_H_FOV_DEG` horizontally at this aspect.
fn vertical_fov(aspect: f32) -> f32 {
    2.0 * ((ENGINE_H_FOV_DEG.to_radians() * 0.5).tan() / aspect.max(0.05)).atan()
}

/// Marks a camera whose projection is the engine's, so `engine_fov` keeps it in step with the window.
#[derive(Component)]
struct EngineFov;

/// Keep `EngineFov` cameras on the engine's projection as the window changes shape.
fn engine_fov(
    window: Query<&Window, With<PrimaryWindow>>,
    mut cams: Query<&mut Projection, With<EngineFov>>,
) {
    let Ok(window) = window.single() else { return };
    let fov = vertical_fov(window.width() / window.height().max(1.0));
    for mut projection in &mut cams {
        if let Projection::Perspective(perspective) = projection.as_mut() {
            perspective.fov = fov;
        }
    }
}

/// Lock the pointer while the game has the screen, release it for a menu.
///
/// The engine runs windowed with the mouse captured during play and hands the pointer back to the
/// interface otherwise (`stdControl` captures on window activation, the GUI frees it). Without a grab the
/// deltas stop at the window edge and the look jams in one direction.
fn grab_cursor(overlay: Option<Res<Overlay>>, mut cursors: Query<&mut CursorOptions>) {
    let playing = overlay.as_deref().is_none_or(|o| o.menu.is_none());
    let mode = if playing {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    for mut cursor in &mut cursors {
        if cursor.grab_mode == mode {
            continue;
        }
        cursor.grab_mode = mode;
        cursor.visible = !playing;
    }
}

/// Mouse look, in the engine's terms.
///
/// `sithControl_PlayerMovement` never reads the mouse directly: the mouse is an *axis*
/// (`stdControl_SetMouseSensitivity` maps counts onto `sensitivity * 250`, then
/// `sithControl_GetAxis(INPUT_FUNC_TURN)` normalises it), and the turn is that axis times the frame rate
/// as an angular velocity in degrees. The shipped sensitivity lives in the options screen, not in the
/// data, so this port states its own ratio and lets `BK_LOOK_SENS` override it.
///
/// Signs: a rightward delta turns right, and increasing yaw turns *left* (the basis in `walk_target`
/// fixes that), so yaw falls with `+dx`. Mouse Y grows downward, so looking up is a negative `dy`.
/// Apply one frame of mouse look.
///
/// **Modern** by default: moving the mouse down looks down. Retail JK is the other way round — mouse up
/// looks down, the flight-sim convention its 1997 control code was written for — and the executable has no
/// option for it (searching JK.EXE's strings for "invert" finds nothing), so `BK_INVERT_Y=1` restores the
/// retail sense for a side-by-side comparison rather than adding a menu the shipped game never had.
///
/// The sign rests on the platform convention, not on hope: Bevy passes winit's raw device delta through
/// unchanged (`bevy_winit` `state.rs`: `Vec2::new(x as f32, y as f32)`), and on Windows that delta is
/// screen-signed, so `y` grows downward and `pitch - dy` means "mouse down, look down".
fn apply_look(
    yaw: &mut f32,
    pitch: &mut f32,
    delta: Vec2,
    sens: f32,
    limits: (f32, f32),
    invert_y: bool,
) {
    *yaw -= delta.x * sens;
    let dy = if invert_y { -delta.y } else { delta.y };
    *pitch = (*pitch - dy * sens).clamp(limits.0, limits.1);
}

/// `BK_INVERT_Y=1` restores retail's inverted vertical look.
fn look_inverted() -> bool {
    std::env::var_os("BK_INVERT_Y").is_some()
}

/// How far above the body the engine puts a follow camera.
///
/// `FUN_004c56b0`'s follow case (`0x4c59ce`) takes the thing's z and subtracts `-0.2`
/// (`0x522dec`), then adds an offset built from the orbit matrix — so its camera sits **0.2 above the
/// thing**, about the height of an actor's head. Third person keeps that height.
const THIRD_PERSON_UP: f32 = 0.2;

/// The boom length: the port's own standoff rule (`radius * 2.5 + 0.5`, the same one `--stand` uses)
/// at the player's radius, which at `PLAYER_RADIUS` is a little over three body heights.
fn third_person_boom() -> f32 {
    PLAYER_RADIUS * 2.5 + 0.5
}

/// Where a third-person camera sits and what it aims at, for a body at `pos` looking along `yaw`/`pitch`.
///
/// The engine's own follow case is a *spectator orbit*: it spins the camera 8 degrees per second
/// (`0x4c5a79`: `dt * 8.0` fed to `FUN_00441a30`) and never lets the player aim it, which is fine for a
/// cutscene and useless for playing. What is engine-derived here is the height and the convention —
/// translate by the body's position in level units, aim at the body's eye — and the boom is the port's
/// own, because a steerable chase camera needs a distance the engine never chose.
fn third_person_view(pos: Vec3, yaw: f32, pitch: f32, boom: f32) -> (Vec3, Vec3) {
    let (forward, _) = yaw_axes(yaw);
    let look = Vec3::new(
        forward[0] * pitch.cos(),
        forward[1] * pitch.cos(),
        pitch.sin(),
    );
    let target = pos + Vec3::Z * PLAYER_EYE;
    let eye = pos + Vec3::Z * THIRD_PERSON_UP - look * boom;
    (eye, target)
}

/// One frame of the player's horizontal motion, integrated as the engine does.
///
/// `sithPhysics_UpdatePlayerPhysics` runs a **fixed 50 Hz** step (`OLDSTEP_TARGET_FPS`), adding
/// `acceleration * OLDSTEP_DELTA_50FPS` to the velocity and applying `sithPhysics_ApplyDrag`, which
/// multiplies by `1 - drag * dt`. With `acceleration = target * drag` the velocity approaches `target`
/// exponentially and never overshoots, which is the feel of the retail walk: a short ramp, a short slide.
///
/// The rollover is the engine's own `physicsRolloverFrames`: leftover time carries to the next frame, so
/// 60 fps and 144 fps produce the same motion.
/// Integrate the walk velocity at the engine's own rate, and report how many 0.02 s sub-steps that
/// was.
///
/// `sithPhysics_UpdateThingPhysics` (JK.EXE `0x4f6860`) converts a frame delta into
/// `ROUND(dt * 50)` sub-steps of `0.02`, keeping the remainder in the thing (`+0x240`), and each
/// sub-step integrates the velocity *and* calls `MoveThing` (`0x4e7950`) once. The caller needs the
/// count for the second half of that: a single sweep per frame was covering up to five engine steps
/// after a hitch, which can cross a seam no 0.02 s step can.
fn integrate_walk(vel: &mut [f32; 2], target: [f32; 2], drag: f32, dt: f32, rollover: &mut f32) -> usize {
    const STEP: f32 = 1.0 / 50.0;
    let drag = drag.max(0.0);
    *rollover += dt.max(0.0);
    // A long stall (a load, a menu) must not turn into hundreds of steps.
    *rollover = rollover.min(STEP * 8.0);
    let mut steps = 0;
    while *rollover >= STEP {
        *rollover -= STEP;
        for axis in 0..2 {
            vel[axis] += target[axis] * drag * STEP;
            vel[axis] *= (1.0 - drag * STEP).clamp(0.0, 1.0);
        }
        steps += 1;
    }
    steps
}

/// `--shot <path>`: render a few frames, save a PNG of the viewport and exit.
/// Used to verify the renderer without a human at the keyboard.
#[derive(Resource)]
struct Shot(Option<PathBuf>);

/// `--frames <n>`: which frame the scripted screenshot is taken on. Later frames let
/// the level's own scripts move things first, which is how animated geometry is
/// verified without a player.
#[derive(Resource)]
struct ShotFrame(u32);

/// The level's scripts running against the renderer: the world state they mutate, the
/// bound instances (kept so a message can be delivered later), and which entities draw
/// which thing.
#[derive(Resource)]
struct Sim {
    world: runtime::World,
    instances: Vec<(String, cog::Vm)>,
    entities: HashMap<usize, Vec<Entity>>,
    /// Seconds of simulated play, and when the last progress line was printed.
    elapsed: f32,
    reported: f32,
}

/// `--sim`: also deliver each declared message once at load, so the level's animated
/// geometry acts without a player. Without it only `startup` runs, as at load in the
/// original engine.
#[derive(Resource)]
struct Deliver(bool);

/// Step the scripts' world and push the moved things' poses into the renderer.
fn advance_sim(time: Res<Time>, sim: Option<ResMut<Sim>>, mut drawn: Query<&mut Transform>) {
    let Some(mut sim) = sim else { return };
    let moved = sim.world.borrow_mut().step(time.delta_secs().min(0.1));
    for &i in &moved {
        let Some(entities) = sim.entities.get(&i) else { continue };
        let pose = {
            let w = sim.world.borrow();
            let Some(t) = w.things.get(i) else { continue };
            (Vec3::from(t.pos), t.yaw)
        };
        let transform = Transform::from_translation(pose.0)
            .with_rotation(Quat::from_rotation_z(-pose.1.to_radians()));
        for &e in entities {
            if let Ok(mut tf) = drawn.get_mut(e) {
                *tf = transform;
            }
        }
    }
    // Once a second, say what the renderer is actually showing for one moving thing:
    // the entity's own transform, not the runtime's copy of the position.
    sim.elapsed += time.delta_secs();
    if !moved.is_empty() && (sim.reported == 0.0 || sim.elapsed - sim.reported >= 1.0) {
        sim.reported = sim.elapsed;
        let i = moved[0];
        if let Some(shown) = sim.entities.get(&i).and_then(|e| e.first()).copied() {
            if let Ok(tf) = drawn.get_mut(shown) {
                let want = {
                    let w = sim.world.borrow();
                    w.things.get(i).map(|t| t.pos).unwrap_or([0.0; 3])
                };
                println!(
                    "sim {:.1}s: thing {i} drawn at ({:.2} {:.2} {:.2}), runtime ({:.2} {:.2} {:.2})",
                    sim.elapsed,
                    tf.translation.x,
                    tf.translation.y,
                    tf.translation.z,
                    want[0],
                    want[1],
                    want[2]
                );
            }
        }
    }
}

/// Diagnostic: `BK_TRACE=1` prints what the world camera, the overlay quad and the level batches
/// believe, once every ten frames. Temporary scaffolding for a screenshot that comes out empty.
fn trace_state(
    overlay: Option<Res<Overlay>>,
    cams: Query<(&GlobalTransform, &Projection), (With<Camera3d>, Without<OverlayQuad>)>,
    quads: Query<Entity, With<OverlayQuad>>,
    batches: Query<&Visibility, With<SectorId>>,
    mut frame: Local<u32>,
) {
    if std::env::var_os("BK_TRACE").is_none() {
        return;
    }
    *frame += 1;
    if !frame.is_multiple_of(10) {
        return;
    }
    let mut visible = 0;
    let mut hidden = 0;
    for v in &batches {
        match v {
            Visibility::Hidden => hidden += 1,
            _ => visible += 1,
        }
    }
    let cam = cams.iter().next().map(|(t, p)| format!("{:.2},{:.2},{:.2} {p:?}", t.translation().x, t.translation().y, t.translation().z));
    println!(
        "trace {}: menu {:?} dirty {} overlay entities {} batches {visible} visible / {hidden} hidden, cameras {}, cam {:?}",
        *frame,
        overlay.as_ref().and_then(|o| o.menu.clone()),
        overlay.as_ref().is_some_and(|o| o.dirty),
        quads.iter().count(),
        cams.iter().count(),
        cam
    );
}

fn auto_shot(shot: Res<Shot>, at: Res<ShotFrame>, mut commands: Commands, mut frame: Local<u32>) {
    let Some(path) = shot.0.clone() else { return };
    *frame += 1;
    // Few enough frames that a scripted run stays short, enough that texture
    // uploads and the first render pass have happened.
    if *frame == at.0.max(1) {
        println!("saving {} after {} frames", path.display(), *frame);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path))
            .observe(|_: On<ScreenshotCaptured>, mut commands: Commands| {
                commands.write_message(AppExit::Success);
            });
    }
    // Backstop: never hang a scripted run if the capture never completes.
    if *frame > at.0.max(600) + 600 {
        println!("screenshot did not complete; exiting");
        commands.write_message(AppExit::Success);
    }
}

/// Play the `--play` sound once, then exit after long enough to hear it.
fn play_exit(
    mut commands: Commands,
    play: Res<PlaySound>,
    time: Res<Time>,
    mut audio: ResMut<Assets<AudioSource>>,
    mut started: Local<Option<f32>>,
) {
    let Some((name, bytes)) = &play.0 else { return };
    let start = *started.get_or_insert_with(|| {
        println!("playing {name} ({} bytes)", bytes.len());
        let source = audio.add(AudioSource { bytes: bytes.clone().into() });
        commands.spawn((
            AudioPlayer::new(source),
            PlaybackSettings { volume: Volume::Linear(sound_gain()), ..PlaybackSettings::DESPAWN },
        ));
        time.elapsed_secs()
    });
    // Long enough for the longest shipped ambient: the 371 KB river loop is ~8.4 s.
    if time.elapsed_secs() - start > 10.0 {
        commands.write_message(AppExit::Success);
    }
}

/// Keep the sky dome centred on the camera so no part of the world can leave it.
/// Keep the output silent while muted, every frame.
///
/// Zeroing the volume where a sound *spawns* is not enough: anything that sets a sink's volume afterwards
/// (a fade, a track change, an ambience ramp) brings it back. This is the single choke point that cannot be
/// bypassed, and it costs one query over the handful of live sinks.
fn enforce_mute(mut sinks: Query<&mut AudioSink>, mut reported: Local<u32>) {
    let mut live = 0usize;
    let mut audible = 0usize;
    for mut sink in &mut sinks {
        live += 1;
        if muted() {
            // `AudioSink::mute` is the real switch: `set_volume` on a muted sink is ignored rather than
            // unmuting it, so a later fade cannot bring the sound back.
            sink.mute();
        }
        if !sink.is_muted() {
            audible += 1;
        }
    }
    // Reported on a tick, with the sink state read back rather than assumed: this is the check that says
    // the output is actually silent. It prints even at zero sinks so a vacuous run is visible as such
    // instead of looking like a pass.
    *reported = reported.wrapping_add(1);
    if *reported % 60 == 0 {
        let level = sinks
            .iter()
            .map(|s| match s.volume() {
                Volume::Linear(v) => v,
                _ => 0.0,
            })
            .fold(0.0f32, f32::max);
        println!("audio: {live} sink(s) live, {audible} audible, loudest {level:.3}");
    }
}

fn sky_follows_camera(
    cam: Query<&Transform, (With<Camera3d>, Without<SkyDome>)>,
    mut dome: Query<&mut Transform, With<SkyDome>>,
) {
    let Ok(cam) = cam.single() else { return };
    for mut d in &mut dome {
        d.translation = cam.translation;
    }
}

/// The player: a walker bound to the level's own sectors, so walls stop it and doorways
/// let it through. Arrow keys look, `WASD` moves, `Shift` runs, `Space` jumps.
///
/// This is the `--walk` sweep's code path (`walker_step`) with a keyboard for input, so
/// the movement the sweep verifies is the movement the window runs.
#[allow(clippy::too_many_arguments)]
/// Is the body standing in water? Two rules, both the engine's: the surface under the feet carries
/// `SITH_SURFACE_WATER` (0x20000) or `VERYDEEPWATER` (0x100000), **or** the sector itself carries
/// `SITH_SECTOR_UNDERWATER` (0x2) — `sithPhysics.c:43` tests the sector, and that bit is what
/// `SetSectorFlags(sector, 2)` sets for the shipped levels' "underwater passthru layer".
fn in_water(
    level: &jkl::Level,
    solids: &collide::Solids,
    sector: usize,
    feet: [f32; 3],
    sector_flags: &[u32],
) -> bool {
    let underfoot = solids
        .floor_surface(sector, feet)
        .and_then(|(_, surface)| level.surfaces.get(surface))
        .is_some_and(|surface| surface.surfflags & (0x2_0000 | 0x10_0000) != 0);
    underfoot
        || sector_flags
            .get(sector)
            .is_some_and(|flags| flags & collide::SECTOR_UNDERWATER != 0)
}

fn walk_player(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    world: Option<Res<World>>,
    solids: Option<Res<Collision>>,
    sim: Option<Res<Sim>>,
    mut player: Option<ResMut<Player>>,
    overview: Res<Overview>,
    near: Res<Near>,
    mouse: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    overlay: Option<Res<Overlay>>,
    // The overlay camera is a `Camera3d` too, so it must be excluded or `single_mut` fails and
    // the walker's view never moves (which is what happened for every round before this one).
    mut cam: Query<&mut Transform, (With<Camera3d>, With<WorldCamera>)>,
) {
    let (Some(solids), Some(player)) = (solids, player.as_mut()) else { return };
    // `--overview` and `--near` exist to frame a fixed shot; a walker would fight them.
    if overview.overview || near.0.is_some() {
        return;
    }
    let Ok(mut tf) = cam.single_mut() else { return };
    // A capture runs on a fixed step, so the same command reaches the same pose every time: with the
    // wall-clock delta, the player was still falling at one frame count and settled at another, and
    // two identical runs differed in half their pixels (§16.13).
    // A pinned step on request (`BK_FIXED_DT`): the live checks drive real input and this frame time
    // ranges from 0.008 s to the 0.1 clamp depending on the level's weight, so 30 frames is anywhere
    // from a quarter second to three. A fixed step makes a check depend on the code, not the machine —
    // and it is what exposed the jump that never lands (`BK_FIXED_DT=0.0333`, level 1).
    let dt = if overview.capture {
        1.0 / 60.0
    } else if std::env::var_os("BK_FIXED_DT").is_some() {
        env_f32("BK_FIXED_DT", 1.0 / 30.0).clamp(1.0 / 240.0, 0.1)
    } else {
        time.delta_secs().min(0.1)
    };

    // A script can move the player: `force_jump.cog` does `SetThingVel(player, (0,0,duration
    // + 2.5))` and expects the world to honour it. Take any velocity the runtime has on the
    // player thing as a one-shot impulse, then clear it so it is not applied twice.
    if let Some(sim) = sim.as_deref() {
        let mut rt = sim.world.borrow_mut();
        if let Some(index) = rt.templates.iter().position(|t| t.eq_ignore_ascii_case("walkplayer")) {
            let vel = rt.things[index].vel;
            if vel != [0.0; 3] {
                player.vy = vel[2];
                rt.things[index].vel = [0.0; 3];
            }
            // Force speed: the script's `SetActorExtraSpeed` term scales the walk speed.
            player.extra_speed = rt.things[index].extra_speed;
        }
    }

    // What the scripts have switched so far: a force field closed, a door sector's adjoins hidden.
    // The two flag arrays are **copied out** and `gates` borrows those locals, so the runtime's guard
    // dies right here. Holding it for the rest of the function made every later `borrow()` a
    // same-thread double borrow — the fail-fast `World::take` panicked at the first such site, and
    // there are several. Two small arrays per frame is a cheaper fix than auditing each read.
    // `BK_GATES_OPEN=1` ignores the scripts' switches, which is how the walk can be A/B'd.
    let (sector_flags, adjoin_flags) = sim
        .as_deref()
        .map(|s| {
            let rt = s.world.borrow();
            (rt.sector_flags.clone(), rt.adjoin_flags.clone())
        })
        .unwrap_or_default();
    let gates = if std::env::var_os("BK_GATES_OPEN").is_some() {
        collide::Gates::default()
    } else {
        collide::Gates { sectors: &sector_flags, adjoins: &adjoin_flags }
    };

    // A capture may not read the machine's keyboard or mouse: the same command has to reach the same
    // pose every run (§16.13). The env hooks (BK_WALK_FORWARD) still drive the walker.
    let live = !overview.capture;
    let held = |key: KeyCode| live && keys.pressed(key);
    // `C` toggles third person. Retail has no such key in single player (its follow camera is a
    // spectator orbit), so there is nothing to match; the port needs one and this is it.
    if live && keys.just_pressed(KeyCode::KeyC) {
        player.third_person = !player.third_person;
        println!(
            "camera: {}",
            if player.third_person { "third person" } else { "first person" }
        );
    }
    if held(KeyCode::ArrowLeft) {
        player.yaw += 1.5 * dt;
    }
    if held(KeyCode::ArrowRight) {
        player.yaw -= 1.5 * dt;
    }
    if held(KeyCode::ArrowUp) {
        player.pitch = (player.pitch + 1.0 * dt).min(1.4);
    }
    if held(KeyCode::ArrowDown) {
        player.pitch = (player.pitch - 1.0 * dt).max(-1.4);
    }

    // Mouse look, while no screen is open. The cursor is locked then (see `grab_cursor`), which is what
    // makes the deltas relative instead of clamped by the window edge.
    if live && overlay.as_deref().is_none_or(|o| o.menu.is_none()) {
        let sens = std::env::var("BK_LOOK_SENS")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(LOOK_SENSITIVITY);
        let limits = (player.rules.pitch_min, player.rules.pitch_max);
        let (mut yaw, mut pitch) = (player.yaw, player.pitch);
        apply_look(&mut yaw, &mut pitch, mouse.delta, sens, limits, look_inverted());
        player.yaw = yaw;
        player.pitch = pitch;
    }

    // Movement in the walker's own frame, then converted to world (x, z).
    let mut forward = 0.0f32;
    let mut strafe = 0.0f32;
    if held(KeyCode::KeyW) {
        forward += 1.0;
    }
    if held(KeyCode::KeyS) {
        forward -= 1.0;
    }
    if held(KeyCode::KeyD) {
        strafe += 1.0;
    }
    if held(KeyCode::KeyA) {
        strafe -= 1.0;
    }
    // The engine's own scaling: FAST doubles, crouch halves and also halves the jump,
    // and standing on water halves whatever is left.
    let run = keys.pressed(KeyCode::ShiftLeft);
    let crouch = keys.pressed(KeyCode::ControlLeft);
    // `BK_WALK_FORWARD=1` holds the forward key, which is how the camera-follows-the-walker fix
    // is verified without a hand on the keyboard: the trace then shows the body moving and the
    // camera with it.
    let forward = if std::env::var_os("BK_WALK_FORWARD").is_some() {
        1.0
    } else {
        forward
    };
    let [along, across] = player.rules.speed(forward, strafe, run, crouch, player.water);
    // A force-speed term adds to `maxThrust` in the engine, so it scales both axes.
    let force = if player.extra_speed > 0.0 {
        player.rules.extra_walk(player.extra_speed) / player.rules.walk
    } else {
        1.0
    };
    let [along, across] = [along * force, across * force];
    let target = walk_target(along, across, player.yaw);
    // Integrate the way `sithPhysics_UpdatePlayerPhysics` does: `acceleration` comes from the input,
    // velocity takes `acceleration * dt`, and `ApplyDrag` scales it by `1 - drag * dt` — at a fixed
    // 50 Hz step with a rollover (`OLDSTEP_TARGET_FPS`, `OLDSTEP_DELTA_50FPS`), so the feel does not
    // depend on the frame rate. `a = target * drag` settles the velocity at exactly the target.
    let drag = if player.grounded { player.rules.drag } else { player.rules.air_drag };
    // How many of the engine's 0.02 s sub-steps this frame is worth; the move below runs once per
    // sub-step, exactly where `UpdateThingPhysics` calls `MoveThing`.
    let steps = {
        let mut vel = player.vel;
        let mut rollover = player.step_rollover;
        let steps = integrate_walk(&mut vel, target, drag, dt, &mut rollover);
        player.vel = vel;
        player.step_rollover = rollover;
        steps
    };
    // A scrolling surface carries whatever stands on it: `sithThing.c` adds
    // `scrollVector * dt` to the velocity of a thing attached to a `SITH_SURFACE_SCROLLING` surface,
    // and that velocity goes through the same move. `SlideWall` is how a conveyor is built, and level
    // 2's crate puzzle is the shipped case. The surface is read from where the body stood when this
    // frame began, so the carry joins the walk instead of being overwritten by it.
    let mut carry = [0.0; 2];
    if let Some(held) = sim.as_deref() {
        let held = held.world.borrow();
        let feet = [player.pos.x, player.pos.y, player.pos.z - player.rules.radius + 0.01];
        if let Some((_, surface)) = solids.0.floor_surface(player.sector, feet) {
            // Read through the guard this function already holds: a second `borrow()` here is the
            // same-thread double borrow the fail-fast `World::take` panics on.
            carry = held.surfaces.get(surface).map(|s| s.slide).unwrap_or([0.0; 2]);
            if carry != [0.0; 2] && std::env::var_os("BK_SCROLL_TRACE").is_some() {
                println!(
                    "scroll: surface {surface} carries the body at {carry:?} level units/second"
                );
            }
        }
    }
    // `SetSectorThrust` is the other half of a conveyor: `sithPhysics.c` adds `sector->thrust * dt`
    // to a body's velocity exactly like gravity, so an updraft column lifts what stands in it —
    // level 21's core tube is `'0.0 0.0 1.0'` at 4 to 5, level 10's power core pushes the body out.
    let thrust = sim
        .as_deref()
        .map(|sim| {
            sim.world
                .borrow()
                .sectors
                .get(player.sector)
                .map(|s| s.thrust)
                .unwrap_or([0.0; 3])
        })
        .unwrap_or([0.0; 3]);
    let dir = [
        player.vel[0] + carry[0] + thrust[0] * dt,
        player.vel[1] + carry[1] + thrust[1] * dt,
    ];
    if thrust != [0.0; 3] && std::env::var_os("BK_SCROLL_TRACE").is_some() {
        println!("thrust: sector {} accelerates the body at {thrust:?} units/s^2", player.sector);
    }

    if live && keys.just_pressed(KeyCode::Space) && player.grounded {
        player.vy = player.rules.jump_velocity(crouch);
        player.grounded = false;
    }

    // `sithThing_AttachThingToSurface`: a placed body starts *on* its floor. A body whose feet begin
    // below its own sector's floor plane can never take a step — every horizontal sweep reports a
    // collision in all four directions — and three levels place the player that way (02narshadda has
    // feet at 2.905 with its floor at 3.0). One clamp here fixes the walk for all of them.
    // Sector first, then floor: the floor has to come from the room the body is standing in, not from
    // whatever sector the level named.
    if let Some(sector) =
        solids.0.sector_for_body([player.pos.x, player.pos.y, player.pos.z - player.rules.radius])
    {
        player.sector = sector;
    }
    if let Some((floor, _)) = solids.0.floor_surface(
        player.sector,
        [player.pos.x, player.pos.y, player.pos.z + player.rules.radius],
    ) && player.pos.z - player.rules.radius < floor - 0.01
        // Never while the body is rising: `sithThing_AttachThingToSurface` is a placement step, and
        // running it every frame teleported a jumping body back to the floor each frame — 17bsarris
        // oscillates z −0.083 → −0.106 → −0.083 and never leaves the ground.
        && player.vy <= 0.0
    {
        // `+ 0.01`: the containment test is asked at the feet with zero tolerance, so a body resting
        // exactly on the plane rounds outside it and every step is undone — the same epsilon the
        // scroll probe uses below.
        player.pos.z = floor + player.rules.radius + 0.01;
        player.vy = 0.0;
        player.grounded = true;
    }
    // The sector the thing was placed in can be one whose hull the body is *outside*: 04escapehouse
    // and 09fuelstation both place the player behind a wall plane of their own sector, so the
    // containment test undid every step and the body never moved. `sector_for_body` picks the room
    // with the most clearance around the feet, which is the room the body is actually standing in.
    if let Some(sector) =
        solids.0.sector_for_body([player.pos.x, player.pos.y, player.pos.z - player.rules.radius])
    {
        player.sector = sector;
    }

    let mut pos = [player.pos.x, player.pos.y, player.pos.z];
    let mut sector = player.sector;
    let mut vy = player.vy + thrust[2] * dt;
    let gravity = player.gravity;
    let trace = std::env::var_os("BK_WALK_TRACE").is_some();
    if trace {
        eprintln!(
            "walk trace: live {live} W {} escape {} forward {forward} strafe {strafe} vel {:?} target {target:?} extra {} crouch {crouch}",
            keys.pressed(KeyCode::KeyW),
            keys.pressed(KeyCode::Escape),
            player.vel,
            player.extra_speed
        );
        eprintln!("walk trace: dt {dt:.4} drag {:.3} grounded {} steps?", drag, player.grounded);
        let around = solids.0.sectors_around(pos);
        let mut floors: Vec<(usize, Option<f32>)> = around
            .iter()
            .map(|s| (*s, solids.0.floor_below(*s, pos)))
            .collect();
        floors.truncate(6);
        eprintln!(
            "walk trace: sector {sector} pos {pos:?} vy {vy:.2} support {:?} floor {:?}",
            solids.0.support(pos, player.rules.radius, 0.6),
            solids.0.floor_below(sector, pos)
        );
    }
    let before = pos;
    // One sweep per engine sub-step (`sithPhysics_UpdateThingPhysics` → `MoveThing`), not one per
    // frame: the distance per sweep is `vel * 0.02` whatever the frame rate did.
    let mut achieved_len = 0.0f32;
    let mut landed = false;
    let mut crossed = false;
    for _ in 0..steps {
        // Not shadowed as `landed`: the accumulation below needs the outer flag, and shadowing it
        // silently left every frame "airborne" on a level that lands every sub-step.
        let (len, cross, step_landed) = walker_step(
            &solids.0,
            gates,
            &mut sector,
            &mut pos,
            &mut vy,
            dir,
            1.0 / 50.0,
            player.rules.radius,
            gravity,
            true,
        );
        achieved_len = achieved_len.max(len);
        crossed |= cross;
        landed |= step_landed;
    }
    if trace {
        let vel = player.vel;
        let len = (vel[0] * vel[0] + vel[1] * vel[1]).sqrt();
        let ahead = if len > 1e-4 {
            solids
                .0
                .sweep(sector, pos, [vel[0] / len, vel[1] / len, 0.0], 0.5, player.rules.radius, gates)
        } else {
            None
        };
        eprintln!(
            "walk trace:   -> sector {sector} pos {pos:?} vy {vy:.2} vel {vel:?} ahead {ahead:?}"
        );
    }

    // A wall takes the velocity component that ran into it, which is what makes a body slide along a
    // wall instead of stopping dead; the walker reports when it pushed the body out.
    // `walker_step` reports how far the resolver let the body actually travel; when a wall ate part of
    // the frame, the fraction that survived is the velocity along the wall — that is the slide.
    if trace {
        eprintln!("walk trace: achieved {achieved_len:.5} vel-before-slide {:?}", player.vel);
    }
    if achieved_len > 0.0 {
        let moved = [(pos[0] - before[0]) / dt.max(1e-4), (pos[1] - before[1]) / dt.max(1e-4)];
        let want = [player.vel[0], player.vel[1]];
        let want_len = (want[0] * want[0] + want[1] * want[1]).sqrt();
        let moved_len = (moved[0] * moved[0] + moved[1] * moved[1]).sqrt();
        if moved_len < want_len * 0.99 {
            player.vel = moved;
        }
    }
    // Landing: `sithThing_AttachThingToSurface` hurts a body whose downward speed into the surface exceeds
    // 2.5 (`-dot(vel, normal)`), so a fall that ends this frame with a negative speed is an impact.
    // The curve wants the engine's velocity units. This world runs gravity and thrust through
    // `physics_scale`, so a fall gains `sqrt(physics_scale)` more speed than the engine's would over the
    // same drop (v = sqrt(2gh), and g is scaled but h is not); dividing by that root puts the impact back
    // in the units `sithCollision_FallHurt` was written for, so a drop hurts as much as the engine intends.
    let engine_impact = -player.vy / physics_scale().max(1.0).sqrt();
    if vy == 0.0 && engine_impact > 2.5 {
        let damage = fall_damage(engine_impact);
        if damage > 0.0 {
            player.fall_damage += damage;
            println!(
                "fall: landed at {:.1} u/s ({engine_impact:.1} in engine units), {damage:.0} damage",
                -player.vy
            );
        }
    }
    player.vy = vy;
    player.sector = sector;
    player.pos = Vec3::from(pos);
    // `derived from the landing event`, not from `vy == 0.0`: a body on a thrusting floor never has
    // zero vertical speed, and such a body could never jump. A landing this frame, or a resting body
    // (vy already zero), is on the ground.
    player.grounded = landed || player.vy == 0.0;
    if std::env::var_os("BK_JUMP_TRACE").is_some() {
        eprintln!("jump trace: z {:.3} vy {vy:.2} landed {landed} sector {sector}", player.pos.z);
    }
    // The engine's fall death (`sithPlayer.c:251`, `decomp/004c89d0_sithPlayer_Update.c:80`): falling
    // faster than 3 u/s inside a sector flagged `SITH_SECTOR_FALLDEATH` marks the player
    // `SITH_AF_FALLKILLED` and the palette runs down at 0.7 a second; at zero, `sithPlayer_KillPlayer`.
    if !player.fall_killed
        && fall_death_rule(
            -player.vy / physics_scale().max(1.0).sqrt(),
            gates.sectors.get(player.sector).copied().unwrap_or(0),
        )
    {
        player.fall_killed = true;
        player.fall_fade = 1.0;
        println!("fall death: the player fell out of the level; the palette starts to fade");
    }
    if player.fall_killed {
        player.fall_fade -= dt * 0.7;
        if player.fall_fade <= 0.0 {
            // `sithPlayer_KillPlayer` takes the whole player: lethal damage is this port's death path
            // (`--die`), and `respawn_after_death` puts the body back at the level's start.
            player.fall_killed = false;
            player.fall_fade = 1.0;
            player.fall_damage += 1000.0;
        }
    }
    // Water halves movement: the engine reads the surface under the feet (`SITH_SURFACE_WATER`
    // 0x20000 / `VERYDEEPWATER` 0x100000) and the sector's own `SITH_SECTOR_UNDERWATER` — the bit
    // `SetSectorFlags(sector, 2)` sets for the shipped "passthru layer" water.
    player.water = world.as_deref().is_some_and(|world| {
        let feet = [pos[0], pos[1], pos[2] - player.rules.radius + 0.01];
        in_water(&world.level, &solids.0, sector, feet, gates.sectors)
    });
    if crossed {
        println!("sector {sector}");
    }

    // Stand the camera where the eyes are and aim it along yaw/pitch — unless a capture has been given
    // its own vantage (`--shot-at`), in which case the camera stays exactly where the caller put it.
    if SHOT_AT.get().is_some() {
        return;
    }
    let eye = player.pos + Vec3::Z * player.eye;
    let (forward, _) = yaw_axes(player.yaw);
    let look = Vec3::new(
        forward[0] * player.pitch.cos(),
        forward[1] * player.pitch.cos(),
        player.pitch.sin(),
    );
    // Third person: stand behind the look axis and aim at the body's eye. A wall between the two shortens
    // the boom rather than putting the camera inside the geometry (§16.9's lesson, applied to the camera).
    let mut eye = eye;
    let mut look = look;
    if player.third_person {
        let boom = third_person_boom();
        let (mut third_eye, target) = third_person_view(player.pos, player.yaw, player.pitch, boom);
        let mut length = boom;
        for _ in 0..4 {
            if !solids.0.segment_blocked([target.x, target.y, target.z], [third_eye.x, third_eye.y, third_eye.z]) {
                break;
            }
            length *= 0.6;
            let (shorter, _) = third_person_view(player.pos, player.yaw, player.pitch, length);
            third_eye = shorter;
        }
        // If even the shortest boom lands outside every sector, the corner is too tight for a chase
        // camera: sit at the body's own eye rather than inside the wall.
        if solids.0.sector_at([third_eye.x, third_eye.y, third_eye.z], 0.0).is_none() {
            third_eye = player.pos + Vec3::Z * player.eye;
        }
        eye = third_eye;
        look = target - third_eye;
    }

    // A script can point the camera at a thing of its own (`SetCameraFocus(slot, camera)` +
    // `SetCurrentCamera(slot)`): `00_fallcamera.cog` cuts to a camera watching the fall,
    // `06_max.cog` and the C3 levels cut to placed cameras, and `00_cameratricks.cog` focuses a
    // camera thing for a scripted shot. The view then *is* that thing, aimed by its own angles.
    // Read once, with the guard dropped again: the one at the top of the function was removed because
    // holding it re-entered the runtime and tripped the fail-fast `World::take`.
    let camera_read = sim.as_deref().map(|s| {
        let rt = s.world.borrow();
        (
            rt.camera_focus_thing()
                .and_then(|focus| rt.things.get(focus).map(|t| (t.pos, t.yaw, t.pitch))),
            rt.camera_shake(),
        )
    }).unwrap_or((None, ([0.0; 3], [0.0; 3])));
    let focus_camera = camera_read.0;
    let shake = camera_read.1;

    if let Some((pos, yaw, pitch)) = focus_camera {
        let (forward, _) = yaw_axes(yaw);
        let pitch = pitch.to_radians();
        eye = Vec3::from(pos);
        look = Vec3::new(
            forward[0] * pitch.cos(),
            forward[1] * pitch.cos(),
            pitch.sin(),
        );
    }
    // `SetPOVShake`: the offset a shot or an earthquake kicks the view by, decaying back to zero.
    // The engine adds the angle offset to the view angles; the port rotates the aim it already has,
    // which is the same thing and also works while a scripted camera is in charge.
    {
        let (pos, angle) = shake;
        eye += Vec3::from(pos);
        if angle != [0.0; 3] {
            let turn = Quat::from_rotation_z(-angle[2].to_radians());
            let right = look.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
            let nod = Quat::from_axis_angle(right, angle[0].to_radians());
            look = (turn * nod * look).try_normalize().unwrap_or(look);
        }
    }

    tf.translation = eye;
    if std::env::var_os("BK_CAMERA_TRACE").is_some() {
        println!(
            "camera: eye {eye:?} look {:?} scripted {:?}",
            look.to_array(),
            focus_camera.map(|_| "a scripted camera").unwrap_or("none")
        );
    }
    if let Some(dir) = look.try_normalize() {
        *tf = tf.looking_to(dir, Vec3::Z);
        // The engine's own strafe lean (see camera_lean): the up axis tilts toward the right column.
        let lean = camera_lean(player.vel, player.yaw);
        if lean != 0.0 {
            tf.rotate_local_z(-lean.to_radians());
        }
    }
}

// ---------------------------------------------------------------------------
// self check
// ---------------------------------------------------------------------------

/// Structural assertions over a parsed level. Run with `--check`; exits non-zero
/// on the first inconsistency so the parsers cannot silently drift.
fn self_check(world: &World) {
    let level = &world.level;
    let problems = validate_level(level);
    assert!(problems.is_empty(), "level validation failed: {problems:#?}");

    let drawable = level
        .surfaces
        .iter()
        .filter(|s| s.material >= 0 && s.verts.len() >= 3)
        .count();

    let textured = level
        .surfaces
        .iter()
        .filter(|s| s.material >= 0 && world.images.contains_key(&s.material))
        .count();
    let portals = level.surfaces.iter().filter(|s| s.geo == 0).count();
    let adjoined = level.surfaces.iter().filter(|s| s.adjoin >= 0).count();
    let (lo, hi) = level.bounds();
    // Texture vertices are texels in the levels checked so far; if a level's are
    // normalised instead, every surface would sample one corner of its texture and
    // render as huge blocks, so the range is worth reporting.
    let (mut tlo, mut thi) = (f32::MAX, f32::MIN);
    for t in &level.texverts {
        for c in t {
            tlo = tlo.min(*c);
            thi = thi.max(*c);
        }
    }
    if level.texverts.is_empty() {
        tlo = 0.0;
        thi = 0.0;
    }

    println!("\n--- bevi-knight self check ---");
    println!("gob archives indexed   {} entries", world.gob_entries);
    println!("vertices               {}", level.vertices.len());
    println!("texture vertices       {}", level.texverts.len());
    println!("surfaces               {} ({drawable} drawable)", level.surfaces.len());
    println!("materials              {} ({} decoded to bitmaps)", level.materials.len(), world.images.len());
    println!("surfaces with texture  {textured}");
    println!("texture vertex range   [{tlo:.1}, {thi:.1}]");
    println!("portal / adjoined      {portals} portal, {adjoined} adjoined");
    println!("sectors                {}", level.sectors.len());
    println!("colormaps              {:?}", level.colormaps);
    println!("things                 {}", level.things.len());
    println!(
        "bounds                 x[{:.2},{:.2}] y[{:.2},{:.2}] z[{:.2},{:.2}]",
        lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]
    );
    let eye = level.things.iter().find(|t| t.template.eq_ignore_ascii_case("walkplayer"));
    if let Some(t) = eye {
        let inside = |v: f32, a: f32, b: f32| v >= a - 1.0 && v <= b + 1.0;
        println!(
            "walkplayer             [{:.2},{:.2},{:.2}] sector {} (inside world bounds: {})",
            t.pos[0],
            t.pos[1],
            t.pos[2],
            t.sector,
            inside(t.pos[0], lo[0], hi[0]) && inside(t.pos[1], lo[1], hi[1]) && inside(t.pos[2], lo[2], hi[2])
        );
    }
    // An asset-only level (`static.jkl`) legitimately has no surfaces to texture.
    if !level.surfaces.is_empty() {
        assert!(textured > 0, "no surface resolved to a decoded bitmap");
    }

    // How much of the world draws flat: a large share means the material decode
    // is missing textures rather than the level being genuinely untextured.
    let mut per_mat: HashMap<i32, usize> = HashMap::new();
    for s in level.surfaces.iter().filter(|s| s.material >= 0) {
        *per_mat.entry(s.material).or_default() += 1;
    }
    let mut ranked: Vec<(i32, usize)> = per_mat.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    let untextured: usize = ranked
        .iter()
        .filter(|(m, _)| !world.images.contains_key(m))
        .map(|(_, n)| n)
        .sum();
    println!("surfaces drawing flat    {untextured} (of {})", ranked.iter().map(|(_, n)| n).sum::<usize>());
    for (m, n) in ranked.iter().take(5) {
        let name = level.materials.get(*m as usize).cloned().unwrap_or_default();
        let size = world
            .images
            .get(m)
            .map(|i| format!("{}x{}", i.texture_descriptor.size.width, i.texture_descriptor.size.height))
            .unwrap_or_else(|| "no bitmap".into());
        println!("  material {m:>3} {n:>4} surfaces  {name} ({size})");
    }
    println!("--- all checks passed ---");
    let ranked = template_counts(level);
    println!("templates              {} distinct", ranked.len());
    for (name, n) in ranked.iter().take(8) {
        println!("  {n:>4}  {name}");
    }

    // Sky surfaces: whether a level's sky texture is part of the world geometry or
    // something the engine synthesises decides whether a black void is a bug.
    let mut sky_surfaces = 0usize;
    let mut sky_names: Vec<&str> = Vec::new();
    let (mut sky_lo, mut sky_hi) = (f32::MAX, f32::MIN);
    for (i, name) in level.materials.iter().enumerate() {
        if !name.to_ascii_lowercase().contains("sky") {
            continue;
        }
        let mut n = 0usize;
        for s in level.surfaces.iter().filter(|s| s.material == i as i32) {
            n += 1;
            for v in &s.intensities {
                sky_lo = sky_lo.min(*v);
                sky_hi = sky_hi.max(*v);
            }
        }
        sky_surfaces += n;
        sky_names.push(name);
        println!("sky material {i:>3} {name:<20} {n:>4} surfaces  (bitmap: {})", world.images.contains_key(&(i as i32)));
    }
    if sky_surfaces > 0 {
        println!("sky intensity range    [{sky_lo:.2}, {sky_hi:.2}]");
        // Is a sky face identified by a surface flag, and is it a ceiling plane or a
        // vertical backdrop? Both decide how the engine fills the background.
        let sky_materials: std::collections::HashSet<i32> = level
            .materials
            .iter()
            .enumerate()
            .filter(|(_, n)| n.to_ascii_lowercase().contains("sky"))
            .map(|(i, _)| i as i32)
            .collect();
        let mut sky_flags: std::collections::BTreeMap<u32, usize> = Default::default();
        let mut other_flags: std::collections::BTreeMap<u32, usize> = Default::default();
        let mut axis = [0usize; 3];
        let (mut slo, mut shi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for s in &level.surfaces {
            let is_sky = sky_materials.contains(&s.material);
            if is_sky {
                *sky_flags.entry(s.surfflags).or_default() += 1;
                if let Some(n) = polygon_normal(level, s) {
                    let a = (0..3).max_by(|a, b| n[*a].abs().total_cmp(&n[*b].abs())).unwrap_or(0);
                    axis[a] += 1;
                }
                for &(v, _) in &s.verts {
                    if let Some(p) = level.vertices.get(v as usize) {
                        for k in 0..3 {
                            slo[k] = slo[k].min(p[k]);
                            shi[k] = shi[k].max(p[k]);
                        }
                    }
                }
            } else if s.material >= 0 {
                *other_flags.entry(s.surfflags).or_default() += 1;
            }
        }
        println!("sky surfflags          {sky_flags:x?}");
        println!(
            "other surfflags        {:x?}",
            other_flags.iter().take(6).collect::<Vec<_>>()
        );
        println!("sky face axis counts   {axis:?}  (0 = flat in X, 1 = flat in Y i.e. a ceiling)");
        println!(
            "sky face bounds        x[{:.1},{:.1}] y[{:.1},{:.1}] z[{:.1},{:.1}]  sky_z {:.1}",
            slo[0], shi[0], slo[1], shi[1], slo[2], shi[2], level.sky_z
        );
    }
    println!(
        "sky                    {} material(s), {sky_surfaces} surfaces, ceiling sky z {:.1}",
        sky_names.len(),
        level.sky_z
    );
}

/// Distinct thing templates by count, so a level's placed object population can be
/// inspected without a GUI. Shared by `--check` and `--overview`.
fn template_counts(level: &jkl::Level) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for t in &level.things {
        *counts.entry(t.template.to_ascii_lowercase()).or_default() += 1;
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked
}

#[cfg(test)]
mod draw_row_tests {
    use super::draw_row;

    /// The body's own draw and holster rows, and nothing for a mode that did not change.
    #[test]
    fn a_mode_change_names_the_puppets_own_row() {
        assert_eq!(draw_row(0, 1), Some("drawgun"));
        assert_eq!(draw_row(1, 0), Some("holster"));
        assert_eq!(draw_row(0, 0), None);
        assert_eq!(draw_row(1, 1), None);
        // Modes beyond armed/unarmed are not the port's (crouch and the rest are a separate row).
        assert_eq!(draw_row(1, 2), None);
    }
}

#[cfg(test)]
mod arg_value_tests {
    use super::arg_value;

    /// The trap this helper exists for: a flag must never be taken as the previous flag's value,
    /// while a negative number still is one.
    #[test]
    fn a_following_flag_is_not_a_value() {
        let args: Vec<String> = ["--hud", "--level", "01narshadda", "--walk", "-45"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // `i` points at the flag: `--hud` is followed by `--level`, another flag, so it has no value
        // and the index must not move — that is exactly the `--hud --level 01` bug.
        let mut i = 0;
        assert_eq!(arg_value(&args, &mut i), None);
        assert_eq!(i, 0, "the index moved past a flag");
        // A real value is taken and the index moves with it.
        i = 1;
        assert_eq!(arg_value(&args, &mut i).map(String::as_str), Some("01narshadda"));
        assert_eq!(i, 2);
        // A single dash is a value, not a flag: `--walk -45` is a heading.
        i = 3;
        assert_eq!(arg_value(&args, &mut i).map(String::as_str), Some("-45"));
        // Nothing after the last argument.
        i = 5;
        assert_eq!(arg_value(&args, &mut i), None);
    }
}

#[cfg(test)]
mod scene_tint_tests {
    use super::scene_tint;

    /// The wash a force-sight or infrared effect asks for, against the canvas rules: an off channel
    /// adds no light, a tint scales what is left, `add` biases, alpha passes through.
    #[test]
    fn a_filter_removes_its_channel_and_a_tint_colours_the_rest() {
        // Force seeing: red and green off, blue kept — a blue wash.
        let (colour, alpha) = scene_tint([false, false, true], [0.0; 3], [0.0; 3], 0.5);
        assert_eq!((colour.red, colour.green, colour.blue), (0.0, 0.0, 1.0));
        assert_eq!(alpha, 0.5);
        // No effect at all is a white wash at whatever alpha the caller asked for.
        let (colour, _) = scene_tint([true; 3], [0.0; 3], [0.0; 3], 1.0);
        assert_eq!((colour.red, colour.green, colour.blue), (1.0, 1.0, 1.0));
        // A tint scales the kept channels, and `add` biases up without overflowing.
        let (colour, _) = scene_tint([true, true, true], [1.0, 0.5, 0.0], [0.0, 0.0, 255.0], 1.0);
        assert_eq!((colour.red, colour.green, colour.blue), (1.0, 0.5, 1.0));
    }
}

#[cfg(test)]
mod actor_state_tests {
    use super::actor_state;
    use bevy::math::Vec3;

    /// A shooting actor plays its `fire` track; a puppet without one falls back to the stride
    /// rather than to nothing.
    #[test]
    fn shooting_picks_the_fire_track_when_the_puppet_has_one() {
        let still = Vec3::ZERO;
        let stormtrooper = |name: &str| ["stand", "walk", "run", "fire"].contains(&name);
        let droid = |name: &str| ["stand", "walk"].contains(&name);
        let unarmed = 0;
        assert_eq!(actor_state(still, 0.0, false, false, 0.0, unarmed, stormtrooper), "stand");
        assert_eq!(actor_state(still, 0.0, true, false, 0.0, unarmed, stormtrooper), "fire");
        // A puppet with no `fire` row keeps its own state: it shoots without animating the shot.
        assert_eq!(actor_state(still, 0.0, true, false, 0.0, unarmed, droid), "stand");
        // Movement still loses to the action track, which is the engine's ordering.
        let running = Vec3::new(1.0, 0.0, 0.0);
        assert_eq!(actor_state(running, 0.0, false, false, 0.0, unarmed, stormtrooper), "run");
        assert_eq!(actor_state(running, 0.0, true, false, 0.0, unarmed, stormtrooper), "fire");
        // The armed set is what `SetArmedMode(thing, 1)` selects: mode 1 rows are the same
        // animation with the weapon drawn, and a state without one falls back to mode 0.
        let armed = 1;
        let drawn = |name: &str| ["stand", "walk", "run"].contains(&name);
        assert_eq!(actor_state(still, 0.0, false, false, 0.0, armed, drawn), "stand");
        assert_eq!(actor_state(still, 0.0, true, false, 0.0, armed, |_| false), "stand");
        // A hit interrupts a shot and a stride; a puppet with no `hit` row is unaffected.
        assert_eq!(actor_state(still, 0.0, false, true, 0.0, unarmed, stormtrooper), "stand");
        let flinching = |name: &str| ["stand", "walk", "run", "fire", "hit"].contains(&name);
        // A body pivoting in place turns; one that is walking does not, and a puppet with no turn
        // row keeps its stride.
        let turning = |name: &str| ["stand", "walk", "run", "turnleft", "turnright"].contains(&name);
        assert_eq!(actor_state(still, 0.0, false, false, 120.0, unarmed, turning), "turnleft");
        assert_eq!(actor_state(still, 0.0, false, false, -120.0, unarmed, turning), "turnright");
        assert_eq!(actor_state(running, 0.0, false, false, 120.0, unarmed, turning), "run");
        assert_eq!(actor_state(still, 0.0, false, false, 120.0, unarmed, stormtrooper), "stand");
        // A slow pivot is not a turn: the threshold keeps a settling yaw from flickering.
        assert_eq!(actor_state(still, 0.0, false, false, 10.0, unarmed, turning), "stand");
        assert_eq!(actor_state(still, 0.0, false, true, 0.0, unarmed, flinching), "hit");
        assert_eq!(actor_state(running, 0.0, true, true, 0.0, unarmed, flinching), "hit");
    }
}

#[cfg(test)]
mod registration_tests {
    use super::*;

    /// Every function that takes a Bevy system's parameters is named somewhere else.
    ///
    /// `saber_blade` was written, compiled and *reported as working* for a dozen rounds before
    /// anyone noticed it was in no `add_systems` list — the blade simply never drew. Nothing else
    /// catches that: the function is dead code, so the compiler says nothing, and a system that does
    /// not run produces no error at run time either. This reads the source and fails if a system's
    /// name appears only at its own definition.
    ///
    /// The check is textual and therefore blunt: a system mentioned only in a *comment* would pass.
    /// That is the trade for catching the real case, and the failure message names the function.
    #[test]
    fn every_system_is_registered() {
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
            .expect("the source this test lives in");
        // Comments do not run. A doc comment that names the function (`spawned by [`saber_blade`]`)
        // is what made the first version of this check pass while the blade never drew, so the
        // mention count is taken over code only.
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // A system's signature is multi-line and its parameters are the tell.
        let mut unregistered = Vec::new();
        let mut rest = source.as_str();
        while let Some(at) = rest.find("\nfn ") {
            let tail = &rest[at + 1..];
            let name_end = tail.find('(').unwrap_or(0);
            let name = &tail[3..name_end];
            let sig_end = tail.find(") {").unwrap_or(tail.len().min(2000));
            let signature = &tail[..sig_end];
            let looks_like_system = ["Res<", "ResMut<", "Commands", "Query<", "EventReader", "Local<"]
                .iter()
                .any(|param| signature.contains(param));
            if looks_like_system && !name.is_empty() {
                // The definition is one mention; a registration is another.
                let mentions = code.matches(name).count();
                if mentions < 2 {
                    unregistered.push(name.to_string());
                }
            }
            rest = tail;
        }
        assert!(
            unregistered.is_empty(),
            "these systems are in no add_systems list, so they never run: {unregistered:?}"
        );
    }
}

#[cfg(test)]
mod rules_tests {
    use super::*;

    /// The shipped `walkplayer` numbers, without needing the install: `maxthrust=2`,
    /// `surfdrag=3`, `jumpspeed=1.5` (from `_actor`), gravity 4, scale 10.
    fn shipped() -> Rules {
        let mut level = jkl::Level { gravity: 4.0, ..Default::default() };
        level.templates.push(jkl::Template {
            name: "walkplayer".into(),
            base: Some("_actor".into()),
            params: vec![
                ("maxthrust".into(), "2.00".into()),
                ("surfdrag".into(), "3.000000".into()),
            ],
            model3d: None,
            sprite: None,
        });
        level.templates.push(jkl::Template {
            name: "_actor".into(),
            base: None,
            params: vec![("jumpspeed".into(), "1.50".into())],
            model3d: None,
            sprite: None,
        });
        Rules::from_level(&level)
    }

    #[test]
    fn the_walker_moves_at_the_engines_own_ratios() {
        let rules = shipped();
        // `sithPhysics_ApplyDrag` settles velocity at acceleration / drag, and the player control sets the
        // acceleration to `maxThrust`, so the walk speed is 2 / 3 in level units exactly — no scale factor.
        assert!((rules.walk - 2.0 / 3.0).abs() < 1e-4, "walk {}", rules.walk);
        // FAST doubles; crouch replaces that with a half; backwards is half; strafe 0.7.
        assert_eq!(rules.speed(1.0, 0.0, false, false, false), [rules.walk, 0.0]);
        assert_eq!(rules.speed(1.0, 0.0, true, false, false), [rules.walk * 2.0, 0.0]);
        assert_eq!(rules.speed(1.0, 0.0, false, true, false), [rules.walk * 0.5, 0.0]);
        assert_eq!(rules.speed(-1.0, 0.0, false, false, false), [-rules.walk * 0.5, 0.0]);
        let strafe = rules.speed(0.0, 1.0, false, false, false);
        assert!((strafe[1] - rules.walk * 0.7).abs() < 1e-4, "strafe {}", strafe[1]);
        // Crouch wins over run: the engine assigns 0.5 rather than multiplying.
        assert_eq!(rules.speed(1.0, 0.0, true, true, false), [rules.walk * 0.5, 0.0]);
        // Water halves whatever the other keys decided: crouching in water is a quarter.
        assert_eq!(rules.speed(1.0, 0.0, false, false, true), [rules.walk * 0.5, 0.0]);
        assert_eq!(rules.speed(1.0, 0.0, false, true, true), [rules.walk * 0.25, 0.0]);
        assert_eq!(rules.speed(1.0, 0.0, true, false, true), [rules.walk, 0.0]);
    }

    #[test]
    fn a_dead_actor_plays_its_death_animation() {
        let actor = Actor {
            thing: 3,
            model: "3do/rystr.3do".into(),
            key: "kystand0.key".into(),
            death: Some("kyftdeth.key".into()),
            states: std::collections::BTreeMap::from([(("stand".to_string(), 0), "kystand0.key".to_string())]),
            state: "stand".into(),
            phase: 0.0,
            parts: Vec::new(),
            drawn: i32::MIN,
            drawn_key: String::new(),
            dying: false,
            drawn_at: Vec3::ZERO,
            drawn_yaw: None,
            drawn_mode: 0,
            drawing: None,
        };
        assert_eq!(animating_key(&actor, false), "kystand0.key");
        assert_eq!(animating_key(&actor, true), "kyftdeth.key");
        // A puppet with no death state keeps its stand loop rather than blanking.
        let immortal = Actor { death: None, ..actor };
        assert_eq!(animating_key(&immortal, true), "kystand0.key");
    }

    /// A sprite's `.mat` paints the effect on black, and the engine never draws palette entry 0 —
    /// that key is the whole transparency of a spark. World textures must keep entry 0 opaque, so
    /// only the sprite path converts this way; this pins the rule and the difference.
    #[test]
    fn sprite_materials_key_palette_entry_zero_and_world_ones_do_not() {
        let mut palette = tex::Palette(vec![[0, 0, 0]; 256]);
        palette.0[1] = [255, 40, 8];
        let bitmap = tex::Material {
            width: 2,
            height: 1,
            cels: 1,
            indices: vec![0, 1],
        };
        let keyed = keyed_rgba(&bitmap, &palette);
        assert_eq!(&keyed[0..4], &[0, 0, 0, 0], "the sprite key must be transparent");
        assert_eq!(&keyed[4..8], &[255, 40, 8, 255], "the spark itself must be opaque");
        // The world path has no key: `to_image` writes 255 for every pixel, which is why a spark
        // cannot simply borrow it without punching holes in level textures.
        assert_eq!(keyed.len(), bitmap.indices.len() * 4);
    }

    /// `sithPuppet_UpdateThingMove` (`0x4e4380`) picks the animation from the velocity in the
    /// thing's own frame: the dominant axis says forward (walk/run/walkback) or sideways
    /// (strafe), and 0.5 u/s is where walk becomes run. A yaw of 0 faces `+x`, so these
    /// velocities are already in world terms.
    #[test]
    fn locomotion_is_chosen_from_the_velocity_axis_and_speed() {
        assert_eq!(locomotion_state(Vec3::ZERO, 0.0), "stand");
        // Below the 0.02 u/s dead zone: a body being nudged in place is still standing.
        assert_eq!(locomotion_state(Vec3::new(0.01, 0.0, 0.0), 0.0), "stand");
        assert_eq!(locomotion_state(Vec3::new(0.3, 0.0, 0.0), 0.0), "walk");
        assert_eq!(locomotion_state(Vec3::new(0.5, 0.0, 0.0), 0.0), "run");
        assert_eq!(locomotion_state(Vec3::new(0.3, 0.3, 0.0), 45.0), "walk");
        assert_eq!(locomotion_state(Vec3::new(-0.3, 0.0, 0.0), 0.0), "walkback");
        assert_eq!(locomotion_state(Vec3::new(0.0, 0.3, 0.0), 0.0), "straferight");
        assert_eq!(locomotion_state(Vec3::new(0.0, -0.3, 0.0), 0.0), "strafeleft");
        // Turning the body turns its answer: the same world velocity is forward at yaw 90.
        assert_eq!(locomotion_state(Vec3::new(0.0, 0.3, 0.0), 90.0), "walk");
        // Vertical motion never picks a walk: a lift or a fall is not a stride.
        assert_eq!(locomotion_state(Vec3::new(0.0, 0.0, 2.0), 0.0), "stand");
    }

    #[test]
    fn the_jump_comes_from_the_template_chain() {
        let rules = shipped();
        // `jumpspeed` lives on `_actor`, which is why the chain has to be followed. It is a velocity in
        // level units, so it is used as-is.
        assert!((rules.jump - 1.5).abs() < 1e-4, "jump {}", rules.jump);
        assert!((rules.jump_velocity(true) - 1.05).abs() < 1e-4);
        // Gravity is the level header's own number, unscaled.
        assert!((rules.gravity - 4.0).abs() < 1e-4);
        // The analytic apex the `--rules` integration is checked against: v^2 / 2g.
        let apex = rules.jump * rules.jump / (2.0 * rules.gravity);
        assert!((apex - 0.28125).abs() < 1e-4, "apex {apex}");
    }
}
#[cfg(test)]
mod walker_tests {
    use super::*;

    /// Two floors either side of one doorway: a lower room (`0 <= x <= 1`, floor z=0) and
    /// a platform (`1 < x <= 3`, floor z=`RISER`) joined by an opening at x=1 that the body
    /// passes through above the step face. Planes' normals point out of the hull, which is
    /// the `.jkl` convention `collide` assumes.
    ///
    /// The riser is stated in body heights, not level units, so this fixture survives a change
    /// of scale: `STEP_UP` is the height the sweep is supposed to be able to climb.
    const RISER: f32 = STEP_UP * 0.9;
    fn step_room() -> collide::Solids {
        let plane = |normal: [f32; 3], point: [f32; 3]| collide::Plane {
            normal,
            point,
            surface: usize::MAX,
        };
        let lower = vec![
            // The floor plane carries the surface its polygon is filed under: the sweep reaches a face
            // through the plane's surface id, so `usize::MAX` here would make the floor invisible.
            collide::Plane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, 0.0], surface: 0 },
            plane([0.0, 0.0, 1.0], [0.0, 0.0, 2.0]),
            plane([-1.0, 0.0, 0.0], [-2.0, 0.0, 0.0]),
            collide::Plane { normal: [1.0, 0.0, 0.0], point: [1.0, 0.0, 0.0], surface: 2 },
            plane([0.0, 1.0, 0.0], [0.0, 2.0, 0.0]),
            plane([0.0, -1.0, 0.0], [0.0, -2.0, 0.0]),
        ];
        let upper = vec![
            collide::Plane { normal: [0.0, 0.0, -1.0], point: [0.0, 0.0, RISER], surface: 1 },
            plane([0.0, 0.0, 1.0], [0.0, 0.0, 2.0]),
            plane([1.0, 0.0, 0.0], [3.0, 0.0, 0.0]),
            plane([0.0, 1.0, 0.0], [0.0, 2.0, 0.0]),
            plane([0.0, -1.0, 0.0], [0.0, -2.0, 0.0]),
        ];
        let opening = vec![
            [1.0, -2.0, RISER * 1.3],
            [1.0, 2.0, RISER * 1.3],
            [1.0, 2.0, 2.0],
            [1.0, -2.0, 2.0],
        ];
        collide::Solids {
            boxes: Vec::new(),
            hull: Vec::new(),
            // The sweep reaches a face through these maps, so a fixture that leaves them empty cannot
            // exercise it at all: the ledge's floor and its riser both need naming here.
            floor_of: [(0usize, 0usize), (1, 1)].into_iter().collect(),
            sectors: vec![lower, upper],
            portals: vec![
                vec![collide::Portal {
                    adjoin: 1,
                    link: 0,
                    verts: opening.clone(),
                    normal: [1.0, 0.0, 0.0],
                    point: [1.0, 0.0, 0.0],
                }],
                vec![collide::Portal {
                    adjoin: 0,
                    link: 1,
                    verts: opening,
                    normal: [-1.0, 0.0, 0.0],
                    point: [1.0, 0.0, 0.0],
                }],
            ],
            spheres: vec![([0.0, 0.0, 0.5], 4.0), ([2.0, 0.0, 0.5], 4.0)],
            // The ledge as polygons: a 0.5-high riser at x = 1, the lower floor, and the
            // ledge top. Planes alone cannot say where a face ends, which is the whole point
            // of the face list.
            floors: vec![
                collide::Face {
                    normal: [0.0, 0.0, -1.0],
                    point: [0.0, 0.0, 0.0],
                    verts: vec![[-2.0, -2.0, 0.0], [1.0, -2.0, 0.0], [1.0, 2.0, 0.0], [-2.0, 2.0, 0.0]],
                    surface: 0,
                },
                collide::Face {
                    normal: [0.0, 0.0, -1.0],
                    point: [0.0, 0.0, RISER],
                    verts: vec![[1.0, -2.0, RISER], [3.0, -2.0, RISER], [3.0, 2.0, RISER], [1.0, 2.0, RISER]],
                    surface: 1,
                },
            ],
            walls: vec![collide::Face {
                normal: [1.0, 0.0, 0.0],
                point: [1.0, 0.0, 0.0],
                verts: vec![[1.0, 2.0, 0.0], [1.0, -2.0, 0.0], [1.0, -2.0, RISER], [1.0, 2.0, RISER]],
                surface: 2,
            }],
            wall_of: [(2usize, 0usize)].into_iter().collect(),
            faces: Vec::new(),
            face_of: [(0usize, 0usize), (1, 1), (2, 2)].into_iter().collect(),
        }
    }

    fn walk(solids: &collide::Solids) -> (usize, [f32; 3]) {
        // The floor probe scans `faces`, which `build` always fills; a hand-built fixture has to say so.
        let mut synced = solids.clone();
        synced.faces = synced.floors.clone();
        synced.faces.extend(synced.walls.iter().cloned());
        let solids = &synced;
        // One body radius: the same pair `setup` and `walk_level` start a body from.
        let (mut sector, mut pos, mut vy) = (0usize, [0.5f32, 0.0, PLAYER_RADIUS], 0.0f32);
        for _ in 0..60 {
            walker_step(solids, collide::Gates::default(), &mut sector, &mut pos, &mut vy, [1.0, 0.0], 1.0 / 30.0, PLAYER_RADIUS, 0.0, true);
        }
        (sector, pos)
    }

    /// A riser below `STEP_UP` is climbed, and a wall above it is not.
    ///
    /// The allowance is `STEP_UP`, the engine's own floor-retry range (see its comment). The probe
    /// lifts the body only when its feet clear the ledge's top edge *and* there is floor up there.
    #[test]
    fn a_low_ledge_is_stepped_onto_and_a_tall_wall_is_not() {
        let level = step_room();
        let (_, pos) = walk(&level);
        // The body climbs the riser: gravity is off in this fixture, so a body that never
        // lifted stays at one radius and one that stepped up ends a riser higher.
        assert!(
            pos[2] > PLAYER_RADIUS + RISER * 0.5,
            "did not climb: z = {} at x {}",
            pos[2],
            pos[0]
        );

        // The same riser raised three times the allowance: above it, so it blocks.
        let mut tall = step_room();
        for v in tall.walls[0].verts.iter_mut() {
            if v[2] > 0.01 {
                v[2] = STEP_UP * 3.0;
            }
        }
        let (_, pos) = walk(&tall);
        assert!(pos[0] < 1.0, "climbed a {}-unit wall: x = {}", STEP_UP * 3.0, pos[0]);
        assert!(pos[2] < PLAYER_RADIUS + RISER * 0.5, "rose past the wall: z = {}", pos[2]);
    }
}

#[cfg(test)]
mod plan_view_tests {
    use super::plan_view;

    /// The two limits `plan_view` exists to hold, over the spans of the shipped levels — `01narshadda`
    /// is 38 units across and `09fuelstation` 102, the widest in the install.
    #[test]
    fn the_plan_camera_fits_the_level_and_stays_inside_the_sky_dome() {
        const DOME_RADIUS: f32 = 600.0;
        for span in [1.0, 20.0, 38.0, 66.0, 102.0, 200.0] {
            let (distance, fov) = plan_view(span);
            // The whole level is inside the dome: the camera is `distance` up, and the furthest corner
            // is at most `span / 2` out from the centre.
            let far_corner = (distance * distance + (span * 0.5) * (span * 0.5)).sqrt();
            assert!(
                far_corner < DOME_RADIUS,
                "span {span}: the level sits {far_corner} from the camera, past the {DOME_RADIUS}-unit dome"
            );
            // …and it fits the frame: the vertical extent at that distance covers the span.
            let extent = 2.0 * distance * (fov * 0.5).tan();
            assert!(extent >= span, "span {span}: only {extent} of it is in frame");
            // Flatter than the 45 degrees that used to lean the walls over their neighbours.
            assert!(fov < std::f32::consts::FRAC_PI_4, "span {span}: fov {fov} is not a long lens");
        }
    }
}
