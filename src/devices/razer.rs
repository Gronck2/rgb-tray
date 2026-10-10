//! Razer devices over the 90-byte Chroma control report (as documented by OpenRazer).

use std::time::Duration;

use super::{Battery, Device, Effect, Rgb};
use crate::hid::{self, Hid};

const VID: u16 = 0x1532;
const VARSTORE: u8 = 0x01;

struct Model {
    pid: u16,
    key: &'static str,
    transaction_id: u8,
    leds: &'static [u8],
    has_battery: bool,
}

/// A Razer product id this app drives.
pub fn is_supported(pid: u16) -> bool {
    MODELS.iter().any(|m| m.pid == pid)
}

const MODELS: &[Model] = &[
    // Naga V2 Pro: wireless (HyperSpeed) and wired PIDs. 0x01 scroll wheel, 0x04 logo, 0x05 side.
    Model { pid: 0x00A8, key: "naga", transaction_id: 0x1F, leds: &[0x01, 0x04, 0x05], has_battery: true },
    Model { pid: 0x00A7, key: "naga", transaction_id: 0x1F, leds: &[0x01, 0x04, 0x05], has_battery: true },
    Model { pid: 0x00A4, key: "dock", transaction_id: 0xFF, leds: &[0x00], has_battery: false },
];

pub struct Razer {
    hid: Hid,
    model: &'static Model,
    /// Unique id ("<kind>-<serial>"); None when a wireless device doesn't answer (no link),
    /// so its serial can't be read.
    pub id: Option<String>,
}

impl Razer {
    pub fn find_all() -> Vec<Razer> {
        hid::enumerate(VID)
            .into_iter()
            .filter(|i| i.usage_page == 0x0001 && i.usage == 0x0002 && i.feature_len == 91)
            .filter_map(|i| {
                let model = MODELS.iter().find(|m| m.pid == i.pid)?;
                // Mouse collections can't be opened for read/write, but feature reports work.
                let mut d = Razer { hid: Hid::open(&i.path, false, false)?, model, id: None };
                d.id = match d.command(0x00, 0x82, &[]) {
                    Ok(r) => {
                        let serial: String = r[9..9 + 22].iter().take_while(|&&b| b != 0).filter(|b| b.is_ascii_alphanumeric()).map(|&b| b as char).collect();
                        let serial = if serial.is_empty() { super::path_hash(&i.path) } else { serial };
                        Some(format!("{}-{serial}", model.key))
                    }
                    Err(e) if e == super::NO_LINK => None,
                    Err(_) => Some(format!("{}-{}", model.key, super::path_hash(&i.path))),
                };
                Some(d)
            })
            .collect()
    }

    pub fn key(&self) -> &'static str {
        self.model.key
    }

    /// Sends a command and returns the device's response report.
    fn command(&self, class: u8, id: u8, args: &[u8]) -> Result<[u8; 91], String> {
        // Byte 0 is the HID report ID (0); the 90-byte Razer report follows.
        let mut buf = [0u8; 91];
        buf[2] = self.model.transaction_id;
        buf[6] = args.len() as u8;
        buf[7] = class;
        buf[8] = id;
        buf[9..9 + args.len()].copy_from_slice(args);
        buf[89] = buf[3..89].iter().fold(0, |a, b| a ^ b);
        if !self.hid.set_feature(&buf) {
            return Err("Razer: send failed".into());
        }
        for _ in 0..15 {
            std::thread::sleep(Duration::from_millis(20));
            let mut r = [0u8; 91];
            if !self.hid.get_feature(&mut r) {
                return Err("Razer: read failed".into());
            }
            match r[1] {
                0x01 => continue, // busy
                0x02 => return Ok(r),
                0x04 => return Err(super::NO_LINK.into()),
                0x05 => return Err("Razer: command not supported".into()),
                s => return Err(format!("Razer: status {s:#04x}")),
            }
        }
        Err("Razer: device busy".into())
    }
}

impl Razer {
    /// Applies the effect to each LED with its own color.
    fn apply_leds(&self, effect: Effect, leds: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        let level = if effect == Effect::Off { 0 } else { (brightness.min(100) as u32 * 255 / 100) as u8 };
        let mut last_err = None;
        let mut any_ok = false;
        // Not every LED id exists on every revision; unsupported ones are skipped.
        for &(led, Rgb(r, g, b)) in leds {
            let res = self.command(0x0F, 0x04, &[VARSTORE, led, level]).and_then(|_| match effect {
                Effect::Off => Ok([0; 91]),
                Effect::Static => self.command(0x0F, 0x02, &[VARSTORE, led, 0x01, 0, 0, 0x01, r, g, b]),
                Effect::Breathe => self.command(0x0F, 0x02, &[VARSTORE, led, 0x02, 0x01, 0, 0x01, r, g, b]),
                Effect::Cycle => self.command(0x0F, 0x02, &[VARSTORE, led, 0x03, 0, 0, 0]),
            });
            match res {
                Ok(_) => any_ok = true,
                Err(e) => last_err = Some(e),
            }
        }
        match (any_ok, last_err) {
            (false, Some(e)) => Err(e),
            _ => Ok(()),
        }
    }
}

impl Device for Razer {
    fn apply(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String> {
        let leds: Vec<(u8, Rgb)> = self.model.leds.iter().map(|&led| (led, color)).collect();
        self.apply_leds(effect, &leds, brightness)
    }

    fn apply_zones(&self, effect: Effect, zones: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        self.apply_leds(effect, zones, brightness)
    }

    fn battery(&self) -> Option<Battery> {
        if !self.model.has_battery {
            return None;
        }
        let level = self.command(0x07, 0x80, &[0, 0]).ok()?[10];
        let charging = self.command(0x07, 0x84, &[0, 0]).map(|r| r[10] == 1).unwrap_or(false);
        Some(Battery { percent: (level as u32 * 100 / 255) as u8, charging })
    }
}
