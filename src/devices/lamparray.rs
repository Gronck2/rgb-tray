//! HID LampArray (usage page 0x59): the standard lighting interface behind Windows Dynamic
//! Lighting. The device describes itself (lamp count, kind, every lamp's position, purpose
//! and key), and every device is driven the same way. Report ids differ between devices, so
//! they are looked up by usage in the report descriptor instead of being hard-coded.

use super::Rgb;
use crate::hid::{self, Hid, HidInfo, Preparsed};
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
    hid: Hid,
    pp: Preparsed,
    len: usize,
    ids: Ids,
    pub attributes: Attributes,
    /// Autonomous mode was switched off: hand the lighting back when done.
    controlled: std::cell::Cell<bool>,
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
        let mut la = LampArray { hid, pp, len: info.feature_len as usize, ids, attributes: Attributes::default(), controlled: false.into() };
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
        let ok = self.hid.set_feature(&r);
        if ok {
            self.controlled.set(!on);
        }
        ok
    }

    /// Every lamp in one color (brightness already applied by the caller).
    pub fn fill(&self, c: Rgb) -> bool {
        let count = self.attributes.lamp_count;
        if count == 0 {
            return false;
        }
        let Some(mut r) = self.report(self.ids.range) else { return false };
        self.set_value(&mut r, UPDATE_FLAGS, UPDATE_COMPLETE);
        self.set_value(&mut r, LAMP_ID_START, 0);
        self.set_value(&mut r, LAMP_ID_END, count as u32 - 1);
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

impl Drop for LampArray {
    /// Hands the lighting back to the device if it was taken.
    fn drop(&mut self) {
        if self.controlled.get() {
            self.set_autonomous(true);
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
