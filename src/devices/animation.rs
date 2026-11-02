//! Effects played by the app, for devices whose firmware has none (LampArray) or whose effect
//! numbers aren't known (QMK/VIA keyboards). One thread draws every running animation about 60
//! times a second; each animation owns its device handle.

use std::sync::{Condvar, Mutex, MutexGuard};
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
    running: bool,
    /// The frame function; None while stopped, and while the frame thread draws with it.
    frame: Option<Frame>,
    /// The frame thread is drawing a frame of it, outside the lock.
    busy: bool,
}

/// Every animation started so far (entries are kept, with their time, when they stop).
struct Animations(Vec<Animation>);

/// A frame taken out of the list to be drawn without holding the lock: (entry, time, frame).
type Taken = (usize, f32, Frame);

impl Animations {
    /// Starts or replaces the animation at `path`, taking the phase of another running one of
    /// the same speed. Its frame must not be out (`busy`).
    fn start(&mut self, path: &[u16], speed: f32, frame: Frame) {
        let peer = self.0.iter().find(|a| a.path != path && a.running && a.speed == speed).map(|a| a.t);
        match self.0.iter_mut().find(|a| a.path == path) {
            Some(a) => {
                a.t = peer.unwrap_or(a.t);
                a.speed = speed;
                a.running = true;
                a.frame = Some(frame);
            }
            None => self.0.push(Animation { path: path.to_vec(), speed, t: peer.unwrap_or(0.0), running: true, frame: Some(frame), busy: false }),
        }
    }

    /// Stops the animation at `path`; its frame must not be out (`busy`).
    fn stop(&mut self, path: &[u16]) -> Option<Frame> {
        let a = self.0.iter_mut().find(|a| a.path == path)?;
        a.running = false;
        a.frame.take()
    }

    /// The frame thread is drawing a frame of the animation at `path`.
    fn busy(&self, path: &[u16]) -> bool {
        self.0.iter().any(|a| a.path == path && a.busy)
    }

    /// Some animation has a frame to draw.
    fn has_frames(&self) -> bool {
        self.0.iter().any(|a| a.frame.is_some())
    }

    /// Moves every running animation `dt` seconds on and takes out its frame to be drawn.
    fn take(&mut self, dt: f32) -> Vec<Taken> {
        let mut out = Vec::new();
        for (i, a) in self.0.iter_mut().enumerate() {
            let Some(frame) = a.frame.take() else { continue };
            // Wrapped at a whole number of both periods: an f32 that only grows loses its
            // fraction after a few days, and the effects speed up, then stop.
            a.t = (a.t + dt * a.speed) % EFFECT_TIME_WRAP;
            a.busy = true;
            out.push((i, a.t, frame));
        }
        out
    }

    /// Puts a drawn frame back; one whose device stopped answering (`alive` false) ends.
    fn put_back(&mut self, i: usize, frame: Frame, alive: bool) {
        let a = &mut self.0[i];
        a.busy = false;
        if alive {
            a.frame = Some(frame);
        } else {
            a.running = false;
        }
    }
}

static ANIMATIONS: Mutex<Animations> = Mutex::new(Animations(Vec::new()));
/// Signalled when an animation starts (the frame thread waits for one) and when frames are put
/// back (`start` and `stop` wait for a frame of their device being drawn).
static CHANGED: Condvar = Condvar::new();
static THREAD: std::sync::Once = std::sync::Once::new();

/// About 60 frames a second: LEDs take whole brightness steps, and small steps at a high
/// rate look smooth where coarse ones at a low rate look like flicker.
const FRAME: Duration = Duration::from_millis(16);
const BREATHE_PERIOD: f32 = 4.0;
const CYCLE_PERIOD: f32 = 10.0;
/// Effect time goes back to 0 here without a jump: a multiple of both periods.
const EFFECT_TIME_WRAP: f32 = 20.0;

/// The list, once no frame of the device at `path` is being drawn: after `stop` returns, no
/// old frame can paint over what the caller sends next.
fn idle(path: &[u16]) -> MutexGuard<'static, Animations> {
    CHANGED.wait_while(ANIMATIONS.lock().unwrap(), |list| list.busy(path)).unwrap()
}

/// Starts (or replaces) the animation of the device at `path`, at the speed set for it
/// (`super::speed`). It takes the phase of another running animation of the same speed, so
/// devices with the same speed breathe in step; otherwise it goes on from its own phase.
pub fn start(path: &[u16], frame: Frame) {
    let speed = super::speed();
    let mut list = idle(path);
    list.start(path, speed, frame);
    drop(list);
    CHANGED.notify_all();
    THREAD.call_once(|| {
        std::thread::spawn(run);
    });
}

/// Stops the animation of the device at `path`, if any.
pub fn stop(path: &[u16]) {
    let old = idle(path).stop(path);
    // Closes the old frame's device handle outside the lock.
    drop(old);
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

/// The frame thread. Frames are drawn without holding the lock, so a slow device delays only
/// the frame rate, not `start`/`stop` of other devices; it sleeps while nothing animates.
fn run() {
    let mut last = Instant::now();
    loop {
        let mut list = ANIMATIONS.lock().unwrap();
        if !list.has_frames() {
            list = CHANGED.wait_while(list, |list| !list.has_frames()).unwrap();
            // Time doesn't run for stopped animations.
            last = Instant::now();
        }
        let now = Instant::now();
        let taken = list.take((now - last).as_secs_f32());
        last = now;
        drop(list);
        let drawn: Vec<(usize, Frame, bool)> = taken
            .into_iter()
            .map(|(i, t, mut frame)| {
                let alive = frame(t);
                (i, frame, alive)
            })
            .collect();
        let mut list = ANIMATIONS.lock().unwrap();
        for (i, frame, alive) in drawn {
            list.put_back(i, frame, alive);
        }
        drop(list);
        CHANGED.notify_all();
        // To a deadline: the time a frame took isn't added to the wait.
        if let Some(wait) = (now + FRAME).checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
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

    #[test]
    fn frames_are_taken_out_and_put_back() {
        let (a, b) = ([1u16, 0], [2u16, 0]);
        let mut list = Animations(Vec::new());
        assert!(!list.has_frames());
        list.start(&a, 1.0, Box::new(|_| true));
        list.start(&b, 2.0, Box::new(|_| false));
        let taken = list.take(0.5);
        assert_eq!(taken.iter().map(|f| (f.0, f.1)).collect::<Vec<_>>(), [(0, 0.5), (1, 1.0)]);
        // While drawn, the frames are out and their devices busy.
        assert!(!list.has_frames() && list.busy(&a) && list.busy(&b));
        for (i, t, mut frame) in taken {
            let alive = frame(t);
            list.put_back(i, frame, alive);
        }
        // The one whose device stopped answering has ended; the other goes on.
        assert!(!list.busy(&a) && !list.busy(&b));
        assert!(list.0[0].frame.is_some() && list.0[0].running);
        assert!(list.0[1].frame.is_none() && !list.0[1].running);
        // Time wraps at a whole number of periods.
        let taken = list.take(EFFECT_TIME_WRAP);
        assert_eq!(taken.len(), 1);
        assert!((taken[0].1 - 0.5).abs() < 1e-4);
    }

    #[test]
    fn a_start_takes_the_phase_of_a_running_peer_of_the_same_speed() {
        let (a, b, c) = ([1u16, 0], [2u16, 0], [3u16, 0]);
        let mut list = Animations(Vec::new());
        list.start(&a, 1.0, Box::new(|_| true));
        for (i, t, frame) in list.take(3.0) {
            list.put_back(i, frame, t > 0.0);
        }
        list.start(&b, 1.0, Box::new(|_| true));
        list.start(&c, 2.0, Box::new(|_| true));
        assert_eq!(list.0[1].t, 3.0);
        assert_eq!(list.0[2].t, 0.0);
        // Stopped ones lend their time to nobody...
        assert!(list.stop(&a).is_some());
        assert!(list.stop(&a).is_none());
        list.stop(&b);
        list.start(&[4, 0], 1.0, Box::new(|_| true));
        assert_eq!(list.0[3].t, 0.0);
        // ...and go on from it when started again with no running peer.
        list.stop(&[4, 0]);
        list.start(&a, 1.0, Box::new(|_| true));
        assert_eq!(list.0[0].t, 3.0);
    }
}
