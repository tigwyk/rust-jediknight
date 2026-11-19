//! The engine's control read and the player's movement assembly, transliterated.
//!
//! The spec is the decompiled engine: the per-key frame bookkeeping `stdControl_ReadControls`
//! (`decomp/0042e560_stdControl_ReadControls.c`) and its callees `stdControl_keyidk`
//! (`decomp/0042ee20`), `stdControl_ReadKey` (`decomp/0042eba0`) and `stdControl_ReadKeyAsAxis`
//! (`decomp/0042ead0`); the binding reads `sithControl_GetKey` (`decomp/004d7010`) and
//! `sithControl_GetKeyAsAxisNormalized` (`decomp/004d6d70`); and the movement assembly
//! `sithControl_PlayerMovement` (`decomp/004d8a00`). Constants are read from the shipped image's
//! `.rdata`, cited by virtual address at each use.
//!
//! The engine's devices are DirectInput (buffered keyboard with event timestamps, immediate
//! keyboard state, joystick buttons through `stdControl_keyidk`). The port has one polled device
//! per frame — Bevy's keyboard, the walk harness's held-forward hook and the replay's input rows —
//! so edges enter through `stdControl_keyidk` with the frame's own timestamp, which is exactly how
//! the engine's joystick buttons enter (`ReadControls` passes `DAT_0055d5ec` as `keyidk`'s
//! timestamp). A device that knew the press instant would pass it and get the same partial-frame
//! ramp the buffered keyboard gets.
//!
//! What the caller owns per frame, in the engine's own order (`sithUpdate`, `0x4c4c00`):
//! `begin_frame` (the `ReadControls` prologue), the device edges (`poll`/`keyidk`), then
//! `player_movement` before the physics step consumes the thrust.

/// `stdControl`'s key array length: `0x11c` dwords cleared per frame at `0x42e560:34`.
pub const KEY_SLOTS: usize = 284;

/// The engine's control indices, in the binding order `tools/parity/control-names.txt` records
/// from `JK.EXE`'s string table (`sithControl_RegisterKeyboardBindings`, `0x4d7d40`).
pub const CTRL_FORWARD: usize = 0;
pub const CTRL_SLIDE: usize = 2;
pub const CTRL_SLIDETOGGLE: usize = 3;
pub const CTRL_JUMP: usize = 4;
pub const CTRL_DUCK: usize = 5;
pub const CTRL_FAST: usize = 6;
pub const CTRL_SLOW: usize = 7;

/// The binding slots this port's devices press. Live play binds the keyboard; the walk harness
/// binds its held-forward hook; the replay binds its rows' forward/strafe magnitudes as the
/// binding sensitivities, which is the engine's own per-binding scale (the `puVar3[2]` multiply
/// in `GetKeyAsAxisNormalized`, `0x4d6d70:39`).
pub const KEY_FORWARD: usize = 200;
pub const KEY_FORWARD_BACK: usize = 201;
pub const KEY_SLIDE_LEFT: usize = 202;
pub const KEY_SLIDE_RIGHT: usize = 203;
pub const KEY_JUMP: usize = 204;
pub const KEY_DUCK: usize = 205;
pub const KEY_FAST: usize = 206;
pub const KEY_SLOW: usize = 207;

/// One binding: a key slot, a sign and a sensitivity, the row `sithControl_BindControl`
/// (`0x4d6940`) writes at `0x833b48` (stride `0x19`: count, then `{device, flags, scale}` per
/// binding — flags bit 2 negates, the scale multiplies).
#[derive(Clone, Copy, Debug)]
pub struct Binding {
    pub key: usize,
    pub negate: bool,
    pub sensitivity: f32,
}

impl Binding {
    pub fn key(key: usize) -> Self {
        Binding { key, negate: false, sensitivity: 1.0 }
    }

    /// A binding at a magnitude: the sign is the negate flag (the engine's encoding of backing
    /// up and counter-sliding — the axis clamps after the sign), the magnitude the sensitivity.
    pub fn scaled(key: usize, magnitude: f32) -> Self {
        let negate = magnitude < 0.0;
        Binding { key, negate, sensitivity: magnitude.abs() }
    }
}

/// `stdControl`'s per-key frame state: the down array `0x55cce0`, the held-this-frame
/// milliseconds `0x55c870`, the press counts `0x55d158`, and the frame's clock readings
/// `0x55d5ec` (now) and `0x55d5f8` (elapsed).
#[derive(Debug)]
pub struct StdControl {
    down: [bool; KEY_SLOTS],
    held_ms: [u32; KEY_SLOTS],
    presses: [u32; KEY_SLOTS],
    now_ms: u32,
    elapsed_ms: u32,
}

impl Default for StdControl {
    fn default() -> Self {
        StdControl {
            down: [false; KEY_SLOTS],
            held_ms: [0; KEY_SLOTS],
            presses: [0; KEY_SLOTS],
            now_ms: 0,
            elapsed_ms: 0,
        }
    }
}

impl StdControl {
    /// `stdControl_ReadControls`' prologue (`0x42e560:34-56`): the hold and press arrays are
    /// zeroed every frame, the clock is read (`timeGetTime` into `0x55d5d5ec`) and the elapsed
    /// milliseconds since the last frame stored (`0x55d5f8`). The axis scale
    /// `_DAT_0053eee4 = _DAT_00522928 / elapsed` is folded into [`Self::read_key_as_axis`].
    pub fn begin_frame(&mut self, now_ms: u32) {
        for slot in &mut self.held_ms {
            *slot = 0;
        }
        for slot in &mut self.presses {
            *slot = 0;
        }
        self.elapsed_ms = now_ms.wrapping_sub(self.now_ms);
        self.now_ms = now_ms;
    }

    /// `stdControl_keyidk` (`0x42ee20`): one button edge. A press latches the key down, credits
    /// the milliseconds the press precedes the frame's read (`now - timestamp`) and counts the
    /// press; a release drops the key and credits the frame fraction the key was still held
    /// (`elapsed + (timestamp - now)` — zero at the frame's own timestamp).
    pub fn keyidk(&mut self, key: usize, down: bool, timestamp_ms: u32) {
        if down {
            if !self.down[key] {
                let held = self.now_ms.wrapping_sub(timestamp_ms);
                self.down[key] = true;
                self.held_ms[key] = held;
                self.presses[key] += 1;
            }
        } else if self.down[key] {
            self.down[key] = false;
            if self.held_ms[key] == 0 {
                self.held_ms[key] = self.elapsed_ms;
            }
            self.held_ms[key] = self.held_ms[key].wrapping_add(timestamp_ms.wrapping_sub(self.now_ms));
        }
    }

    /// Apply one device's polled state for the frame, deriving the edges. The engine's immediate
    /// keyboard path (`0x42e560:87-113`, the `GetDeviceState` fallback) applies state exactly this
    /// way: a key newly down gets hold 0 and a press count, a key newly up keeps a full frame of
    /// hold credit.
    pub fn poll(&mut self, key: usize, down: bool) {
        self.keyidk(key, down, self.now_ms);
    }

    /// `stdControl_ReadKey` (`0x42eba0`): the key's down state, accumulating this frame's press
    /// count into `presses` the way the engine's second argument does.
    pub fn read_key(&self, key: usize, presses: &mut u32) -> bool {
        *presses += self.presses[key];
        self.down[key]
    }

    /// `stdControl_ReadKeyAsAxis` (`0x42ead0`): the time-corrected axis in `[0, 1]`.
    ///
    /// The engine's scale is `_DAT_00522928` = **1.0** (read at VA 0x522928), so the axis is the
    /// held fraction of the frame: a key held through the whole frame reads `held_ms == 0` with
    /// the key down — the comma-operator preload at `0x42ead0:15-16` credits the full elapsed
    /// time — a key pressed mid-frame reads its partial hold (clamped to the elapsed time at
    /// `0x42ead0:20-22`), and a key that is up reads `_DAT_00522930` = **0.0** (VA 0x522930). The
    /// release frame keeps one full frame of credit through `keyidk`'s `elapsed` fallback, which
    /// is the hold-drop: the frame the key comes up still moves at full strength, the next frame
    /// does not.
    pub fn read_key_as_axis(&self, key: usize) -> f32 {
        let mut held = self.held_ms[key];
        let down = self.down[key];
        if held == 0 && !down {
            return 0.0;
        }
        if held == 0 {
            held = self.elapsed_ms;
        }
        if self.elapsed_ms == 0 {
            return if down { 1.0 } else { 0.0 };
        }
        if self.elapsed_ms <= held {
            held = self.elapsed_ms;
        }
        (held as f32) * (1.0 / self.elapsed_ms as f32)
    }

    /// `sithControl_GetKey` (`0x4d7010`): the OR of a control's bound keys, with the press counts
    /// accumulated the engine's way (the caller's out-parameter — `sithControl_Update`'s
    /// `GetKey(4, &local_4)` is the jump edge).
    pub fn get_key(&self, control: usize, bindings: &[Binding], presses: &mut u32) -> bool {
        let _ = control;
        let mut any = false;
        for binding in bindings {
            if self.read_key(binding.key, presses) {
                any = true;
            }
        }
        any
    }

    /// `sithControl_GetKeyAsAxisNormalized` (`0x4d6d70`): the summed, clamped axis of a control.
    /// Per binding: the time-corrected key axis, negated under the binding's sign flag
    /// (`0x4d6d70:36-38`) and scaled by its sensitivity (`0x4d6d70:39-41`, the sentinel
    /// `_DAT_00522ef8` = 0.0 at VA 0x522ef8 meaning "unscaled"), each clamped to
    /// `[_DAT_00522efc, _DAT_00522f00]` = **[-1, 1]** (VAs 0x522efc/0x522f00) before
    /// accumulating, and the sum clamped the same way (`0x4d6d70:55-60`).
    pub fn get_key_as_axis_normalized(&self, control: usize, bindings: &[Binding]) -> f32 {
        let _ = control;
        let mut sum = 0.0f32;
        for binding in bindings {
            let mut axis = self.read_key_as_axis(binding.key);
            if axis != 0.0 {
                if binding.negate {
                    axis = -axis;
                }
                if binding.sensitivity != 0.0 {
                    axis *= binding.sensitivity;
                }
                axis = axis.clamp(-1.0, 1.0);
                sum += axis;
            }
        }
        sum.clamp(-1.0, 1.0)
    }
}


/// `sithControl_PlayerMovement`'s output (`0x4d8a00`): the thrust the physics update consumes
/// and the input reads the caller maps onto its own movement state.
#[derive(Clone, Copy, Debug, Default)]
pub struct MovementIntent {
    /// The forward thrust (thing `+0x1f0`) and the slide thrust (thing `+0x1ec`), already scaled
    /// by the speed multiplier — body-frame accelerations the physics update rotates by the
    /// thing's orientation (`rdMatrix_TransformVector34Acc` at `0x4f6860:133`).
    pub thrust_forward: f32,
    pub thrust_slide: f32,
    /// The forward control's axis (`GetKeyAsAxisNormalized(0)`), for callers that express the
    /// walk as a settled speed: the engine's thrust is `base * axis * multiplier`.
    pub axis_forward: f32,
    /// The slide control's axis, before the 0.7 scale.
    pub axis_slide: f32,
    /// The speed multiplier the ladder produced (`param_1`).
    pub multiplier: f32,
    /// `GetKey(4)`'s press count: `sithPlayerActions_JumpWithVel(thing, 1.0)` fires on an edge.
    pub jump: bool,
    /// `GetKey(5)`'s state: writes physflag `0x10000`; the caller applies it to the thing.
    pub duck: bool,
}

/// `sithControl_PlayerMovement` (`0x4d8a00`), movement half. The turn assembly (the
/// `+0x1e4`/`+0x14c` writes and the SLIDETOGGLE branch's key turn) is the look path; this port
/// turns the body outside the movement step, so only its input reads are shared.
///
/// `base_thrust` is the engine's `thing+0x144 + thing+0x148` sum — the thrust base no recovered
/// template parser produces (the deviation ledger's "player walking" row); this port derives it
/// from its measured settled walk so the terminal speed is unchanged. In the engine the physics
/// update decays the thrust fields by `_DAT_005231a8` = **0.3** (VA 0x5231a8) before integrating
/// them (`0x4f6860:111-115`), and this function rewrites the fields every control frame, so the
/// decay is a constant 0.3 scale on the base the caller has already absorbed.
///
/// `always_run` is the engine's runtime config bit `DAT_008bd020 & 2` (`0x4d8a00:21`): set, plain
/// forward movement runs at the 2.0 scale. `slow_water` stands in for the physflag `0x200000`
/// branch (`0x4d8a00:39`) — the halved speed state the caller's thing carries.
pub fn player_movement(
    control: &StdControl,
    forward: &[Binding],
    slide: &[Binding],
    jump: &[Binding],
    duck: &[Binding],
    fast: &[Binding],
    slow: &[Binding],
    base_thrust: f32,
    always_run: bool,
    slow_water: bool,
) -> MovementIntent {
    // `0x4d8a00:19-30`: the speed multiplier starts at 1.0; without the always-run bit the FAST
    // key must be held for the 2.0 run scale, with it the scale is unconditional. The SLOW key
    // multiplies by `_DAT_00522f1c` = **0.5** (VA 0x522f1c).
    let mut multiplier = 1.0f32;
    let fast_down = control.get_key(CTRL_FAST, fast, &mut 0);
    if always_run || fast_down {
        multiplier = 2.0;
    }
    let slow_down = control.get_key(CTRL_SLOW, slow, &mut 0);
    if slow_down {
        multiplier *= 0.5;
    }
    // `0x4d8a00:31-38`: DUCK replaces the multiplier with 0.5 and writes physflag 0x10000 (set on
    // hold, cleared on release).
    let duck_down = control.get_key(CTRL_DUCK, duck, &mut 0);
    if duck_down {
        multiplier = 0.5;
    }
    // `0x4d8a00:39-41`: physflag 0x200000 halves whatever is left.
    if slow_water {
        multiplier = 0.5;
    }
    // The slide and forward thrusts (`0x4d8a00:62-87`): the slide axis scales the base by
    // `_DAT_00522f24` = **0.7** (VA 0x522f24); the forward thrust halves under
    // `_DAT_00522f08` = **0.0** (VA 0x522f08) — backing up is half thrust. The `+0x1f4`
    // vertical thrust is zeroed (`0x4d8a00:93`), which the caller's jump carries instead.
    let slide_axis = control.get_key_as_axis_normalized(CTRL_SLIDE, slide);
    let forward_axis = control.get_key_as_axis_normalized(CTRL_FORWARD, forward);
    let mut thrust_slide = base_thrust * slide_axis * 0.7;
    let mut thrust_forward = base_thrust * forward_axis;
    if forward_axis <= 0.0 {
        thrust_forward *= 0.5;
    }
    // `0x4d8a00:92-97`: a multiplier other than `_DAT_00522f14` = **1.0** (VA 0x522f14) scales
    // both thrusts.
    if multiplier != 1.0 {
        thrust_forward *= multiplier;
        thrust_slide *= multiplier;
    }
    // `0x4d8a00:98-101`: the jump fires on JUMP's press count, not its level.
    let mut presses = 0u32;
    let jump = control.get_key(CTRL_JUMP, jump, &mut presses) && presses != 0;
    MovementIntent {
        thrust_forward,
        thrust_slide,
        axis_forward: forward_axis,
        axis_slide: slide_axis,
        multiplier,
        jump,
        duck: duck_down,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctrl: &mut StdControl, ms: u32) {
        let now = ctrl.now_ms + ms;
        ctrl.begin_frame(now);
    }

    #[test]
    fn keyidk_counts_presses_and_holds_partial_frames() {
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        // A press halfway into the frame (timestamp 8 ms in) credits 8 ms of hold.
        ctrl.keyidk(KEY_FORWARD, true, 8);
        assert_eq!(ctrl.presses[KEY_FORWARD], 1);
        assert_eq!(ctrl.held_ms[KEY_FORWARD], 8);
        assert_eq!(ctrl.read_key_as_axis(KEY_FORWARD), 8.0 / 16.0);
    }

    #[test]
    fn a_key_held_through_the_frame_reads_the_full_axis() {
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.keyidk(KEY_FORWARD, true, 16);
        assert_eq!(ctrl.held_ms[KEY_FORWARD], 0);
        assert_eq!(ctrl.read_key_as_axis(KEY_FORWARD), 1.0);
        // The next frame sees no events: hold stays 0, the key stays down, the axis is full.
        frame(&mut ctrl, 16);
        assert_eq!(ctrl.read_key_as_axis(KEY_FORWARD), 1.0);
    }

    #[test]
    fn the_release_frame_keeps_one_full_frame_of_credit_then_drops() {
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.keyidk(KEY_FORWARD, true, 16);
        frame(&mut ctrl, 16);
        ctrl.keyidk(KEY_FORWARD, false, 32);
        assert!(!ctrl.down[KEY_FORWARD]);
        // keyidk's elapsed fallback: the release frame still reads full.
        assert_eq!(ctrl.read_key_as_axis(KEY_FORWARD), 1.0);
        frame(&mut ctrl, 16);
        // The frame after the release reads zero (`_DAT_00522930`).
        assert_eq!(ctrl.read_key_as_axis(KEY_FORWARD), 0.0);
    }

    #[test]
    fn a_partial_press_clamps_to_the_frame() {
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        // A timestamp before the frame began cannot credit more than the frame.
        ctrl.keyidk(KEY_FORWARD, true, 0);
        assert_eq!(ctrl.read_key_as_axis(KEY_FORWARD), 1.0);
    }

    #[test]
    fn normalized_sums_negates_scales_and_clamps() {
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.poll(KEY_FORWARD, true);
        ctrl.poll(KEY_SLIDE_RIGHT, true);
        let bindings = [Binding::key(KEY_FORWARD), Binding::scaled(KEY_SLIDE_RIGHT, 0.2)];
        let axis = ctrl.get_key_as_axis_normalized(CTRL_FORWARD, &bindings);
        assert!((axis - 1.0).abs() < 1e-6, "the sensitivity 0.2 binding clamps after summing");
        let negated = [Binding::scaled(KEY_FORWARD, -1.0)];
        assert!((ctrl.get_key_as_axis_normalized(CTRL_FORWARD, &negated) + 1.0).abs() < 1e-6);
    }

    fn harness_bindings() -> [Vec<Binding>; 6] {
        [
            vec![Binding::key(KEY_FORWARD)],
            vec![Binding::key(KEY_SLIDE_RIGHT)],
            vec![Binding::key(KEY_JUMP)],
            vec![Binding::key(KEY_DUCK)],
            vec![Binding::key(KEY_FAST)],
            vec![Binding::key(KEY_SLOW)],
        ]
    }

    #[test]
    fn plain_forward_is_the_base_and_backing_up_halves() {
        let [fwd, slide, jump, duck, fast, slow] = harness_bindings();
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.poll(KEY_FORWARD, true);
        let intent = player_movement(&ctrl, &fwd, &slide, &jump, &duck, &fast, &slow, 12.0, false, false);
        assert!((intent.thrust_forward - 12.0).abs() < 1e-6);
        assert_eq!(intent.thrust_slide, 0.0);
        // Backing up (a negated forward binding) is half thrust (`_DAT_00522f08` = 0.0 gate).
        let back = [Binding::scaled(KEY_FORWARD, -1.0)];
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.poll(KEY_FORWARD, true);
        let intent = player_movement(&ctrl, &back, &slide, &jump, &duck, &fast, &slow, 12.0, false, false);
        assert!((intent.thrust_forward + 6.0).abs() < 1e-6);
    }

    #[test]
    fn always_run_doubles_slow_halves_and_duck_replaces() {
        let [fwd, slide, jump, duck, fast, slow] = harness_bindings();
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.poll(KEY_FORWARD, true);
        let intent = player_movement(&ctrl, &fwd, &slide, &jump, &duck, &fast, &slow, 12.0, true, false);
        assert!((intent.thrust_forward - 24.0).abs() < 1e-6);
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.poll(KEY_FORWARD, true);
        ctrl.poll(KEY_SLOW, true);
        let intent = player_movement(&ctrl, &fwd, &slide, &jump, &duck, &fast, &slow, 12.0, true, false);
        assert!((intent.thrust_forward - 12.0).abs() < 1e-6);
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.poll(KEY_FORWARD, true);
        ctrl.poll(KEY_DUCK, true);
        let intent = player_movement(&ctrl, &fwd, &slide, &jump, &duck, &fast, &slow, 12.0, true, false);
        assert!((intent.thrust_forward - 6.0).abs() < 1e-6);
        assert!(intent.duck);
        // The slow-water physflag (0x200000) halves whatever is left.
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.poll(KEY_FORWARD, true);
        let intent = player_movement(&ctrl, &fwd, &slide, &jump, &duck, &fast, &slow, 12.0, false, true);
        assert!((intent.thrust_forward - 6.0).abs() < 1e-6);
    }

    #[test]
    fn the_fast_key_runs_without_the_config_bit() {
        let [fwd, slide, jump, duck, fast, slow] = harness_bindings();
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.poll(KEY_FORWARD, true);
        ctrl.poll(KEY_FAST, true);
        let intent = player_movement(&ctrl, &fwd, &slide, &jump, &duck, &fast, &slow, 12.0, false, false);
        assert!((intent.thrust_forward - 24.0).abs() < 1e-6);
    }

    #[test]
    fn the_slide_thrust_scales_by_seven_tenths() {
        let [fwd, slide, jump, duck, fast, slow] = harness_bindings();
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.poll(KEY_SLIDE_RIGHT, true);
        let intent = player_movement(&ctrl, &fwd, &slide, &jump, &duck, &fast, &slow, 12.0, false, false);
        assert!((intent.thrust_slide - 12.0 * 0.7).abs() < 1e-6);
    }

    #[test]
    fn the_jump_fires_on_the_press_edge_only() {
        let [fwd, slide, jump, duck, fast, slow] = harness_bindings();
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.poll(KEY_JUMP, true);
        assert!(player_movement(&ctrl, &fwd, &slide, &jump, &duck, &fast, &slow, 12.0, false, false).jump);
        frame(&mut ctrl, 16);
        assert!(!player_movement(&ctrl, &fwd, &slide, &jump, &duck, &fast, &slow, 12.0, false, false).jump);
    }

    #[test]
    fn a_scaled_binding_walks_at_partial_strength() {
        let [fwd, _, jump, duck, fast, slow] = harness_bindings();
        let slide = vec![Binding::scaled(KEY_SLIDE_RIGHT, 0.2)];
        let mut ctrl = StdControl::default();
        frame(&mut ctrl, 16);
        ctrl.poll(KEY_SLIDE_RIGHT, true);
        let intent = player_movement(&ctrl, &fwd, &slide, &jump, &duck, &fast, &slow, 12.0, false, false);
        assert!((intent.thrust_slide - 12.0 * 0.7 * 0.2).abs() < 1e-6);
        assert!((intent.axis_slide - 0.2).abs() < 1e-6);
    }
}
