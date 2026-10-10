//! ASUS laptop keyboard backlight (Aura) over vendor feature reports 0x5D / 0x5A.

use super::lamparray::LampArray;
use super::{Device, Effect, Rgb};
use crate::hid::{self, Hid};

const VID: u16 = 0x0B05;
const AURA_REPORT: u8 = 0x5D;
const KBD_REPORT: u8 = 0x5A;
/// Aura effect speeds: slow, medium, fast (as in asusctl).
const SPEEDS: [u8; 3] = [0xE1, 0xEB, 0xF5];
/// Device id of the laptop keyboard.
const ID: &str = "asus";

pub struct Asus {
    aura: Hid,
    kbd: Option<Hid>,
    /// The HID LampArray interface (Windows Dynamic Lighting): its zones (keyboard zones and
    /// body lamps) are colored through it; single colors and effects go through Aura.
    lamp_array: Option<LampArray>,
}

impl Asus {
    pub fn find() -> Option<Asus> {
        let infos = hid::enumerate(VID);
        let open =
            |usage: u16| infos.iter().find(|i| i.usage_page == 0xFF31 && i.usage == usage && i.feature_len >= 64).and_then(|i| Hid::open(&i.path, true, false));
        let lamp_array = infos.iter().find(|i| i.usage_page == 0x0059 && i.usage == 0x0001 && i.feature_len > 1).and_then(|i| LampArray::open(i).ok());
        if let Some(la) = &lamp_array {
            la.register_zones(ID);
        }
        Some(Asus { aura: open(0x0079)?, kbd: open(0x0076), lamp_array })
    }

    /// While LampArray (this app's zones or Windows Dynamic Lighting) drives the keyboard, the
    /// firmware ignores Aura commands: hand control back to the firmware first.
    fn take_from_dynamic_lighting(&self) {
        if let Some(la) = &self.lamp_array {
            la.release();
        }
    }

    fn send(hid: &Hid, bytes: &[u8]) -> Result<(), String> {
        let mut buf = [0u8; 64];
        buf[..bytes.len()].copy_from_slice(bytes);
        if hid.set_feature(&buf) {
            Ok(())
        } else {
            Err("ASUS: send failed".into())
        }
    }
}

impl Device for Asus {
    fn apply(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String> {
        self.take_from_dynamic_lighting();
        // The keyboard has 4 backlight levels: 0 (off) ..= 3.
        let level = match effect {
            Effect::Off => 0,
            _ => ((brightness.min(100) as u32 * 3 + 50) / 100).max(1) as u8,
        };
        if let Some(kbd) = &self.kbd {
            Self::send(kbd, &[KBD_REPORT, 0xBA, 0xC5, 0xC4, level])?;
        }
        let mode = match effect {
            Effect::Off => return Ok(()),
            Effect::Static => 0,
            Effect::Breathe => 1,
            Effect::Cycle => 2,
        };
        let Rgb(r, g, b) = color;
        let breathe = (effect == Effect::Breathe) as u8;
        Self::send(&self.aura, &[AURA_REPORT, 0xB3, 0x00, mode, r, g, b, SPEEDS[(super::speed() as usize * 3 / 101).min(2)], 0x00, breathe])?;
        Self::send(&self.aura, &[AURA_REPORT, 0xB5, 0, 0, 0])?;
        Self::send(&self.aura, &[AURA_REPORT, 0xB4])
    }

    /// Zone colors go through LampArray; without it the first zone's color for everything.
    fn apply_zones(&self, effect: Effect, zones: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        match &self.lamp_array {
            Some(la) if matches!(effect, Effect::Static | Effect::Breathe) => la.apply_zone_colors(ID, effect, zones, brightness),
            _ => self.apply(effect, zones.first().map_or(Rgb(255, 255, 255), |z| z.1), brightness),
        }
    }
}
