//! Effects played by the app, for devices whose firmware has none (LampArray) or whose effect
//! numbers aren't known (QMK/VIA keyboards). One thread draws every running animation about 60
//! times a second; each animation owns its device handle.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Draws one frame at `t` seconds; false when the device stopped answering (the animation
/// ends; the next apply starts it again).
pub type Frame = Box<dyn FnMut(f32) -> bool + Send>;

/// The animation of one device (by HID path). Each runs on its own effect time: real time
/// scaled by its device's speed, accumulated frame by frame so a speed change goes on smoothly
/// from the current phase. A stopped one keeps its time for the next start, so re-applying
/// the same effect doesn't jump.
struct Animation {
    path: Vec<u16>,
    speed: f32,
    t: f32,
    frame: Option<Frame>,
}

static ANIMATIONS: Mutex<Vec<Animation>> = Mutex::new(Vec::new());
static THREAD: std::sync::Once = std::sync::Once::new();

/// About 60 frames a second: LEDs take whole brightness steps, and small steps at a high
/// rate look smooth where coarse ones at a low rate look like flicker.
const FRAME: Duration = Duration::from_millis(16);
const BREATHE_PERIOD: f32 = 4.0;
const CYCLE_PERIOD: f32 = 10.0;
/// Effect time goes back to 0 here without a jump: a multiple of both periods.
const EFFECT_TIME_WRAP: f32 = 20.0;

/// Starts (or replaces) the animation of the device at `path`, at the speed set for it
/// (`super::speed`). It takes the phase of another running animation of the same speed, so
/// devices with the same speed breathe in step; otherwise it goes on from its own phase.
pub fn start(path: &[u16], frame: Frame) {
    let speed = super::speed();
    let mut list = ANIMATIONS.lock().unwrap();
    let peer = list.iter().find(|a| a.path != path && a.frame.is_some() && a.speed == speed).map(|a| a.t);
    match list.iter_mut().find(|a| a.path == path) {
        Some(a) => {
            a.t = peer.unwrap_or(a.t);
            a.speed = speed;
            a.frame = Some(frame);
        }
        None => list.push(Animation { path: path.to_vec(), speed, t: peer.unwrap_or(0.0), frame: Some(frame) }),
    }
    drop(list);
    THREAD.call_once(|| {
        std::thread::spawn(run);
    });
}

/// Stops the animation of the device at `path`, if any.
pub fn stop(path: &[u16]) {
    if let Some(a) = ANIMATIONS.lock().unwrap().iter_mut().find(|a| a.path == path) {
        a.frame = None;
    }
}

/// Lowest breathing level. The darkest part is where LEDs show their brightness steps the
/// most (a step of 1 near 0 is a big change for the eye), so it stays a little lit.
const BREATHE_FLOOR: f32 = 0.08;

/// Breathing level at `t`: BREATHE_FLOOR → 1 → BREATHE_FLOOR over BREATHE_PERIOD seconds.
pub fn breathe(t: f32) -> f32 {
    let k = 0.5 - 0.5 * (t / BREATHE_PERIOD * std::f32::consts::TAU).cos();
    BREATHE_FLOOR + (1.0 - BREATHE_FLOOR) * k
}

/// Spectrum hue at `t`, in degrees: a full circle every CYCLE_PERIOD seconds.
pub fn cycle_hue(t: f32) -> f32 {
    t / CYCLE_PERIOD * 360.0
}

fn run() {
    let mut last = Instant::now();
    loop {
        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;
        for a in ANIMATIONS.lock().unwrap().iter_mut() {
            // Wrapped at a whole number of both periods: an f32 that only grows loses its
            // fraction after a few days, and the effects speed up, then stop.
            a.t = (a.t + dt * a.speed) % EFFECT_TIME_WRAP;
            if a.frame.as_mut().is_some_and(|frame| !frame(a.t)) {
                a.frame = None;
            }
        }
        std::thread::sleep(FRAME);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breathing_keeps_its_floor() {
        assert!((breathe(0.0) - BREATHE_FLOOR).abs() < 1e-6);
        assert!((breathe(BREATHE_PERIOD / 2.0) - 1.0).abs() < 1e-6);
        assert!((breathe(BREATHE_PERIOD) - BREATHE_FLOOR).abs() < 1e-5);
        for i in 0..1000 {
            let b = breathe(i as f32 * 0.01);
            assert!((BREATHE_FLOOR - 1e-6..=1.0 + 1e-6).contains(&b));
        }
    }

    #[test]
    fn spectrum_turns_once_per_period() {
        assert_eq!(cycle_hue(0.0), 0.0);
        assert_eq!(cycle_hue(CYCLE_PERIOD / 2.0), 180.0);
        assert_eq!(cycle_hue(CYCLE_PERIOD), 360.0);
    }

    #[test]
    fn effect_time_wraps_without_a_jump() {
        assert!((breathe(EFFECT_TIME_WRAP) - breathe(0.0)).abs() < 1e-5);
        assert!(cycle_hue(EFFECT_TIME_WRAP).rem_euclid(360.0).abs() < 1e-3);
    }
}
