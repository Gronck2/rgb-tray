//! Logitech keyboards over HID++ 2.0 (G915 TKL via LIGHTSPEED receiver or cable).
//! Feature 0x8071 (RGB effects): the firmware renders the effect itself.
//! Feature 0x8081 (per-key lighting): every key gets its own static color.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use super::{Battery, Device, Effect, Rgb};
use crate::hid::{self, Hid, Patience};

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
// Byte layouts as in OpenRGB (LogitechG915Controller) and g810-led, which agree on them.
const EFFECT_WAVE: u16 = 0x0004;
const EFFECT_RIPPLE: u16 = 0x000B;

/// Effect periods at normal speed.
const BREATHE_PERIOD_MS: u16 = 4000;
const CYCLE_PERIOD_MS: u16 = 10000;
const WAVE_PERIOD_MS: u16 = 5000;
/// Ripple speed: ms per step, 2 (fast) ..= 200 (slow).
const RIPPLE_STEP_MS: f32 = 20.0;

/// An effect period at the current speed setting.
fn period_ms(normal: u16) -> u16 {
    (normal as f32 / super::speed()).round().clamp(500.0, u16::MAX as f32) as u16
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
    path: Vec<u16>,
    dev: u8,
    sw: u8,
    feat: u8,
    feat_keys: u8,
    feat_battery: u8,
    /// Effect IDs supported by each zone, indexed by effect index (empty when found by `probe_all`).
    pub zones: Vec<Vec<u16>>,
    /// Unique id of this keyboard ("logitech-<unit id>").
    pub id: String,
}

impl Logitech {
    /// Every connected keyboard. Each Logitech HID++ interface is tried (several receivers may
    /// be plugged in, e.g. one of a Logitech mouse); one keyboard per receiver.
    pub fn find_all(sw: u8) -> Vec<Logitech> {
        Self::search(sw, true)
    }

    /// Every connected keyboard, found by feature lookups only: its lighting zones aren't read
    /// (dozens of requests), so it can't be lit. For battery polls and link checks.
    pub fn probe_all(sw: u8) -> Vec<Logitech> {
        Self::search(sw, false)
    }

    fn search(sw: u8, zones: bool) -> Vec<Logitech> {
        let infos = hid::enumerate(VID).into_iter().filter(|i| i.usage_page == 0xFF00 && i.usage == 0x0002);
        let mut out = Vec::new();
        let mut silent = false;
        for info in infos {
            match Self::find_on(&info, sw, zones) {
                Ok(kb) => {
                    let mut seen = SEEN.lock().unwrap();
                    seen.retain(|(p, _)| *p != info.path);
                    seen.push((info.path.clone(), kb.id.clone()));
                    let mut driven = DRIVEN.lock().unwrap();
                    if !driven.contains(&(info.path.clone(), kb.dev)) {
                        driven.push((info.path.clone(), kb.dev));
                    }
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

    /// The keyboard on one interface, with its lighting zones read if `zones`; Err(true) when the
    /// receiver's keyboard slot didn't answer.
    fn find_on(info: &hid::HidInfo, sw: u8, zones: bool) -> Result<Logitech, bool> {
        let hid = Hid::open(&info.path, true, true).ok_or(false)?;
        // Through a LIGHTSPEED receiver the keyboard sits in slot 1; over cable it answers as 0xFF.
        let receiver = RECEIVER_PIDS.contains(&info.pid);
        let dev_indexes: &[u8] = if receiver { &[1, 2] } else { &[0xFF] };
        let mut silent = false;
        for &dev in dev_indexes {
            // One short try: retrying a sleeping keyboard's slots took ~3 s on every search. An
            // awake keyboard answers in ~30 ms. A waking one doesn't announce itself (no
            // connection notification), so the link watch looks for it every few seconds.
            let r = match request_within(&hid, dev, sw, 0, 0, &FEATURE_RGB_EFFECTS.to_be_bytes(), LOOKUP_WAIT) {
                Ok(r) => r,
                Err(e) => {
                    // The link watch's frequent probes of a sleeping keyboard aren't news.
                    if zones {
                        crate::log(&format!("logitech dev {dev}: {e}"));
                    }
                    silent |= receiver && dev == 1 && e == NO_RESPONSE;
                    continue;
                }
            };
            if r[4] == 0 {
                continue;
            }
            let feat_keys = request(&hid, dev, sw, 0, 0, &FEATURE_PER_KEY.to_be_bytes()).map(|r| r[4]).unwrap_or(0);
            // Effects alone (e.g. a mouse in this slot) aren't this keyboard: it has per-key colors.
            if feat_keys == 0 {
                continue;
            }
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
            let mut kb = Logitech { hid, path: info.path.clone(), dev, sw, feat: r[4], feat_keys, feat_battery, zones: Vec::new(), id };
            if !zones || (kb.read_zones().is_ok() && !kb.zones.is_empty()) {
                return Ok(kb);
            }
            return Err(false);
        }
        Err(silent)
    }

    fn req(&self, func: u8, args: &[u8]) -> Result<[u8; 20], String> {
        request(&self.hid, self.dev, self.sw, self.feat, func, args)
    }

    /// Sends any firmware effect to a zone with raw parameter bytes, for trying out effects
    /// and parameter layouts on the hardware (`--logitech-effect-test`). A steady color goes
    /// first, as in `apply`, since going from one animation straight to another hangs the G915.
    pub fn raw_effect(&self, zone: usize, effect: u16, params: &[u8; 10]) -> Result<(), String> {
        let ids = self.zones.get(zone).ok_or(format!("no zone {zone}"))?;
        let idx = ids.iter().position(|&e| e == effect).ok_or(format!("zone {zone} has no effect {effect:#06X}"))?;
        self.req(5, &[0x01, 0x01])?;
        self.static_first(zone, ids, Rgb(params[0], params[1], params[2]))?;
        self.set_zone_effect(zone, idx, params)
    }

    /// 0x8071 SetZoneEffect: the effect at `idx` in the zone's list, with its parameters.
    fn set_zone_effect(&self, zone: usize, idx: usize, params: &[u8; 10]) -> Result<(), String> {
        let mut args = [0u8; 13];
        args[0] = zone as u8;
        args[1] = idx as u8;
        args[2..12].copy_from_slice(params);
        args[12] = 0x01;
        self.req(1, &args).map(|_| ())
    }

    /// A steady color on the zone, if it has the static effect: going straight from one
    /// animation to another hangs the G915, passing through a steady color first doesn't.
    fn static_first(&self, zone: usize, ids: &[u16], color: Rgb) -> Result<(), String> {
        match ids.iter().position(|&e| e == EFFECT_STATIC) {
            Some(idx) => self.set_zone_effect(zone, idx, &steady(color)),
            None => Ok(()),
        }
    }

    /// 0x8071 SWControl with raw bytes: [1 (set), control flags (bit 0 all clusters, bit 1 power
    /// modes), event flags]; returns the flags now in force. For `--logitech-battery-led-test`.
    pub fn sw_control(&self, args: &[u8]) -> Result<[u8; 20], String> {
        self.req(5, args)
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
            // A zone without it is left alone: index 0 would be whatever effect it lists first.
            let Some(idx) = ids.iter().position(|&e| e == effect) else { continue };
            self.set_zone_effect(zone, idx, &[0; 10])?;
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
    fn hid_path(&self) -> Option<&[u16]> {
        Some(&self.path)
    }

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
        let scaled = color.scaled(brightness);
        let Rgb(r, g, b) = scaled;
        let (id, params): (u16, [u8; 10]) = match effect {
            Effect::Off => (EFFECT_OFF, [0; 10]),
            Effect::Static => (EFFECT_STATIC, steady(scaled)),
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
            Effect::Wave => {
                // Period low byte, direction (1 = horizontal), intensity, period high byte.
                let [ph, pl] = period_ms(WAVE_PERIOD_MS).to_be_bytes();
                (EFFECT_WAVE, [0, 0, 0, 0, 0, 0, pl, 0x01, brightness.clamp(1, 100), ph])
            }
            Effect::Ripple => {
                let step = (RIPPLE_STEP_MS / super::speed()).round().clamp(2.0, 200.0) as u16;
                let [sh, sl] = step.to_be_bytes();
                (EFFECT_RIPPLE, [r, g, b, 0, sh, sl, 0, 0, 0, 0])
            }
            // Not in this firmware (the panel doesn't offer them): the color, steadily.
            Effect::Reactive | Effect::Starlight => (EFFECT_STATIC, steady(scaled)),
        };
        let [ch, cl] = period_ms(CYCLE_PERIOD_MS).to_be_bytes();
        for (zone, ids) in self.zones.iter().enumerate() {
            // G915 firmware bug: breathing on the logo (zone 0), or the firmware "off" effect on
            // any zone, now and then makes the keyboard ignore every later effect until it is
            // switched off and on. The logo breathes as a steady color instead, and "off" is
            // steady black everywhere.
            let (id, params) = match (zone, id) {
                (0, EFFECT_BREATHE) => (EFFECT_STATIC, steady(scaled)),
                (_, EFFECT_OFF) => (EFFECT_STATIC, [0; 10]),
                // A zone without wave (the logo) cycles along; one without ripple shows the color.
                (_, EFFECT_WAVE) if !ids.contains(&EFFECT_WAVE) => (EFFECT_CYCLE, [0, 0, 0, 0, 0, ch, cl, brightness.clamp(1, 100), 0, 0]),
                (_, EFFECT_RIPPLE) if !ids.contains(&EFFECT_RIPPLE) => (EFFECT_STATIC, steady(scaled)),
                _ => (id, params),
            };
            // Going straight from one animation to another (breathing → spectrum) also hangs the
            // firmware; passing through a steady color first doesn't.
            if id != EFFECT_STATIC {
                self.static_first(zone, ids, scaled)?;
            }
            let Some(idx) = ids.iter().position(|&e| e == id) else { continue };
            self.set_zone_effect(zone, idx, &params)?;
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

// ---------------------------------------------------------------- batteries of other devices

/// Software id of the battery reads of devices the app doesn't light, and the one of `--system-batteries` (another
/// process: its answers mustn't be taken for the tray app's).
pub const SW_ID_OTHERS: u8 = 0x0D;
pub const SW_ID_REPORT: u8 = 0x0E;
const FEATURE_NAME: u16 = 0x0005;
const FEATURE_UNIFIED_BATTERY: u16 = 0x1004;
const FEATURE_ADC: u16 = 0x1F20;
/// The HID++ collection of Logitech headsets (G535: `046D:0AC4`, its own dongle, device index 0xFF).
const HEADSET_PAGE: (u16, u16) = (0xFF43, 0x0202);

/// What a HID++ device says its type is (0x0005 getDeviceType), headset for the headset collection.
fn class_of(kind: Option<u8>, headset: bool) -> crate::sysbat::SysClass {
    use crate::sysbat::SysClass;
    match kind {
        Some(0 | 2) => SysClass::Keyboard,
        Some(3..=5) => SysClass::Mouse,
        Some(8) => SysClass::Headset,
        Some(11 | 12) => SysClass::Gamepad,
        _ if headset => SysClass::Headset,
        _ => SysClass::Other,
    }
}

/// The charge from a 0x1001 (voltage), 0x1004 (unified battery, `caps` = the flags byte of its getCapabilities)
/// or 0x1F20 (ADC voltage) answer.
fn percent_of(feature: u16, r: &[u8; 20], caps: Option<u8>) -> Option<u8> {
    match feature {
        // getStatus: byte 4 = state of charge, byte 5 = level bitmask. Without the state-of-charge capability (bit 1)
        // byte 4 is 0, not a level: map the bitmask as Solaar does. Unreadable capabilities: a nonzero byte 4 counts.
        FEATURE_UNIFIED_BATTERY => {
            if caps.map_or(r[4] > 0, |c| c & 0x02 != 0) {
                (r[4] <= 100).then_some(r[4])
            } else {
                [(8, 90), (4, 50), (2, 20), (1, 5)].into_iter().find(|&(bit, _)| r[5] & bit != 0).map(|(_, p)| p)
            }
        }
        // 0x1F20: bit 0 of the flags = a battery is there (as Solaar reads it).
        FEATURE_ADC if r[6] & 0x01 == 0 => None,
        _ => Some(percent_from_millivolts(u16::from_be_bytes([r[4], r[5]]))),
    }
}

/// The name a HID++ device gives (0x0005: length, then up to 16 characters per request).
fn name_of(hid: &Hid, dev: u8, sw: u8, feat: u8) -> Option<String> {
    let len = request_tries(hid, dev, sw, feat, 0, &[], 1).ok()?[4] as usize;
    let mut name = Vec::new();
    while name.len() < len.min(64) {
        let r = request_tries(hid, dev, sw, feat, 1, &[name.len() as u8], 1).ok()?;
        let part: Vec<u8> = r[4..].iter().copied().take(len - name.len()).take_while(|&b| b != 0).collect();
        if part.is_empty() {
            break;
        }
        name.extend(part);
    }
    let name = short_name(String::from_utf8_lossy(&name).trim());
    (!name.is_empty()).then_some(name)
}

/// "G502 LIGHTSPEED Wireless Gaming Mouse" → "G502 LIGHTSPEED": the generic words at the end don't fit the overlay
/// and say nothing its icon doesn't. The first word always stays.
fn short_name(name: &str) -> String {
    const GENERIC: [&str; 6] = ["wireless", "gaming", "mouse", "headset", "keyboard", "receiver"];
    let mut words: Vec<&str> = name.split_whitespace().collect();
    while words.len() > 1 && words.last().is_some_and(|w| GENERIC.contains(&w.to_lowercase().as_str())) {
        words.pop();
    }
    words.join(" ")
}

/// One device on a HID++ collection: Ok(None) when there is none with a battery there (or it is a keyboard, which the
/// app's own drivers cover), Err when it didn't answer (asleep, off, out of range, or the receiver busy with another request).
/// Never called for a slot the app drives (`other_batteries` skips those before any request).
fn other_battery(hid: &Hid, info: &hid::HidInfo, dev: u8, headset: bool, sw: u8) -> Result<Option<crate::sysbat::SysDevice>, String> {
    // The feature's index, None when the device hasn't it.
    let lookup = |f: u16| request_tries(hid, dev, sw, 0, 0, &f.to_be_bytes(), 2).map(|r| Some(r[4]).filter(|&i| i != 0));
    let mut battery = None;
    for f in [FEATURE_UNIFIED_BATTERY, FEATURE_BATTERY_VOLTAGE, FEATURE_ADC] {
        if let Some(index) = lookup(f)? {
            battery = Some((f, index));
            break;
        }
    }
    let Some((feature, index)) = battery else { return Ok(None) };
    let unified = feature == FEATURE_UNIFIED_BATTERY;
    // 0x1004 getCapabilities (function 0), byte 5 = flags: tells whether getStatus carries a state of charge.
    let caps = if unified { request_tries(hid, dev, sw, index, 0, &[], 1).ok().map(|r| r[5]) } else { None };
    let answer = request_tries(hid, dev, sw, index, if unified { 1 } else { 0 }, &[], 2)?;
    let Some(percent) = percent_of(feature, &answer, caps) else { return Ok(None) };
    // The name and type are nice to have: a failed read doesn't drop the level.
    let name_feat = lookup(FEATURE_NAME).ok().flatten();
    let name = name_feat.and_then(|f| name_of(hid, dev, sw, f)).unwrap_or_else(|| if headset { "Logitech headset".into() } else { "Logitech device".into() });
    let kind = name_feat.and_then(|f| request_tries(hid, dev, sw, f, 2, &[], 1).ok()).map(|r| r[4]);
    // A keyboard (the G915 or any other) is not for this list.
    if matches!(kind, Some(0 | 2)) {
        return Ok(None);
    }
    Ok(Some(crate::sysbat::SysDevice { id: format!("system-{}{dev:02X}", super::path_hash(&info.path)), name, class: class_of(kind, headset), percent }))
}

/// How long a device that stopped answering keeps its last level while its receiver or dongle is still plugged in:
/// a mouse asleep in the middle of a session shouldn't drop out of the overlay.
const OTHERS_HOLD: Duration = Duration::from_secs(10 * 60);

/// The last level read from a device: (collection path, device index, the device, when it was read).
type LastRead = (Vec<u16>, u8, crate::sysbat::SysDevice, std::time::Instant);
/// The last levels read, to show while a device doesn't answer.
static OTHERS_LAST: Mutex<Vec<LastRead>> = Mutex::new(Vec::new());

/// Receivers other than LIGHTSPEED (Unifying, Bolt, Nano): not read (a receiver answers a cabled-device request with a
/// HID++ 1.0 error, 2 x 500 ms lost). Mice and headsets on them are not covered.
const OTHER_RECEIVER_PIDS: [u16; 6] = [0xC52B, 0xC52F, 0xC531, 0xC532, 0xC534, 0xC548];

/// Slots the app's keyboard search found a keyboard in: (collection path, device index). Never sent anything by the
/// battery reads of other devices.
static DRIVEN: Mutex<Vec<(Vec<u16>, u8)>> = Mutex::new(Vec::new());
/// Slots the battery reads of other devices found nothing for (no battery, or a keyboard): not asked again while plugged in.
static OTHERS_SKIP: Mutex<Vec<(Vec<u16>, u8)>> = Mutex::new(Vec::new());

/// Held entries (a device that didn't answer) without those a fresh read of this pass has the name of: a mouse on its
/// charging cable answers on the cable while its receiver slot is still held.
fn without_fresh(fresh: &[crate::sysbat::SysDevice], held: Vec<crate::sysbat::SysDevice>) -> Vec<crate::sysbat::SysDevice> {
    held.into_iter().filter(|h| !fresh.iter().any(|f| f.name == h.name)).collect()
}

/// Batteries of Logitech HID++ devices the app doesn't light: mice on a LIGHTSPEED receiver or cable, headsets. Only feature
/// lookups and battery and name reads; for the system devices of the overlay (`sysbat`), read on its worker thread.
/// Nothing is sent to a slot the app's own keyboard search found a keyboard in. `sw` = the software id of the caller.
/// A device that doesn't answer keeps its last level for `OTHERS_HOLD`; one whose collection is gone drops out.
pub fn other_batteries(sw: u8) -> Vec<crate::sysbat::SysDevice> {
    let (mut fresh, mut held) = (Vec::new(), Vec::new());
    let infos: Vec<hid::HidInfo> = hid::enumerate(VID)
        .into_iter()
        .filter(|i| (matches!((i.usage_page, i.usage), (0xFF00, 0x0002)) || (i.usage_page, i.usage) == HEADSET_PAGE) && !OTHER_RECEIVER_PIDS.contains(&i.pid))
        .collect();
    let mut last = OTHERS_LAST.lock().unwrap();
    last.retain(|(path, ..)| infos.iter().any(|i| i.path == *path));
    OTHERS_SKIP.lock().unwrap().retain(|(path, _)| infos.iter().any(|i| i.path == *path));
    for info in &infos {
        let headset = (info.usage_page, info.usage) == HEADSET_PAGE;
        let mut hid = None;
        let devs: &[u8] = if RECEIVER_PIDS.contains(&info.pid) { &[1, 2] } else { &[0xFF] };
        for &dev in devs {
            let pair = (info.path.clone(), dev);
            if DRIVEN.lock().unwrap().contains(&pair) || OTHERS_SKIP.lock().unwrap().contains(&pair) {
                continue;
            }
            let hid = hid.get_or_insert_with(|| Hid::open(&info.path, true, true));
            let read = hid.as_ref().ok_or_else(|| "cannot open".to_string()).and_then(|h| other_battery(h, info, dev, headset, sw));
            let at = last.iter().position(|(p, d, ..)| *p == info.path && *d == dev);
            if matches!(read, Ok(None)) {
                OTHERS_SKIP.lock().unwrap().push(pair);
            }
            match (read, at) {
                (Ok(Some(d)), at) => {
                    let entry = (info.path.clone(), dev, d.clone(), std::time::Instant::now());
                    match at {
                        Some(i) => last[i] = entry,
                        None => last.push(entry),
                    }
                    fresh.push(d);
                }
                (Err(e), Some(i)) if last[i].3.elapsed() < OTHERS_HOLD => {
                    crate::log(&format!("battery of {}: {e}; showing the last level", last[i].2.name));
                    held.push(last[i].2.clone());
                }
                // Not a device of ours any more, or silent for too long.
                (_, Some(i)) => {
                    last.remove(i);
                }
                // Nothing in this slot that was ever read (an empty receiver slot doesn't answer either).
                (_, None) => {}
            }
        }
    }
    let held = without_fresh(&fresh, held);
    fresh.extend(held);
    fresh
}

/// Sends a long HID++ 2.0 request and waits for its response. A sleeping wireless keyboard
/// drops the packet that wakes it up, so unanswered requests are retried.
///
/// Responses only echo the feature index and function, not the arguments, so a late answer
/// to an earlier attempt would be taken for the answer to the next request (e.g. one feature
/// lookup returning the index of another). Stale input is therefore drained before every
/// request, and after a retry the late duplicate answer is swallowed.
fn request(hid: &Hid, dev: u8, sw: u8, feat: u8, func: u8, args: &[u8]) -> Result<[u8; 20], String> {
    request_tries(hid, dev, sw, feat, func, args, 3)
}

/// `request` with up to `tries` attempts (at least one).
fn request_tries(hid: &Hid, dev: u8, sw: u8, feat: u8, func: u8, args: &[u8], tries: usize) -> Result<[u8; 20], String> {
    let mut result = request_once(hid, dev, sw, feat, func, args);
    let mut retried = false;
    for _ in 1..tries {
        match &result {
            Err(e) if e == NO_RESPONSE => {
                retried = true;
                result = request_once(hid, dev, sw, feat, func, args);
            }
            _ => break,
        }
    }
    if retried && result.is_ok() {
        // The late duplicate answer.
        hid.drain(&mut [0u8; 20], usize::MAX, Duration::from_millis(600));
    }
    result
}

const NO_RESPONSE: &str = "HID++: no response";

/// How long a request waits for its answer; the first lookup of a search waits less.
const REQUEST_WAIT: Duration = Duration::from_millis(500);
const LOOKUP_WAIT: Duration = Duration::from_millis(250);

/// Drops stale input, sends one request and waits for its response, skipping traffic of other apps.
fn request_once(hid: &Hid, dev: u8, sw: u8, feat: u8, func: u8, args: &[u8]) -> Result<[u8; 20], String> {
    request_within(hid, dev, sw, feat, func, args, REQUEST_WAIT)
}

/// `request_once` waiting at most `wait` for the answer.
fn request_within(hid: &Hid, dev: u8, sw: u8, feat: u8, func: u8, args: &[u8], wait: Duration) -> Result<[u8; 20], String> {
    let mut msg = [0u8; 20];
    msg[0] = LONG;
    msg[1] = dev;
    msg[2] = feat;
    msg[3] = (func << 4) | sw;
    msg[4..4 + args.len()].copy_from_slice(args);
    if crate::trace_enabled() {
        crate::log(&format!("hid++ > {:02X?}", &msg[..4 + args.len()]));
    }
    let mut buf = [0u8; 20];
    let answer = |r: &[u8]| {
        if r[0] != LONG || r[1] != dev {
            return None;
        }
        if r[2] == 0xFF && r[3] == feat && r[4] == msg[3] {
            return Some(Err(format!("HID++: error {:#04x} (feat {feat:#04x} fn {func})", r[5])));
        }
        if r[2] == feat && r[3] == msg[3] {
            if crate::trace_enabled() {
                crate::log(&format!("hid++ < {:02X?}", &r[..8]));
            }
            return Some(Ok(r.try_into().unwrap_or([0; 20])));
        }
        None
    };
    match hid.exchange(&msg, &mut buf, usize::MAX, Patience::Total(wait), answer) {
        Ok(result) => result,
        Err(hid::Exchange::Write(e)) => Err(format!("HID++: write failed ({e})")),
        Err(hid::Exchange::Gone) => Err("HID++: device disconnected".into()),
        Err(hid::Exchange::NoAnswer) => Err(NO_RESPONSE.into()),
    }
}

/// 0x8071 effect parameters of a steady color.
fn steady(Rgb(r, g, b): Rgb) -> [u8; 10] {
    [r, g, b, 0, 0, 0, 0, 0, 0, 0]
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

/// How the app finds Logitech keyboards (see `devices::DRIVERS`).
pub const DRIVER: super::Driver = super::Driver {
    discover,
    batteries: || Logitech::probe_all(SW_ID_POLL).into_iter().filter_map(|kb| Some((kb.id.clone(), kb.battery()?))).collect(),
    answering: || Logitech::probe_all(SW_ID_POLL).into_iter().map(|k| k.id).collect(),
    open_for_keys: |id| match id.starts_with("logitech-") {
        true => Logitech::find_id(SW_ID_EDITOR, id).map(|k| Box::new(k) as Box<dyn Device>),
        false => None,
    },
    note: |i| match (i.vid, i.usage_page, i.usage) {
        (VID, 0xFF00, 0x0002) if RECEIVER_PIDS.contains(&i.pid) => Some("SUPPORTED: Logitech receiver (HID++)"),
        (VID, 0xFF00, 0x0002) => Some("Logitech HID++ (protocol known)"),
        _ => None,
    },
};

fn discover(d: &mut super::Discovery) {
    let mut ids = Vec::new();
    for kb in Logitech::find_all(SW_ID_APPLY) {
        ids.push(kb.id.clone());
        d.found.push(super::Found { id: kb.id.clone(), dev: Box::new(kb) });
    }
    let no_link = Logitech::no_link(&ids);
    // Asleep already when the app started: which keyboard isn't known yet.
    if no_link.is_empty() && Logitech::receiver_silent() {
        d.no_link_kinds.push("logitech");
    }
    d.no_link.extend(no_link);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_battery_with_and_without_state_of_charge() {
        let status = |soc: u8, level: u8| {
            let mut r = [0u8; 20];
            r[4] = soc;
            r[5] = level;
            r
        };
        // State of charge supported (flags bit 1): byte 4 is the percent, 0 included.
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &status(73, 4), Some(0x02)), Some(73));
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &status(0, 1), Some(0x02)), Some(0));
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &status(120, 4), Some(0x02)), None);
        // Levels only: byte 4 is 0 and means nothing; the bitmask gives the percent.
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &status(0, 8), Some(0x00)), Some(90));
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &status(0, 4), Some(0x01)), Some(50));
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &status(0, 2), Some(0x00)), Some(20));
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &status(0, 1), Some(0x00)), Some(5));
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &status(0, 0), Some(0x00)), None);
        // Capabilities unreadable: a nonzero byte 4 counts, a zero one falls back to the levels.
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &status(55, 4), None), Some(55));
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &status(0, 4), None), Some(50));
    }

    #[test]
    fn held_entry_dropped_when_a_fresh_read_has_its_name() {
        use crate::sysbat::{SysClass, SysDevice};
        let dev = |id: &str, name: &str, percent| SysDevice { id: id.into(), name: name.into(), class: SysClass::Mouse, percent };
        let fresh = [dev("system-A", "G502 LIGHTSPEED", 80)];
        let held = vec![dev("system-B01", "G502 LIGHTSPEED", 82), dev("system-B02", "G304", 40)];
        let kept = without_fresh(&fresh, held);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].name, "G304");
    }

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

    #[test]
    fn other_device_battery_answers() {
        let answer = |b: [u8; 3]| {
            let mut r = [0u8; 20];
            r[4..7].copy_from_slice(&b);
            r
        };
        // G535 over 0x1F20: 4073 mV, battery there, not charging (read 22.11.2026).
        assert_eq!(percent_of(FEATURE_ADC, &answer([0x0F, 0xE9, 0x01]), None), Some(91));
        assert_eq!(percent_of(FEATURE_ADC, &answer([0x0F, 0xE9, 0x00]), None), None);
        // G502 LIGHTSPEED over 0x1001: 3920 mV.
        assert_eq!(percent_of(FEATURE_BATTERY_VOLTAGE, &answer([0x0F, 0x50, 0x00]), None), Some(70));
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &answer([55, 4, 0]), None), Some(55));
        assert_eq!(percent_of(FEATURE_UNIFIED_BATTERY, &answer([200, 0, 0]), None), None);
    }

    #[test]
    fn other_device_classes() {
        use crate::sysbat::SysClass;
        assert_eq!(class_of(Some(3), false), SysClass::Mouse);
        assert_eq!(class_of(Some(8), false), SysClass::Headset);
        assert_eq!(class_of(None, true), SysClass::Headset);
        assert_eq!(class_of(None, false), SysClass::Other);
    }

    #[test]
    fn generic_words_leave_the_name() {
        assert_eq!(short_name("G502 LIGHTSPEED Wireless Gaming Mouse"), "G502 LIGHTSPEED");
        assert_eq!(short_name("G535 Wireless Gaming Headset"), "G535");
        assert_eq!(short_name("Wireless Mouse"), "Wireless");
        assert_eq!(short_name("PRO X  Headset "), "PRO X");
    }
}
