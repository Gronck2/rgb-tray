//! Geekboard MacroPad v2 (QMK firmware) over the VIA raw HID protocol, version 9:
//! 32-byte reports on usage page FF60 / usage 0x61. Lighting values are set in RAM only
//! (no EEPROM save), so macros and saved settings in the firmware stay untouched.

use super::{Device, Effect, Rgb};
use crate::hid::{self, Hid};

const VID: u16 = 0x0483;
const PID: u16 = 0xA372;

const CMD_SET_LIGHTING: u8 = 0x07;
const BRIGHTNESS: u8 = 0x80;
const EFFECT: u8 = 0x81;
const COLOR: u8 = 0x83;

/// QMK rgblight modes: 0 disables the lighting.
const MODE_OFF: u8 = 0;
const MODE_STATIC: u8 = 1;
const MODE_CYCLE: u8 = 6;

pub struct MacroPad {
    hid: Hid,
    /// The firmware reports no serial number, so the id comes from the USB port.
    pub id: String,
}

impl MacroPad {
    pub fn find_all() -> Vec<MacroPad> {
        hid::enumerate(VID)
            .into_iter()
            .filter(|i| i.pid == PID && i.usage_page == 0xFF60 && i.usage == 0x61)
            .filter_map(|i| Some(MacroPad { hid: Hid::open(&i.path, true, true)?, id: format!("macropad-{}", super::path_hash(&i.path)) }))
            .collect()
    }

    /// Sends one VIA command and waits for its echo.
    fn send(&self, bytes: &[u8]) -> Result<(), String> {
        let mut buf = [0u8; 33];
        buf[1..1 + bytes.len()].copy_from_slice(bytes);
        if !self.hid.write(&buf) {
            return Err("MacroPad: ошибка отправки".into());
        }
        let mut resp = [0u8; 33];
        for _ in 0..4 {
            match self.hid.read(&mut resp, 300) {
                Ok(Some(_)) if resp[1..3] == buf[1..3] => return Ok(()),
                Ok(Some(_)) => continue,
                _ => break,
            }
        }
        Err("MacroPad: нет ответа".into())
    }
}

/// RGB to QMK hue/saturation (0..=255 each); QMK takes brightness separately.
fn hue_sat(Rgb(r, g, b): Rgb) -> (u8, u8) {
    let (r, g, b) = (r as f32, g as f32, b as f32);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    if max == 0.0 || d == 0.0 {
        return (0, 0);
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    ((h / 6.0 * 255.0).round() as u8, (d / max * 255.0).round() as u8)
}

impl Device for MacroPad {
    fn apply(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String> {
        let mode = match effect {
            Effect::Off => return self.send(&[CMD_SET_LIGHTING, EFFECT, MODE_OFF]),
            Effect::Static => MODE_STATIC,
            // This firmware has no usable breathing (its modes 2-5 blink single keys): steady color.
            Effect::Breathe => MODE_STATIC,
            Effect::Cycle => MODE_CYCLE,
        };
        let (h, s) = hue_sat(color);
        // QMK value scales the color: the color's own lightness times the brightness slider.
        let Rgb(r, g, b) = color;
        let value = r.max(g).max(b) as u32 * brightness.min(100) as u32 / 100;
        self.send(&[CMD_SET_LIGHTING, EFFECT, mode])?;
        self.send(&[CMD_SET_LIGHTING, COLOR, h, s])?;
        self.send(&[CMD_SET_LIGHTING, BRIGHTNESS, value as u8])
    }
}
