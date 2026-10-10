use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::devices::{kind_of, Effect, Rgb, KINDS};

/// Settings of one device (by device id: "naga-123456H246", "logitech-1A2B3C4D", ...).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Name given by the user or to tell identical devices apart; empty = the model name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub effect: Effect,
    /// "#RRGGBB"
    pub color: String,
    /// Separate colors of lighting zones: LED id (decimal) -> "#RRGGBB". Empty = all zones use `color`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub zones: BTreeMap<String, String>,
    /// Brightness relative to the overall one, percent (devices differ a lot in how bright they are).
    #[serde(default = "full_brightness", skip_serializing_if = "is_full_brightness")]
    pub brightness: u8,
    /// The keyboard shows `keys` instead of its effect.
    #[serde(default, skip_serializing_if = "is_false")]
    pub per_key: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: KeyColors,
    /// When the device was last connected (Unix seconds).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub last_seen: u64,
}

fn full_brightness() -> u8 {
    100
}

fn is_full_brightness(b: &u8) -> bool {
    *b == 100
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

impl Default for DeviceConfig {
    fn default() -> Self {
        DeviceConfig {
            name: String::new(),
            effect: Effect::Static,
            color: "#FFFFFF".into(),
            zones: BTreeMap::new(),
            brightness: 100,
            per_key: false,
            keys: KeyColors::new(),
            last_seen: 0,
        }
    }
}

impl DeviceConfig {
    pub fn zone_rgb(&self, led: u8) -> Rgb {
        self.zones.get(&led.to_string()).map(|s| parse_hex(s)).unwrap_or_else(|| self.rgb())
    }

    /// Sets one zone; the other zones keep showing their current color.
    pub fn set_zone(&mut self, led: u8, c: Rgb, all_leds: &[u8]) {
        for &other in all_leds {
            let current = self.zone_rgb(other);
            self.zones.entry(other.to_string()).or_insert_with(|| to_hex(current));
        }
        self.zones.insert(led.to_string(), to_hex(c));
    }

    pub fn rgb(&self) -> Rgb {
        parse_hex(&self.color)
    }

    pub fn set_rgb(&mut self, c: Rgb) {
        self.color = to_hex(c);
    }

    pub fn key_color(&self, code: u8) -> Rgb {
        self.keys.get(&code.to_string()).map(|s| parse_hex(s)).unwrap_or(Rgb(0, 0, 0))
    }

    /// Off keys are stored too ("#000000"): a missing key is one that was never set, e.g. a key
    /// added to the layout later, and gets the keyboard's current color in the editor.
    pub fn set_key(&mut self, code: u8, c: Rgb) {
        self.keys.insert(code.to_string(), to_hex(c));
    }

    pub fn has_key(&self, code: u8) -> bool {
        self.keys.contains_key(&code.to_string())
    }

    /// Every addressable key with its color (off keys included), ready to send.
    pub fn all_keys(&self) -> Vec<(u8, Rgb)> {
        crate::keymap::all_codes().into_iter().map(|c| (c, self.key_color(c))).collect()
    }

    /// Only the lighting (what presets save), without name and bookkeeping.
    fn lighting(&self) -> DeviceConfig {
        DeviceConfig { name: String::new(), last_seen: 0, ..self.clone() }
    }

    fn set_lighting(&mut self, from: &DeviceConfig) {
        *self = DeviceConfig { name: std::mem::take(&mut self.name), last_seen: self.last_seen, ..from.lighting() };
    }

    fn same_lighting(&self, other: &DeviceConfig) -> bool {
        let (a, b) = (self.lighting(), other.lighting());
        // Per-key colors matter only while they are shown; compared key by key, since an off
        // key may be stored as black or (older versions) left out.
        let keys_equal = !a.per_key && !b.per_key || a.all_keys() == b.all_keys();
        keys_equal && DeviceConfig { keys: KeyColors::new(), ..a } == DeviceConfig { keys: KeyColors::new(), ..b }
    }
}

fn parse_hex(s: &str) -> Rgb {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
    Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn to_hex(c: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

/// Colors of individual keyboard keys: zone id (decimal) -> "#RRGGBB". Missing keys are off.
pub type KeyColors = BTreeMap<String, String>;

#[derive(Clone, Serialize, Deserialize)]
pub struct Scheme {
    pub name: String,
    pub keys: KeyColors,
}

/// Global hotkey: MOD_* flags of RegisterHotKey and a virtual-key code.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Hotkey {
    pub mods: u32,
    pub vk: u32,
}

impl Hotkey {
    pub fn label(&self) -> String {
        let mut s = String::new();
        for (flag, name) in [(0x2, "Ctrl+"), (0x1, "Alt+"), (0x4, "Shift+"), (0x8, "Win+")] {
            if self.mods & flag != 0 {
                s.push_str(name);
            }
        }
        let key = match self.vk {
            0x70..=0x87 => format!("F{}", self.vk - 0x6F),
            0x30..=0x39 | 0x41..=0x5A => char::from_u32(self.vk).map(String::from).unwrap_or_default(),
            0x60..=0x69 => format!("Num{}", self.vk - 0x60),
            vk => format!("0x{vk:02X}"),
        };
        s + &key
    }
}

/// Snapshot of all lighting settings, switchable in one click or by hotkey.
#[derive(Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub brightness: u8,
    pub devices: BTreeMap<String, DeviceConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<Hotkey>,
    /// Per-key colors of versions before 0.4 (now part of the keyboard's settings).
    #[serde(default, skip_serializing_if = "is_false")]
    per_key: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    keys: KeyColors,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 0..=100
    pub brightness: u8,
    /// Every device ever connected, by device id (disconnected ones are the archive).
    pub devices: BTreeMap<String, DeviceConfig>,
    /// Custom colors of the color picker, as COLORREF (0x00BBGGRR).
    pub custom_colors: Vec<u32>,
    pub schemes: Vec<Scheme>,
    /// ISO layout (tall Enter, short left Shift) in the key editor.
    pub iso_layout: bool,
    pub presets: Vec<Preset>,
    /// Per-key colors of versions before 0.4, moved to the first Logitech keyboard found
    /// (kept in the file until then).
    #[serde(skip_serializing_if = "is_false")]
    per_key: bool,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    keys: KeyColors,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            brightness: 100,
            devices: BTreeMap::new(),
            custom_colors: vec![0xFFFFFF; 16],
            schemes: Vec::new(),
            iso_layout: false,
            presets: Vec::new(),
            per_key: false,
            keys: KeyColors::new(),
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Config {
    pub fn snapshot(&self, name: String) -> Preset {
        Preset {
            name,
            brightness: self.brightness,
            devices: self.devices.iter().map(|(id, d)| (id.clone(), d.lighting())).collect(),
            hotkey: None,
            per_key: false,
            keys: KeyColors::new(),
        }
    }

    /// Takes over the preset's lighting; devices the preset doesn't know keep theirs.
    pub fn load_preset(&mut self, p: &Preset) {
        self.brightness = p.brightness;
        for (id, d) in self.devices.iter_mut() {
            if let Some(pd) = p.devices.get(id) {
                d.set_lighting(pd);
            }
        }
    }

    /// Whether the current lighting is exactly this preset.
    pub fn matches(&self, p: &Preset) -> bool {
        self.brightness == p.brightness
            && self.devices.iter().all(|(id, d)| p.devices.get(id).map_or(true, |pd| d.same_lighting(pd)))
    }

    /// The color all these devices show, when they all show the same single color.
    pub fn uniform_color(&self, ids: &[String]) -> Option<Rgb> {
        let devices: Vec<DeviceConfig> = ids.iter().map(|id| self.device(id)).collect();
        let first = devices.first()?;
        let single = |d: &DeviceConfig| {
            matches!(d.effect, Effect::Static | Effect::Breathe) && !d.per_key && d.zones.is_empty() && d.color == first.color
        };
        devices.iter().all(single).then(|| first.rgb())
    }

    /// What a device is actually driven at: the overall brightness times its own share.
    pub fn effective_brightness(&self, id: &str) -> u8 {
        let b = self.brightness as u32 * self.device(id).brightness.min(100) as u32 / 100;
        b.max(1) as u8
    }

    /// The device's name on the panel.
    pub fn display_name(&self, id: &str) -> String {
        match self.devices.get(id) {
            Some(d) if !d.name.is_empty() => d.name.clone(),
            _ => kind_of(id).name.to_string(),
        }
    }

    /// Records connected devices. A new one takes over the settings of the same model from an
    /// older version (or, for devices identified by port, from an absent one of its model);
    /// otherwise it starts with the look of the other devices. Returns whether to save.
    pub fn register(&mut self, connected: &[String]) -> bool {
        let mut changed = false;
        let now = now();
        for id in connected {
            if !self.devices.contains_key(id) {
                let kind = kind_of(id);
                let adopt = if id != kind.key && self.devices.contains_key(kind.key) {
                    Some(kind.key.to_string())
                } else if kind.port_id {
                    self.devices.keys().find(|k| kind_of(k).key == kind.key && !connected.contains(k)).cloned()
                } else {
                    None
                };
                let mut d = match &adopt {
                    Some(old) => {
                        for p in self.presets.iter_mut() {
                            if let Some(pd) = p.devices.remove(old) {
                                p.devices.insert(id.clone(), pd);
                            }
                        }
                        self.devices.remove(old).unwrap_or_default()
                    }
                    None => {
                        let mut d = self.inherited();
                        let same_model = self.devices.keys().filter(|k| kind_of(k).key == kind.key).count();
                        if same_model > 0 {
                            d.name = format!("{} {}", kind.name, same_model + 1);
                        }
                        d
                    }
                };
                // Per-key colors of older versions belong to the first keyboard found.
                if kind.per_key && (self.per_key || !self.keys.is_empty()) {
                    d.per_key = std::mem::take(&mut self.per_key);
                    d.keys = std::mem::take(&mut self.keys);
                    for p in self.presets.iter_mut() {
                        let pd = p.devices.entry(id.clone()).or_default();
                        pd.per_key = std::mem::take(&mut p.per_key);
                        pd.keys = std::mem::take(&mut p.keys);
                    }
                }
                self.devices.insert(id.clone(), d);
                changed = true;
            }
            let d = self.devices.get_mut(id).unwrap();
            // Saved at most hourly; it only orders the list of disconnected devices.
            if now.saturating_sub(d.last_seen) > 3600 {
                d.last_seen = now;
                changed = true;
            }
        }
        changed
    }

    /// The look of the first configured device, for a newly connected one.
    fn inherited(&self) -> DeviceConfig {
        KINDS
            .iter()
            .find_map(|k| self.devices.iter().find(|(id, _)| kind_of(id).key == k.key).map(|(_, d)| d))
            .map(|d| DeviceConfig { effect: d.effect, color: d.color.clone(), ..DeviceConfig::default() })
            .unwrap_or_default()
    }

    /// Removes a device and its place in presets.
    pub fn forget(&mut self, id: &str) {
        self.devices.remove(id);
        for p in self.presets.iter_mut() {
            p.devices.remove(id);
        }
    }

    /// Device ids in panel order: by model, then by name.
    pub fn sorted(&self, ids: impl IntoIterator<Item = String>) -> Vec<String> {
        let mut v: Vec<String> = ids.into_iter().collect();
        let rank = |id: &String| KINDS.iter().position(|k| k.key == kind_of(id).key).unwrap_or(usize::MAX);
        v.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| self.display_name(a).cmp(&self.display_name(b))));
        v
    }
}

impl Config {
    pub fn dir() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("rgbtray")
    }

    fn path() -> PathBuf {
        Self::dir().join("config.toml")
    }

    pub fn load() -> Config {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(Self::dir());
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), s);
        }
    }

    pub fn device(&self, id: &str) -> DeviceConfig {
        self.devices.get(id).cloned().unwrap_or_else(|| self.inherited())
    }

    pub fn device_mut(&mut self, id: &str) -> &mut DeviceConfig {
        let inherited = self.inherited();
        self.devices.entry(id.to_string()).or_insert(inherited)
    }
}
