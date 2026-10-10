//! Effects played by the app, for devices whose firmware has none (LampArray) or whose effect
//! numbers aren't known (QMK/VIA keyboards). One thread draws every running animation about 60
//! times a second; each animation owns its device handle.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Draws one frame at `t` seconds; false when the device stopped answering (the animation
/// ends; the next apply starts it again).
pub type Frame = Box<dyn FnMut(f32) -> bool + Send>;

/// Running animations, by HID path of their device.
static ANIMATIONS: Mutex<Vec<(Vec<u16>, Frame)>> = Mutex::new(Vec::new());
static THREAD: std::sync::Once = std::sync::Once::new();

/// About 60 frames a second: LEDs take whole brightness steps, and small steps at a high
/// rate look smooth where coarse ones at a low rate look like flicker.
const FRAME: Duration = Duration::from_millis(16);
const BREATHE_PERIOD: f32 = 4.0;
const CYCLE_PERIOD: f32 = 10.0;

/// Starts (or replaces) the animation of the device at `path`.
pub fn start(path: &[u16], frame: Frame) {
    let mut list = ANIMATIONS.lock().unwrap();
    list.retain(|(p, _)| p != path);
    list.push((path.to_vec(), frame));
    drop(list);
    THREAD.call_once(|| {
        std::thread::spawn(run);
    });
}

/// Stops the animation of the device at `path`, if any.
pub fn stop(path: &[u16]) {
    ANIMATIONS.lock().unwrap().retain(|(p, _)| p != path);
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
    let start = Instant::now();
    loop {
        let t = start.elapsed().as_secs_f32();
        ANIMATIONS.lock().unwrap().retain_mut(|(_, frame)| frame(t));
        std::thread::sleep(FRAME);
    }
}
