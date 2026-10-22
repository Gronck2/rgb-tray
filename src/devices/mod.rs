pub mod animation;
pub mod asus;
pub mod lamparray;
pub mod logitech;
pub mod profile;
pub mod razer;
pub mod tuya;
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
    /// A rainbow running across the keyboard (firmware).
    Wave,
    /// Rings spreading from pressed keys (Logitech firmware).
    Ripple,
    /// Pressed keys light up and fade (Razer firmware).
    Reactive,
    /// Keys twinkle at random (Razer firmware).
    Starlight,
}

impl Effect {
    /// The effects every device has, on the panel's buttons.
    pub const ALL: [Effect; 4] = [Effect::Static, Effect::Breathe, Effect::Cycle, Effect::Off];
    /// Effects only some firmware has, under "More" on the panel.
    pub const EXTRA: [Effect; 4] = [Effect::Wave, Effect::Ripple, Effect::Reactive, Effect::Starlight];

    pub fn label(self) -> &'static str {
        match self {
            Effect::Static => tr!("Статичный", "Static"),
            Effect::Breathe => tr!("Дыхание", "Breathing"),
            Effect::Cycle => tr!("Смена цветов", "Color cycle"),
            Effect::Off => tr!("Выключено", "Off"),
            Effect::Wave => tr!("Волна", "Wave"),
            Effect::Ripple => tr!("Рябь от нажатий", "Ripple"),
            Effect::Reactive => tr!("Отклик на нажатия", "Reactive"),
            Effect::Starlight => tr!("Звёзды", "Starlight"),
        }
    }

    /// The name on the panel's narrow "More" button.
    pub fn short_label(self) -> &'static str {
        match self {
            Effect::Ripple => tr!("Рябь", "Ripple"),
            Effect::Reactive => tr!("Отклик", "Reactive"),
            e => e.label(),
        }
    }

    /// Moves over time: the speed setting matters.
    pub fn animated(self) -> bool {
        !matches!(self, Effect::Static | Effect::Off)
    }

    /// Shows the chosen color (the others are rainbow or dark).
    pub fn colored(self) -> bool {
        matches!(self, Effect::Static | Effect::Breathe | Effect::Ripple | Effect::Reactive | Effect::Starlight)
    }
}

/// Speed of breathing and spectrum of the device being applied: how many times faster than
/// normal (f32 bits). Settings use 0..=100 (`speed_factor`); this is their product.
static SPEED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000); // 1.0
pub const NORMAL_SPEED: u8 = 50;

/// Sets the speed factor of breathing and spectrum for the device applied next (the apply
/// loop sets each device's own before applying it).
pub fn set_speed(factor: f32) {
    SPEED.store(factor.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// How many times faster than normal the device being applied animates (1 = normal).
pub fn speed() -> f32 {
    f32::from_bits(SPEED.load(std::sync::atomic::Ordering::Relaxed))
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

/// A driver: how its devices are found, polled and described. Each is listed once in `DRIVERS`.
pub struct Driver {
    /// Adds its connected devices to the search result (and the ones whose receiver or dock
    /// is plugged in but which don't answer).
    pub discover: fn(&mut Discovery),
    /// Battery levels of its wireless devices, by id; cheap enough to poll.
    pub batteries: fn() -> Vec<(String, Battery)>,
    /// Ids of its wireless devices that answer right now; no lighting is sent.
    pub answering: fn() -> Vec<String>,
    /// One of its keyboards opened for the key editor; None when the id isn't its.
    pub open_for_keys: fn(&str) -> Option<Box<dyn Device>>,
    /// What a HID interface is to this driver, for the device report.
    pub note: fn(&crate::hid::HidInfo) -> Option<&'static str>,
}

impl Driver {
    /// A driver that does nothing: the base of the `DRIVER` of each module (`..Driver::NONE`).
    pub const NONE: Driver = Driver { discover: |_| {}, batteries: Vec::new, answering: Vec::new, open_for_keys: |_| None, note: |_| None };
}

/// Every driver. A native driver comes before the generic LampArray and QMK/VIA ones:
/// `discover()` keeps the first device with a given id.
pub const DRIVERS: &[Driver] = &[logitech::DRIVER, razer::DRIVER, asus::DRIVER, via::DRIVER, lamparray::DRIVER, tuya::DRIVER];

/// Battery levels of all connected wireless devices, by device id. Opens only devices that
/// have a battery, so it is cheap enough to run periodically.
pub fn batteries() -> Vec<(String, Battery)> {
    DRIVERS.iter().flat_map(|d| (d.batteries)()).collect()
}

/// Ids of wireless devices that answer right now (cheap: no lighting is sent).
pub fn answering() -> Vec<String> {
    DRIVERS.iter().flat_map(|d| (d.answering)()).collect()
}

/// A supported device model.
#[derive(Clone, Debug)]
pub struct Kind {
    /// Prefix of device ids and the config key of old (single-device) versions.
    pub key: &'static str,
    /// Default name (Russian, English); see `name()`.
    pub names: Text,
    /// Segoe Fluent Icons glyph.
    pub icon: &'static str,
    /// Independently lit zones: (LED id, name); empty = one color only.
    pub zones: &'static [(u8, Text)],
    /// Keyboard with per-key colors (the key editor): its layout (`keymap::get`).
    pub layout: Option<&'static str>,
    /// The firmware has no serial number: the id comes from the USB port, so a device moved
    /// to another port takes over the settings of an absent one of the same kind.
    pub port_id: bool,
    /// Effects the device can show.
    pub effects: &'static [Effect],
    /// How the speed of breathing and spectrum can be set.
    pub speed: Speed,
    pub class: Class,
    /// How the device is driven: its own Rust driver, or a known protocol with the profile's parameters.
    pub protocol: Protocol,
    /// Checked on real hardware; an untested model says so on its card, asking for a report.
    pub tested: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum Protocol {
    /// A driver of its own (Logitech, ASUS, QMK/VIA, LampArray), found by its code.
    Native,
    Razer(&'static razer::RazerModel),
    Via(&'static via::ViaModel),
}

/// Every model in panel order: the Rust models, with the profile models by their protocol
/// (see below). Loaded once; profile problems go to the log and `--diagnose`.
pub fn kinds() -> &'static [Kind] {
    static KINDS: std::sync::OnceLock<Vec<Kind>> = std::sync::OnceLock::new();
    KINDS.get_or_init(|| {
        let (profiles, errors) = profile::load();
        for (file, e) in &errors {
            crate::log(&format!("profile {file}: {e}"));
        }
        *PROFILE_ERRORS.lock().unwrap() = errors;
        // Profile models sit by their protocol: Razer ones after the Logitech keyboard, QMK/VIA
        // ones before the generic QMK/VIA model.
        let profiles: Vec<Kind> = profiles.into_iter().filter(|p| NATIVE.iter().all(|n| n.key != p.key)).collect();
        let of = |via: bool| profiles.iter().filter(move |p| matches!(p.protocol, Protocol::Via(_)) == via).cloned();
        let mut all = Vec::new();
        for n in NATIVE.iter() {
            if n.key == "via" {
                all.extend(of(true));
            }
            all.push(n.clone());
            if n.key == "logitech" {
                all.extend(of(false));
            }
        }
        all
    })
}

/// Profiles that couldn't be read: (file, why).
pub static PROFILE_ERRORS: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

impl Kind {
    /// Default name of the model in the interface language.
    pub fn name(&self) -> &'static str {
        crate::i18n::pick(self.names)
    }
}

/// What a device is, for wording on the panel ("Вся мышь").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Keyboard,
    Mouse,
    Other,
    /// A room light: it doesn't go dark or dim with the display.
    Light,
}

/// How the speed of breathing and spectrum can be set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Speed {
    /// The protocol has no speed.
    None,
    /// A few fixed speeds, slowest first (ASUS Aura: 3).
    Steps(u8),
    /// Any speed (`speed_factor`).
    Smooth,
}

/// What a device can do: the panel shows only these controls. The model gives what is known
/// in advance; drivers add what they read from the device itself (LampArray zones and keys).
#[derive(Clone, PartialEq, Debug)]
pub struct Caps {
    pub effects: &'static [Effect],
    pub speed: Speed,
    /// Independently lit zones: (LED / zone id, name (Russian, English)).
    pub zones: Vec<(u8, (String, String))>,
    /// Per-key colors (the key editor).
    pub per_key: bool,
    /// The key layout of a keyboard with per-key colors (`keymap::get`).
    pub layout: &'static str,
    pub class: Class,
}

impl Caps {
    pub fn has_effect(&self, e: Effect) -> bool {
        self.effects.contains(&e)
    }

    /// Breathing or spectrum can run at another speed.
    pub fn has_speed(&self) -> bool {
        self.speed != Speed::None
    }
}

/// Capabilities of a device, by id: its model's, with what its driver read from the device
/// (only while it was found in this run). Works for disconnected devices too.
pub fn caps(id: &str) -> Caps {
    let kind = kind_of(id);
    let mut zones: Vec<(u8, (String, String))> = kind.zones.iter().map(|&(led, (ru, en))| (led, (ru.to_string(), en.to_string()))).collect();
    if zones.is_empty() {
        zones = lamparray::zones(id).into_iter().map(|z| (z.led, z.names)).collect();
    }
    // A LampArray keyboard with lamps on keypad keys is full size.
    let layout = kind.layout.unwrap_or(if lamparray::has_keypad(id) { crate::keymap::FULL } else { crate::keymap::TKL });
    Caps { effects: kind.effects, speed: kind.speed, zones, per_key: kind.layout.is_some() || lamparray::has_keys(id), layout, class: kind.class }
}

const ALL_EFFECTS: &[Effect] = &Effect::ALL;

/// Models written in Rust, in panel order (profile models are placed among them by `kinds`).
const NATIVE: [Kind; 5] = [
    Kind {
        key: "logitech",
        names: ("Logitech G915 TKL", "Logitech G915 TKL"),
        icon: "\u{E765}",
        zones: &[],
        layout: Some(crate::keymap::G915_TKL),
        port_id: false,
        effects: &[Effect::Static, Effect::Breathe, Effect::Cycle, Effect::Off, Effect::Wave, Effect::Ripple],
        speed: Speed::Smooth,
        class: Class::Keyboard,
        protocol: Protocol::Native,
        tested: true,
    },
    Kind {
        key: "asus",
        names: ("Клавиатура ASUS", "ASUS keyboard"),
        icon: "\u{E7F8}",
        zones: &[],
        layout: None,
        port_id: false,
        effects: &[Effect::Static, Effect::Breathe, Effect::Cycle, Effect::Off, Effect::Wave],
        speed: Speed::Steps(3),
        class: Class::Keyboard,
        protocol: Protocol::Native,
        tested: true,
    },
    // Any QMK keyboard with VIA support and RGB lighting.
    Kind {
        key: "via",
        names: ("Клавиатура QMK", "QMK keyboard"),
        icon: "\u{E765}",
        zones: &[],
        layout: None,
        port_id: true,
        effects: ALL_EFFECTS,
        speed: Speed::Smooth,
        class: Class::Keyboard,
        protocol: Protocol::Native,
        tested: true,
    },
    // Any device with the standard HID LampArray interface and no driver of its own.
    Kind {
        key: "lamparray",
        names: ("Устройство LampArray", "LampArray device"),
        icon: "\u{E82F}",
        zones: &[],
        layout: None,
        port_id: true,
        effects: ALL_EFFECTS,
        speed: Speed::Smooth,
        class: Class::Other,
        protocol: Protocol::Native,
        tested: true,
    },
    // Tuya Wi-Fi bulbs on the local network, listed in tuya.toml.
    Kind {
        key: "tuya",
        names: ("Лампа Tuya", "Tuya bulb"),
        icon: "\u{E82F}",
        zones: &[],
        layout: None,
        port_id: false,
        effects: &[Effect::Static, Effect::Breathe, Effect::Cycle, Effect::Off],
        speed: Speed::Smooth,
        class: Class::Light,
        protocol: Protocol::Native,
        tested: true,
    },
];

/// Opens a keyboard for the key editor, by id.
pub fn open_for_keys(id: &str) -> Option<Box<dyn Device>> {
    DRIVERS.iter().find_map(|d| (d.open_for_keys)(id))
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
    caps(id).zones.into_iter().map(|(led, (ru, en))| (led, if crate::i18n::en() { en } else { ru })).collect()
}

/// What a HID interface is to this app, for the device report: a supported device, or a
/// known lighting protocol that a new driver could build on.
pub fn interface_note(i: &crate::hid::HidInfo) -> Option<&'static str> {
    DRIVERS.iter().find_map(|d| (d.note)(i)).or((i.usage_page >= 0xFF00).then_some("vendor-specific"))
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
    let all = kinds();
    all.iter().find(|k| k.key == key).unwrap_or(&all[0])
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
    let mut d = Discovery { found: Vec::new(), no_link: Vec::new(), no_link_kinds: Vec::new() };
    for driver in DRIVERS {
        (driver.discover)(&mut d);
    }
    // One device reachable two ways (e.g. a keyboard on cable and receiver) is driven once.
    let mut seen = Vec::new();
    d.found.retain(|f| {
        let first = !seen.contains(&f.id);
        seen.push(f.id.clone());
        first
    });
    d
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
        assert_eq!(kind_of("toaster-1").key, kinds()[0].key);
    }

    #[test]
    fn kind_keys_are_unique_and_have_no_dash() {
        for (i, k) in kinds().iter().enumerate() {
            assert!(!k.key.contains('-'), "{}", k.key);
            assert!(kinds()[i + 1..].iter().all(|o| o.key != k.key), "{}", k.key);
        }
    }

    #[test]
    fn caps_come_from_the_model() {
        // A disconnected Naga still has its zones, no speed, and is a mouse.
        let naga = caps("naga-123456H78900012");
        assert_eq!(naga.zones.iter().map(|z| z.0).collect::<Vec<_>>(), vec![0x04, 0x05]);
        assert_eq!(naga.zones[0].1 .1, "Logo");
        assert!(!naga.has_speed() && !naga.per_key && naga.class == Class::Mouse);
        assert!(Effect::ALL.iter().all(|&e| naga.has_effect(e)));
        let g915 = caps("logitech-1A2B3C4D");
        assert!(g915.per_key && g915.has_effect(Effect::Wave) && g915.has_effect(Effect::Ripple) && !g915.has_effect(Effect::Starlight));
        assert!(caps("asus").has_effect(Effect::Wave) && !caps("asus").has_effect(Effect::Ripple));
        // Extra effects are opt-in per profile: none for the Naga.
        assert!(Effect::EXTRA.iter().all(|&e| !naga.has_effect(e)));
        // Settings files spell them in lower case.
        assert_eq!(toml::to_string(&std::collections::BTreeMap::from([("e", Effect::Starlight)])).unwrap().trim(), "e = \"starlight\"");
        assert_eq!(caps("asus").speed, Speed::Steps(3));
        // A LampArray device not found in this run has no zones or keys yet.
        let la = caps("lamparray-DEADBEEF");
        assert!(la.zones.is_empty() && !la.per_key && la.speed == Speed::Smooth);
    }

    #[test]
    fn panel_order_of_the_owners_devices() {
        // Profiles sit by their protocol, so moving models to profiles keeps the panel order.
        let pos = |key: &str| kinds().iter().position(|k| k.key == key).unwrap();
        let order = ["logitech", "naga", "dock", "asus", "macropad", "via", "lamparray"].map(pos);
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    }

    #[test]
    fn every_model_can_be_switched_off() {
        // "Off" is how the app blanks devices with the display; every model needs it.
        assert!(kinds().iter().all(|k| k.effects.contains(&Effect::Off)));
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
