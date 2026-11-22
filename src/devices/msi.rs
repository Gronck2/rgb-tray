//! MSI Vigor GK50 Elite (USB 0DB0:0B5B), protocol read from a USB capture of MSI Center.
//!
//! The vendor collection (FF00:0001) takes 64-byte output reports (no report id) and answers
//! each one. Every change is a transaction: `41 80` (begin), `56 21 <page> 00 <data>` (write a
//! settings page), `51 28 00 00 <mode>`, `41` (end). Modes: 01 static (page 01), 02 breathe and
//! 03 wave (page 02), 0B per-key (pages 05-0C), 0C off.
//!
//! MSI Center also sends `50 55` before the end; it most likely saves to flash, so it is never
//! sent here. Without it the lighting lives in RAM: on replug the keyboard shows its saved
//! lighting again (checked 22.11.2026).
//!
//! The per-key table is 132 RGB slots, column by column, 6 rows each (Esc, `, Tab, Caps, Shift,
//! Ctrl in column 0), 22 columns. `SLOTS` names the key of each slot; the empty slots are the
//! gaps of the full-size layout (MSI Center reads them back as black).

use super::{animation, Device, Effect, Rgb};
use crate::hid::{self, Hid, HidInfo};
use std::sync::Mutex;

const VID: u16 = 0x0DB0;
const PID: u16 = 0x0B5B;
const USAGE_PAGE: u16 = 0xFF00;
const USAGE: u16 = 0x0001;

/// Key of each slot of the per-key table, by its name in `keymap` ("" = no LED).
#[rustfmt::skip]
const SLOTS: [&str; 132] = [
    // Column 0..=5.
    "Esc", "Grave", "Tab", "CapsLock", "LShift", "LCtrl",
    "", "1", "Q", "A", "NonUsBackslash", "LWin",
    "F1", "2", "W", "S", "Z", "LAlt",
    "F2", "3", "E", "D", "X", "",
    "F3", "4", "R", "F", "C", "",
    "F4", "5", "T", "G", "V", "",
    // Column 6..=11.
    "", "6", "Y", "H", "B", "Space",
    "F5", "7", "U", "J", "N", "",
    "F6", "8", "I", "K", "M", "",
    "F7", "9", "O", "L", "Comma", "",
    "F8", "0", "P", "Semicolon", "Period", "RAlt",
    "F9", "Minus", "LBracket", "Quote", "Slash", "",
    // Column 12..=14. The GK50 has Fn where others have the right Win key.
    "F10", "Equal", "RBracket", "NonUsHash", "", "RWin",
    "F11", "", "", "", "RShift", "Menu",
    "F12", "Backspace", "Backslash", "Enter", "", "RCtrl",
    // Column 15..=17: navigation and arrows.
    "PrintScreen", "Insert", "Delete", "", "", "Left",
    "ScrollLock", "Home", "End", "", "Up", "Down",
    "Pause", "PageUp", "PageDown", "", "", "Right",
    // Column 18..=21: keypad.
    "", "NumLock", "Kp7", "Kp4", "Kp1", "Kp0",
    "", "KpSlash", "Kp8", "Kp5", "Kp2", "",
    "", "KpAsterisk", "Kp9", "Kp6", "Kp3", "KpDecimal",
    "", "KpMinus", "KpPlus", "", "", "KpEnter",
];

/// Page 05 as MSI Center writes it before the per-key table.
const KEYS_HEAD: [u8; 60] = [
    0x01, 0x00, 0x00, 0x00, 0x1E, 0xFF, 0x12, 0xFF, 0x00, 0x00, 0x04, 0x00, 0x00, 0x90, 0x1A, 0x32, 0x4A, 0x00, 0x00, 0x00, 0x30, 0x80, 0x00, 0x00, 0x01, 0x00,
    0x00, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x10, 0x25, 0x50, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0xC1, 0x00, 0x00, 0x00, 0x00,
    0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00,
];
/// What follows the last 12 slots in page 0C.
const KEYS_TAIL: [u8; 13] = [0x56, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0xC1, 0x00, 0x00, 0x00, 0x00, 0xBE];

/// Page 01 (static color): the color and brightness go at byte 44 of the page data.
fn static_page(c: Rgb) -> Vec<u8> {
    let mut p = vec![0xFF; 24];
    p.extend_from_slice(&[0x0B, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0xC1, 0x00, 0x00, 0x00, 0x00]);
    p.extend_from_slice(&[c.0, c.1, c.2, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00, 0x50, 0x30]);
    p
}

/// A 64-byte report (with the report id 00 in front: 65 bytes).
fn report(bytes: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; 65];
    r[1..1 + bytes.len()].copy_from_slice(bytes);
    r
}

fn page(n: u8, data: &[u8]) -> Vec<u8> {
    report(&[&[0x56, 0x21, n, 0x00][..], data].concat())
}

/// A whole transaction: begin, the pages, the mode, end.
fn transaction(pages: Vec<Vec<u8>>, mode: u8) -> Vec<Vec<u8>> {
    let mut out = vec![report(&[0x41, 0x80])];
    out.extend(pages);
    out.push(report(&[0x51, 0x28, 0x00, 0x00, mode]));
    out.push(report(&[0x41]));
    out
}

fn static_reports(c: Rgb) -> Vec<Vec<u8>> {
    transaction(vec![page(0x01, &static_page(c))], 0x01)
}

/// The per-key reports for a color of each slot.
fn keys_reports(slots: &[Rgb; 132]) -> Vec<Vec<u8>> {
    let rgb: Vec<u8> = slots.iter().flat_map(|c| [c.0, c.1, c.2]).collect();
    let mut pages = vec![page(0x05, &KEYS_HEAD)];
    for p in 0..6 {
        pages.push(page(0x06 + p as u8, &rgb[p * 60..p * 60 + 60]));
    }
    pages.push(page(0x0C, &[&rgb[360..], &KEYS_TAIL[..]].concat()));
    transaction(pages, 0x0B)
}

/// Slot of each editor key code.
fn slot_of(code: u8) -> Option<usize> {
    static CODES: std::sync::OnceLock<Vec<Option<u8>>> = std::sync::OnceLock::new();
    let codes = CODES.get_or_init(|| SLOTS.iter().map(|n| crate::keymap::code_of(n)).collect());
    codes.iter().position(|&c| c == Some(code))
}

/// Per-key colors shown on each keyboard (by HID path), for changes of a few keys.
static KEYS: Mutex<Vec<(Vec<u16>, [Rgb; 132])>> = Mutex::new(Vec::new());

pub struct Gk50 {
    /// "gk50-<hash of the HID path>": the keyboard has no serial number.
    pub id: String,
    path: Vec<u16>,
    hid: Hid,
}

fn send(hid: &Hid, reports: &[Vec<u8>]) -> bool {
    reports.iter().all(|r| hid.write(r))
}

fn ours(i: &HidInfo) -> bool {
    i.vid == VID && i.pid == PID && i.usage_page == USAGE_PAGE && i.usage == USAGE && i.output_len == 65
}

fn find_all() -> Vec<Gk50> {
    hid::enumerate(VID)
        .into_iter()
        .filter(ours)
        .filter_map(|i| Some(Gk50 { id: format!("gk50-{}", super::path_hash(&i.path)), hid: Hid::open(&i.path, true, true)?, path: i.path }))
        .collect()
}

impl Gk50 {
    fn send(&self, reports: &[Vec<u8>]) -> Result<(), String> {
        if send(&self.hid, reports) {
            Ok(())
        } else {
            Err("GK50: send failed".into())
        }
    }

    fn show_keys(&self, slots: [Rgb; 132], brightness: u8) -> Result<(), String> {
        let scaled = slots.map(|c| c.scaled(brightness));
        let mut all = KEYS.lock().unwrap();
        match all.iter_mut().find(|(p, _)| *p == self.path) {
            Some(k) => k.1 = slots,
            None => all.push((self.path.clone(), slots)),
        }
        drop(all);
        self.send(&keys_reports(&scaled))
    }
}

impl Device for Gk50 {
    fn hid_path(&self) -> Option<&[u16]> {
        Some(&self.path)
    }

    fn apply(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String> {
        animation::stop(&self.path);
        match effect {
            Effect::Off => self.send(&static_reports(Rgb(0, 0, 0))),
            Effect::Breathe | Effect::Cycle => {
                self.send(&static_reports(color.scaled(brightness)))?;
                // Drawn by the app, from its own handle: the firmware effects need page 02,
                // which isn't worked out.
                let hid = Hid::open(&self.path, true, true).ok_or("GK50: cannot open")?;
                animation::start(
                    &self.path,
                    Box::new(move |t| {
                        let c = match effect {
                            Effect::Breathe => color.scaled((brightness as f32 * animation::breathe(t)) as u8),
                            _ => crate::hue(animation::cycle_hue(t)).scaled(brightness),
                        };
                        send(&hid, &static_reports(c))
                    }),
                );
                Ok(())
            }
            _ => self.send(&static_reports(color.scaled(brightness))),
        }
    }

    fn apply_keys(&self, keys: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        animation::stop(&self.path);
        let mut slots = [Rgb(0, 0, 0); 132];
        for &(code, c) in keys {
            if let Some(s) = slot_of(code) {
                slots[s] = c;
            }
        }
        self.show_keys(slots, brightness)
    }

    fn update_keys(&self, keys: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        let shown = KEYS.lock().unwrap().iter().find(|(p, _)| *p == self.path).map(|k| k.1);
        let Some(mut slots) = shown else { return self.apply_keys(keys, brightness) };
        for &(code, c) in keys {
            if let Some(s) = slot_of(code) {
                slots[s] = c;
            }
        }
        self.show_keys(slots, brightness)
    }
}

pub const DRIVER: super::Driver = super::Driver {
    discover: |d| {
        for kb in find_all() {
            d.found.push(super::Found { id: kb.id.clone(), dev: Box::new(kb) });
        }
    },
    open_for_keys: |id| match id.starts_with("gk50-") {
        true => find_all().into_iter().find(|k| k.id == id).map(|k| Box::new(k) as Box<dyn Device>),
        false => None,
    },
    note: |i| ours(i).then_some("SUPPORTED: MSI Vigor GK50 Elite lighting"),
    ..super::Driver::NONE
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_slot_names_a_known_key_once() {
        let named: Vec<&str> = SLOTS.iter().copied().filter(|n| !n.is_empty()).collect();
        // 104 keys of the full ANSI layout and the two ISO ones.
        assert_eq!(named.len(), 106);
        for (i, n) in named.iter().enumerate() {
            assert!(crate::keymap::code_of(n).is_some(), "unknown key {n}");
            assert!(!named[i + 1..].contains(n), "{n} twice");
        }
        // Every key of the full layout has a slot.
        let full = crate::keymap::get(crate::keymap::FULL);
        assert!(full.codes().iter().all(|&c| slot_of(c).is_some()));
    }

    #[test]
    fn reports_match_the_capture() {
        // Static red as MSI Center sent it (page 01, bytes 48-51 of the report data).
        let r = static_reports(Rgb(0xFF, 0x28, 0x21));
        assert_eq!(r.len(), 4);
        assert!(r.iter().all(|r| r.len() == 65 && r[0] == 0));
        assert_eq!(&r[0][1..3], &[0x41, 0x80]);
        assert_eq!(&r[1][1..5], &[0x56, 0x21, 0x01, 0x00]);
        assert_eq!(&r[1][49..53], &[0xFF, 0x28, 0x21, 0xFF]);
        assert_eq!(&r[1][59..63], &[0x00, 0x00, 0x50, 0x30]);
        assert_eq!(&r[2][1..6], &[0x51, 0x28, 0x00, 0x00, 0x01]);
        assert_eq!(&r[3][1..3], &[0x41, 0x00]);
        // No "save" (50 55) anywhere.
        assert!(r.iter().all(|r| r[1..3] != [0x50, 0x55]));

        let mut slots = [Rgb(0, 0, 0); 132];
        slots[0] = Rgb(1, 2, 3);
        slots[131] = Rgb(4, 5, 6);
        let k = keys_reports(&slots);
        // Begin, page 05, pages 06-0C, mode 0B, end.
        assert_eq!(k.len(), 1 + 1 + 7 + 1 + 1);
        assert_eq!(&k[2][1..8], &[0x56, 0x21, 0x06, 0x00, 1, 2, 3]);
        assert_eq!(&k[8][1..5], &[0x56, 0x21, 0x0C, 0x00]);
        assert_eq!(&k[8][5 + 33..5 + 36], &[4, 5, 6]);
        assert_eq!(&k[8][5 + 36..5 + 49], &KEYS_TAIL);
        assert_eq!(&k[9][1..6], &[0x51, 0x28, 0x00, 0x00, 0x0B]);
        assert!(k.iter().all(|r| r.len() == 65 && r[1..3] != [0x50, 0x55]));
    }
}
