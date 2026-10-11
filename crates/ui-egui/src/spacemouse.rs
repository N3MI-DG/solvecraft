//! The 3D mouse in the viewport. Events come from the host's device reader
//! (`Services::spacemouse`) or the control channel (`ui.spacemouse`). Every frame the cap's
//! deflection moves the camera at a speed: sliding pans, pushing or pulling zooms, tilting turns
//! the view up or down (on over the top, with no stop) and twisting spins it about the up axis.
//! The camera has no roll, so rolling the cap does nothing. Buttons run view actions.

use serde_json::{Value, json};
use solvecraft_spacemouse::{Buttons, Decoder, Event, Motion};

use crate::SolveApp;

/// What a button can do: nothing, Fit, Home or a standard view.
pub const ACTIONS: [&str; 10] = ["none", "fit", "home", "front", "back", "top", "bottom", "left", "right", "iso"];

/// Speeds at full deflection: pan in viewport heights per second, zoom as e^ZOOM_SPEED per
/// second, tilt and spin in radians per second.
const PAN_SPEED: f64 = 4.0;
const ZOOM_SPEED: f64 = 6.0;
const TURN_SPEED: f64 = 8.0;
/// `Camera::orbit_free` turns this many radians per pixel.
const ORBIT_RAD_PER_PX: f64 = 0.008;
/// How long a control-channel motion holds without `seconds`, and at most (seconds). A device's
/// motion holds until its next report: it reports rest when the cap is let go.
const DEFAULT_HOLD: f64 = 0.5;
const MAX_HOLD: f64 = 60.0;
/// How long after the cap last moved the mouse wheel stays ignored (seconds).
const WHEEL_QUIET: f64 = 0.3;
/// The frame time of a movement's first frame (the time since the last frame is idle time then).
const FIRST_STEP: f64 = 1.0 / 60.0;
/// How quickly the frame time follows the measured ones, and their bounds (seconds).
const STEP_SMOOTHING: f64 = 0.1;
const STEP_RANGE: std::ops::RangeInclusive<f64> = 0.001..=0.1;
/// A gliding motion below this on every axis has come to rest.
const REST: f32 = 1e-3;

/// The 3D mouse's state.
#[derive(Default)]
pub struct State {
    /// The host's reader, running while the 3D mouse is turned on.
    source: Option<crate::SpacemouseEvents>,
    /// The cap's last reported deflection.
    pub motion: Motion,
    /// Until when (`SolveApp::now`) that motion applies.
    pub until: f64,
    pub buttons: Buttons,
    /// Decodes raw reports sent over the control channel.
    decoder: Decoder,
    /// The frame time the motion steps by; 0 at rest.
    step: f64,
    /// The motion the view moves by, gliding toward the shaped one.
    smoothed: Motion,
    /// When the cap last read anything but rest.
    moved_at: Option<f64>,
}

/// Whether the mouse wheel is ignored: while the 3D mouse is in use and briefly after. The
/// vendor's driver turns cap movement into wheel scrolling for apps it doesn't know, which would
/// zoom against the cap.
pub fn wheel_blocked(app: &SolveApp) -> bool {
    let s = &app.spacemouse;
    app.preferences.spacemouse && (!s.motion.is_zero() || s.moved_at.is_some_and(|t| app.now - t < WHEEL_QUIET))
}

/// Take the host reader's events since the last frame. The reader runs only while the 3D mouse
/// is turned on: turned off, it is stopped (closing the device) and the cap's state let go.
pub fn poll(app: &mut SolveApp) {
    if !app.preferences.spacemouse {
        app.spacemouse.source = None;
        app.spacemouse.motion = Motion::default();
        app.spacemouse.buttons = Buttons(0);
        app.spacemouse.smoothed = Motion::default();
        return;
    }
    if app.spacemouse.source.is_none() {
        app.spacemouse.source = app.services.spacemouse.as_ref().map(|start| start());
    }
    let events = app.spacemouse.source.as_ref().map(|f| f()).unwrap_or_default();
    receive(app, events);
}

/// Apply device events: the motion to move with, and the actions of newly pressed buttons.
/// Ignored while the 3D mouse is turned off.
pub fn receive(app: &mut SolveApp, events: impl IntoIterator<Item = Event>) {
    if !app.preferences.spacemouse {
        return;
    }
    for e in events {
        match e {
            Event::Motion(m) => {
                app.spacemouse.motion = m;
                app.spacemouse.until = f64::INFINITY;
                if !m.is_zero() {
                    app.spacemouse.moved_at = Some(app.now);
                }
            }
            Event::Buttons(b) => {
                let pressed = b.pressed_since(app.spacemouse.buttons);
                app.spacemouse.buttons = b;
                for n in 0..64 {
                    if pressed.is_down(n)
                        && let Some(a) = app.preferences.spacemouse_buttons.get(n as usize).cloned()
                    {
                        act(app, &a);
                    }
                }
            }
        }
    }
}

fn act(app: &mut SolveApp, action: &str) {
    if action != "none" && ACTIONS.contains(&action) {
        app.animate_view(action);
    }
}

/// The motion after the preferences: the dead zone taken off each axis (the rest rescaled to
/// reach 1), then the response curve `a·(1 − r) + a³·r` (0 linear, 1 cubic), which fades the
/// small stray readings a cap gives on axes it isn't pushed along while keeping full deflection
/// full. With `dominant` only the axis pushed furthest is kept.
pub fn shaped(m: Motion, dead_zone: f32, response: f32, dominant: bool) -> Motion {
    let dz = if dead_zone.is_finite() { dead_zone.clamp(0.0, 0.9) } else { 0.0 };
    let r = if response.is_finite() { response.clamp(0.0, 1.0) } else { 0.0 };
    let shape = |v: f32| {
        let v = if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
        if v.abs() <= dz {
            return 0.0;
        }
        let a = (v.abs() - dz) / (1.0 - dz);
        v.signum() * (a * (1.0 - r) + a * a * a * r)
    };
    let mut out = Motion { translation: m.translation.map(shape), rotation: m.rotation.map(shape) };
    if dominant {
        let max = out.translation.iter().chain(&out.rotation).fold(0.0f32, |m, v| m.max(v.abs()));
        let mut kept = false;
        for v in out.translation.iter_mut().chain(out.rotation.iter_mut()) {
            if !kept && max > 0.0 && v.abs() == max {
                kept = true;
            } else {
                *v = 0.0;
            }
        }
    }
    out
}

/// The shaped motion that applies now: none when the 3D mouse is off or a control-channel hold
/// has run out.
pub fn current(app: &SolveApp) -> Motion {
    let p = &app.preferences;
    if !p.spacemouse || !(app.now < app.spacemouse.until) {
        return Motion::default();
    }
    shaped(app.spacemouse.motion, p.spacemouse_dead_zone, p.spacemouse_response, p.spacemouse_dominant)
}

/// The frame time a moving view steps by: [`FIRST_STEP`] at the start of a movement, then the
/// measured times smoothed. Frames are woken by device reports, so measured times wobble while
/// the screen shows frames evenly; stepping by them raw makes the view judder.
fn step(state: &mut State, dt: f64) -> f64 {
    let dt = dt.clamp(*STEP_RANGE.start(), *STEP_RANGE.end());
    state.step = if state.step > 0.0 { state.step + (dt - state.step) * STEP_SMOOTHING } else { FIRST_STEP };
    state.step
}

/// Glide `smoothed` toward `target` over a frame: each axis covers `1 − e^(−dt/τ)` of the way (τ
/// the smoothing time; 0 goes straight there). All axes glide together, so moving several at once
/// keeps its mix.
fn glide(smoothed: Motion, target: Motion, dt: f64, tau: f32) -> Motion {
    let tau = if tau.is_finite() { f64::from(tau.clamp(0.0, 0.5)) } else { 0.0 };
    let k = if tau > 0.0 { (1.0 - (-dt / tau).exp()) as f32 } else { 1.0 };
    let mix = |s: f32, t: f32| s + (t - s) * k;
    Motion {
        translation: [0, 1, 2].map(|i| mix(smoothed.translation[i], target.translation[i])),
        rotation: [0, 1, 2].map(|i| mix(smoothed.rotation[i], target.rotation[i])),
    }
}

/// Move the camera by the 3D mouse for a frame of `dt` seconds in a viewport `h` pixels high.
/// Returns whether it moved (the view is being navigated: no hover, keep drawing).
pub fn navigate(app: &mut SolveApp, dt: f64, h: f64) -> bool {
    let target = current(app);
    if !(dt > 0.0) || !dt.is_finite() {
        return false;
    }
    if target.is_zero() && app.spacemouse.smoothed.is_zero() {
        app.spacemouse.step = 0.0;
        return false;
    }
    let dt = step(&mut app.spacemouse, dt);
    let m = glide(app.spacemouse.smoothed, target, dt, app.preferences.spacemouse_smoothing);
    if target.is_zero() && m.translation.iter().chain(&m.rotation).all(|v| v.abs() < REST) {
        app.spacemouse.smoothed = Motion::default();
        app.spacemouse.step = 0.0;
        return false;
    }
    app.spacemouse.smoothed = m;
    let p = &app.preferences;
    let sign = |i: usize| if p.spacemouse_invert.get(i).copied().unwrap_or(false) { -1.0 } else { 1.0 };
    let speed = |s: f32| f64::from(s.clamp(0.1, 5.0)) * dt;
    let (pan, zoom, turn) = (speed(p.spacemouse_pan_speed), speed(p.spacemouse_zoom_speed), speed(p.spacemouse_turn_speed));
    let [tx, ty, tz] = m.translation.map(f64::from);
    let [rx, _roll, rz] = m.rotation.map(f64::from);
    let px = TURN_SPEED * turn / ORBIT_RAD_PER_PX;
    let (pan_x, pan_y, zoom_dir, tilt, spin) = (sign(0), sign(1), sign(2), sign(3), sign(4));
    app.cancel_view_animation();
    app.viewport.spin = false;
    let cam = &mut app.cam;
    // Pan by pixels, as a mouse drag does: the model follows the cap.
    cam.pan(tx * PAN_SPEED * h * pan * pan_x, tz * PAN_SPEED * h * pan * pan_y, h);
    // +Y zooms in.
    cam.zoom_at((-ty * ZOOM_SPEED * zoom * zoom_dir).exp(), None);
    // Twist spins about the up axis, also upside down, so nothing changes direction over the top.
    cam.orbit_free(-rz * px * spin, rx * px * tilt);
    true
}

/// The control channel's `ui.spacemouse`: a motion (`tx`, `ty`, `tz`, `rx`, `ry`, `rz`, each
/// −1…1) held for `seconds` (default 0.5, at most 60), `buttons` (a bit mask), or `report` (a
/// device report's bytes, decoded as the device's are). Returns the state.
pub fn control(app: &mut SolveApp, p: &Value) -> Result<Value, String> {
    let num = |k: &str| -> Result<Option<f32>, String> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => match v.as_f64() {
                Some(x) if x.is_finite() => Ok(Some((x as f32).clamp(-1.0, 1.0))),
                _ => Err(format!("`{k}` must be a number from -1 to 1")),
            },
        }
    };
    let axes = ["tx", "ty", "tz", "rx", "ry", "rz"].map(num);
    let mut values = [0.0f32; 6];
    let mut any_axis = false;
    for (v, a) in values.iter_mut().zip(axes) {
        if let Some(x) = a? {
            *v = x;
            any_axis = true;
        }
    }
    let seconds = match p.get("seconds") {
        None | Some(Value::Null) => DEFAULT_HOLD,
        Some(v) => match v.as_f64() {
            Some(s) if s.is_finite() && s >= 0.0 => s.min(MAX_HOLD),
            _ => return Err("`seconds` must be a number of seconds, 0 or more".into()),
        },
    };
    if any_axis {
        let [tx, ty, tz, rx, ry, rz] = values;
        receive(app, [Event::Motion(Motion { translation: [tx, ty, tz], rotation: [rx, ry, rz] })]);
        app.spacemouse.until = app.now + seconds;
    }
    if let Some(b) = p.get("buttons").filter(|v| !v.is_null()) {
        let bits = b.as_u64().ok_or("`buttons` must be a bit mask (0 or more)")?;
        receive(app, [Event::Buttons(Buttons(bits))]);
    }
    if let Some(r) = p.get("report").filter(|v| !v.is_null()) {
        let list = r.as_array().ok_or("`report` must be a list of bytes")?;
        if list.len() > 64 {
            return Err("`report` is longer than 64 bytes".into());
        }
        let bytes = list
            .iter()
            .map(|b| b.as_u64().and_then(|x| u8::try_from(x).ok()))
            .collect::<Option<Vec<u8>>>()
            .ok_or("`report` must be bytes (0…255)")?;
        if let Some(e) = app.spacemouse.decoder.decode(&bytes) {
            receive(app, [e]);
        }
    }
    Ok(state(app))
}

/// The 3D mouse's state, as the control channel reports it.
pub fn state(app: &SolveApp) -> Value {
    let s = &app.spacemouse;
    json!({
        "enabled": app.preferences.spacemouse,
        "translation": s.motion.translation,
        "rotation": s.motion.rotation,
        "active": !current(app).is_zero(),
        "buttons": s.buttons.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use solvecraft_engine::Session;

    /// A steady 60 frames a second.
    const FRAME: f64 = 1.0 / 60.0;

    /// How far an angle moved, the shorter way round (headings wrap at a full turn).
    fn turn(from: f64, to: f64) -> f64 {
        (to - from + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
    }

    /// An app whose 3D mouse (turned on) moves the view by the raw mapping: no smoothing, a
    /// linear response.
    fn app() -> SolveApp {
        let mut a = SolveApp::new(Session::default(), crate::Services::default());
        a.preferences.spacemouse = true;
        a.preferences.spacemouse_smoothing = 0.0;
        a.preferences.spacemouse_response = 0.0;
        a
    }

    fn motion(t: [f32; 3], r: [f32; 3]) -> Motion {
        Motion { translation: t, rotation: r }
    }

    #[test]
    fn dead_zone_and_dominant_axis() {
        let m = motion([0.04, -0.5, 1.0], [0.0, 0.2, -0.05]);
        let s = shaped(m, 0.05, 0.0, false);
        assert_eq!(s.translation[0], 0.0, "inside the dead zone");
        assert!((s.translation[1] + 0.45 / 0.95).abs() < 1e-6, "rescaled past it");
        assert_eq!(s.translation[2], 1.0, "full deflection stays full");
        assert_eq!(s.rotation[2], 0.0);
        let d = shaped(m, 0.05, 0.0, true);
        assert_eq!(d.translation, [0.0, 0.0, 1.0]);
        assert_eq!(d.rotation, [0.0, 0.0, 0.0]);
        // Hostile values shape to nothing rather than spreading NaN.
        let h = shaped(motion([f32::NAN, f32::INFINITY, 0.0], [0.0; 3]), f32::NAN, f32::NAN, true);
        assert!(h.translation.iter().chain(&h.rotation).all(|v| v.is_finite()));
    }

    /// The response curve weakens small deflections far more than large ones; full stays full.
    #[test]
    fn response_curve_fades_small_deflections() {
        let m = motion([0.2, -0.5, 1.0], [0.0; 3]);
        let linear = shaped(m, 0.0, 0.0, false);
        let curved = shaped(m, 0.0, 0.5, false);
        let cubic = shaped(m, 0.0, 1.0, false);
        assert_eq!(linear.translation, [0.2, -0.5, 1.0]);
        assert!((curved.translation[0] - (0.1 + 0.004)).abs() < 1e-6, "{}", curved.translation[0]);
        assert!((cubic.translation[1] + 0.125).abs() < 1e-6, "keeps its sign");
        assert_eq!(cubic.translation[2], 1.0);
        // A small stray reading next to a large deliberate one: a far smaller share curved.
        let share = |s: Motion| s.translation[0] / s.translation[2];
        assert!(share(curved) < 0.6 * share(linear));
    }

    /// Smoothing: the view glides to a new reading and back to rest, all axes together, so a
    /// combined move keeps its mix of directions.
    #[test]
    fn smoothing_glides_every_axis_together() {
        let mut a = app();
        a.preferences.spacemouse_smoothing = 0.06;
        // Tilt and twist together, the twist twice the tilt.
        receive(&mut a, [Event::Motion(motion([0.0; 3], [0.25, 0.0, 0.5]))]);
        let (mut first, mut last) = (0.0, 0.0);
        for i in 0..60 {
            let (yaw, pitch) = (a.cam.yaw, a.cam.pitch);
            assert!(navigate(&mut a, FRAME, 600.0));
            let (dy, dp) = (turn(yaw, a.cam.yaw).abs(), turn(pitch, a.cam.pitch).abs());
            assert!((dy - 2.0 * dp).abs() < 1e-9, "frame {i}: the mix of directions holds ({dy} vs {dp})");
            if i == 0 {
                first = dy;
            }
            last = dy;
        }
        assert!(first < 0.5 * last, "it eases in ({first} then {last})");
        assert!((last - 0.5 * TURN_SPEED * FRAME).abs() < 1e-3 * TURN_SPEED, "and reaches full speed ({last})");
        // Let go: it eases out, then stops.
        receive(&mut a, [Event::Motion(Motion::default())]);
        let mut frames = 0;
        while navigate(&mut a, FRAME, 600.0) {
            frames += 1;
            assert!(frames < 60, "comes to rest");
        }
        assert!(frames > 2, "eases out over a few frames ({frames})");
        assert!(a.spacemouse.smoothed.is_zero());
    }

    /// The wheel is ignored while the cap moves and briefly after (the vendor's driver scrolls it
    /// as the cap moves), and works otherwise.
    #[test]
    fn the_wheel_rests_while_the_cap_moves() {
        let mut a = app();
        assert!(!wheel_blocked(&a), "never moved");
        receive(&mut a, [Event::Motion(motion([0.0, 0.3, 0.0], [0.0; 3]))]);
        assert!(wheel_blocked(&a));
        a.now += 5.0;
        assert!(wheel_blocked(&a), "still held");
        receive(&mut a, [Event::Motion(Motion::default())]);
        assert!(wheel_blocked(&a), "just let go");
        a.now += WHEEL_QUIET + 0.01;
        assert!(!wheel_blocked(&a), "quiet again");
        // With the 3D mouse turned off, the wheel always works.
        receive(&mut a, [Event::Motion(motion([0.0, 0.3, 0.0], [0.0; 3]))]);
        a.preferences.spacemouse = false;
        assert!(!wheel_blocked(&a));
    }

    #[test]
    fn twisting_spins_and_tilting_turns_up_or_down() {
        let mut a = app();
        let pitch = a.cam.pitch;
        receive(&mut a, [Event::Motion(motion([0.0; 3], [0.0, 0.0, 1.0]))]);
        // One second at full twist: TURN_SPEED radians about the up axis, none up or down.
        let mut turned = 0.0;
        for _ in 0..60 {
            let before = a.cam.yaw;
            assert!(navigate(&mut a, FRAME, 600.0));
            turned += turn(before, a.cam.yaw).abs();
        }
        assert!((turned - TURN_SPEED).abs() < 1e-6, "turned {turned}");
        assert_eq!(a.cam.pitch, pitch);
        receive(&mut a, [Event::Motion(motion([0.0; 3], [0.5, 0.0, 0.0]))]);
        let yaw = a.cam.yaw;
        assert!(navigate(&mut a, FRAME, 600.0));
        assert_eq!(a.cam.yaw, yaw);
        assert!(((a.cam.pitch - pitch).abs() - 0.5 * TURN_SPEED * FRAME).abs() < 1e-6);
    }

    /// Tilting carries the view over the top with no stop, and a twist held at the same time
    /// keeps turning the same way through straight up and down: nothing bounces back.
    #[test]
    fn tilting_turns_over_the_top_without_a_bounce() {
        let mut a = app();
        receive(&mut a, [Event::Motion(motion([0.0; 3], [1.0, 0.0, 0.2]))]);
        let (mut pitch, mut yaw) = (a.cam.pitch, a.cam.yaw);
        let mut turned = 0.0;
        let mut crossed = 0;
        // Three seconds at full tilt (past straight up or down and on) with a little twist.
        for _ in 0..180 {
            let was = a.cam.upside_down();
            assert!(navigate(&mut a, FRAME, 600.0));
            crossed += usize::from(was != a.cam.upside_down());
            let (dp, dy) = (turn(pitch, a.cam.pitch), turn(yaw, a.cam.yaw));
            assert!((dp.abs() - TURN_SPEED * FRAME).abs() < 1e-9, "a frame tilted {dp} at pitch {}", a.cam.pitch);
            assert!((dy + 0.2 * TURN_SPEED * FRAME).abs() < 1e-9, "a frame twisted {dy} at pitch {} (no bounce)", a.cam.pitch);
            turned += dp.abs();
            (pitch, yaw) = (a.cam.pitch, a.cam.yaw);
        }
        assert!((turned - 3.0 * TURN_SPEED).abs() < 1e-6);
        assert!(crossed >= 2, "went over the top and on ({crossed} crossings)");
    }

    #[test]
    fn sliding_pans_and_pushing_zooms() {
        let mut a = app();
        let (target, distance) = (a.cam.target, a.cam.distance);
        receive(&mut a, [Event::Motion(motion([1.0, 0.0, 0.0], [0.0; 3]))]);
        assert!(navigate(&mut a, FRAME, 600.0));
        assert!((a.cam.target - target).len() > 0.0);
        assert_eq!(a.cam.distance, distance, "panning doesn't zoom");
        receive(&mut a, [Event::Motion(motion([0.0, 1.0, 0.0], [0.0; 3]))]);
        let target = a.cam.target;
        assert!(navigate(&mut a, FRAME, 600.0));
        assert!(((a.cam.distance / distance).ln().abs() - ZOOM_SPEED * FRAME).abs() < 1e-6, "zoomed by e^{}", (a.cam.distance / distance).ln());
        assert!((a.cam.target - target).len() < 1e-9, "zooming doesn't pan");
        // Reversed in Preferences: the other way.
        let d = a.cam.distance;
        a.preferences.spacemouse_invert[2] = true;
        assert!(navigate(&mut a, FRAME, 600.0));
        assert!((a.cam.distance - distance).abs() < 1e-6 * distance.max(1.0), "back to {distance}, at {}", a.cam.distance);
        assert!(a.cam.distance != d);
    }

    /// The first frame of a movement steps one frame's worth, not the idle time before it, and
    /// uneven frame times (frames woken by the device's reports) step evenly.
    #[test]
    fn movements_start_with_one_frame_and_step_evenly() {
        let mut a = app();
        receive(&mut a, [Event::Motion(motion([0.0; 3], [0.0, 0.0, 1.0]))]);
        let yaw = a.cam.yaw;
        // The app was idle for 0.8 s before this frame.
        assert!(navigate(&mut a, 0.8, 600.0));
        assert!((turn(yaw, a.cam.yaw).abs() - TURN_SPEED * FIRST_STEP).abs() < 1e-9, "the first frame turned {}", turn(yaw, a.cam.yaw));
        // Frame times wobbling between 14 and 19 ms: each step stays within a hair of 16.7 ms.
        for i in 0..120 {
            let yaw = a.cam.yaw;
            assert!(navigate(&mut a, if i % 2 == 0 { 0.014 } else { 0.019 }, 600.0));
            let turned = turn(yaw, a.cam.yaw).abs() / TURN_SPEED;
            assert!((turned - 0.0166).abs() < 0.0007, "frame {i} stepped {turned} s");
        }
        // At rest, then moving again: one frame again.
        receive(&mut a, [Event::Motion(Motion::default())]);
        assert!(!navigate(&mut a, FRAME, 600.0));
        receive(&mut a, [Event::Motion(motion([0.0; 3], [0.0, 0.0, 1.0]))]);
        let yaw = a.cam.yaw;
        assert!(navigate(&mut a, 2.0, 600.0));
        assert!((turn(yaw, a.cam.yaw).abs() - TURN_SPEED * FIRST_STEP).abs() < 1e-9);
    }

    #[test]
    fn rest_off_and_expired_holds_move_nothing() {
        let mut a = app();
        let cam = a.cam;
        assert!(!navigate(&mut a, 0.1, 600.0), "at rest");
        receive(&mut a, [Event::Motion(motion([0.0; 3], [0.0, 0.0, 1.0]))]);
        a.preferences.spacemouse = false;
        assert!(!navigate(&mut a, 0.1, 600.0), "turned off");
        assert_eq!(a.cam.yaw, cam.yaw);
        a.preferences.spacemouse = true;
        // A device's motion holds however long the cap is held still (no reports come), until
        // the rest report.
        a.now += 30.0;
        assert!(navigate(&mut a, 0.1, 600.0), "held still: still turning");
        receive(&mut a, [Event::Motion(Motion::default())]);
        assert!(!navigate(&mut a, 0.1, 600.0), "let go");
        // A motion from the control channel holds for its seconds.
        control(&mut a, &json!({"rz": 1, "seconds": 0.2})).unwrap();
        assert!(navigate(&mut a, 0.1, 600.0));
        a.now += 0.21;
        assert!(!navigate(&mut a, 0.1, 600.0), "the hold ran out");
        // A hostile frame time moves nothing either.
        receive(&mut a, [Event::Motion(motion([0.0; 3], [0.0, 0.0, 1.0]))]);
        assert!(!navigate(&mut a, f64::NAN, 600.0));
        assert!(!navigate(&mut a, -1.0, 600.0));
        assert!(a.cam.is_valid());
    }

    #[test]
    fn moving_stops_an_animation_and_a_spin() {
        let mut a = app();
        a.animate_view("top");
        a.viewport.spin = true;
        receive(&mut a, [Event::Motion(motion([0.0; 3], [0.0, 0.0, 0.3]))]);
        assert!(navigate(&mut a, 1.0 / 60.0, 600.0));
        assert!(a.cam_anim.is_none() && !a.viewport.spin);
    }

    #[test]
    fn buttons_run_their_actions_once_per_press() {
        let mut a = app();
        // View changes apply at once (no animation), so the camera shows each action.
        a.preferences.look_at_anim = false;
        a.preferences.spacemouse_buttons = vec!["top".into(), "front".into()];
        receive(&mut a, [Event::Buttons(Buttons(0b01))]);
        assert!(a.cam.pitch > 1.5, "button 1 turned to the top view (pitch {})", a.cam.pitch);
        // Held down, it doesn't repeat.
        a.cam.pitch = 0.3;
        receive(&mut a, [Event::Buttons(Buttons(0b01))]);
        assert_eq!(a.cam.pitch, 0.3);
        // The second button pressed while the first is held runs its own action.
        receive(&mut a, [Event::Buttons(Buttons(0b11))]);
        assert_eq!((a.cam.yaw, a.cam.pitch), (0.0, 0.0), "button 2 turned to the front view");
        // A button with no action in the list does nothing.
        a.cam.pitch = 0.3;
        receive(&mut a, [Event::Buttons(Buttons(0)), Event::Buttons(Buttons(1 << 40))]);
        assert_eq!(a.cam.pitch, 0.3);
    }

    #[test]
    fn control_drives_it_like_the_device() {
        let mut a = app();
        let s = control(&mut a, &json!({"rz": 0.5, "seconds": 2})).unwrap();
        assert_eq!(s["active"], json!(true));
        assert_eq!(s["rotation"][2], json!(0.5));
        // A raw report, decoded as the device's are: report 1 with all six axes.
        let s = control(&mut a, &json!({"report": [1, 0, 0, 0x5E, 1, 0, 0, 0, 0, 0, 0, 0, 0]})).unwrap();
        assert_eq!(s["translation"][1], json!(1.0));
        let s = control(&mut a, &json!({"report": [3, 2]})).unwrap();
        assert_eq!(s["buttons"], json!(2));
        // Hostile parameters are refused, and change nothing.
        for bad in [
            json!({"tx": "fast"}),
            json!({"seconds": -1}),
            json!({"seconds": "NaN"}),
            json!({"buttons": -3}),
            json!({"report": "01 02"}),
            json!({"report": [1, 300]}),
            json!({"report": vec![1; 65]}),
        ] {
            assert!(control(&mut a, &bad).is_err(), "{bad}");
        }
        // Out-of-range axes are clamped; seconds are capped.
        let s = control(&mut a, &json!({"tx": 1e9, "seconds": 1e9})).unwrap();
        assert_eq!(s["translation"][0], json!(1.0));
        assert!(a.spacemouse.until <= a.now + MAX_HOLD);
    }

    /// Turned off, the 3D mouse does nothing: its buttons run no actions, its motion moves nothing.
    #[test]
    fn turned_off_its_buttons_and_motion_do_nothing() {
        let mut a = app();
        a.preferences.spacemouse = false;
        a.preferences.look_at_anim = false;
        let cam = a.cam;
        receive(&mut a, [Event::Buttons(Buttons(0b11))]);
        receive(&mut a, [Event::Motion(motion([0.5; 3], [0.5; 3]))]);
        assert!(!navigate(&mut a, 0.1, 600.0));
        assert_eq!((a.cam.yaw, a.cam.pitch, a.cam.distance), (cam.yaw, cam.pitch, cam.distance));
        assert_eq!(a.spacemouse.buttons, Buttons(0), "a press while off isn't held over");
        assert!(control(&mut a, &json!({"report": [3, 1]})).is_ok_and(|s| s["buttons"] == json!(0)));
    }

    /// The host's reader runs only while the 3D mouse is turned on: started once when it is,
    /// stopped (dropped) when it's turned off, with the cap's state let go.
    #[test]
    fn the_reader_runs_only_while_turned_on() {
        use std::cell::Cell;
        use std::rc::Rc;
        struct Stopped(Rc<Cell<u32>>);
        impl Drop for Stopped {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        let (started, stopped) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let mut a = app();
        let (s, d) = (started.clone(), stopped.clone());
        a.services.spacemouse = Some(Box::new(move || {
            s.set(s.get() + 1);
            let guard = Stopped(d.clone());
            Box::new(move || {
                let _ = &guard;
                vec![Event::Motion(motion([0.0; 3], [0.0, 0.0, 0.5]))]
            })
        }));
        a.preferences.spacemouse = false;
        poll(&mut a);
        assert_eq!(started.get(), 0, "off: not started");
        a.preferences.spacemouse = true;
        poll(&mut a);
        poll(&mut a);
        assert_eq!((started.get(), stopped.get()), (1, 0), "on: started once");
        assert!(!a.spacemouse.motion.is_zero(), "its events arrive");
        a.preferences.spacemouse = false;
        poll(&mut a);
        assert_eq!(stopped.get(), 1, "off: stopped");
        assert!(a.spacemouse.motion.is_zero(), "and let go");
        a.preferences.spacemouse = true;
        poll(&mut a);
        assert_eq!(started.get(), 2, "on again: started again");
    }
}
