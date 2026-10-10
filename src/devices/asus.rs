//! ASUS laptop keyboard backlight (Aura) over vendor feature reports 0x5D / 0x5A.

use super::{Device, Effect, Rgb};
use crate::hid::{self, Hid};

const VID: u16 = 0x0B05;
const AURA_REPORT: u8 = 0x5D;
const KBD_REPORT: u8 = 0x5A;
const SPEED_MEDIUM: u8 = 0xEB;
const LAMP_ARRAY_CONTROL_REPORT: u8 = 0x06;

pub struct Asus {
    aura: Hid,
    kbd: Option<Hid>,
    /// HID LampArray interface (used by Windows Dynamic Lighting) and its feature report size.
    lamp_array: Option<(Hid, usize)>,
}

impl Asus {
    pub fn find() -> Option<Asus> {
        let infos = hid::enumerate(VID);
        let open = |usage: u16| {
            infos
                .iter()
                .find(|i| i.usage_page == 0xFF31 && i.usage == usage && i.feature_len >= 64)
                .and_then(|i| Hid::open(&i.path, true, false))
        };
        let lamp_array = infos
            .iter()
            .find(|i| i.usage_page == 0x0059 && i.usage == 0x0001 && i.feature_len > 1)
            .and_then(|i| Some((Hid::open(&i.path, true, false)?, i.feature_len as usize)));
        Some(Asus { aura: open(0x0079)?, kbd: open(0x0076), lamp_array })
    }

    /// While Dynamic Lighting drives the keyboard, the firmware ignores Aura commands.
    /// Setting LampArray AutonomousMode (report 6) hands control back to the firmware.
    fn take_from_dynamic_lighting(&self) {
        if let Some((hid, len)) = &self.lamp_array {
            let mut buf = vec![0u8; *len];
            buf[0] = LAMP_ARRAY_CONTROL_REPORT;
            buf[1] = 1;
            hid.set_feature(&buf);
        }
    }

    fn send(hid: &Hid, bytes: &[u8]) -> Result<(), String> {
        let mut buf = [0u8; 64];
        buf[..bytes.len()].copy_from_slice(bytes);
        if hid.set_feature(&buf) { Ok(()) } else { Err("ASUS: send failed".into()) }
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
        Self::send(&self.aura, &[AURA_REPORT, 0xB3, 0x00, mode, r, g, b, SPEED_MEDIUM, 0x00, breathe])?;
        Self::send(&self.aura, &[AURA_REPORT, 0xB5, 0, 0, 0])?;
        Self::send(&self.aura, &[AURA_REPORT, 0xB4])
    }
}
