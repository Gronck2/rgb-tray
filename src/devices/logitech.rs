//! Logitech keyboards over HID++ 2.0 (G915 TKL via LIGHTSPEED receiver or cable).
//! Feature 0x8071 (RGB effects): the firmware renders the effect itself.
//! Feature 0x8081 (per-key lighting): every key gets its own static color.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{Battery, Device, Effect, Rgb};
use crate::hid::{self, Hid};

const VID: u16 = 0x046D;
pub const RECEIVER_PIDS: [u16; 6] = [0xC539, 0xC53A, 0xC53F, 0xC541, 0xC545, 0xC547];
const LONG: u8 = 0x11;
/// Software id for requests from regular applies; the key editor uses its own
/// so responses to two concurrent connections can't be mixed up.
pub const SW_ID_APPLY: u8 = 0x0A;
pub const SW_ID_EDITOR: u8 = 0x0B;
pub const SW_ID_POLL: u8 = 0x0C;
const FEATURE_RGB_EFFECTS: u16 = 0x8071;
const FEATURE_PER_KEY: u16 = 0x8081;
const FEATURE_BATTERY_VOLTAGE: u16 = 0x1001;
const FEATURE_DEVICE_INFO: u16 = 0x0003;

/// Li-ion discharge curve used by Logitech devices: (millivolts, percent), high to low.
const VOLTAGE_CURVE: [(u16, u8); 13] =
    [(4186, 100), (4067, 90), (3989, 80), (3922, 70), (3859, 60), (3811, 50), (3778, 40), (3751, 30), (3717, 20), (3671, 10), (3646, 5), (3579, 2), (3500, 0)];

fn percent_from_millivolts(mv: u16) -> u8 {
    if mv >= VOLTAGE_CURVE[0].0 {
        return 100;
    }
    for pair in VOLTAGE_CURVE.windows(2) {
        let ((hi_mv, hi_p), (lo_mv, lo_p)) = (pair[0], pair[1]);
        if mv >= lo_mv {
            let t = (mv - lo_mv) as f32 / (hi_mv - lo_mv) as f32;
            return (lo_p as f32 + t * (hi_p - lo_p) as f32).round() as u8;
        }
    }
    0
}

const EFFECT_OFF: u16 = 0x0000;
const EFFECT_STATIC: u16 = 0x0001;
const EFFECT_CYCLE: u16 = 0x0003;
const EFFECT_BREATHE: u16 = 0x000A;

/// Effect periods at normal speed.
const BREATHE_PERIOD_MS: u16 = 4000;
const CYCLE_PERIOD_MS: u16 = 10000;

/// An effect period at the current speed setting.
fn period_ms(normal: u16) -> u16 {
    (normal as f32 / super::speed_factor(super::speed())).round().clamp(500.0, u16::MAX as f32) as u16
}

// 0x8081 functions.
const KEYS_SET_INDIVIDUAL: u8 = 1; // up to 4 × [zone, r, g, b]
const KEYS_SET_SAME_COLOR: u8 = 6; // [r, g, b] + up to 13 zones
const KEYS_FRAME_END: u8 = 7;

/// Interfaces (receiver or cable) where keyboards were found: (HID path, device id).
static SEEN: Mutex<Vec<(Vec<u16>, String)>> = Mutex::new(Vec::new());
/// At the last search a receiver's keyboard slot didn't answer (a sleeping or switched-off
/// keyboard; a receiver with nothing paired there, or with a mouse, answers or refuses).
static SILENT_RECEIVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct Logitech {
    hid: Hid,
    dev: u8,
    sw: u8,
    feat: u8,
    feat_keys: u8,
    feat_battery: u8,
    /// Effect IDs supported by each zone, indexed by effect index.
    zones: Vec<Vec<u16>>,
    /// Unique id of this keyboard ("logitech-<unit id>").
    pub id: String,
}

impl Logitech {
    /// Every connected keyboard. Each Logitech HID++ interface is tried (several receivers may
    /// be plugged in, e.g. one of a Logitech mouse); one keyboard per receiver.
    pub fn find_all(sw: u8) -> Vec<Logitech> {
        let infos = hid::enumerate(VID).into_iter().filter(|i| i.usage_page == 0xFF00 && i.usage == 0x0002);
        let mut out = Vec::new();
        let mut silent = false;
        for info in infos {
            match Self::find_on(&info, sw) {
                Ok(kb) => {
                    let mut seen = SEEN.lock().unwrap();
                    seen.retain(|(p, _)| *p != info.path);
                    seen.push((info.path.clone(), kb.id.clone()));
                    out.push(kb);
                }
                Err(no_answer) => silent |= no_answer,
            }
        }
        SILENT_RECEIVER.store(silent, std::sync::atomic::Ordering::Relaxed);
        out
    }

    /// A receiver's keyboard slot didn't answer at the last search.
    pub fn receiver_silent() -> bool {
        SILENT_RECEIVER.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// One keyboard by its id.
    pub fn find_id(sw: u8, id: &str) -> Option<Logitech> {
        Self::find_all(sw).into_iter().find(|k| k.id == id)
    }

    /// Keyboards whose interface is still plugged in though they didn't answer this time
    /// (`found`): off, asleep or out of range.
    pub fn no_link(found: &[String]) -> Vec<String> {
        let paths: Vec<Vec<u16>> = hid::enumerate(VID).into_iter().map(|i| i.path).collect();
        let seen = SEEN.lock().unwrap();
        seen.iter().filter(|(p, id)| paths.contains(p) && !found.contains(id)).map(|(_, id)| id.clone()).collect()
    }

    /// The keyboard on one interface; Err(true) when the receiver's keyboard slot didn't answer.
    fn find_on(info: &hid::HidInfo, sw: u8) -> Result<Logitech, bool> {
        let hid = Hid::open(&info.path, true, true).ok_or(false)?;
        // Through a LIGHTSPEED receiver the keyboard sits in slot 1; over cable it answers as 0xFF.
        let receiver = RECEIVER_PIDS.contains(&info.pid);
        let dev_indexes: &[u8] = if receiver { &[1, 2] } else { &[0xFF] };
        let mut silent = false;
        for &dev in dev_indexes {
            let r = match request(&hid, dev, sw, 0, 0, &FEATURE_RGB_EFFECTS.to_be_bytes()) {
                Ok(r) => r,
                Err(e) => {
                    crate::log(&format!("logitech dev {dev}: {e}"));
                    silent |= receiver && dev == 1 && e == NO_RESPONSE;
                    continue;
                }
            };
            if r[4] == 0 {
                continue;
            }
            let feat_keys = request(&hid, dev, sw, 0, 0, &FEATURE_PER_KEY.to_be_bytes()).map(|r| r[4]).unwrap_or(0);
            let feat_battery = request(&hid, dev, sw, 0, 0, &FEATURE_BATTERY_VOLTAGE.to_be_bytes()).map(|r| r[4]).unwrap_or(0);
            let feat_info = request(&hid, dev, sw, 0, 0, &FEATURE_DEVICE_INFO.to_be_bytes()).map(|r| r[4]).unwrap_or(0);
            // Different features always have different indexes; equal ones mean mixed-up answers.
            let distinct = |a: u8, b: u8| a == 0 || b == 0 || a != b;
            if !(distinct(r[4], feat_keys) && distinct(r[4], feat_battery) && distinct(feat_keys, feat_battery)) {
                crate::log(&format!("logitech: inconsistent feature indexes {} {} {}", r[4], feat_keys, feat_battery));
                return Err(false);
            }
            // 0x0003 getDeviceInfo: entity count, then the 4-byte unit id (serial).
            let unit = match feat_info {
                0 => None,
                f => request(&hid, dev, sw, f, 0, &[]).ok().map(|r| format!("{:02X}{:02X}{:02X}{:02X}", r[5], r[6], r[7], r[8])),
            };
            let id = format!("logitech-{}", unit.unwrap_or_else(|| super::path_hash(&info.path)));
            let mut kb = Logitech { hid, dev, sw, feat: r[4], feat_keys, feat_battery, zones: Vec::new(), id };
            if kb.read_zones().is_ok() && !kb.zones.is_empty() {
                return Ok(kb);
            }
            return Err(false);
        }
        Err(silent)
    }

    fn req(&self, func: u8, args: &[u8]) -> Result<[u8; 20], String> {
        request(&self.hid, self.dev, self.sw, self.feat, func, args)
    }

    fn req_keys(&self, func: u8, args: &[u8]) -> Result<[u8; 20], String> {
        if self.feat_keys == 0 {
            return Err("the keyboard has no per-key colors".into());
        }
        request(&self.hid, self.dev, self.sw, self.feat_keys, func, args)
    }

    fn read_zones(&mut self) -> Result<(), String> {
        let info = self.req(0, &[0xFF, 0xFF, 0x00])?;
        for zone in 0..info[6] {
            let zi = self.req(0, &[zone, 0xFF, 0x00])?;
            let mut ids = Vec::new();
            for e in 0..zi[8] {
                let ei = self.req(0, &[zone, e, 0x00])?;
                ids.push(u16::from_be_bytes([ei[6], ei[7]]));
            }
            self.zones.push(ids);
        }
        Ok(())
    }

    /// Switches the keyboard to per-key mode and sets every given key.
    pub fn apply_keys(&self, keys: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        // The G915 sometimes stops taking effects until it is switched off and on; the
        // sequence of commands before that shows in the log.
        crate::log(&format!("{}: per-key colors, {} keys, brightness {brightness}", self.id, keys.len()));
        self.req(5, &[0x01, 0x01])?;
        // Firmware effects must be disabled, otherwise they paint over the keys.
        for (zone, ids) in self.zones.iter().enumerate() {
            // "Off" hangs the logo (see `apply`); steady black does the same job there.
            let effect = if zone == 0 { EFFECT_STATIC } else { EFFECT_OFF };
            let idx = ids.iter().position(|&e| e == effect).unwrap_or(0);
            let mut args = vec![zone as u8, idx as u8];
            args.extend_from_slice(&[0; 10]);
            args.push(0x01);
            self.req(1, &args)?;
        }
        self.update_keys(keys, brightness)
    }

    /// Changes some keys while already in per-key mode.
    pub fn update_keys(&self, keys: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        let mut by_color: BTreeMap<(u8, u8, u8), Vec<u8>> = BTreeMap::new();
        for &(code, c) in keys {
            let Rgb(r, g, b) = c.scaled(brightness);
            by_color.entry((r, g, b)).or_default().push(code);
        }
        let mut singles = Vec::new();
        for ((r, g, b), codes) in by_color {
            // Same-color packets carry 13 keys, individual ones only 4.
            if codes.len() > 4 {
                for chunk in codes.chunks(13) {
                    let mut args = vec![r, g, b];
                    args.extend_from_slice(chunk);
                    self.req_keys(KEYS_SET_SAME_COLOR, &args)?;
                }
            } else {
                singles.extend(codes.into_iter().map(|code| [code, r, g, b]));
            }
        }
        for chunk in singles.chunks(4) {
            let args: Vec<u8> = chunk.iter().flatten().copied().collect();
            self.req_keys(KEYS_SET_INDIVIDUAL, &args)?;
        }
        self.req_keys(KEYS_FRAME_END, &[])?;
        Ok(())
    }
}

impl Device for Logitech {
    fn apply(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String> {
        crate::log(&format!("{}: {effect:?} {color:?} brightness {brightness}", self.id));
        // Steady colors and "off" are per-key colors all alike. The firmware effect engine
        // (0x8071) now and then stops showing anything it is sent, until the keyboard is
        // switched off and on; per-key colors (0x8081) keep working then (verified 11.10.2026).
        if matches!(effect, Effect::Static | Effect::Off) && self.feat_keys != 0 {
            let c = if effect == Effect::Off { Rgb(0, 0, 0) } else { color };
            let keys: Vec<(u8, Rgb)> = crate::keymap::get(crate::keymap::G915_TKL).codes().iter().map(|&code| (code, c)).collect();
            return Logitech::apply_keys(self, &keys, brightness);
        }
        // Take lighting control from the onboard mode.
        self.req(5, &[0x01, 0x01])?;
        let Rgb(r, g, b) = color.scaled(brightness);
        let (id, params): (u16, [u8; 10]) = match effect {
            Effect::Off => (EFFECT_OFF, [0; 10]),
            Effect::Static => (EFFECT_STATIC, [r, g, b, 0, 0, 0, 0, 0, 0, 0]),
            Effect::Breathe => {
                // A pre-dimmed color pulses too faintly to see at low brightness; the
                // effect has its own intensity parameter (1..=100) instead.
                let Rgb(r, g, b) = color;
                let [ph, pl] = period_ms(BREATHE_PERIOD_MS).to_be_bytes();
                (EFFECT_BREATHE, [r, g, b, ph, pl, 0, brightness.clamp(1, 100), 0, 0, 0])
            }
            Effect::Cycle => {
                let [ph, pl] = period_ms(CYCLE_PERIOD_MS).to_be_bytes();
                (EFFECT_CYCLE, [0, 0, 0, 0, 0, ph, pl, brightness.clamp(1, 100), 0, 0])
            }
        };
        for (zone, ids) in self.zones.iter().enumerate() {
            // G915 firmware bug: breathing on the logo (zone 0), or the firmware "off" effect on
            // any zone, now and then makes the keyboard ignore every later effect until it is
            // switched off and on. The logo breathes as a steady color instead, and "off" is
            // steady black everywhere.
            let (id, params) = match (zone, id) {
                (0, EFFECT_BREATHE) => (EFFECT_STATIC, [r, g, b, 0, 0, 0, 0, 0, 0, 0]),
                (_, EFFECT_OFF) => (EFFECT_STATIC, [0; 10]),
                _ => (id, params),
            };
            // Going straight from one animation to another (breathing → spectrum) also hangs the
            // firmware; passing through a steady color first doesn't.
            if id != EFFECT_STATIC {
                if let Some(idx) = ids.iter().position(|&e| e == EFFECT_STATIC) {
                    let mut args = vec![zone as u8, idx as u8, r, g, b];
                    args.extend_from_slice(&[0; 7]);
                    args.push(0x01);
                    self.req(1, &args)?;
                }
            }
            let Some(idx) = ids.iter().position(|&e| e == id) else { continue };
            let mut args = vec![zone as u8, idx as u8];
            args.extend_from_slice(&params);
            args.push(0x01);
            self.req(1, &args)?;
        }
        Ok(())
    }

    fn apply_keys(&self, keys: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        Logitech::apply_keys(self, keys, brightness)
    }

    fn update_keys(&self, keys: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        Logitech::update_keys(self, keys, brightness)
    }

    fn battery(&self) -> Option<Battery> {
        if self.feat_battery == 0 {
            return None;
        }
        // 0x1001 GetBatteryVoltage: millivolts (big endian), flags; bit 7 = charging.
        let r = request(&self.hid, self.dev, self.sw, self.feat_battery, 0, &[]).ok()?;
        let mv = u16::from_be_bytes([r[4], r[5]]);
        Some(Battery { percent: percent_from_millivolts(mv), charging: r[6] & 0x80 != 0 })
    }
}

/// Sends a long HID++ 2.0 request and waits for its response. A sleeping wireless keyboard
/// drops the packet that wakes it up, so unanswered requests are retried.
///
/// Responses only echo the feature index and function, not the arguments, so a late answer
/// to an earlier attempt would be taken for the answer to the next request (e.g. one feature
/// lookup returning the index of another). Stale input is therefore drained before every
/// request, and after a retry the late duplicate answer is swallowed.
fn request(hid: &Hid, dev: u8, sw: u8, feat: u8, func: u8, args: &[u8]) -> Result<[u8; 20], String> {
    drain(hid, Duration::ZERO);
    let mut result = request_once(hid, dev, sw, feat, func, args);
    let mut retried = false;
    for _ in 0..2 {
        match &result {
            Err(e) if e == NO_RESPONSE => {
                retried = true;
                drain(hid, Duration::ZERO);
                result = request_once(hid, dev, sw, feat, func, args);
            }
            _ => break,
        }
    }
    if retried && result.is_ok() {
        drain(hid, Duration::from_millis(600));
    }
    result
}

/// Discards pending input reports; with a non-zero `wait`, also those arriving within it.
fn drain(hid: &Hid, wait: Duration) {
    let deadline = Instant::now() + wait;
    let mut r = [0u8; 20];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match hid.read(&mut r, left.as_millis() as u32) {
            Ok(Some(_)) => continue,
            _ => break,
        }
    }
}

const NO_RESPONSE: &str = "HID++: no response";

/// Sends one request and waits for its response, skipping traffic of other apps.
fn request_once(hid: &Hid, dev: u8, sw: u8, feat: u8, func: u8, args: &[u8]) -> Result<[u8; 20], String> {
    let mut msg = [0u8; 20];
    msg[0] = LONG;
    msg[1] = dev;
    msg[2] = feat;
    msg[3] = (func << 4) | sw;
    msg[4..4 + args.len()].copy_from_slice(args);
    if crate::trace_enabled() {
        crate::log(&format!("hid++ > {:02X?}", &msg[..4 + args.len()]));
    }
    if !hid.write(&msg) {
        return Err(format!("HID++: write failed ({})", std::io::Error::last_os_error()));
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut r = [0u8; 20];
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match hid.read(&mut r, left.as_millis() as u32) {
            Err(()) => return Err("HID++: device disconnected".into()),
            Ok(None) => break,
            Ok(Some(_)) => {}
        }
        if r[0] != LONG || r[1] != dev {
            continue;
        }
        if r[2] == 0xFF && r[3] == feat && r[4] == msg[3] {
            return Err(format!("HID++: error {:#04x} (feat {feat:#04x} fn {func})", r[5]));
        }
        if r[2] == feat && r[3] == msg[3] {
            if crate::trace_enabled() {
                crate::log(&format!("hid++ < {:02X?}", &r[..8]));
            }
            return Ok(r);
        }
    }
    Err(NO_RESPONSE.into())
}

/// Calls `on_change` whenever a device connects to or loses its link with a receiver
/// (the keyboard wakes up, sleeps, is switched off), so the lighting can be re-applied and
/// the status updated. Watches every Logitech receiver; runs forever on its own threads.
pub fn spawn_watcher(on_change: impl Fn() + Send + Sync + 'static) {
    let on_change = std::sync::Arc::new(on_change);
    let watched: std::sync::Arc<Mutex<Vec<Vec<u16>>>> = Default::default();
    std::thread::spawn(move || loop {
        let shorts = hid::enumerate(VID).into_iter().filter(|i| i.usage_page == 0xFF00 && i.usage == 0x0001);
        for info in shorts {
            if watched.lock().unwrap().contains(&info.path) {
                continue;
            }
            let Some(hid) = Hid::open(&info.path, true, true) else { continue };
            watched.lock().unwrap().push(info.path.clone());
            let (on_change, watched) = (on_change.clone(), watched.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 7];
                loop {
                    match hid.read(&mut buf, 60_000) {
                        Err(()) => break,
                        Ok(None) => {}
                        // 0x41: device connection notification (bit 6 of byte 4 set = link lost).
                        Ok(Some(_)) => {
                            if buf[0] == 0x10 && buf[2] == 0x41 {
                                let lost = buf[4] & 0x40 != 0;
                                crate::log(&format!("logitech receiver: device {} {}", buf[1], if lost { "link lost" } else { "connected" }));
                                on_change();
                            }
                        }
                    }
                }
                // The receiver was unplugged.
                watched.lock().unwrap().retain(|p| *p != info.path);
            });
        }
        std::thread::sleep(Duration::from_secs(5));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_percent_from_voltage() {
        assert_eq!(percent_from_millivolts(4300), 100);
        assert_eq!(percent_from_millivolts(4186), 100);
        assert_eq!(percent_from_millivolts(3811), 50);
        assert_eq!(percent_from_millivolts(3835), 55);
        assert_eq!(percent_from_millivolts(3671), 10);
        assert_eq!(percent_from_millivolts(3500), 0);
        assert_eq!(percent_from_millivolts(3000), 0);
    }

    #[test]
    fn battery_percent_never_rises_as_voltage_drops() {
        let mut last = 100;
        for mv in (3000..4300).rev() {
            let p = percent_from_millivolts(mv);
            assert!(p <= last, "{mv} mV: {p}% after {last}%");
            last = p;
        }
    }
}
