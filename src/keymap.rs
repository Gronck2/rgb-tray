//! Physical layout of the Logitech G915 TKL and the per-key zone ids it uses.
//! Positions are in key units (1u = one standard key).

/// Per-key zone id of a regular key from its HID keyboard usage.
const fn k(usage: u8) -> u8 {
    usage - 0x03
}

/// Per-key zone id of a modifier (HID usages 0xE0..=0xE7).
const fn m(usage: u8) -> u8 {
    usage - 0x78
}

pub const LOGO: u8 = 0xD2;
pub const BRIGHTNESS: u8 = 0x99;
// Checked on a G915 TKL by painting each code in its own color.
pub const MEDIA_PLAY: u8 = 0x9B;
pub const MEDIA_MUTE: u8 = 0x9C;
pub const MEDIA_NEXT: u8 = 0x9D;
pub const MEDIA_PREV: u8 = 0x9E;

#[derive(Clone, Copy)]
pub struct KeyDef {
    /// Zone id sent to the keyboard; 0 = not addressable (drawn only).
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

const fn key(code: u8, label: &'static str, ru: &'static str, x: f32, y: f32, w: f32) -> KeyDef {
    KeyDef { code, label, ru, icon: false, x, y, w, h: 1.0 }
}

const fn small(code: u8, label: &'static str, icon: bool, x: f32) -> KeyDef {
    KeyDef { code, label, ru: "", icon, x, y: 0.0, w: 0.9, h: 0.6 }
}

pub const WIDTH: f32 = 18.25;
pub const HEIGHT: f32 = 7.1;

const R0: f32 = 0.85; // F-row
const R1: f32 = 2.1;
const R2: f32 = 3.1;
const R3: f32 = 4.1;
const R4: f32 = 5.1;
const R5: f32 = 6.1;

/// Keys shared by the ANSI and ISO layouts.
const COMMON: &[KeyDef] = &[
    // Top strip: logo, brightness button, media keys.
    small(LOGO, "G", false, 0.0),
    small(BRIGHTNESS, "\u{E706}", true, 3.0),
    small(MEDIA_PREV, "\u{E892}", true, 14.35),
    small(MEDIA_PLAY, "\u{E768}", true, 15.35),
    small(MEDIA_NEXT, "\u{E893}", true, 16.35),
    small(MEDIA_MUTE, "\u{E74F}", true, 17.35),
    // F-row.
    key(k(0x29), "Esc", "", 0.0, R0, 1.0),
    key(k(0x3A), "F1", "", 2.0, R0, 1.0),
    key(k(0x3B), "F2", "", 3.0, R0, 1.0),
    key(k(0x3C), "F3", "", 4.0, R0, 1.0),
    key(k(0x3D), "F4", "", 5.0, R0, 1.0),
    key(k(0x3E), "F5", "", 6.5, R0, 1.0),
    key(k(0x3F), "F6", "", 7.5, R0, 1.0),
    key(k(0x40), "F7", "", 8.5, R0, 1.0),
    key(k(0x41), "F8", "", 9.5, R0, 1.0),
    key(k(0x42), "F9", "", 11.0, R0, 1.0),
    key(k(0x43), "F10", "", 12.0, R0, 1.0),
    key(k(0x44), "F11", "", 13.0, R0, 1.0),
    key(k(0x45), "F12", "", 14.0, R0, 1.0),
    key(k(0x46), "PrtSc", "", 15.25, R0, 1.0),
    key(k(0x47), "ScrLk", "", 16.25, R0, 1.0),
    key(k(0x48), "Pause", "", 17.25, R0, 1.0),
    // Number row.
    key(k(0x35), "`", "Ё", 0.0, R1, 1.0),
    key(k(0x1E), "1", "", 1.0, R1, 1.0),
    key(k(0x1F), "2", "", 2.0, R1, 1.0),
    key(k(0x20), "3", "", 3.0, R1, 1.0),
    key(k(0x21), "4", "", 4.0, R1, 1.0),
    key(k(0x22), "5", "", 5.0, R1, 1.0),
    key(k(0x23), "6", "", 6.0, R1, 1.0),
    key(k(0x24), "7", "", 7.0, R1, 1.0),
    key(k(0x25), "8", "", 8.0, R1, 1.0),
    key(k(0x26), "9", "", 9.0, R1, 1.0),
    key(k(0x27), "0", "", 10.0, R1, 1.0),
    key(k(0x2D), "-", "", 11.0, R1, 1.0),
    key(k(0x2E), "=", "", 12.0, R1, 1.0),
    key(k(0x2A), "Backspace", "", 13.0, R1, 2.0),
    key(k(0x49), "Ins", "", 15.25, R1, 1.0),
    key(k(0x4A), "Home", "", 16.25, R1, 1.0),
    key(k(0x4B), "PgUp", "", 17.25, R1, 1.0),
    // Top letter row.
    key(k(0x2B), "Tab", "", 0.0, R2, 1.5),
    key(k(0x14), "Q", "Й", 1.5, R2, 1.0),
    key(k(0x1A), "W", "Ц", 2.5, R2, 1.0),
    key(k(0x08), "E", "У", 3.5, R2, 1.0),
    key(k(0x15), "R", "К", 4.5, R2, 1.0),
    key(k(0x17), "T", "Е", 5.5, R2, 1.0),
    key(k(0x1C), "Y", "Н", 6.5, R2, 1.0),
    key(k(0x18), "U", "Г", 7.5, R2, 1.0),
    key(k(0x0C), "I", "Ш", 8.5, R2, 1.0),
    key(k(0x12), "O", "Щ", 9.5, R2, 1.0),
    key(k(0x13), "P", "З", 10.5, R2, 1.0),
    key(k(0x2F), "[", "Х", 11.5, R2, 1.0),
    key(k(0x30), "]", "Ъ", 12.5, R2, 1.0),
    key(k(0x4C), "Del", "", 15.25, R2, 1.0),
    key(k(0x4D), "End", "", 16.25, R2, 1.0),
    key(k(0x4E), "PgDn", "", 17.25, R2, 1.0),
    // Home row.
    key(k(0x39), "Caps", "", 0.0, R3, 1.75),
    key(k(0x04), "A", "Ф", 1.75, R3, 1.0),
    key(k(0x16), "S", "Ы", 2.75, R3, 1.0),
    key(k(0x07), "D", "В", 3.75, R3, 1.0),
    key(k(0x09), "F", "А", 4.75, R3, 1.0),
    key(k(0x0A), "G", "П", 5.75, R3, 1.0),
    key(k(0x0B), "H", "Р", 6.75, R3, 1.0),
    key(k(0x0D), "J", "О", 7.75, R3, 1.0),
    key(k(0x0E), "K", "Л", 8.75, R3, 1.0),
    key(k(0x0F), "L", "Д", 9.75, R3, 1.0),
    key(k(0x33), ";", "Ж", 10.75, R3, 1.0),
    key(k(0x34), "'", "Э", 11.75, R3, 1.0),
    // Bottom letter row (shift keys differ between layouts).
    key(k(0x1D), "Z", "Я", 2.25, R4, 1.0),
    key(k(0x1B), "X", "Ч", 3.25, R4, 1.0),
    key(k(0x06), "C", "С", 4.25, R4, 1.0),
    key(k(0x19), "V", "М", 5.25, R4, 1.0),
    key(k(0x05), "B", "И", 6.25, R4, 1.0),
    key(k(0x11), "N", "Т", 7.25, R4, 1.0),
    key(k(0x10), "M", "Ь", 8.25, R4, 1.0),
    key(k(0x36), ",", "Б", 9.25, R4, 1.0),
    key(k(0x37), ".", "Ю", 10.25, R4, 1.0),
    key(k(0x38), "/", "", 11.25, R4, 1.0),
    key(m(0xE5), "Shift", "", 12.25, R4, 2.75),
    key(k(0x52), "\u{2191}", "", 16.25, R4, 1.0),
    // Space row.
    key(m(0xE0), "Ctrl", "", 0.0, R5, 1.25),
    key(m(0xE3), "Win", "", 1.25, R5, 1.25),
    key(m(0xE2), "Alt", "", 2.5, R5, 1.25),
    key(k(0x2C), "", "", 3.75, R5, 6.25),
    key(m(0xE6), "Alt", "", 10.0, R5, 1.25),
    key(m(0xE7), "Win", "", 11.25, R5, 1.25),
    key(k(0x65), "Menu", "", 12.5, R5, 1.25),
    key(m(0xE4), "Ctrl", "", 13.75, R5, 1.25),
    key(k(0x50), "\u{2190}", "", 15.25, R5, 1.0),
    key(k(0x51), "\u{2193}", "", 16.25, R5, 1.0),
    key(k(0x4F), "\u{2192}", "", 17.25, R5, 1.0),
];

const ANSI: &[KeyDef] = &[
    key(k(0x31), "\\", "/", 13.5, R2, 1.5),
    key(k(0x28), "Enter", "", 12.75, R3, 2.25),
    key(m(0xE1), "Shift", "", 0.0, R4, 2.25),

];

const ISO: &[KeyDef] = &[
    KeyDef { code: k(0x28), label: "Enter", ru: "", icon: false, x: 13.75, y: R2, w: 1.25, h: 2.0 },
    key(k(0x32), "\\", "/", 12.75, R3, 1.0),
    key(m(0xE1), "Shift", "", 0.0, R4, 1.25),
    key(k(0x64), "\\", "", 1.25, R4, 1.0),
];

pub fn layout(iso: bool) -> Vec<KeyDef> {
    let mut keys = COMMON.to_vec();
    keys.extend_from_slice(if iso { ISO } else { ANSI });
    keys
}

/// Every addressable zone id of either layout.
pub fn all_codes() -> Vec<u8> {
    let mut codes: Vec<u8> = COMMON.iter().chain(ANSI).chain(ISO).map(|k| k.code).filter(|&c| c != 0).collect();
    codes.sort_unstable();
    codes.dedup();
    codes
}

/// Key that types the given character (Latin or Cyrillic ЙЦУКЕН, digits, space).
pub fn code_for_char(c: char) -> Option<u8> {
    let up: String = c.to_uppercase().collect();
    if c == ' ' {
        return Some(k(0x2C));
    }
    COMMON
        .iter()
        .chain(ANSI)
        // y > 0 skips the top strip (its "G" is the logo, not the letter).
        .find(|kd| kd.code != 0 && kd.y > 0.0 && kd.label.chars().count() == 1 && (kd.label == up || kd.ru == up))
        .map(|kd| kd.code)
}

pub fn code_by_label(label: &str) -> Option<u8> {
    COMMON.iter().find(|kd| kd.y > 0.0 && kd.label == label).map(|kd| kd.code)
}
