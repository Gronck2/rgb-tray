//! Devices described only by a profile's `[hid]` section: HID reports written as byte templates,
//! with the color put into them. Enough for simple protocols, where one or a few reports set a
//! color. Anything that needs answers, checksums or state is a driver of its own.
//!
//! Breathing and spectrum use the firmware's effect if the profile gives its reports; otherwise
//! the app draws them by sending the static reports about 60 times a second.

use super::{animation, Device, Effect, Kind, Rgb};
use crate::hid::{self, Hid, HidInfo};

/// One byte of a report template: a number, or a value filled in when it is sent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Byte {
    Lit(u8),
    /// The color scaled by brightness: `{r}`, `{g}`, `{b}`.
    R,
    G,
    B,
    /// The color as chosen, for devices that take brightness separately: `{red}`, `{green}`, `{blue}`.
    Red,
    Green,
    Blue,
    /// Brightness 0..=255: `{brightness}`.
    Brightness,
    /// Brightness 0..=100: `{percent}`.
    Percent,
    /// Effect speed 0..=255, about 127 normal, higher is faster: `{speed}`.
    Speed,
    /// The zone's LED id (`[[zones]]`): `{zone}`. A report with it is sent once per zone.
    Zone,
}

/// A report: its bytes from the report id on; the rest of the report is zeros.
pub type Packet = &'static [Byte];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Report {
    /// HidD_SetFeature, the report length of the collection's feature reports.
    Feature,
    /// WriteFile, the length of its output reports.
    Output,
}

/// Parameters of a model, from its profile (section `[hid]`).
#[derive(Debug)]
pub struct HidModel {
    pub vid: u16,
    pub pids: &'static [u16],
    /// The HID collection the reports go to.
    pub usage_page: u16,
    pub usage: Option<u16>,
    pub report: Report,
    pub static_: &'static [Packet],
    /// Empty: static black.
    pub off: &'static [Packet],
    /// Firmware effects; empty: the app draws it.
    pub breathe: &'static [Packet],
    pub cycle: &'static [Packet],
    /// Pause between reports, for devices that drop reports sent too fast.
    pub delay_ms: u16,
}

/// Reads a template: hex bytes ("07", "0x07") and placeholders, separated by spaces.
pub fn parse_packet(text: &str) -> Result<Vec<Byte>, String> {
    let bytes: Vec<Byte> = text
        .split_whitespace()
        .map(|t| {
            Ok(match t {
                "{r}" => Byte::R,
                "{g}" => Byte::G,
                "{b}" => Byte::B,
                "{red}" => Byte::Red,
                "{green}" => Byte::Green,
                "{blue}" => Byte::Blue,
                "{brightness}" => Byte::Brightness,
                "{percent}" => Byte::Percent,
                "{speed}" => Byte::Speed,
                "{zone}" => Byte::Zone,
                _ if t.starts_with('{') => return Err(format!("unknown placeholder {t}")),
                _ => Byte::Lit(u8::from_str_radix(t.trim_start_matches("0x").trim_start_matches("0X"), 16).map_err(|_| format!("\"{t}\" is not a hex byte"))?),
            })
        })
        .collect::<Result<_, String>>()?;
    if bytes.is_empty() {
        return Err("empty report".into());
    }
    if !matches!(bytes[0], Byte::Lit(_)) {
        return Err("a report starts with its report id, a number".into());
    }
    Ok(bytes)
}

/// Effect speed byte of a speed factor (¼..4 over the range).
fn speed_byte(factor: f32) -> u8 {
    let s = (50.0 + 25.0 * factor.max(1e-3).log2()).clamp(0.0, 100.0);
    (s * 255.0 / 100.0) as u8
}

/// The reports of templates for zone colors (LED id, color): a template with `{zone}` once
/// per zone, the others once with the first zone's color. Each is `len` bytes long.
pub fn packets(templates: &[Packet], zones: &[(u8, Rgb)], brightness: u8, speed: f32, len: usize) -> Result<Vec<Vec<u8>>, String> {
    let first = zones.first().copied().unwrap_or((0, Rgb(255, 255, 255)));
    let mut out = Vec::new();
    for t in templates {
        let each: &[(u8, Rgb)] = if t.contains(&Byte::Zone) { zones } else { std::slice::from_ref(&first) };
        for &(zone, color) in each {
            if t.len() > len {
                return Err(format!("report of {} bytes, the device takes {len}", t.len()));
            }
            let scaled = color.scaled(brightness);
            let mut buf = vec![0u8; len];
            for (i, b) in t.iter().enumerate() {
                buf[i] = match *b {
                    Byte::Lit(v) => v,
                    Byte::R => scaled.0,
                    Byte::G => scaled.1,
                    Byte::B => scaled.2,
                    Byte::Red => color.0,
                    Byte::Green => color.1,
                    Byte::Blue => color.2,
                    Byte::Brightness => (255 * brightness.min(100) as u32 / 100) as u8,
                    Byte::Percent => brightness.min(100),
                    Byte::Speed => speed_byte(speed),
                    Byte::Zone => zone,
                };
            }
            out.push(buf);
        }
    }
    Ok(out)
}

/// The collection a model's reports go to, and their length.
fn matches(m: &HidModel, i: &HidInfo) -> Option<usize> {
    let len = match m.report {
        Report::Feature => i.feature_len,
        Report::Output => i.output_len,
    } as usize;
    (i.vid == m.vid && m.pids.contains(&i.pid) && i.usage_page == m.usage_page && m.usage.is_none_or(|u| u == i.usage) && len > 0).then_some(len)
}

pub struct RawHid {
    /// "<kind>-<hash of the HID path>": the id comes from the USB port.
    pub id: String,
    path: Vec<u16>,
    hid: Hid,
    model: &'static HidModel,
    len: usize,
    /// LED ids of the model's zones; empty: one color.
    zones: Vec<u8>,
}

/// The connected devices of a `[hid]` model.
pub fn find(kind: &'static Kind) -> Vec<RawHid> {
    let super::Protocol::Hid(model) = kind.protocol else { return Vec::new() };
    hid::enumerate(model.vid)
        .into_iter()
        .filter_map(|i| {
            let len = matches(model, &i)?;
            let overlapped = model.report == Report::Output;
            // Mouse and keyboard collections can't be opened for read/write, but feature reports work.
            let hid = Hid::open(&i.path, true, overlapped).or_else(|| if overlapped { None } else { Hid::open(&i.path, false, false) })?;
            Some(RawHid {
                id: format!("{}-{}", kind.key, super::path_hash(&i.path)),
                path: i.path.clone(),
                hid,
                model,
                len,
                zones: kind.zones.iter().map(|z| z.0).collect(),
            })
        })
        .collect()
}

/// Sends reports in order, with the model's pause between them.
fn send(hid: &Hid, model: &HidModel, reports: &[Vec<u8>]) -> bool {
    reports.iter().enumerate().all(|(n, r)| {
        if n > 0 && model.delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(model.delay_ms as u64));
        }
        match model.report {
            Report::Feature => hid.set_feature(r),
            Report::Output => hid.write(r),
        }
    })
}

impl RawHid {
    fn send(&self, templates: &[Packet], zones: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        let reports = packets(templates, zones, brightness, super::speed(), self.len)?;
        if send(&self.hid, self.model, &reports) {
            Ok(())
        } else {
            Err("HID profile: send failed".into())
        }
    }

    /// Plays breathing or spectrum with the static reports, from its own handle to the device.
    fn start_animation(&self, effect: Effect, zones: Vec<(u8, Rgb)>, brightness: u8) -> Result<(), String> {
        let overlapped = self.model.report == Report::Output;
        let hid = Hid::open(&self.path, true, overlapped).or_else(|| if overlapped { None } else { Hid::open(&self.path, false, false) });
        let hid = hid.ok_or("HID profile: cannot open")?;
        let (model, len) = (self.model, self.len);
        animation::start(
            &self.path,
            Box::new(move |t| {
                let (zones, level) = match effect {
                    Effect::Breathe => (zones.clone(), (brightness as f32 * animation::breathe(t)) as u8),
                    _ => (zones.iter().map(|z| (z.0, crate::hue(animation::cycle_hue(t)))).collect(), brightness),
                };
                packets(model.static_, &zones, level, 1.0, len).is_ok_and(|r| send(&hid, model, &r))
            }),
        );
        Ok(())
    }
}

impl Device for RawHid {
    fn apply(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String> {
        let zones: Vec<(u8, Rgb)> = if self.zones.is_empty() { vec![(0, color)] } else { self.zones.iter().map(|&z| (z, color)).collect() };
        self.apply_zones(effect, &zones, brightness)
    }

    fn apply_zones(&self, effect: Effect, zones: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        animation::stop(&self.path);
        let m = self.model;
        match effect {
            Effect::Off if m.off.is_empty() => self.send(m.static_, &zones.iter().map(|z| (z.0, Rgb(0, 0, 0))).collect::<Vec<_>>(), brightness),
            Effect::Off => self.send(m.off, zones, brightness),
            Effect::Breathe if !m.breathe.is_empty() => self.send(m.breathe, zones, brightness),
            Effect::Cycle if !m.cycle.is_empty() => self.send(m.cycle, zones, brightness),
            Effect::Breathe | Effect::Cycle => {
                self.send(m.static_, zones, brightness)?;
                self.start_animation(effect, zones.to_vec(), brightness)
            }
            _ => self.send(m.static_, zones, brightness),
        }
    }
}

/// How the app finds devices of `[hid]` profiles (see `devices::DRIVERS`).
pub const DRIVER: super::Driver = super::Driver {
    discover: |d| {
        for kind in super::kinds() {
            d.found.extend(find(kind).into_iter().map(|r| super::Found { id: r.id.clone(), dev: Box::new(r) }));
        }
    },
    note: |i| {
        let ours = super::kinds().iter().any(|k| matches!(k.protocol, super::Protocol::Hid(m) if matches(m, i).is_some()));
        ours.then_some("SUPPORTED: device with a [hid] profile")
    },
    ..super::Driver::NONE
};

/// Hex of bytes, without the zero padding at the end.
fn hex(bytes: &[u8]) -> String {
    let end = bytes.iter().rposition(|&b| b != 0).map_or(1, |p| p + 1);
    let mut s: Vec<String> = bytes[..end].iter().map(|b| format!("{b:02X}")).collect();
    if end < bytes.len() {
        s.push(format!("(+{} zeros)", bytes.len() - end));
    }
    s.join(" ")
}

/// `--hid-send <vid:pid[:usage_page[:usage]]> <feature|output|get> [report...]`: sends reports
/// (hex, as in a profile, without placeholders) to the first matching collection and reads the
/// answers. For working out a protocol before writing a profile. Returns what happened.
pub fn send_cli(args: &[String]) -> String {
    let usage = "usage: --hid-send <out.txt> <vid:pid[:usage_page[:usage]]> <feature|output|get> \"<hex bytes>\"...";
    let (Some(target), Some(mode)) = (args.first(), args.get(1)) else { return usage.into() };
    let nums: Vec<Option<u16>> = target.split(':').map(|n| u16::from_str_radix(n.trim_start_matches("0x"), 16).ok()).collect();
    let (Some(Some(vid)), Some(Some(pid))) = (nums.first(), nums.get(1)) else { return format!("bad device \"{target}\"\n{usage}") };
    let page = nums.get(2).copied().flatten();
    let use_ = nums.get(3).copied().flatten();
    let feature = mode != "output";
    let found = hid::enumerate(*vid).into_iter().find(|i| {
        i.pid == *pid
            && page.is_none_or(|p| p == i.usage_page)
            && use_.is_none_or(|u| u == i.usage)
            && if feature { i.feature_len > 0 } else { i.output_len > 0 }
    });
    let Some(info) = found else { return format!("no collection of {target} with {mode} reports") };
    let len = if feature { info.feature_len } else { info.output_len } as usize;
    let mut out = format!(
        "{:04X}:{:04X} usage {:04X}:{:04X}, feature {} / output {} / input {} bytes\n",
        info.vid, info.pid, info.usage_page, info.usage, info.feature_len, info.output_len, info.input_len
    );
    let Some(hid) = Hid::open(&info.path, true, !feature).or_else(|| if feature { Hid::open(&info.path, false, false) } else { None }) else {
        return out + "cannot open\n";
    };
    for text in &args[2..] {
        let bytes = match parse_packet(text) {
            Ok(b) => match b.iter().map(|b| if let Byte::Lit(v) = b { Some(*v) } else { None }).collect::<Option<Vec<u8>>>() {
                Some(bytes) => bytes,
                None => {
                    out += &format!("\"{text}\": placeholders only work in profiles\n");
                    continue;
                }
            },
            Err(e) => {
                out += &format!("\"{text}\": {e}\n");
                continue;
            }
        };
        if bytes.len() > len {
            out += &format!("\"{text}\": {} bytes, the report is {len}\n", bytes.len());
            continue;
        }
        let mut buf = vec![0u8; len];
        buf[..bytes.len()].copy_from_slice(&bytes);
        match mode.as_str() {
            "get" => {
                let ok = hid.get_feature(&mut buf);
                out += &format!("get {:02X}: {}\n", bytes[0], if ok { hex(&buf) } else { "failed".into() });
            }
            "output" => {
                let ok = hid.write(&buf);
                out += &format!("> {}: {}\n", hex(&buf), if ok { "sent" } else { "failed" });
                let mut answer = vec![0u8; (info.input_len as usize).max(1)];
                for _ in 0..3 {
                    match hid.read(&mut answer, 300) {
                        Ok(Some(n)) => out += &format!("< {}\n", hex(&answer[..n])),
                        _ => break,
                    }
                }
            }
            _ => {
                let ok = hid.set_feature(&buf);
                out += &format!("> {}: {}\n", hex(&buf), if ok { "sent" } else { "failed" });
                let mut answer = vec![0u8; len];
                answer[0] = buf[0];
                if hid.get_feature(&mut answer) {
                    out += &format!("< {}\n", hex(&answer));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leak(v: Vec<Byte>) -> Packet {
        Box::leak(v.into_boxed_slice())
    }

    #[test]
    fn templates_parse() {
        assert_eq!(parse_packet("07 0x0A {r} {g} {b}").unwrap(), vec![Byte::Lit(7), Byte::Lit(10), Byte::R, Byte::G, Byte::B]);
        assert_eq!(parse_packet("00 {zone} {red} {green} {blue} {brightness} {percent} {speed}").unwrap().len(), 8);
        assert!(parse_packet("07 {rgb}").unwrap_err().contains("{rgb}"));
        assert!(parse_packet("07 1FF").unwrap_err().contains("1FF"));
        assert!(parse_packet("  ").is_err());
        assert!(parse_packet("{r} 00").unwrap_err().contains("report id"));
    }

    #[test]
    fn colors_go_into_the_report() {
        let t = leak(parse_packet("07 {r} {g} {b} {red} {brightness} {percent}").unwrap());
        let r = packets(&[t], &[(0, Rgb(200, 100, 0))], 50, 1.0, 10).unwrap();
        assert_eq!(r, vec![vec![7, 100, 50, 0, 200, 127, 50, 0, 0, 0]]);
        // Too long for the device.
        assert!(packets(&[t], &[(0, Rgb(1, 2, 3))], 100, 1.0, 4).is_err());
    }

    #[test]
    fn zone_reports_repeat_per_zone() {
        let head = leak(parse_packet("01 AA").unwrap());
        let zone = leak(parse_packet("02 {zone} {r}").unwrap());
        let commit = leak(parse_packet("03").unwrap());
        let r = packets(&[head, zone, commit], &[(4, Rgb(10, 0, 0)), (5, Rgb(20, 0, 0))], 100, 1.0, 3).unwrap();
        assert_eq!(r, vec![vec![1, 0xAA, 0], vec![2, 4, 10], vec![2, 5, 20], vec![3, 0, 0]]);
    }

    #[test]
    fn speed_bytes() {
        assert_eq!(speed_byte(1.0), 127);
        assert_eq!(speed_byte(0.25), 0);
        assert_eq!(speed_byte(4.0), 255);
    }

    #[test]
    fn hex_drops_trailing_zeros() {
        assert_eq!(hex(&[7, 0, 1, 0, 0]), "07 00 01 (+2 zeros)");
        assert_eq!(hex(&[0, 0]), "00 (+1 zeros)");
    }
}
