pub mod animation;
pub mod asus;
pub mod lamparray;
pub mod logitech;
pub mod razer;
pub mod via;

use crate::i18n::Text;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Static,
    Breathe,
    Cycle,
    Off,
}

impl Effect {
    pub const ALL: [Effect; 4] = [Effect::Static, Effect::Breathe, Effect::Cycle, Effect::Off];

    pub fn label(self) -> &'static str {
        match self {
            Effect::Static => tr!("Статичный", "Static"),
            Effect::Breathe => tr!("Дыхание", "Breathing"),
            Effect::Cycle => tr!("Смена цветов", "Color cycle"),
            Effect::Off => tr!("Выключено", "Off"),
        }
    }
}

/// Speed of breathing and spectrum, 0..=100; 50 is the normal speed.
static SPEED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(NORMAL_SPEED);
pub const NORMAL_SPEED: u8 = 50;

/// Sets the speed of breathing and spectrum for the device applied next (the apply loop sets
/// each device's own before applying it).
pub fn set_speed(speed: u8) {
    SPEED.store(speed.min(100), std::sync::atomic::Ordering::Relaxed);
}

pub fn speed() -> u8 {
    SPEED.load(std::sync::atomic::Ordering::Relaxed)
}

/// How many times faster than normal: 1/4 at speed 0, 1 at 50, 4 at 100.
pub fn speed_factor(speed: u8) -> f32 {
    2f32.powf((speed.min(100) as f32 - NORMAL_SPEED as f32) / 25.0)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Scales the color by a 0..=100 brightness percentage.
    pub fn scaled(self, pct: u8) -> Rgb {
        let s = |c: u8| (c as u32 * pct.min(100) as u32 / 100) as u8;
        Rgb(s(self.0), s(self.1), s(self.2))
    }
}

pub trait Device {
    fn apply(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String>;

    /// Per-key colors (zone id, color); only keyboards with per-key lighting support it.
    fn apply_keys(&self, _keys: &[(u8, Rgb)], _brightness: u8) -> Result<(), String> {
        Err("per-key colors are not supported".into())
    }

    /// Changes some key colors while per-key colors are shown (the key editor).
    fn update_keys(&self, keys: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        self.apply_keys(keys, brightness)
    }

    /// The effect with a separate color per lighting zone (LED id, color).
    fn apply_zones(&self, effect: Effect, zones: &[(u8, Rgb)], brightness: u8) -> Result<(), String> {
        let color = zones.first().map(|z| z.1).unwrap_or(Rgb(255, 255, 255));
        self.apply(effect, color, brightness)
    }

    /// Battery state of wireless devices.
    fn battery(&self) -> Option<Battery> {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

impl Battery {
    pub fn label(&self) -> String {
        if self.charging {
            format!("{} {}%", tr!("заряжается", "charging"), self.percent)
        } else {
            format!("{} {}%", tr!("батарея", "battery"), self.percent)
        }
    }

    pub fn is_low(&self) -> bool {
        !self.charging && self.percent <= LOW_BATTERY
    }
}

/// Battery level (percent) at which the user is warned; again at CRITICAL_BATTERY.
pub const LOW_BATTERY: u8 = 10;
pub const CRITICAL_BATTERY: u8 = 5;

/// Battery levels of all connected wireless devices, by device id. Opens only devices that
/// have a battery, so it is cheap enough to run periodically.
pub fn batteries() -> Vec<(String, Battery)> {
    let mut out = Vec::new();
    for kb in logitech::Logitech::find_all(logitech::SW_ID_POLL) {
        if let Some(b) = kb.battery() {
            out.push((kb.id.clone(), b));
        }
    }
    for d in razer::Razer::find_all() {
        if let (Some(id), Some(b)) = (d.id.clone(), d.battery()) {
            out.push((id, b));
        }
    }
    out
}

/// Ids of wireless devices that answer right now (cheap: no lighting is sent).
pub fn answering() -> Vec<String> {
    let mut ids: Vec<String> = logitech::Logitech::find_all(logitech::SW_ID_POLL).into_iter().map(|k| k.id).collect();
    ids.extend(razer::Razer::find_all().into_iter().filter_map(|d| d.id));
    ids
}

/// A supported device model.
pub struct Kind {
    /// Prefix of device ids and the config key of old (single-device) versions.
    pub key: &'static str,
    /// Default name (Russian, English); see `name()`.
    pub names: Text,
    /// Segoe Fluent Icons glyph.
    pub icon: &'static str,
    /// Independently lit zones: (LED id, name); empty = one color only.
    pub zones: &'static [(u8, Text)],
    /// Keyboard with per-key colors (the key editor).
    pub per_key: bool,
    /// The firmware has no serial number: the id comes from the USB port, so a device moved
    /// to another port takes over the settings of an absent one of the same kind.
    pub port_id: bool,
}

impl Kind {
    /// Default name of the model in the interface language.
    pub fn name(&self) -> &'static str {
        crate::i18n::pick(self.names)
    }
}

/// Every supported model, in panel order.
pub const KINDS: [Kind; 7] = [
    Kind { key: "logitech", names: ("Logitech G915 TKL", "Logitech G915 TKL"), icon: "\u{E765}", zones: &[], per_key: true, port_id: false },
    Kind { key: "naga", names: ("Razer Naga V2 Pro", "Razer Naga V2 Pro"), icon: "\u{E962}", zones: &NAGA_ZONES, per_key: false, port_id: false },
    Kind { key: "dock", names: ("Razer Mouse Dock Pro", "Razer Mouse Dock Pro"), icon: "\u{E772}", zones: &[], per_key: false, port_id: false },
    Kind { key: "asus", names: ("Клавиатура ASUS", "ASUS keyboard"), icon: "\u{E7F8}", zones: &[], per_key: false, port_id: false },
    Kind { key: "macropad", names: ("Geekboard MacroPad", "Geekboard MacroPad"), icon: "\u{E92E}", zones: &[], per_key: false, port_id: true },
    // Any QMK keyboard with VIA support and RGB lighting.
    Kind { key: "via", names: ("Клавиатура QMK", "QMK keyboard"), icon: "\u{E765}", zones: &[], per_key: false, port_id: true },
    // Any device with the standard HID LampArray interface and no driver of its own.
    Kind { key: "lamparray", names: ("Устройство LampArray", "LampArray device"), icon: "\u{E82F}", zones: &[], per_key: false, port_id: true },
];

/// The device has per-key colors (the key editor): a model with them (G915), or a LampArray
/// keyboard whose lamps are bound to keys.
pub fn has_per_key(id: &str) -> bool {
    kind_of(id).per_key || lamparray::has_keys(id)
}

/// Opens a keyboard for the key editor, by id.
pub fn open_for_keys(id: &str) -> Option<Box<dyn Device>> {
    match kind_of(id).key {
        "logitech" => logitech::Logitech::find_id(logitech::SW_ID_EDITOR, id).map(|k| Box::new(k) as Box<dyn Device>),
        "lamparray" => lamparray::find_all().into_iter().find(|d| d.id == id).map(|d| Box::new(d) as Box<dyn Device>),
        _ => None,
    }
}

/// Names devices report about themselves, by id: the default name of devices of a generic
/// driver (LampArray, QMK/VIA), whose model isn't known in advance.
static NAMES: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

pub fn set_name(id: &str, name: &str) {
    let mut names = NAMES.lock().unwrap();
    names.retain(|(i, _)| i != id);
    if !name.is_empty() {
        names.push((id.to_string(), name.to_string()));
    }
}

pub fn name_of(id: &str) -> Option<String> {
    NAMES.lock().unwrap().iter().find(|(i, _)| i == id).map(|(_, n)| n.clone())
}

/// "ASUSTek Computer Inc." + "N-KEY Device" → "ASUSTek N-KEY Device": the product name with the
/// maker's first word, unless the product name already starts with it.
pub fn device_name(info: &crate::hid::HidInfo) -> String {
    let product = info.product();
    let maker = info.manufacturer();
    let maker = maker.split_whitespace().next().unwrap_or("").trim_end_matches(',');
    if maker.is_empty() || product.to_lowercase().starts_with(&maker.to_lowercase()) {
        product
    } else if product.is_empty() {
        maker.to_string()
    } else {
        format!("{maker} {product}")
    }
}

/// Independently lit zones of a device: (zone / LED id, name in the interface language).
/// Fixed for a model (Naga), or read from the device (LampArray, incl. the ASUS keyboard).
pub fn zones_of(id: &str) -> Vec<(u8, String)> {
    let kind = kind_of(id);
    if !kind.zones.is_empty() {
        return kind.zones.iter().map(|&(led, name)| (led, crate::i18n::pick(name).to_string())).collect();
    }
    lamparray::zones(id).into_iter().map(|z| (z.led, if crate::i18n::en() { z.names.1 } else { z.names.0 })).collect()
}

/// Independently lit zones of the Naga V2 Pro: (LED id, name).
pub const NAGA_ZONES: [(u8, Text); 2] = [(0x04, ("Логотип", "Logo")), (0x05, ("Боковая панель", "Side panel"))];

/// What a HID interface is to this app, for the device report: a supported device, or a
/// known lighting protocol that a new driver could build on.
pub fn interface_note(i: &crate::hid::HidInfo) -> Option<&'static str> {
    Some(match (i.vid, i.usage_page, i.usage) {
        (0x046D, 0xFF00, 0x0002) if logitech::RECEIVER_PIDS.contains(&i.pid) => "SUPPORTED: Logitech receiver (HID++)",
        (0x046D, 0xFF00, 0x0002) => "Logitech HID++ (protocol known)",
        // Razer lighting goes through 91-byte feature reports on the first mouse collection.
        (0x1532, 0x0001, 0x0002) if i.feature_len == 91 && razer::is_supported(i.pid) => "SUPPORTED: Razer",
        (0x1532, _, _) if i.feature_len == 91 => "Razer control interface (protocol known)",
        (0x0B05, 0xFF31, 0x0079) => "SUPPORTED: ASUS Aura",
        (via::MACROPAD_VID, 0xFF60, 0x0061) if i.pid == via::MACROPAD_PID => "SUPPORTED: Geekboard MacroPad (VIA)",
        (_, 0xFF60, 0x0061) => "SUPPORTED: QMK/VIA keyboard (if it has RGB lighting)",
        (_, 0x0059, 0x0001) => "HID LampArray (Windows Dynamic Lighting standard)",
        (_, page, _) if page >= 0xFF00 => "vendor-specific",
        _ => return None,
    })
}

/// Plain name of a standard HID usage, to make the report readable.
pub fn usage_name(page: u16, usage: u16) -> &'static str {
    match (page, usage) {
        (0x0001, 0x0002) => "mouse",
        (0x0001, 0x0006) => "keyboard",
        (0x0001, 0x0080) => "system control",
        (0x0001, 0x000C) => "wireless radio",
        (0x000C, 0x0001) => "media keys",
        (0x000D, _) => "touch",
        (0x0059, _) => "lighting",
        (p, _) if p >= 0xFF00 => "vendor",
        _ => "",
    }
}

/// Vendors known for RGB peripherals or lighting controllers (USB vendor id, name).
pub const RGB_VENDORS: &[(u16, &str)] = &[
    (0x03F0, "HP / HyperX"),
    (0x0483, "STMicroelectronics (QMK boards)"),
    (0x048D, "ITE (laptop / motherboard RGB)"),
    (0x046D, "Logitech"),
    (0x04D9, "Holtek (keyboards)"),
    (0x0951, "Kingston / HyperX"),
    (0x0B05, "ASUS"),
    (0x0C45, "Microdia (keyboards)"),
    (0x1038, "SteelSeries"),
    (0x1044, "Gigabyte"),
    (0x1462, "MSI"),
    (0x1532, "Razer"),
    (0x187C, "Alienware"),
    (0x1B1C, "Corsair"),
    (0x1E71, "NZXT"),
    (0x1E7D, "ROCCAT / Turtle Beach"),
    (0x2516, "Cooler Master"),
    (0x258A, "SinoWealth (keyboards, mice)"),
    (0x26CE, "ASRock"),
    (0x31E3, "Wooting"),
    (0x3434, "Keychron"),
];

/// The model of a device id ("naga-123456…" → Naga); unknown ids get the first model.
pub fn kind_of(id: &str) -> &'static Kind {
    let key = id.split('-').next().unwrap_or(id);
    KINDS.iter().find(|k| k.key == key).unwrap_or(&KINDS[0])
}

/// Short stable hash of a HID path, for devices without a serial number.
pub fn path_hash(path: &[u16]) -> String {
    let mut h: u32 = 0x811C9DC5;
    for &c in path.iter().take_while(|&&c| c != 0) {
        let c = char::from_u32(c as u32).unwrap_or('?').to_ascii_lowercase();
        h = (h ^ c as u32).wrapping_mul(0x0100_0193);
    }
    format!("{h:08X}")
}

/// Error of a device that is plugged in (its receiver or dock is) but doesn't answer:
/// switched off, asleep or out of range.
pub const NO_LINK: &str = "no link to the device";

/// A connected device ready to be driven.
pub struct Found {
    pub id: String,
    pub dev: Box<dyn Device>,
}

/// Result of a device search: devices that answer, and devices whose receiver or dock is
/// plugged in but which don't answer — by id, or only by model when the id is unknown.
pub struct Discovery {
    pub found: Vec<Found>,
    pub no_link: Vec<String>,
    pub no_link_kinds: Vec<&'static str>,
}

/// Finds every supported device.
pub fn discover() -> Discovery {
    let mut found: Vec<Found> = Vec::new();
    let mut no_link_kinds = Vec::new();
    for kb in logitech::Logitech::find_all(logitech::SW_ID_APPLY) {
        found.push(Found { id: kb.id.clone(), dev: Box::new(kb) });
    }
    let ids: Vec<String> = found.iter().map(|f| f.id.clone()).collect();
    let no_link = logitech::Logitech::no_link(&ids);
    // Asleep already when the app started: which keyboard isn't known yet.
    if no_link.is_empty() && logitech::Logitech::receiver_silent() {
        no_link_kinds.push("logitech");
    }
    for d in razer::Razer::find_all() {
        match d.id.clone() {
            Some(id) if !found.iter().any(|f| f.id == id) => found.push(Found { id, dev: Box::new(d) }),
            Some(_) => {}
            None => no_link_kinds.push(d.key()),
        }
    }
    if let Some(d) = asus::Asus::find() {
        found.push(Found { id: "asus".into(), dev: Box::new(d) });
    }
    for d in via::find_all() {
        found.push(Found { id: d.id.clone(), dev: Box::new(d) });
    }
    for d in lamparray::find_all() {
        found.push(Found { id: d.id.clone(), dev: Box::new(d) });
    }
    // One device reachable two ways (e.g. a keyboard on cable and receiver) is driven once.
    let mut seen = Vec::new();
    found.retain(|f| {
        let first = !seen.contains(&f.id);
        seen.push(f.id.clone());
        first
    });
    Discovery { found, no_link, no_link_kinds }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_color() {
        assert_eq!(Rgb(255, 100, 0).scaled(50), Rgb(127, 50, 0));
        assert_eq!(Rgb(255, 100, 0).scaled(100), Rgb(255, 100, 0));
        assert_eq!(Rgb(255, 100, 0).scaled(250), Rgb(255, 100, 0));
        assert_eq!(Rgb(255, 100, 0).scaled(0), Rgb(0, 0, 0));
    }

    #[test]
    fn speed_factors() {
        assert_eq!(speed_factor(NORMAL_SPEED), 1.0);
        assert_eq!(speed_factor(0), 0.25);
        assert_eq!(speed_factor(100), 4.0);
        assert_eq!(speed_factor(250), 4.0);
        assert!((0..100).all(|s| speed_factor(s) < speed_factor(s + 1)));
    }

    #[test]
    fn kind_from_id() {
        assert_eq!(kind_of("naga-123456H78900012").key, "naga");
        assert_eq!(kind_of("asus").key, "asus");
        assert_eq!(kind_of("lamparray-1A2B3C4D").key, "lamparray");
        // Unknown ids fall back to the first model.
        assert_eq!(kind_of("toaster-1").key, KINDS[0].key);
    }

    #[test]
    fn kind_keys_are_unique_and_have_no_dash() {
        for (i, k) in KINDS.iter().enumerate() {
            assert!(!k.key.contains('-'), "{}", k.key);
            assert!(KINDS[i + 1..].iter().all(|o| o.key != k.key), "{}", k.key);
        }
    }

    #[test]
    fn path_hash_ignores_case_and_what_follows_nul() {
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(path_hash(&w("\\\\?\\HID#VID_0483")), path_hash(&w("\\\\?\\hid#vid_0483\0junk")));
        assert_ne!(path_hash(&w("a")), path_hash(&w("b")));
        assert_eq!(path_hash(&w("a")).len(), 8);
    }

    #[test]
    fn low_battery() {
        assert!(Battery { percent: LOW_BATTERY, charging: false }.is_low());
        assert!(!Battery { percent: LOW_BATTERY, charging: true }.is_low());
        assert!(!Battery { percent: LOW_BATTERY + 1, charging: false }.is_low());
    }
}
