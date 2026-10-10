//! QMK keyboards with VIA support: raw HID, 32-byte reports on usage page FF60 / usage 0x61.
//! One protocol for hundreds of keyboards (Keychron, hand-built boards, the Geekboard MacroPad).
//!
//! Protocol 12 and later (VIA 3) addresses lighting by channel (`07 <channel> <value> <data>`;
//! RGB Matrix = 3, RGBLight = 2); older ones use fixed value ids (`07 80 <data>`...). Values are
//! set in RAM only (no EEPROM save), so the keyboard's own saved settings stay untouched.
//!
//! Effect numbers differ between firmwares, except 0 (off) and 1 (static color), so breathing
//! and spectrum are played by the app, unless a known model says which firmware effect to use.

use super::{animation, Device, Effect, Rgb};
use crate::hid::{self, Hid};

const USAGE_PAGE: u16 = 0xFF60;
const USAGE: u16 = 0x61;

const CMD_PROTOCOL_VERSION: u8 = 0x01;
const CMD_SET: u8 = 0x07;
const CMD_GET: u8 = 0x08;
/// The firmware's answer to a command it doesn't handle.
const UNHANDLED: u8 = 0xFF;

/// Effect numbers that mean the same in every QMK firmware.
const EFFECT_OFF: u8 = 0;
const EFFECT_STATIC: u8 = 1;

/// Geekboard MacroPad v2.
pub const MACROPAD_VID: u16 = 0x0483;
pub const MACROPAD_PID: u16 = 0xA372;

/// What is known about a particular keyboard beyond the protocol.
struct Model {
    vid: u16,
    pid: u16,
    /// Device id prefix (its kind in KINDS).
    key: &'static str,
    /// Firmware effect for "Spectrum", if it has a usable one.
    cycle: Option<u8>,
}

const MODELS: &[Model] = &[
    // Mode 6 is a rainbow; its modes 2-5 blink single keys instead of breathing.
    Model { vid: MACROPAD_VID, pid: MACROPAD_PID, key: "macropad", cycle: Some(6) },
];

/// How lighting values are addressed.
#[derive(Clone, Copy, Debug)]
enum Channel {
    /// Protocol up to 11: value ids 0x80 brightness, 0x81 effect, 0x82 speed, 0x83 color.
    Legacy,
    /// Protocol 12+: a channel (3 RGB Matrix, 2 RGBLight) with values 1 brightness, 2 effect, 3 speed, 4 color.
    Custom(u8),
}

impl Channel {
    /// The command bytes that address a value: brightness, effect or color.
    fn address(self, value: Value) -> Vec<u8> {
        match self {
            Channel::Legacy => vec![
                0x80 + match value {
                    Value::Brightness => 0,
                    Value::Effect => 1,
                    Value::Speed => 2,
                    Value::Color => 3,
                },
            ],
            Channel::Custom(ch) => vec![
                ch,
                match value {
                    Value::Brightness => 1,
                    Value::Effect => 2,
                    Value::Speed => 3,
                    Value::Color => 4,
                },
            ],
        }
    }
}

#[derive(Clone, Copy)]
enum Value {
    Brightness,
    Effect,
    Color,
    Speed,
}

pub struct Via {
    /// "<kind>-<hash of the HID path>": QMK reports no serial number, so the id comes from the
    /// USB port.
    pub id: String,
    path: Vec<u16>,
    hid: Hid,
    channel: Channel,
    cycle: Option<u8>,
}

/// VIA keyboards with RGB lighting.
pub fn find_all() -> Vec<Via> {
    let mut out = Vec::new();
    for info in hid::enumerate_all().into_iter().filter(|i| i.usage_page == USAGE_PAGE && i.usage == USAGE) {
        let Some(hid) = Hid::open(&info.path, true, true) else { continue };
        let model = MODELS.iter().find(|m| m.vid == info.vid && m.pid == info.pid);
        let Some(channel) = probe(&hid) else { continue };
        let key = model.map_or("via", |m| m.key);
        let id = format!("{key}-{}", super::path_hash(&info.path));
        if model.is_none() {
            super::set_name(&id, &super::device_name(&info));
        }
        out.push(Via { id, path: info.path.clone(), hid, channel, cycle: model.and_then(|m| m.cycle) });
    }
    out
}

/// The lighting channel of a keyboard, or None without RGB lighting.
fn probe(hid: &Hid) -> Option<Channel> {
    let r = request(hid, &[CMD_PROTOCOL_VERSION])?;
    // Answer: 01 <version, big-endian>.
    let version = u16::from_be_bytes([r[1], r[2]]);
    if version >= 12 {
        // RGB Matrix first: keyboards with both use it for the keys.
        [3u8, 2].into_iter().map(Channel::Custom).find(|&ch| {
            let mut cmd = vec![CMD_GET];
            cmd.extend(ch.address(Value::Brightness));
            request(hid, &cmd).is_some()
        })
    } else {
        let mut cmd = vec![CMD_GET];
        cmd.extend(Channel::Legacy.address(Value::Brightness));
        request(hid, &cmd).map(|_| Channel::Legacy)
    }
}

/// Sends a command and returns its answer (bytes after the report id), None if the firmware
/// doesn't handle it or doesn't answer.
fn request(hid: &Hid, bytes: &[u8]) -> Option<[u8; 32]> {
    // Answers left over from animation frames, which aren't waited for.
    let mut stale = [0u8; 33];
    for _ in 0..64 {
        if !matches!(hid.read(&mut stale, 0), Ok(Some(_))) {
            break;
        }
    }
    let mut buf = [0u8; 33];
    buf[1..1 + bytes.len()].copy_from_slice(bytes);
    if !hid.write(&buf) {
        return None;
    }
    let mut resp = [0u8; 33];
    for _ in 0..4 {
        match hid.read(&mut resp, 300) {
            Ok(Some(_)) if resp[1] == UNHANDLED => return None,
            Ok(Some(_)) if resp[1..1 + bytes.len().min(3)] == buf[1..1 + bytes.len().min(3)] => {
                let mut out = [0u8; 32];
                out.copy_from_slice(&resp[1..]);
                return Some(out);
            }
            Ok(Some(_)) => continue,
            _ => break,
        }
    }
    None
}

impl Via {
    fn set(&self, value: Value, data: &[u8]) -> Result<(), String> {
        let mut cmd = vec![CMD_SET];
        cmd.extend(self.channel.address(value));
        cmd.extend_from_slice(data);
        request(&self.hid, &cmd).map(|_| ()).ok_or_else(|| "VIA: no response".to_string())
    }

    /// Plays breathing or spectrum with its own handle to the keyboard. Frames don't wait for
    /// the keyboard's answers.
    fn start_animation(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String> {
        let hid = Hid::open(&self.path, true, true).ok_or("VIA: cannot open")?;
        let channel = self.channel;
        let frame = move |value: Value, data: &[u8]| {
            let mut buf = [0u8; 33];
            buf[1] = CMD_SET;
            let addr = channel.address(value);
            buf[2..2 + addr.len()].copy_from_slice(&addr);
            buf[2 + addr.len()..2 + addr.len() + data.len()].copy_from_slice(data);
            hid.write(&buf)
        };
        let (h, s) = hue_sat(color);
        let level = value_of(color, brightness);
        animation::start(
            &self.path,
            Box::new(move |t| match effect {
                Effect::Breathe => frame(Value::Brightness, &[(level as f32 * animation::breathe(t)) as u8]),
                _ => {
                    let hue = (animation::cycle_hue(t).rem_euclid(360.0) / 360.0 * 255.0) as u8;
                    frame(Value::Color, &[hue, 255])
                }
            }),
        );
        // The color and brightness the frames don't change.
        self.set(Value::Color, &[h, if effect == Effect::Cycle { 255 } else { s }])?;
        if effect == Effect::Cycle {
            self.set(Value::Brightness, &[(255 * brightness.min(100) as u32 / 100) as u8])?;
        }
        Ok(())
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

/// QMK value (brightness 0..=255): the color's own lightness times the brightness slider.
fn value_of(Rgb(r, g, b): Rgb, brightness: u8) -> u8 {
    (r.max(g).max(b) as u32 * brightness.min(100) as u32 / 100) as u8
}

impl Device for Via {
    fn apply(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String> {
        animation::stop(&self.path);
        match effect {
            Effect::Off => return self.set(Value::Effect, &[EFFECT_OFF]),
            Effect::Cycle if self.cycle.is_some() => {
                self.set(Value::Effect, &[self.cycle.unwrap_or(EFFECT_STATIC)])?;
                // QMK effect speed 0..=255, about 128 normal. Not every firmware has it, so a refusal is ignored.
                let _ = self.set(Value::Speed, &[(super::speed() as u32 * 255 / 100) as u8]);
                let (h, s) = hue_sat(color);
                self.set(Value::Color, &[h, s])?;
                return self.set(Value::Brightness, &[value_of(color, brightness)]);
            }
            _ => {}
        }
        self.set(Value::Effect, &[EFFECT_STATIC])?;
        if matches!(effect, Effect::Breathe | Effect::Cycle) {
            return self.start_animation(effect, color, brightness);
        }
        let (h, s) = hue_sat(color);
        self.set(Value::Color, &[h, s])?;
        self.set(Value::Brightness, &[value_of(color, brightness)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qmk_hue_and_saturation() {
        assert_eq!(hue_sat(Rgb(255, 0, 0)), (0, 255));
        assert_eq!(hue_sat(Rgb(0, 255, 0)), (85, 255));
        assert_eq!(hue_sat(Rgb(0, 0, 255)), (170, 255));
        assert_eq!(hue_sat(Rgb(255, 128, 128)), (0, 127));
        assert_eq!(hue_sat(Rgb(128, 128, 128)), (0, 0));
        assert_eq!(hue_sat(Rgb(0, 0, 0)), (0, 0));
    }

    #[test]
    fn qmk_value() {
        assert_eq!(value_of(Rgb(255, 0, 0), 100), 255);
        assert_eq!(value_of(Rgb(255, 0, 0), 50), 127);
        assert_eq!(value_of(Rgb(100, 20, 0), 100), 100);
        assert_eq!(value_of(Rgb(255, 0, 0), 200), 255);
    }
}
