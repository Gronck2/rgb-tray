pub mod asus;
pub mod logitech;
pub mod macropad;
pub mod razer;

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
            Effect::Static => "Статичный",
            Effect::Breathe => "Дыхание",
            Effect::Cycle => "Смена цветов",
            Effect::Off => "Выключено",
        }
    }
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
        Err("цвета отдельных клавиш не поддерживаются".into())
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
            format!("заряжается {}%", self.percent)
        } else {
            format!("батарея {}%", self.percent)
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
    pub name: &'static str,
    /// Segoe Fluent Icons glyph.
    pub icon: &'static str,
    /// Independently lit zones: (LED id, name); empty = one color only.
    pub zones: &'static [(u8, &'static str)],
    /// Keyboard with per-key colors (the key editor).
    pub per_key: bool,
    /// The firmware has no serial number: the id comes from the USB port, so a device moved
    /// to another port takes over the settings of an absent one of the same kind.
    pub port_id: bool,
}

/// Every supported model, in panel order.
pub const KINDS: [Kind; 5] = [
    Kind { key: "logitech", name: "Logitech G915 TKL", icon: "\u{E765}", zones: &[], per_key: true, port_id: false },
    Kind { key: "naga", name: "Razer Naga V2 Pro", icon: "\u{E962}", zones: &NAGA_ZONES, per_key: false, port_id: false },
    Kind { key: "dock", name: "Razer Mouse Dock Pro", icon: "\u{E772}", zones: &[], per_key: false, port_id: false },
    Kind { key: "asus", name: "Клавиатура ASUS", icon: "\u{E7F8}", zones: &[], per_key: false, port_id: false },
    Kind { key: "macropad", name: "Geekboard MacroPad", icon: "\u{E92E}", zones: &[], per_key: false, port_id: true },
];

/// Independently lit zones of the Naga V2 Pro: (LED id, name).
pub const NAGA_ZONES: [(u8, &str); 2] = [(0x04, "Логотип"), (0x05, "Боковая панель")];

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
pub const NO_LINK: &str = "нет связи с устройством";

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
    for d in macropad::MacroPad::find_all() {
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