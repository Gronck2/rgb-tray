//! Keyboard layouts for the key editor. What each key is (its code, legends) is in `NAMES`,
//! by the HID usage it sends; where keys sit is data, in `layouts/*.toml` (compiled in).
//! Positions are in key units (1u = one standard key).
//!
//! Key codes are the per-key zone ids of the Logitech G915, which are HID usages shifted
//! (`k`, `m`); they key the saved key colors, so they must not change.

use serde::Deserialize;

/// Per-key zone id of a regular key from its HID keyboard usage.
const fn k(usage: u8) -> u8 {
    usage - 0x03
}

/// Per-key zone id of a modifier (HID usages 0xE0..=0xE7).
const fn m(usage: u8) -> u8 {
    usage - 0x78
}

/// The HID keyboard usage (page 7) of an editor key code, for keyboards addressed by usage
/// (LampArray). None for the logo, brightness and media keys, which have no such usage.
pub fn usage_of(code: u8) -> Option<u32> {
    match code {
        LOGO | BRIGHTNESS | MEDIA_PLAY | MEDIA_MUTE | MEDIA_NEXT | MEDIA_PREV | 0 => None,
        c @ 0x68..=0x6F => Some(c as u32 + 0x78),
        c => Some(c as u32 + 0x03),
    }
}

pub const LOGO: u8 = 0xD2;
pub const BRIGHTNESS: u8 = 0x99;
// Checked on a G915 TKL by painting each code in its own color.
pub const MEDIA_PLAY: u8 = 0x9B;
pub const MEDIA_MUTE: u8 = 0x9C;
pub const MEDIA_NEXT: u8 = 0x9D;
pub const MEDIA_PREV: u8 = 0x9E;

/// Keys that aren't letters or symbols: no character maps to them.
fn is_special(code: u8) -> bool {
    matches!(code, LOGO | BRIGHTNESS | MEDIA_PLAY | MEDIA_MUTE | MEDIA_NEXT | MEDIA_PREV)
}

/// A key of a layout: what it is and where it is.
#[derive(Clone, Copy, Debug)]
pub struct KeyDef {
    /// Zone id sent to the keyboard.
    pub code: u8,
    pub label: &'static str,
    /// Cyrillic legend (ЙЦУКЕН), empty if none.
    pub ru: &'static str,
    /// True when `label` is a Segoe Fluent Icons glyph.
    pub icon: bool,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Every key a layout may name: (name in the layout files, code, label, Cyrillic legend, icon label).
const NAMES: &[(&str, u8, &str, &str, bool)] = &[
    ("Logo", LOGO, "G", "", false),
    ("Brightness", BRIGHTNESS, "\u{E706}", "", true),
    ("MediaPrev", MEDIA_PREV, "\u{E892}", "", true),
    ("MediaPlay", MEDIA_PLAY, "\u{E768}", "", true),
    ("MediaNext", MEDIA_NEXT, "\u{E893}", "", true),
    ("Mute", MEDIA_MUTE, "\u{E74F}", "", true),
    ("Esc", k(0x29), "Esc", "", false),
    ("F1", k(0x3A), "F1", "", false),
    ("F2", k(0x3B), "F2", "", false),
    ("F3", k(0x3C), "F3", "", false),
    ("F4", k(0x3D), "F4", "", false),
    ("F5", k(0x3E), "F5", "", false),
    ("F6", k(0x3F), "F6", "", false),
    ("F7", k(0x40), "F7", "", false),
    ("F8", k(0x41), "F8", "", false),
    ("F9", k(0x42), "F9", "", false),
    ("F10", k(0x43), "F10", "", false),
    ("F11", k(0x44), "F11", "", false),
    ("F12", k(0x45), "F12", "", false),
    ("PrintScreen", k(0x46), "PrtSc", "", false),
    ("ScrollLock", k(0x47), "ScrLk", "", false),
    ("Pause", k(0x48), "Pause", "", false),
    ("Grave", k(0x35), "`", "Ё", false),
    ("1", k(0x1E), "1", "", false),
    ("2", k(0x1F), "2", "", false),
    ("3", k(0x20), "3", "", false),
    ("4", k(0x21), "4", "", false),
    ("5", k(0x22), "5", "", false),
    ("6", k(0x23), "6", "", false),
    ("7", k(0x24), "7", "", false),
    ("8", k(0x25), "8", "", false),
    ("9", k(0x26), "9", "", false),
    ("0", k(0x27), "0", "", false),
    ("Minus", k(0x2D), "-", "", false),
    ("Equal", k(0x2E), "=", "", false),
    ("Backspace", k(0x2A), "Backspace", "", false),
    ("Insert", k(0x49), "Ins", "", false),
    ("Home", k(0x4A), "Home", "", false),
    ("PageUp", k(0x4B), "PgUp", "", false),
    ("Tab", k(0x2B), "Tab", "", false),
    ("Q", k(0x14), "Q", "Й", false),
    ("W", k(0x1A), "W", "Ц", false),
    ("E", k(0x08), "E", "У", false),
    ("R", k(0x15), "R", "К", false),
    ("T", k(0x17), "T", "Е", false),
    ("Y", k(0x1C), "Y", "Н", false),
    ("U", k(0x18), "U", "Г", false),
    ("I", k(0x0C), "I", "Ш", false),
    ("O", k(0x12), "O", "Щ", false),
    ("P", k(0x13), "P", "З", false),
    ("LBracket", k(0x2F), "[", "Х", false),
    ("RBracket", k(0x30), "]", "Ъ", false),
    ("Backslash", k(0x31), "\\", "/", false),
    ("Delete", k(0x4C), "Del", "", false),
    ("End", k(0x4D), "End", "", false),
    ("PageDown", k(0x4E), "PgDn", "", false),
    ("CapsLock", k(0x39), "Caps", "", false),
    ("A", k(0x04), "A", "Ф", false),
    ("S", k(0x16), "S", "Ы", false),
    ("D", k(0x07), "D", "В", false),
    ("F", k(0x09), "F", "А", false),
    ("G", k(0x0A), "G", "П", false),
    ("H", k(0x0B), "H", "Р", false),
    ("J", k(0x0D), "J", "О", false),
    ("K", k(0x0E), "K", "Л", false),
    ("L", k(0x0F), "L", "Д", false),
    ("Semicolon", k(0x33), ";", "Ж", false),
    ("Quote", k(0x34), "'", "Э", false),
    ("NonUsHash", k(0x32), "\\", "/", false),
    ("Enter", k(0x28), "Enter", "", false),
    ("LShift", m(0xE1), "Shift", "", false),
    ("NonUsBackslash", k(0x64), "\\", "", false),
    ("Z", k(0x1D), "Z", "Я", false),
    ("X", k(0x1B), "X", "Ч", false),
    ("C", k(0x06), "C", "С", false),
    ("V", k(0x19), "V", "М", false),
    ("B", k(0x05), "B", "И", false),
    ("N", k(0x11), "N", "Т", false),
    ("M", k(0x10), "M", "Ь", false),
    ("Comma", k(0x36), ",", "Б", false),
    ("Period", k(0x37), ".", "Ю", false),
    ("Slash", k(0x38), "/", "", false),
    ("RShift", m(0xE5), "Shift", "", false),
    ("Up", k(0x52), "\u{2191}", "", false),
    ("LCtrl", m(0xE0), "Ctrl", "", false),
    ("LWin", m(0xE3), "Win", "", false),
    ("LAlt", m(0xE2), "Alt", "", false),
    ("Space", k(0x2C), "", "", false),
    ("RAlt", m(0xE6), "Alt", "", false),
    ("RWin", m(0xE7), "Win", "", false),
    ("Menu", k(0x65), "Menu", "", false),
    ("RCtrl", m(0xE4), "Ctrl", "", false),
    ("Left", k(0x50), "\u{2190}", "", false),
    ("Down", k(0x51), "\u{2193}", "", false),
    ("Right", k(0x4F), "\u{2192}", "", false),
    ("NumLock", k(0x53), "Num", "", false),
    ("KpSlash", k(0x54), "/", "", false),
    ("KpAsterisk", k(0x55), "*", "", false),
    ("KpMinus", k(0x56), "-", "", false),
    ("KpPlus", k(0x57), "+", "", false),
    ("KpEnter", k(0x58), "Enter", "", false),
    ("Kp1", k(0x59), "1", "", false),
    ("Kp2", k(0x5A), "2", "", false),
    ("Kp3", k(0x5B), "3", "", false),
    ("Kp4", k(0x5C), "4", "", false),
    ("Kp5", k(0x5D), "5", "", false),
    ("Kp6", k(0x5E), "6", "", false),
    ("Kp7", k(0x5F), "7", "", false),
    ("Kp8", k(0x60), "8", "", false),
    ("Kp9", k(0x61), "9", "", false),
    ("Kp0", k(0x62), "0", "", false),
    ("KpDecimal", k(0x63), ".", "", false),
];

/// Layouts compiled in: (file name, contents). The first is the fallback.
const BUILT_IN: &[(&str, &str)] = &[
    ("g915-tkl.toml", include_str!("../layouts/g915-tkl.toml")),
    ("tkl.toml", include_str!("../layouts/tkl.toml")),
    ("full.toml", include_str!("../layouts/full.toml")),
];

/// The G915 TKL's own layout.
pub const G915_TKL: &str = "g915-tkl";
pub const TKL: &str = "tkl";
pub const FULL: &str = "full";

/// A keyboard layout in both variants.
#[derive(Debug)]
pub struct Layout {
    pub name: String,
    pub width: f32,
    pub height: f32,
    ansi: Vec<KeyDef>,
    iso: Vec<KeyDef>,
    /// Every code of either variant, sorted.
    codes: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutFile {
    name: String,
    width: f32,
    height: f32,
    row: Vec<RowFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RowFile {
    y: f32,
    #[serde(default = "one")]
    w: f32,
    #[serde(default = "one")]
    h: f32,
    keys: String,
}

fn one() -> f32 {
    1.0
}

/// Keys of one row in one variant: `Name[:width[:height]][@x]`, `ansi:`/`iso:` prefixes.
fn parse_row(row: &RowFile, iso: bool) -> Result<Vec<KeyDef>, String> {
    let mut out = Vec::new();
    let mut x = 0.0;
    for token in row.keys.split_whitespace() {
        let token = match token.split_once(':') {
            Some(("ansi", _)) if iso => continue,
            Some(("iso", _)) if !iso => continue,
            Some(("ansi" | "iso", rest)) => rest,
            _ => token,
        };
        let (spec, at) = match token.split_once('@') {
            Some((s, a)) => (s, Some(a.parse::<f32>().map_err(|_| format!("bad position in \"{token}\""))?)),
            None => (token, None),
        };
        let mut parts = spec.split(':');
        let name = parts.next().unwrap_or("");
        let num = |p: Option<&str>, default: f32| p.map_or(Ok(default), |s| s.parse::<f32>().map_err(|_| format!("bad size in \"{token}\"")));
        let w = num(parts.next(), row.w)?;
        let h = num(parts.next(), row.h)?;
        let &(_, code, label, ru, icon) = NAMES.iter().find(|n| n.0 == name).ok_or_else(|| format!("unknown key \"{name}\""))?;
        if let Some(a) = at {
            x = a;
        }
        out.push(KeyDef { code, label, ru, icon, x, y: row.y, w, h });
        x += w;
    }
    Ok(out)
}

fn parse(text: &str) -> Result<Layout, String> {
    let f: LayoutFile = toml::from_str(text).map_err(|e| e.message().to_string())?;
    let variant = |iso: bool| -> Result<Vec<KeyDef>, String> {
        let mut keys = Vec::new();
        for row in &f.row {
            keys.extend(parse_row(row, iso)?);
        }
        Ok(keys)
    };
    let (ansi, iso) = (variant(false)?, variant(true)?);
    for keys in [&ansi, &iso] {
        if let Some(d) = keys.iter().enumerate().find(|(i, a)| keys[i + 1..].iter().any(|b| b.code == a.code)) {
            return Err(format!("key {} twice", d.1.label));
        }
    }
    let mut codes: Vec<u8> = ansi.iter().chain(&iso).map(|k| k.code).collect();
    codes.sort_unstable();
    codes.dedup();
    Ok(Layout { name: f.name, width: f.width, height: f.height, ansi, iso, codes })
}

fn all() -> &'static [Layout] {
    static ALL: std::sync::OnceLock<Vec<Layout>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| BUILT_IN.iter().map(|(file, text)| parse(text).unwrap_or_else(|e| panic!("layouts/{file}: {e}"))).collect())
}

/// A layout by name; unknown names get the first one.
pub fn get(name: &str) -> &'static Layout {
    let all = all();
    all.iter().find(|l| l.name == name).unwrap_or(&all[0])
}

impl Layout {
    pub fn keys(&self, iso: bool) -> &[KeyDef] {
        if iso {
            &self.iso
        } else {
            &self.ansi
        }
    }

    /// Every addressable code of either variant: what the keyboard is sent.
    pub fn codes(&self) -> &[u8] {
        &self.codes
    }

    /// Key that types the given character (Latin or Cyrillic ЙЦУКЕН, digits, space).
    pub fn code_for_char(&self, c: char) -> Option<u8> {
        let up: String = c.to_uppercase().collect();
        if c == ' ' {
            return self.ansi.iter().find(|kd| kd.code == k(0x2C)).map(|kd| kd.code);
        }
        // The logo's "G" is not the letter.
        self.ansi.iter().find(|kd| !is_special(kd.code) && kd.label.chars().count() == 1 && (kd.label == up || kd.ru == up)).map(|kd| kd.code)
    }

    pub fn code_by_label(&self, label: &str) -> Option<u8> {
        self.ansi.iter().find(|kd| !is_special(kd.code) && kd.label == label).map(|kd| kd.code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_load() {
        for (file, text) in BUILT_IN {
            parse(text).unwrap_or_else(|e| panic!("{file}: {e}"));
        }
        assert_eq!(get(G915_TKL).name, G915_TKL);
        assert_eq!(get("nonsense").name, G915_TKL);
    }

    #[test]
    fn g915_codes_did_not_change() {
        // G915 TKL, ANSI and ISO together (the log shows "per-key colors, 95 keys").
        let codes = get(G915_TKL).codes();
        assert_eq!(codes.len(), 95);
        assert!(codes.windows(2).all(|w| w[0] < w[1]) && !codes.contains(&0));
        // A few keys at their places from before the layouts were data.
        let at = |label: &str, iso: bool| get(G915_TKL).keys(iso).iter().find(|k| k.label == label && !is_special(k.code)).map(|k| (k.x, k.y, k.w, k.h));
        assert_eq!(at("Esc", false), Some((0.0, 0.85, 1.0, 1.0)));
        assert_eq!(at("F5", false), Some((6.5, 0.85, 1.0, 1.0)));
        assert_eq!(at("Enter", false), Some((12.75, 4.1, 2.25, 1.0)));
        assert_eq!(at("Enter", true), Some((13.75, 3.1, 1.25, 2.0)));
        assert_eq!(at("Z", true), Some((2.25, 5.1, 1.0, 1.0)));
        assert_eq!(at("Ctrl", false), Some((0.0, 6.1, 1.25, 1.0)));
        assert_eq!(at("\u{2192}", false), Some((17.25, 6.1, 1.0, 1.0)));
        let logo = get(G915_TKL).keys(false).iter().find(|k| k.code == LOGO).unwrap();
        assert_eq!((logo.x, logo.y, logo.w, logo.h), (0.0, 0.0, 0.9, 0.6));
    }

    #[test]
    fn full_size_is_tenkeyless_plus_keypad() {
        let (tkl, full) = (get(TKL).codes(), get(FULL).codes());
        assert!(tkl.iter().all(|c| full.contains(c)));
        assert_eq!(full.len() - tkl.len(), 17);
        // Tenkeyless is the G915 TKL without its top strip.
        assert!(get(G915_TKL).codes().iter().all(|&c| is_special(c) || tkl.contains(&c)));
        assert_eq!(usage_of(get(FULL).code_by_label("Num").unwrap()), Some(0x53));
        // Keys don't overlap within a row.
        for l in all() {
            for iso in [false, true] {
                let keys = l.keys(iso);
                for (i, a) in keys.iter().enumerate() {
                    for b in &keys[i + 1..] {
                        let overlap = a.x < b.x + b.w - 0.01 && b.x < a.x + a.w - 0.01 && a.y < b.y + b.h - 0.01 && b.y < a.y + a.h - 0.01;
                        assert!(!overlap, "{}: {} and {} overlap (iso {iso})", l.name, a.label, b.label);
                    }
                    assert!(a.x + a.w <= l.width + 0.01 && a.y + a.h <= l.height + 0.01, "{}: {} sticks out", l.name, a.label);
                }
            }
        }
    }

    #[test]
    fn bad_rows_say_why() {
        let row = |keys: &str| RowFile { y: 0.0, w: 1.0, h: 1.0, keys: keys.into() };
        assert!(parse_row(&row("Esc Toaster"), false).unwrap_err().contains("Toaster"));
        assert!(parse_row(&row("Esc:wide"), false).unwrap_err().contains("size"));
        assert!(parse_row(&row("Esc@far"), false).unwrap_err().contains("position"));
        let both = |keys: &str, iso: bool| parse_row(&row(keys), iso).unwrap().iter().map(|k| (k.label, k.x)).collect::<Vec<_>>();
        assert_eq!(both("Tab:1.5 ansi:Q iso:W E", false), [("Tab", 0.0), ("Q", 1.5), ("E", 2.5)]);
        assert_eq!(both("Tab:1.5 ansi:Q iso:W E", true), [("Tab", 0.0), ("W", 1.5), ("E", 2.5)]);
    }

    #[test]
    fn hid_usages() {
        assert_eq!(usage_of(k(0x04)), Some(0x04)); // A
        assert_eq!(usage_of(k(0x2C)), Some(0x2C)); // Space
        assert_eq!(usage_of(m(0xE0)), Some(0xE0)); // Left Ctrl
        assert_eq!(usage_of(m(0xE7)), Some(0xE7)); // Right Win
        assert_eq!(usage_of(k(0x62)), Some(0x62)); // Keypad 0
        assert_eq!(usage_of(LOGO), None);
        assert_eq!(usage_of(MEDIA_PLAY), None);
        // Every named key has a code of its own.
        for (i, a) in NAMES.iter().enumerate() {
            assert!(NAMES[i + 1..].iter().all(|b| b.1 != a.1 && b.0 != a.0), "{}", a.0);
        }
    }

    #[test]
    fn characters_to_keys() {
        let l = get(G915_TKL);
        assert_eq!(l.code_for_char('a'), Some(k(0x04)));
        assert_eq!(l.code_for_char('A'), Some(k(0x04)));
        assert_eq!(l.code_for_char('ф'), Some(k(0x04)));
        assert_eq!(l.code_for_char('1'), Some(k(0x1E)));
        assert_eq!(l.code_for_char(' '), Some(k(0x2C)));
        // "G" is also printed on the logo in the top strip; the letter key is meant.
        assert_eq!(l.code_for_char('g'), Some(k(0x0A)));
        assert_eq!(l.code_for_char('€'), None);
        // On a full-size keyboard digits are the number row, not the keypad.
        assert_eq!(get(FULL).code_for_char('7'), Some(k(0x24)));
    }
}
