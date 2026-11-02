//! ASUS laptop keyboard backlight (Aura) over vendor feature reports 0x5D / 0x5A.

use super::lamparray::LampArray;
use super::{Device, Effect, Rgb};
use crate::hid::{self, Hid};

const VID: u16 = 0x0B05;
const AURA_REPORT: u8 = 0x5D;
const KBD_REPORT: u8 = 0x5A;
/// Aura effect speeds: slow, medium, fast (as in asusctl).
const SPEEDS: [u8; 3] = [0xE1, 0xEB, 0xF5];

/// The Aura speed (index in SPEEDS) nearest a speed factor: the thirds of the slider range.
fn speed_step(factor: f32) -> usize {
    match factor {
        f if f < 0.63 => 0,
        f if f > 1.59 => 2,
        _ => 1,
    }
}
/// Device id of the laptop keyboard.
const ID: &str = "asus";

/// One of the vendor collections, with its feature report length (64 or more).
struct Collection {
    hid: Hid,
    path: Vec<u16>,
    len: usize,
}

pub struct Asus {
    aura: Collection,
    kbd: Option<Collection>,
    /// The HID LampArray interface (Windows Dynamic Lighting): its zones (keyboard zones and
    /// body lamps) are colored through it; single colors and effects go through Aura.
    lamp_array: Option<LampArray>,
}

impl Asus {
    pub fn find() -> Option<Asus> {
        let infos = hid::enumerate(VID);
        let open = |usage: u16| {
            let i = infos.iter().find(|i| i.usage_page == 0xFF31 && i.usage == usage && i.feature_len >= 64)?;
            Some(Collection { hid: Hid::open(&i.path, true, false)?, path: i.path.clone(), len: i.feature_len as usize })
        };
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

    /// Sends a feature report, padded with zeros to the collection's report length.
    fn send(c: &Collection, bytes: &[u8]) -> Result<(), String> {
        let mut buf = vec![0u8; c.len.max(bytes.len())];
        buf[..bytes.len()].copy_from_slice(bytes);
        if c.hid.set_feature(&buf) {
            Ok(())
        } else {
            Err("ASUS: send failed".into())
        }
    }
}

impl Device for Asus {
    fn hid_path(&self) -> Option<&[u16]> {
        Some(&self.aura.path)
    }

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
            // Backlight level 0 is off.
            Effect::Off if self.kbd.is_some() => return Ok(()),
            // Without the backlight level collection: steady black through Aura (below).
            Effect::Off => 0,
            Effect::Static => 0,
            Effect::Breathe => 1,
            Effect::Cycle => 2,
            // RainbowWave in asusctl; not every laptop has it (an unknown mode is ignored).
            Effect::Wave => 3,
            // Not in this firmware (the panel doesn't offer them): the color, steadily.
            Effect::Ripple | Effect::Reactive | Effect::Starlight => 0,
        };
        let Rgb(r, g, b) = if effect == Effect::Off { Rgb(0, 0, 0) } else { color };
        let breathe = (effect == Effect::Breathe) as u8;
        Self::send(&self.aura, &[AURA_REPORT, 0xB3, 0x00, mode, r, g, b, SPEEDS[speed_step(super::speed())], 0x00, breathe])?;
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

/// How the app finds the ASUS laptop keyboard (see `devices::DRIVERS`).
pub const DRIVER: super::Driver = super::Driver {
    discover: |d| {
        if let Some(a) = Asus::find() {
            d.found.push(super::Found { id: ID.into(), dev: Box::new(a) });
        }
    },
    note: |i| match (i.vid, i.usage_page, i.usage) {
        (VID, 0xFF31, 0x0079) => Some("SUPPORTED: ASUS Aura"),
        _ => None,
    },
    ..super::Driver::NONE
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aura_speed_steps() {
        assert_eq!(speed_step(1.0), 1);
        assert_eq!(speed_step(0.25), 0);
        assert_eq!(speed_step(4.0), 2);
        assert_eq!(speed_step(16.0), 2);
        assert_eq!(speed_step(1.0 / 16.0), 0);
    }
}
