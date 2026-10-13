//! Razer devices over the 90-byte Chroma control report (as documented by OpenRazer).

use std::time::Duration;

use super::{Battery, Device, Effect, Rgb};
use crate::hid::{self, Hid};

const VID: u16 = 0x1532;
const VARSTORE: u8 = 0x01;

/// Parameters of a Razer model, from its profile (`profiles/razer-*.toml`).
#[derive(Debug)]
pub struct RazerModel {
    /// USB product ids, e.g. wireless and wired.
    pub pids: &'static [u16],
    pub transaction_id: u8,
    /// LED ids lit as "the whole device".
    pub leds: &'static [u8],
    pub battery: bool,
}

/// The model (its id prefix and parameters) of a Razer product id.
fn model_of(pid: u16) -> Option<(&'static str, &'static RazerModel)> {
    super::kinds().iter().find_map(|k| match k.protocol {
        super::Protocol::Razer(m) if m.pids.contains(&pid) => Some((k.key, m)),
        _ => None,
    })
}

/// Id ("<kind>-<serial>") from the answer to the serial number request.
/// A wired-only device without a readable serial is told apart by its HID path. A wireless one
/// isn't: its dongle's path would make a second device of the same mouse whenever the mouse
/// doesn't answer properly (12.10.2026, Naga V2 Pro charging from a wall charger), so it has no link.
fn device_id(key: &str, wireless: bool, answer: Result<[u8; 91], String>, path_hash: impl FnOnce() -> String) -> Option<String> {
    if answer.as_ref().is_err_and(|e| e == super::NO_LINK) {
        return None;
    }
    let serial =
        answer.ok().map(|r| r[9..9 + 22].iter().take_while(|&&b| b != 0).filter(|b| b.is_ascii_alphanumeric()).map(|&b| b as char).collect::<String>());
    match serial.filter(|s| !s.is_empty()) {
        Some(s) => Some(format!("{key}-{s}")),
        None if wireless => None,
        None => Some(format!("{key}-{}", path_hash())),
    }
}

/// A Razer product id this app drives.
pub fn is_supported(pid: u16) -> bool {
    model_of(pid).is_some()
}

pub struct Razer {
    hid: Hid,
    key: &'static str,
    model: &'static RazerModel,
    /// Unique id ("<kind>-<serial>"); None when a wireless device doesn't answer (no link),
    /// so its serial can't be read.
    pub id: Option<String>,
}

impl Razer {
    pub fn find_all() -> Vec<Razer> {
        let infos: Vec<hid::HidInfo> = hid::enumerate(VID).into_iter().filter(|i| i.feature_len == 91 && is_supported(i.pid)).collect();
        // The control report is on a top-level collection: the mouse one of mice and the
        // dock, the keyboard one of keyboards, some other one of accessories. Per product id
        // only the best kind is used, so a device isn't found again through other collections.
        let rank = |i: &hid::HidInfo| match (i.usage_page, i.usage) {
            (0x0001, 0x0002) => 0,
            (0x0001, 0x0006) => 1,
            _ => 2,
        };
        let best: Vec<&hid::HidInfo> = infos.iter().filter(|i| infos.iter().filter(|o| o.pid == i.pid).all(|o| rank(o) >= rank(i))).collect();
        let mut found: Vec<Razer> = Vec::new();
        for d in best.into_iter().filter_map(|i| {
            let (key, model) = model_of(i.pid)?;
            // Mouse collections can't be opened for read/write, but feature reports work.
            let mut d = Razer { hid: Hid::open(&i.path, false, false)?, key, model, id: None };
            d.id = device_id(key, d.model.battery, d.command(0x00, 0x82, &[]), || super::path_hash(&i.path));
            Some(d)
        }) {
            // One device with several such collections answers with the same serial on each.
            if d.id.is_none() || !found.iter().any(|f| f.id == d.id) {
                found.push(d);
            }
        }
        found
    }

    pub fn key(&self) -> &'static str {
        self.key
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
                // Only for models whose profile lists them (as OpenRazer drives them; untested).
                // Wave: direction 1, speed 0x28 (OpenRazer's normal; lower is faster).
                Effect::Wave => self.command(0x0F, 0x02, &[VARSTORE, led, 0x04, 0x01, 0x28, 0]),
                // Reactive: speed 1..=4; starlight: speed 1..=3; both in the chosen color.
                Effect::Reactive => self.command(0x0F, 0x02, &[VARSTORE, led, 0x05, 0, 0x02, 0x01, r, g, b]),
                Effect::Starlight => self.command(0x0F, 0x02, &[VARSTORE, led, 0x07, 0, 0x02, 0x01, r, g, b]),
                // No ripple in Razer firmware: the color, steadily.
                Effect::Ripple => self.command(0x0F, 0x02, &[VARSTORE, led, 0x01, 0, 0, 0x01, r, g, b]),
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
        if !self.model.battery {
            return None;
        }
        let level = self.command(0x07, 0x80, &[0, 0]).ok()?[10];
        let charging = self.command(0x07, 0x84, &[0, 0]).map(|r| r[10] == 1).unwrap_or(false);
        Some(Battery { percent: (level as u32 * 100 / 255) as u8, charging })
    }
}

/// How the app finds Razer devices (see `devices::DRIVERS`).
pub const DRIVER: super::Driver = super::Driver {
    discover: |d| {
        for r in Razer::find_all() {
            match r.id.clone() {
                Some(id) => d.found.push(super::Found { id, dev: Box::new(r) }),
                None => d.no_link_kinds.push(r.key()),
            }
        }
    },
    batteries: || Razer::find_all().into_iter().filter_map(|d| Some((d.id.clone()?, d.battery()?))).collect(),
    answering: || Razer::find_all().into_iter().filter_map(|d| d.id).collect(),
    // Lighting goes through 91-byte feature reports (the mouse, keyboard or other collection).
    note: |i| match i.vid {
        VID if i.feature_len == 91 && is_supported(i.pid) => Some("SUPPORTED: Razer"),
        VID if i.feature_len == 91 => Some("Razer control interface (protocol known)"),
        _ => None,
    },
    ..super::Driver::NONE
};

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(serial: &str) -> Result<[u8; 91], String> {
        let mut r = [0u8; 91];
        r[9..9 + serial.len()].copy_from_slice(serial.as_bytes());
        Ok(r)
    }

    #[test]
    fn device_id_uses_the_serial() {
        assert_eq!(device_id("naga", true, answer("123456H78900012"), || "HASH".into()), Some("naga-123456H78900012".into()));
    }

    #[test]
    fn wireless_device_without_serial_has_no_link() {
        assert_eq!(device_id("naga", true, answer(""), || "HASH".into()), None);
        assert_eq!(device_id("naga", true, Err("Razer: status 0x03".into()), || "HASH".into()), None);
        assert_eq!(device_id("naga", true, Err(crate::devices::NO_LINK.into()), || "HASH".into()), None);
    }

    #[test]
    fn wired_device_without_serial_goes_by_path() {
        assert_eq!(device_id("kbd", false, answer(""), || "HASH".into()), Some("kbd-HASH".into()));
        assert_eq!(device_id("kbd", false, Err("Razer: read failed".into()), || "HASH".into()), Some("kbd-HASH".into()));
        assert_eq!(device_id("kbd", false, Err(crate::devices::NO_LINK.into()), || "HASH".into()), None);
    }
}
