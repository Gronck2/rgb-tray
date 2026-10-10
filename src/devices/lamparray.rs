//! HID LampArray (usage page 0x59): the standard lighting interface behind Windows Dynamic
//! Lighting. The device describes itself (lamp count, kind, every lamp's position, purpose
//! and key), and every device is driven the same way. Report ids differ between devices, so
//! they are looked up by usage in the report descriptor instead of being hard-coded.

use super::{animation, Device, Effect, Rgb};
use crate::hid::{self, Hid, HidInfo, Preparsed};
use std::sync::Mutex;
use std::time::Duration;
use windows_sys::Win32::Devices::HumanInterfaceDevice::*;

const PAGE: u16 = 0x59;
const LAMP_ARRAY: u16 = 0x01;
const LAMP_COUNT: u16 = 0x03;
const BOX_WIDTH: u16 = 0x04;
const BOX_HEIGHT: u16 = 0x05;
const BOX_DEPTH: u16 = 0x06;
const ARRAY_KIND: u16 = 0x07;
const MIN_UPDATE_INTERVAL: u16 = 0x08;
const LAMP_ID: u16 = 0x21;
const POSITION_X: u16 = 0x23;
const POSITION_Y: u16 = 0x24;
const POSITION_Z: u16 = 0x25;
const LAMP_PURPOSES: u16 = 0x26;
const RED_LEVELS: u16 = 0x28;
const GREEN_LEVELS: u16 = 0x29;
const BLUE_LEVELS: u16 = 0x2A;
const INTENSITY_LEVELS: u16 = 0x2B;
const IS_PROGRAMMABLE: u16 = 0x2C;
const INPUT_BINDING: u16 = 0x2D;
const RED_CHANNEL: u16 = 0x51;
const GREEN_CHANNEL: u16 = 0x52;
const BLUE_CHANNEL: u16 = 0x53;
const INTENSITY_CHANNEL: u16 = 0x54;
const UPDATE_FLAGS: u16 = 0x55;
const LAMP_ID_START: u16 = 0x61;
const LAMP_ID_END: u16 = 0x62;
const AUTONOMOUS_MODE: u16 = 0x71;
/// LampUpdateFlags: this update is complete, show it.
const UPDATE_COMPLETE: u32 = 1;

/// What the whole array is (LampArrayKind).
pub fn kind_name(kind: u32) -> &'static str {
    match kind {
        1 => "keyboard",
        2 => "mouse",
        3 => "game controller",
        4 => "peripheral",
        5 => "scene",
        6 => "notification",
        7 => "chassis",
        8 => "wearable",
        9 => "furniture",
        10 => "art",
        _ => "undefined",
    }
}

/// One lamp as the device describes it.
#[derive(Clone, Debug)]
pub struct Lamp {
    pub id: u16,
    /// Position in micrometers from the top-left-front corner of the bounding box.
    pub pos: (u32, u32, u32),
    /// LampPurposes bit field: 1 control (key), 2 accent, 4 branding, 8 status, 16 illumination, 32 presentation.
    pub purposes: u32,
    /// Color levels per channel (red, green, blue, intensity).
    pub levels: (u32, u32, u32, u32),
    pub programmable: bool,
    /// Keyboard usage (HID page 7) of the key this lamp lights, 0 if none.
    pub key: u32,
}

/// What the device reports about itself.
#[derive(Clone, Debug, Default)]
pub struct Attributes {
    pub lamp_count: u16,
    pub kind: u32,
    /// Bounding box in micrometers.
    pub size: (u32, u32, u32),
    pub min_update_interval_us: u32,
}

/// Feature report ids of one device, found by usage.
#[derive(Clone, Copy, Debug, Default)]
struct Ids {
    attributes: u8,
    request: u8,
    response: u8,
    range: u8,
    control: u8,
}

pub struct LampArray {
    /// "lamparray-<hash of the HID path>": these devices have no readable serial number.
    pub id: String,
    path: Vec<u16>,
    hid: Hid,
    pp: Preparsed,
    len: usize,
    ids: Ids,
    pub attributes: Attributes,
}

/// LampArray interfaces of real devices. Virtual ones ("VHF") are bridges that vendor
/// services such as Logitech's create for Windows; they are skipped.
pub fn interfaces() -> Vec<HidInfo> {
    hid::enumerate_all()
        .into_iter()
        .filter(|i| i.usage_page == PAGE && i.usage == LAMP_ARRAY && i.feature_len > 1)
        .filter(|i| !String::from_utf16_lossy(&i.path).to_lowercase().contains("hid_device_system_vhf"))
        .collect()
}

impl LampArray {
    pub fn open(info: &HidInfo) -> Result<LampArray, String> {
        let hid = Hid::open(&info.path, true, false).ok_or("LampArray: cannot open")?;
        let pp = hid.preparsed().ok_or("LampArray: no report descriptor")?;
        let report_of = |usage: u16| report_id(&pp, usage);
        let response = report_of(POSITION_X).ok_or("LampArray: no lamp attributes report")?;
        // LampId is in the request, the response and the multi-update reports; the request is
        // the one without color channels.
        let colored = report_ids(&pp, RED_CHANNEL);
        let request = report_ids(&pp, LAMP_ID).into_iter().find(|id| *id != response && !colored.contains(id)).unwrap_or(0);
        let ids = Ids {
            attributes: report_of(LAMP_COUNT).ok_or("LampArray: no attributes report")?,
            request,
            response,
            range: report_of(LAMP_ID_START).ok_or("LampArray: no range update report")?,
            control: report_of(AUTONOMOUS_MODE).unwrap_or(0),
        };
        let id = format!("lamparray-{}", super::path_hash(&info.path));
        let mut la = LampArray { id, path: info.path.clone(), hid, pp, len: info.feature_len as usize, ids, attributes: Attributes::default() };
        la.attributes = la.read_attributes()?;
        Ok(la)
    }

    fn read_attributes(&self) -> Result<Attributes, String> {
        let r = self.get(self.ids.attributes).ok_or("LampArray: attributes report failed")?;
        let v = |usage| self.value(&r, usage).unwrap_or(0);
        Ok(Attributes {
            lamp_count: v(LAMP_COUNT) as u16,
            kind: v(ARRAY_KIND),
            size: (v(BOX_WIDTH), v(BOX_HEIGHT), v(BOX_DEPTH)),
            min_update_interval_us: v(MIN_UPDATE_INTERVAL),
        })
    }

    /// Attributes of every lamp, asked one by one.
    pub fn lamps(&self) -> Vec<Lamp> {
        let mut out = Vec::new();
        if self.ids.request == 0 {
            return out;
        }
        for id in 0..self.attributes.lamp_count {
            let Some(mut req) = self.report(self.ids.request) else { break };
            self.set_value(&mut req, LAMP_ID, id as u32);
            if !self.hid.set_feature(&req) {
                break;
            }
            let Some(r) = self.get(self.ids.response) else { break };
            let v = |usage| self.value(&r, usage).unwrap_or(0);
            out.push(Lamp {
                id: v(LAMP_ID) as u16,
                pos: (v(POSITION_X), v(POSITION_Y), v(POSITION_Z)),
                purposes: v(LAMP_PURPOSES),
                levels: (v(RED_LEVELS), v(GREEN_LEVELS), v(BLUE_LEVELS), v(INTENSITY_LEVELS)),
                programmable: v(IS_PROGRAMMABLE) != 0,
                key: v(INPUT_BINDING),
            });
        }
        out
    }

    /// Autonomous: the device runs its own lighting; off: it shows what the host sends.
    pub fn set_autonomous(&self, on: bool) -> bool {
        if self.ids.control == 0 {
            return true;
        }
        let Some(mut r) = self.report(self.ids.control) else { return false };
        self.set_value(&mut r, AUTONOMOUS_MODE, on as u32);
        self.hid.set_feature(&r)
    }

    /// Every lamp in one color (brightness already applied by the caller).
    pub fn fill(&self, c: Rgb) -> bool {
        let count = self.attributes.lamp_count;
        count > 0 && self.fill_range(0, count - 1, c)
    }

    /// Lamps `first..=last` in one color.
    pub fn fill_range(&self, first: u16, last: u16, c: Rgb) -> bool {
        let Some(mut r) = self.report(self.ids.range) else { return false };
        self.set_value(&mut r, UPDATE_FLAGS, UPDATE_COMPLETE);
        self.set_value(&mut r, LAMP_ID_START, first as u32);
        self.set_value(&mut r, LAMP_ID_END, last as u32);
        let max = |usage| self.logical_max(self.ids.range, usage).unwrap_or(255);
        let scale = |v: u8, usage| (v as u32 * max(usage) + 127) / 255;
        self.set_value(&mut r, RED_CHANNEL, scale(c.0, RED_CHANNEL));
        self.set_value(&mut r, GREEN_CHANNEL, scale(c.1, GREEN_CHANNEL));
        self.set_value(&mut r, BLUE_CHANNEL, scale(c.2, BLUE_CHANNEL));
        // Black lamps are switched off; others at full intensity (the color carries brightness).
        let lit = c != Rgb(0, 0, 0);
        self.set_value(&mut r, INTENSITY_CHANNEL, if lit { max(INTENSITY_CHANNEL) } else { 0 });
        self.hid.set_feature(&r)
    }

    /// An empty feature report with its id set.
    fn report(&self, id: u8) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; self.len];
        let ok = unsafe { HidP_InitializeReportForID(HidP_Feature, id, self.pp.0, buf.as_mut_ptr(), buf.len() as u32) } == HIDP_STATUS_SUCCESS;
        ok.then_some(buf)
    }

    fn get(&self, id: u8) -> Option<Vec<u8>> {
        let mut buf = self.report(id)?;
        self.hid.get_feature(&mut buf).then_some(buf)
    }

    fn value(&self, report: &[u8], usage: u16) -> Option<u32> {
        let mut v = 0u32;
        let st = unsafe { HidP_GetUsageValue(HidP_Feature, PAGE, 0, usage, &mut v, self.pp.0, report.as_ptr(), report.len() as u32) };
        (st == HIDP_STATUS_SUCCESS).then_some(v)
    }

    fn set_value(&self, report: &mut [u8], usage: u16, v: u32) -> bool {
        unsafe { HidP_SetUsageValue(HidP_Feature, PAGE, 0, usage, v, self.pp.0, report.as_mut_ptr(), report.len() as u32) == HIDP_STATUS_SUCCESS }
    }

    fn logical_max(&self, report: u8, usage: u16) -> Option<u32> {
        value_caps(&self.pp, usage).into_iter().find(|c| c.ReportID == report).map(|c| c.LogicalMax.max(0) as u32)
    }
}

// ---------------------------------------------------------------- zones

/// A group of lamps colored together, shown as a zone in the panel.
#[derive(Clone, Debug)]
pub struct Zone {
    /// Zone number in the config (1, 2, ...), like the LED ids of Razer zones.
    pub led: u8,
    /// Name (Russian, English).
    pub names: (String, String),
    pub lamps: Vec<u16>,
}

/// A zone before numbering: names (Russian, English) and its lamps.
type ZoneDraft = ((String, String), Vec<u16>);

/// Most zones shown (the switch has room for about this many next to "All").
const MAX_ZONES: usize = 8;

/// Zones from the lamp layout:
/// - key lamps without a key binding one by one, left to right (laptop keyboards with a few
///   backlight zones), named 1, 2, ...; merged into one "Keys" zone if there are too many;
/// - lamps bound to keys as one zone (their colors are per key);
/// - other lamps grouped by purpose (body, logo, ...); a group of 2–4 lamps is split into
///   single lamps named by where they are (↖ ↗ ↙ ↘) while the zones still fit.
///
/// One group only: no zones.
pub fn zones_from(lamps: &[Lamp]) -> Vec<Zone> {
    type Names = (String, String);
    let names = |ru: &str, en: &str| -> Names { (ru.into(), en.into()) };
    let mut groups: Vec<(Names, Vec<&Lamp>)> = Vec::new();
    let mut key_zones: Vec<&Lamp> = lamps.iter().filter(|l| l.purposes & 1 != 0 && l.key == 0).collect();
    key_zones.sort_by_key(|l| (l.pos.0, l.pos.1));
    for l in lamps.iter().filter(|l| l.purposes & 1 == 0 || l.key != 0) {
        let n = match l.purposes {
            _ if l.key != 0 => names("Клавиши", "Keys"),
            p if p & 2 != 0 => names("Корпус", "Body"),
            p if p & 4 != 0 => names("Логотип", "Logo"),
            p if p & 8 != 0 => names("Статус", "Status"),
            p if p & 16 != 0 => names("Освещение", "Lights"),
            p if p & 32 != 0 => names("Декор", "Display"),
            _ => names("Другое", "Other"),
        };
        match groups.iter_mut().find(|g| g.0 == n) {
            Some(g) => g.1.push(l),
            None => groups.push((n, vec![l])),
        }
    }
    let mut zones: Vec<ZoneDraft> = Vec::new();
    if key_zones.len() + groups.len() <= MAX_ZONES {
        for (k, l) in key_zones.iter().enumerate() {
            let n = (k + 1).to_string();
            zones.push(((n.clone(), n), vec![l.id]));
        }
    } else if !key_zones.is_empty() {
        zones.push((names("Клавиши", "Keys"), key_zones.iter().map(|l| l.id).collect()));
    }
    // Zones if every remaining group stays whole; splitting one adds its size minus one.
    let mut total = zones.len() + groups.len();
    for (n, group) in groups {
        let single = n.0 != "Клавиши" && (2..=4).contains(&group.len()) && total - 1 + group.len() <= MAX_ZONES;
        match single.then(|| by_position(&group)).flatten() {
            Some(split) => {
                total += group.len() - 1;
                zones.extend(split);
            }
            None => zones.push((n, group.iter().map(|l| l.id).collect())),
        }
    }
    if zones.len() < 2 {
        return Vec::new();
    }
    zones.into_iter().take(MAX_ZONES).enumerate().map(|(k, (names, lamps))| Zone { led: k as u8 + 1, names, lamps }).collect()
}

/// Single-lamp zones named by an arrow for where each lamp is in its group: ← → when they
/// differ only left/right, ↑ ↓ only back/front, ↖ ↗ ↙ ↘ for both. Back row first, left to
/// right. None if two lamps would get the same arrow.
fn by_position(group: &[&Lamp]) -> Option<Vec<ZoneDraft>> {
    let n = group.len() as u64;
    let mid_x = group.iter().map(|l| l.pos.0 as u64).sum::<u64>() / n;
    let mid_y = group.iter().map(|l| l.pos.1 as u64).sum::<u64>() / n;
    let spread_x = group.iter().any(|l| (l.pos.0 as u64) != mid_x);
    let spread_y = group.iter().any(|l| (l.pos.1 as u64) != mid_y);
    let mut lamps: Vec<&&Lamp> = group.iter().collect();
    lamps.sort_by_key(|l| ((l.pos.1 as u64) >= mid_y, l.pos.0));
    let mut out: Vec<ZoneDraft> = Vec::new();
    for l in lamps {
        let left = (l.pos.0 as u64) < mid_x;
        let back = (l.pos.1 as u64) < mid_y;
        let arrow = match (spread_x, spread_y) {
            (true, true) => ["↘", "↗", "↙", "↖"][(left as usize) * 2 + back as usize],
            (true, false) => {
                if left {
                    "←"
                } else {
                    "→"
                }
            }
            (false, true) => {
                if back {
                    "↑"
                } else {
                    "↓"
                }
            }
            (false, false) => return None,
        };
        if out.iter().any(|z| z.0 .0 == arrow) {
            return None;
        }
        out.push(((arrow.into(), arrow.into()), vec![l.id]));
    }
    Some(out)
}
/// Zones of the devices found, by device id; and by HID path, so lamps are read only once.
/// (device id, HID path, zones).
type DeviceZones = (String, Vec<u16>, Vec<Zone>);
static ZONES: Mutex<Vec<DeviceZones>> = Mutex::new(Vec::new());

/// The zones of a device found earlier (empty if it has none).
pub fn zones(id: &str) -> Vec<Zone> {
    ZONES.lock().unwrap().iter().find(|z| z.0 == id).map(|z| z.2.clone()).unwrap_or_default()
}

impl LampArray {
    /// Remembers this device's zones under `id`, reading the lamps the first time.
    pub fn register_zones(&self, id: &str) {
        let known = ZONES.lock().unwrap().iter().find(|z| z.1 == self.path).map(|z| z.2.clone());
        let zones = known.unwrap_or_else(|| zones_from(&self.lamps()));
        let mut all = ZONES.lock().unwrap();
        all.retain(|z| z.0 != id && z.1 != self.path);
        all.push((id.to_string(), self.path.clone(), zones));
    }

    /// Colors each zone (zone number, color); lamps outside the zones keep their color.
    fn fill_zones(&self, zones: &[(Vec<u16>, Rgb)]) -> bool {
        zones.iter().all(|(lamps, c)| lamps.iter().all(|&id| self.fill_range(id, id, *c)))
    }

    /// The lamps of each configured zone, with its color.
    fn zone_lamps(&self, id: &str, colors: &[(u8, Rgb)]) -> Vec<(Vec<u16>, Rgb)> {
        let zones = zones(id);
        colors.iter().filter_map(|&(led, c)| zones.iter().find(|z| z.led == led).map(|z| (z.lamps.clone(), c))).collect()
    }

    /// Hands the lighting back to the device's own firmware, e.g. before another driver for
    /// the same device (ASUS Aura) takes over.
    pub fn release(&self) {
        animation::stop(&self.path);
        TAKEN.lock().unwrap().retain(|p| *p != self.path);
        self.set_autonomous(true);
    }

    /// Per-zone colors: static, or breathing played by the app; other effects are whole-device.
    pub fn apply_zone_colors(&self, id: &str, effect: Effect, colors: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        let zones = self.zone_lamps(id, colors);
        if zones.is_empty() || !matches!(effect, Effect::Static | Effect::Breathe) {
            let color = colors.first().map_or(Rgb(255, 255, 255), |z| z.1);
            return self.apply(effect, color, brightness);
        }
        self.take()?;
        if effect == Effect::Breathe {
            return if self.start_animation(effect, Rgb(0, 0, 0), zones, brightness) { Ok(()) } else { Err("LampArray: cannot open".into()) };
        }
        animation::stop(&self.path);
        let scaled: Vec<(Vec<u16>, Rgb)> = zones.into_iter().map(|(l, c)| (l, c.scaled(brightness))).collect();
        if self.fill_zones(&scaled) {
            Ok(())
        } else {
            Err("LampArray: update failed".into())
        }
    }
}

// ---------------------------------------------------------------- driver

/// Devices taken from autonomous mode in this session. Right after that the device drops the
/// first update (seen on ASUS), so the first take waits a moment.
static TAKEN: Mutex<Vec<Vec<u16>>> = Mutex::new(Vec::new());

/// LampArray devices without a driver of their own in this app. A device that has one (e.g. the
/// ASUS keyboard: Aura) is left to it; `RGBTRAY_LAMPARRAY_ALL` takes those too, for testing.
pub fn find_all() -> Vec<LampArray> {
    let all = std::env::var_os("RGBTRAY_LAMPARRAY_ALL").is_some();
    let infos = hid::enumerate_all();
    let native = |i: &HidInfo| infos.iter().any(|o| o.vid == i.vid && o.pid == i.pid && super::interface_note(o).is_some_and(|n| n.starts_with("SUPPORTED")));
    let mut out = Vec::new();
    for info in interfaces() {
        if !all && native(&info) {
            continue;
        }
        match LampArray::open(&info) {
            Ok(la) => {
                la.register_zones(&la.id);
                super::set_name(&la.id, &super::device_name(&info));
                out.push(la);
            }
            Err(e) => crate::log(&format!("lamparray: {e}")),
        }
    }
    out
}

impl LampArray {
    /// Plays breathing or spectrum: the animation thread gets its own handle to the device.
    fn start_animation(&self, effect: Effect, color: Rgb, zones: Vec<(Vec<u16>, Rgb)>, brightness: u8) -> bool {
        let info =
            HidInfo { path: self.path.clone(), vid: 0, pid: 0, usage_page: PAGE, usage: LAMP_ARRAY, feature_len: self.len as u16, input_len: 0, output_len: 0 };
        let Ok(la) = LampArray::open(&info) else { return false };
        animation::start(
            &self.path,
            Box::new(move |t| match effect {
                Effect::Breathe => {
                    let level = (brightness as f32 * animation::breathe(t)) as u8;
                    if zones.is_empty() {
                        la.fill(color.scaled(level))
                    } else {
                        la.fill_zones(&zones.iter().map(|(l, c)| (l.clone(), c.scaled(level))).collect::<Vec<_>>())
                    }
                }
                _ => la.fill(crate::hue(animation::cycle_hue(t)).scaled(brightness)),
            }),
        );
        true
    }

    /// Switches autonomous mode off; waits after the first time for this device.
    fn take(&self) -> Result<(), String> {
        if !self.set_autonomous(false) {
            return Err("LampArray: control report failed".into());
        }
        let mut taken = TAKEN.lock().unwrap();
        if !taken.contains(&self.path) {
            taken.push(self.path.clone());
            drop(taken);
            std::thread::sleep(Duration::from_millis(1000));
        }
        Ok(())
    }
}

impl Device for LampArray {
    fn apply(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String> {
        self.take()?;
        let ok = match effect {
            Effect::Static => {
                animation::stop(&self.path);
                self.fill(color.scaled(brightness))
            }
            Effect::Off => {
                animation::stop(&self.path);
                self.fill(Rgb(0, 0, 0))
            }
            // The standard has no effects: the app plays them frame by frame.
            Effect::Breathe | Effect::Cycle => self.start_animation(effect, color, Vec::new(), brightness),
        };
        if ok {
            Ok(())
        } else {
            Err("LampArray: update failed".into())
        }
    }

    fn apply_zones(&self, effect: Effect, zones: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        self.apply_zone_colors(&self.id, effect, zones, brightness)
    }
}

fn value_caps(pp: &Preparsed, usage: u16) -> Vec<HIDP_VALUE_CAPS> {
    unsafe {
        let mut n: u16 = 16;
        let mut caps: Vec<HIDP_VALUE_CAPS> = vec![std::mem::zeroed(); n as usize];
        if HidP_GetSpecificValueCaps(HidP_Feature, PAGE, 0, usage, caps.as_mut_ptr(), &mut n, pp.0) != HIDP_STATUS_SUCCESS {
            return Vec::new();
        }
        caps.truncate(n as usize);
        caps
    }
}

/// Feature report ids that carry a usage.
fn report_ids(pp: &Preparsed, usage: u16) -> Vec<u8> {
    value_caps(pp, usage).into_iter().map(|c| c.ReportID).collect()
}

fn report_id(pp: &Preparsed, usage: u16) -> Option<u8> {
    report_ids(pp, usage).into_iter().next()
}

/// A short description for the device report, e.g. "keyboard, 102 lamps, 32 x 12 cm, every 10 ms".
pub fn describe(info: &HidInfo) -> String {
    match LampArray::open(info) {
        Ok(la) => {
            let a = &la.attributes;
            let lamps = la.lamps();
            let keys = lamps.iter().filter(|l| l.key != 0).count();
            let fixed = lamps.iter().filter(|l| !l.programmable).count();
            let colors = lamps.first().map(|l| format!(", {}/{}/{} color levels", l.levels.0, l.levels.1, l.levels.2)).unwrap_or_default();
            format!(
                "{}, {} lamps ({} read, {} bound to keys, {} not programmable), {:.1} x {:.1} cm, update every {} us{}",
                kind_name(a.kind),
                a.lamp_count,
                lamps.len(),
                keys,
                fixed,
                a.size.0 as f32 / 10_000.0,
                a.size.1 as f32 / 10_000.0,
                a.min_update_interval_us,
                colors
            )
        }
        Err(e) => e,
    }
}
