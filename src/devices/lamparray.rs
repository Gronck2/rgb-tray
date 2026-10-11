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
    /// LampMultiUpdate: colors of several lamps in one report (0: none).
    multi: u8,
    /// Lamps per multi-update report.
    multi_count: usize,
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
        let range = report_of(LAMP_ID_START).ok_or("LampArray: no range update report")?;
        // The multi update carries color channels too, with a lamp id array.
        let multi = colored.iter().copied().find(|&id| id != range).unwrap_or(0);
        let multi_count = if multi != 0 { slot_caps(&pp, multi, LAMP_ID).iter().map(|c| c.ReportCount as usize).sum() } else { 0 };
        let ids = Ids {
            // LampCount is in the multi update as well; the bounding box only here.
            attributes: report_of(BOX_WIDTH).ok_or("LampArray: no attributes report")?,
            request,
            response,
            range,
            control: report_of(AUTONOMOUS_MODE).unwrap_or(0),
            multi: if multi_count > 0 { multi } else { 0 },
            multi_count,
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

    /// Colors single lamps (brightness already applied): several per report with LampMultiUpdate,
    /// one by one otherwise. Only the last report says the update is complete.
    pub fn set_lamps(&self, lamps: &[(u16, Rgb)]) -> bool {
        if self.ids.multi == 0 || self.ids.multi_count == 0 {
            return lamps.iter().all(|&(id, c)| self.fill_range(id, id, c));
        }
        let max = |usage| self.logical_max(self.ids.multi, usage).unwrap_or(255);
        let (red, green, blue, intensity) = (max(RED_CHANNEL), max(GREEN_CHANNEL), max(BLUE_CHANNEL), max(INTENSITY_CHANNEL));
        let scale = |v: u8, m: u32| ((v as u32 * m + 127) / 255) as u8;
        let chunks: Vec<&[(u16, Rgb)]> = lamps.chunks(self.ids.multi_count).collect();
        for (k, chunk) in chunks.iter().enumerate() {
            let Some(mut r) = self.report(self.ids.multi) else { return false };
            self.set_value(&mut r, LAMP_COUNT, chunk.len() as u32);
            self.set_value(&mut r, UPDATE_FLAGS, if k + 1 == chunks.len() { UPDATE_COMPLETE } else { 0 });
            let ids: Vec<u32> = chunk.iter().map(|l| l.0 as u32).collect();
            let channel = |f: &dyn Fn(Rgb) -> u32| chunk.iter().map(|l| f(l.1)).collect::<Vec<u32>>();
            let ok = self.set_slots(&mut r, LAMP_ID, &ids)
                & self.set_slots(&mut r, RED_CHANNEL, &channel(&|c| scale(c.0, red) as u32))
                & self.set_slots(&mut r, GREEN_CHANNEL, &channel(&|c| scale(c.1, green) as u32))
                & self.set_slots(&mut r, BLUE_CHANNEL, &channel(&|c| scale(c.2, blue) as u32))
                & self.set_slots(&mut r, INTENSITY_CHANNEL, &channel(&|c| if c == Rgb(0, 0, 0) { 0 } else { intensity }));
            if !ok {
                return false;
            }
            if !self.hid.set_feature(&r) {
                return false;
            }
        }
        true
    }

    /// Fills the per-lamp slots of a multi update (lamp ids or one color channel). Descriptors
    /// declare them either as one array field or as one field per slot (ASUS); both are handled.
    fn set_slots(&self, report: &mut [u8], usage: u16, values: &[u32]) -> bool {
        let caps = slot_caps(&self.pp, self.ids.multi, usage);
        let st = match caps.as_slice() {
            [c] if c.ReportCount > 1 => {
                let bytes_per = (c.BitSize as usize).div_ceil(8);
                let mut bytes = vec![0u8; c.ReportCount as usize * bytes_per];
                for (j, v) in values.iter().enumerate().take(c.ReportCount as usize) {
                    bytes[j * bytes_per..(j + 1) * bytes_per].copy_from_slice(&v.to_le_bytes()[..bytes_per]);
                }
                // The same usage is a single value in other reports: name this field's collection.
                unsafe {
                    HidP_SetUsageValueArray(
                        HidP_Feature,
                        PAGE,
                        c.LinkCollection,
                        usage,
                        bytes.as_ptr(),
                        bytes.len() as u16,
                        self.pp.0,
                        report.as_mut_ptr(),
                        report.len() as u32,
                    )
                }
            }
            _ => {
                let mut data: Vec<HIDP_DATA> = caps
                    .iter()
                    .zip(values)
                    .map(|(c, &v)| HIDP_DATA { DataIndex: data_index(c), Reserved: 0, Anonymous: HIDP_DATA_0 { RawValue: v } })
                    .collect();
                let mut n = data.len() as u32;
                unsafe { HidP_SetData(HidP_Feature, data.as_mut_ptr(), &mut n, self.pp.0, report.as_mut_ptr(), report.len() as u32) }
            }
        };
        if st != HIDP_STATUS_SUCCESS {
            let info: Vec<String> =
                caps.iter().map(|c| format!("count {} bits {} range {} index {}", c.ReportCount, c.BitSize, c.IsRange, data_index(c))).collect();
            crate::log(&format!("lamparray: slots of usage {usage:#x}: status {:#x}; fields {info:?}", st as u32));
        }
        st == HIDP_STATUS_SUCCESS
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
/// What is known about a device's lamps: its zones and which lamps light which key.
struct Layout {
    id: String,
    path: Vec<u16>,
    zones: Vec<Zone>,
    /// (HID keyboard usage, lamp id): a key can have several lamps.
    keys: Vec<(u32, u16)>,
}

/// Layouts of the devices found, so lamps are read only once per device.
static LAYOUTS: Mutex<Vec<Layout>> = Mutex::new(Vec::new());

/// The zones of a device found earlier (empty if it has none).
pub fn zones(id: &str) -> Vec<Zone> {
    LAYOUTS.lock().unwrap().iter().find(|l| l.id == id).map(|l| l.zones.clone()).unwrap_or_default()
}

/// The device has lamps bound to keys: its key colors are edited in the key editor.
pub fn has_keys(id: &str) -> bool {
    LAYOUTS.lock().unwrap().iter().any(|l| l.id == id && !l.keys.is_empty())
}

/// The keyboard has lamps on numeric keypad keys (HID usages 0x53..=0x63): it is full size.
pub fn has_keypad(id: &str) -> bool {
    LAYOUTS.lock().unwrap().iter().any(|l| l.id == id && l.keys.iter().any(|k| (0x53..=0x63).contains(&k.0)))
}

/// Lamps lighting a key (HID keyboard usage).
fn key_lamps(id: &str, usage: u32) -> Vec<u16> {
    LAYOUTS.lock().unwrap().iter().find(|l| l.id == id).map(|l| l.keys.iter().filter(|k| k.0 == usage).map(|k| k.1).collect()).unwrap_or_default()
}

impl LampArray {
    /// Remembers this device's zones and key lamps under `id`, reading the lamps the first time.
    pub fn register_zones(&self, id: &str) {
        let known = LAYOUTS.lock().unwrap().iter().find(|l| l.path == self.path).map(|l| (l.zones.clone(), l.keys.clone()));
        let (zones, keys) = known.unwrap_or_else(|| {
            let lamps = self.lamps();
            (zones_from(&lamps), lamps.iter().filter(|l| l.key != 0).map(|l| (l.key, l.id)).collect())
        });
        let mut all = LAYOUTS.lock().unwrap();
        all.retain(|l| l.id != id && l.path != self.path);
        all.push(Layout { id: id.to_string(), path: self.path.clone(), zones, keys });
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

    /// Key colors by the editor's key codes, mapped to the lamps the device binds to each key.
    fn apply_keys(&self, keys: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        if !has_keys(&self.id) {
            return Err("LampArray: no lamps bound to keys".into());
        }
        self.take()?;
        animation::stop(&self.path);
        let lamps: Vec<(u16, Rgb)> = keys
            .iter()
            .filter_map(|&(code, c)| crate::keymap::usage_of(code).map(|u| (u, c.scaled(brightness))))
            .flat_map(|(usage, c)| key_lamps(&self.id, usage).into_iter().map(move |lamp| (lamp, c)))
            .collect();
        if self.set_lamps(&lamps) {
            Ok(())
        } else {
            Err("LampArray: update failed".into())
        }
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

/// The fields of a usage in one report, in declaration order (slot 0 first).
fn slot_caps(pp: &Preparsed, report: u8, usage: u16) -> Vec<HIDP_VALUE_CAPS> {
    let mut caps: Vec<HIDP_VALUE_CAPS> = value_caps(pp, usage).into_iter().filter(|c| c.ReportID == report).collect();
    caps.sort_by_key(data_index);
    caps
}

fn data_index(c: &HIDP_VALUE_CAPS) -> u16 {
    // A usage declared once per field: never a usage range here.
    unsafe {
        if c.IsRange != 0 {
            c.Anonymous.Range.DataIndexMin
        } else {
            c.Anonymous.NotRange.DataIndex
        }
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
                "{}, {} lamps ({} read, {} bound to keys, {} not programmable), {:.1} x {:.1} cm, update every {} us{}, {}",
                kind_name(a.kind),
                a.lamp_count,
                lamps.len(),
                keys,
                fixed,
                a.size.0 as f32 / 10_000.0,
                a.size.1 as f32 / 10_000.0,
                a.min_update_interval_us,
                colors,
                if la.ids.multi != 0 { format!("multi-update {} lamps/report", la.ids.multi_count) } else { "no multi-update".to_string() }
            )
        }
        Err(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp(id: u16, x: u32, y: u32, purposes: u32, key: u32) -> Lamp {
        Lamp { id, pos: (x, y, 0), purposes, levels: (255, 255, 255, 1), programmable: true, key }
    }

    fn names(zones: &[Zone]) -> Vec<&str> {
        zones.iter().map(|z| z.names.1.as_str()).collect()
    }

    #[test]
    fn asus_laptop_keyboard() {
        // As the ASUS 0B05:19B6 reports it: 4 unbound key lamps (listed out of order) and
        // 4 accent lamps at the corners of the body.
        let lamps = [
            lamp(0, 300, 50, 1, 0),
            lamp(1, 100, 50, 1, 0),
            lamp(2, 400, 50, 1, 0),
            lamp(3, 200, 50, 1, 0),
            lamp(4, 0, 0, 2, 0),
            lamp(5, 500, 0, 2, 0),
            lamp(6, 0, 90, 2, 0),
            lamp(7, 500, 90, 2, 0),
        ];
        let zones = zones_from(&lamps);
        assert_eq!(names(&zones), ["1", "2", "3", "4", "↖", "↗", "↙", "↘"]);
        let lamps_of: Vec<&[u16]> = zones.iter().map(|z| z.lamps.as_slice()).collect();
        assert_eq!(lamps_of, [&[1][..], &[3], &[0], &[2], &[4], &[5], &[6], &[7]]);
        assert_eq!(zones.iter().map(|z| z.led).collect::<Vec<_>>(), (1..=8).collect::<Vec<_>>());
    }

    #[test]
    fn one_group_has_no_zones() {
        let lamps: Vec<Lamp> = (0..10).map(|i| lamp(i, i as u32, 0, 2, 0)).collect();
        assert!(zones_from(&lamps).is_empty());
    }

    #[test]
    fn keys_bound_to_usages_are_one_zone() {
        let mut lamps: Vec<Lamp> = (0..20).map(|i| lamp(i, i as u32 * 10, 0, 1, 4 + i as u32)).collect();
        lamps.push(lamp(20, 0, 0, 4, 0));
        let zones = zones_from(&lamps);
        assert_eq!(names(&zones), ["Keys", "Logo"]);
        assert_eq!(zones[0].lamps.len(), 20);
    }

    #[test]
    fn too_many_unbound_key_lamps_merge() {
        let mut lamps: Vec<Lamp> = (0..12).map(|i| lamp(i, i as u32 * 10, 0, 1, 0)).collect();
        lamps.push(lamp(12, 0, 0, 2, 0));
        let zones = zones_from(&lamps);
        assert_eq!(names(&zones), ["Keys", "Body"]);
        assert_eq!(zones[0].lamps.len(), 12);
    }

    #[test]
    fn lamps_in_one_row_are_left_and_right() {
        let lamps = [lamp(0, 0, 0, 2, 0), lamp(1, 100, 0, 2, 0), lamp(2, 50, 0, 4, 0)];
        assert_eq!(names(&zones_from(&lamps)), ["←", "→", "Logo"]);
    }

    #[test]
    fn lamps_at_the_same_place_stay_one_zone() {
        let lamps = [lamp(0, 0, 0, 2, 0), lamp(1, 0, 0, 2, 0), lamp(2, 50, 0, 4, 0)];
        assert_eq!(names(&zones_from(&lamps)), ["Body", "Logo"]);
    }
}
