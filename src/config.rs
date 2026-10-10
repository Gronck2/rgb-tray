use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::devices::{kind_of, Effect, Rgb, KINDS, NORMAL_SPEED};

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

    /// What the device shows: per-key colors that are all the same look exactly like that
    /// color shown steadily (e.g. right after opening the key editor or "fill all"), so the
    /// panel and preset matching treat them as that.
    pub fn visible(&self) -> DeviceConfig {
        let keys = self.all_keys();
        match keys.first() {
            Some(&(_, c)) if self.per_key && keys.iter().all(|&(_, k)| k == c) => {
                let mut d = DeviceConfig { per_key: false, keys: KeyColors::new(), effect: Effect::Static, ..self.clone() };
                if c == Rgb(0, 0, 0) {
                    d.effect = Effect::Off;
                } else {
                    d.set_rgb(c);
                }
                d
            }
            _ => self.clone(),
        }
    }

    fn same_lighting(&self, other: &DeviceConfig) -> bool {
        let (a, b) = (self.visible().lighting(), other.visible().lighting());
        // Per-key colors matter only while they are shown; compared key by key, since an off
        // key may be stored as black or (older versions) left out.
        let keys_equal = !a.per_key && !b.per_key || a.all_keys() == b.all_keys();
        keys_equal && DeviceConfig { keys: KeyColors::new(), ..a } == DeviceConfig { keys: KeyColors::new(), ..b }
    }
}

pub fn parse_hex(s: &str) -> Rgb {
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
    /// Speed of breathing and spectrum (`devices::speed`).
    #[serde(default = "normal_speed", skip_serializing_if = "is_normal_speed")]
    pub speed: u8,
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
    /// Speed of breathing and spectrum for all devices, 0..=100 (`devices::speed`).
    #[serde(skip_serializing_if = "is_normal_speed")]
    pub speed: u8,
    /// Every device ever connected, by device id (disconnected ones are the archive).
    pub devices: BTreeMap<String, DeviceConfig>,
    /// Custom colors of the color picker, as COLORREF (0x00BBGGRR).
    pub custom_colors: Vec<u32>,
    pub schemes: Vec<Scheme>,
    /// ISO layout (tall Enter, short left Shift) in the key editor.
    pub iso_layout: bool,
    /// The panel is pinned: it stays open and on top when it loses focus.
    pub pinned: bool,
    /// Interface language "ru" or "en"; not set = follow Windows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Top-left corner (screen pixels) of the pinned panel after the user dragged it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_pos: Option<(i32, i32)>,
    pub presets: Vec<Preset>,
    /// Programs whose "it drives the lighting too" warning the user hid (`conflicts::App::key`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored_apps: Vec<String>,
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
            speed: NORMAL_SPEED,
            devices: BTreeMap::new(),
            custom_colors: vec![0xFFFFFF; 16],
            schemes: Vec::new(),
            iso_layout: false,
            pinned: false,
            language: None,
            panel_pos: None,
            presets: Vec::new(),
            ignored_apps: Vec::new(),
            per_key: false,
            keys: KeyColors::new(),
        }
    }
}

fn normal_speed() -> u8 {
    NORMAL_SPEED
}

fn is_normal_speed(s: &u8) -> bool {
    *s == NORMAL_SPEED
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
            speed: self.speed,
            per_key: false,
            keys: KeyColors::new(),
        }
    }

    /// Takes over the preset's lighting; devices the preset doesn't know keep theirs.
    pub fn load_preset(&mut self, p: &Preset) {
        self.brightness = p.brightness;
        self.speed = p.speed;
        for (id, d) in self.devices.iter_mut() {
            if let Some(pd) = p.devices.get(id) {
                d.set_lighting(pd);
            }
        }
    }

    /// Whether the current lighting is exactly this preset.
    pub fn matches(&self, p: &Preset) -> bool {
        // The speed shows only while something breathes or cycles.
        let animated = self.devices.values().any(|d| matches!(d.visible().effect, Effect::Breathe | Effect::Cycle) && !d.per_key);
        self.brightness == p.brightness
            && (self.speed == p.speed || !animated)
            && self.devices.iter().all(|(id, d)| p.devices.get(id).is_none_or(|pd| d.same_lighting(pd)))
    }

    /// The color all these devices show, when they all show the same single color.
    pub fn uniform_color(&self, ids: &[String]) -> Option<Rgb> {
        let devices: Vec<DeviceConfig> = ids.iter().map(|id| self.device(id).visible()).collect();
        let first = devices.first()?;
        let single = |d: &DeviceConfig| matches!(d.effect, Effect::Static | Effect::Breathe) && !d.per_key && d.zones.is_empty() && d.color == first.color;
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
            _ => crate::devices::name_of(id).unwrap_or_else(|| kind_of(id).name().to_string()),
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
                            d.name = format!("{} {}", kind.name(), same_model + 1);
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
        std::fs::read_to_string(Self::path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn device(effect: Effect, color: &str) -> DeviceConfig {
        DeviceConfig { effect, color: color.into(), ..DeviceConfig::default() }
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(parse_hex("#0AE3EF"), Rgb(0x0A, 0xE3, 0xEF));
        assert_eq!(parse_hex("0ae3ef"), Rgb(0x0A, 0xE3, 0xEF));
        assert_eq!(to_hex(Rgb(0x0A, 0xE3, 0xEF)), "#0AE3EF");
        // Broken values read as black instead of failing the whole config.
        assert_eq!(parse_hex("#zzzzzz"), Rgb(0, 0, 0));
    }

    #[test]
    fn setting_one_zone_keeps_the_others() {
        let mut d = device(Effect::Static, "#FF0000");
        d.set_zone(5, Rgb(0, 0, 255), &[4, 5]);
        assert_eq!(d.zone_rgb(4), Rgb(255, 0, 0));
        assert_eq!(d.zone_rgb(5), Rgb(0, 0, 255));
        // Changing the main color later doesn't repaint a zone that was set.
        d.set_rgb(Rgb(0, 255, 0));
        assert_eq!(d.zone_rgb(4), Rgb(255, 0, 0));
    }

    #[test]
    fn uniform_key_colors_look_like_a_steady_color() {
        let mut d = DeviceConfig { per_key: true, ..DeviceConfig::default() };
        for c in crate::keymap::all_codes() {
            d.set_key(c, Rgb(0, 255, 0));
        }
        let v = d.visible();
        assert!(!v.per_key && v.effect == Effect::Static && v.rgb() == Rgb(0, 255, 0));

        for c in crate::keymap::all_codes() {
            d.set_key(c, Rgb(0, 0, 0));
        }
        assert!(d.visible().effect == Effect::Off);

        // One key different: still per-key.
        d.set_key(crate::keymap::all_codes()[0], Rgb(1, 2, 3));
        assert!(d.visible().per_key);
    }

    #[test]
    fn preset_round_trip() {
        let mut cfg = Config::default();
        cfg.devices.insert("naga-A".into(), DeviceConfig { name: "Mouse".into(), ..device(Effect::Breathe, "#112233") });
        cfg.brightness = 70;
        let p = cfg.snapshot("Evening".into());
        assert!(cfg.matches(&p));
        // The preset stores lighting only, not the user's name for the device.
        assert!(p.devices["naga-A"].name.is_empty());

        cfg.devices.get_mut("naga-A").unwrap().set_rgb(Rgb(9, 9, 9));
        assert!(!cfg.matches(&p));
        cfg.brightness = 20;
        cfg.load_preset(&p);
        assert!(cfg.matches(&p));
        assert_eq!(cfg.brightness, 70);
        assert_eq!(cfg.devices["naga-A"].name, "Mouse");
    }

    #[test]
    fn preset_speed_matters_only_while_animating() {
        let mut cfg = Config::default();
        cfg.devices.insert("naga-A".into(), device(Effect::Static, "#FF0000"));
        let p = cfg.snapshot("P".into());
        cfg.speed = 90;
        assert!(cfg.matches(&p));
        cfg.devices.get_mut("naga-A").unwrap().effect = Effect::Breathe;
        let p = cfg.snapshot("P".into());
        cfg.speed = 10;
        assert!(!cfg.matches(&p));
        cfg.load_preset(&p);
        assert_eq!(cfg.speed, 90);
        assert!(cfg.matches(&p));
        // Presets of older versions have no speed: normal.
        let old: Preset = toml::from_str("name = \"Old\"\nbrightness = 100\n[devices]\n").unwrap();
        assert_eq!(old.speed, NORMAL_SPEED);
    }

    #[test]
    fn preset_ignores_devices_it_does_not_know() {
        let mut cfg = Config::default();
        cfg.devices.insert("naga-A".into(), device(Effect::Static, "#FF0000"));
        let p = cfg.snapshot("P".into());
        cfg.devices.insert("dock-B".into(), device(Effect::Cycle, "#00FF00"));
        assert!(cfg.matches(&p));
        cfg.load_preset(&p);
        assert!(cfg.devices["dock-B"].effect == Effect::Cycle);
    }

    #[test]
    fn uniform_color() {
        let mut cfg = Config::default();
        cfg.devices.insert("naga-A".into(), device(Effect::Static, "#FF0000"));
        cfg.devices.insert("dock-B".into(), device(Effect::Breathe, "#FF0000"));
        assert_eq!(cfg.uniform_color(&ids(&["naga-A", "dock-B"])), Some(Rgb(255, 0, 0)));
        cfg.devices.get_mut("dock-B").unwrap().effect = Effect::Cycle;
        assert_eq!(cfg.uniform_color(&ids(&["naga-A", "dock-B"])), None);
    }

    #[test]
    fn effective_brightness_never_reaches_zero() {
        let mut cfg = Config::default();
        cfg.devices.insert("naga-A".into(), DeviceConfig { brightness: 50, ..DeviceConfig::default() });
        cfg.brightness = 80;
        assert_eq!(cfg.effective_brightness("naga-A"), 40);
        cfg.brightness = 1;
        assert_eq!(cfg.effective_brightness("naga-A"), 1);
    }

    #[test]
    fn register_moves_single_device_settings_of_old_versions() {
        let mut cfg = Config::default();
        cfg.devices.insert("logitech".into(), device(Effect::Breathe, "#FF0000"));
        let mut p = cfg.snapshot("P".into());
        p.devices.insert("logitech".into(), device(Effect::Static, "#0000FF"));
        cfg.presets.push(p);
        cfg.per_key = true;
        cfg.keys.insert("4".into(), "#00FF00".into());

        assert!(cfg.register(&ids(&["logitech-1A2B3C4D"])));
        assert!(!cfg.devices.contains_key("logitech"));
        let d = &cfg.devices["logitech-1A2B3C4D"];
        assert!(d.effect == Effect::Breathe && d.color == "#FF0000");
        assert!(d.per_key && d.key_color(4) == Rgb(0, 255, 0));
        assert!(!cfg.per_key && cfg.keys.is_empty());
        assert_eq!(cfg.presets[0].devices["logitech-1A2B3C4D"].color, "#0000FF");
        assert!(!cfg.presets[0].devices.contains_key("logitech"));
    }

    #[test]
    fn register_names_a_second_identical_device() {
        let mut cfg = Config::default();
        cfg.devices.insert("naga-A".into(), device(Effect::Static, "#123456"));
        cfg.register(&ids(&["naga-A", "naga-B"]));
        let b = &cfg.devices["naga-B"];
        assert_eq!(b.name, format!("{} 2", kind_of("naga-B").name()));
        // A new device starts with the look of the others.
        assert_eq!(b.color, "#123456");
        assert_eq!(cfg.display_name("naga-A"), kind_of("naga-A").name());
    }

    #[test]
    fn register_by_port_takes_over_an_absent_device_of_its_model() {
        let mut cfg = Config::default();
        cfg.devices.insert("macropad-OLD".into(), device(Effect::Cycle, "#ABCDEF"));
        cfg.register(&ids(&["macropad-NEW"]));
        assert!(!cfg.devices.contains_key("macropad-OLD"));
        assert!(cfg.devices["macropad-NEW"].effect == Effect::Cycle);

        // Both connected: nothing to take over.
        let mut cfg = Config::default();
        cfg.devices.insert("macropad-OLD".into(), device(Effect::Cycle, "#ABCDEF"));
        cfg.register(&ids(&["macropad-OLD", "macropad-NEW"]));
        assert!(cfg.devices.contains_key("macropad-OLD") && cfg.devices.contains_key("macropad-NEW"));
    }

    #[test]
    fn forget_removes_the_device_from_presets() {
        let mut cfg = Config::default();
        cfg.devices.insert("naga-A".into(), DeviceConfig::default());
        cfg.presets.push(cfg.snapshot("P".into()));
        cfg.forget("naga-A");
        assert!(cfg.devices.is_empty() && cfg.presets[0].devices.is_empty());
    }

    #[test]
    fn toml_round_trip_and_old_files() {
        let mut cfg = Config::default();
        cfg.devices.insert("naga-A".into(), DeviceConfig { brightness: 40, ..device(Effect::Breathe, "#010203") });
        cfg.presets.push(Preset { hotkey: Some(Hotkey { mods: 2, vk: 0x7C }), ..cfg.snapshot("P".into()) });
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert!(back.devices == cfg.devices);
        assert!(back.presets[0].hotkey == Some(Hotkey { mods: 2, vk: 0x7C }));

        // A minimal file of an old version: everything missing gets its default.
        let old: Config = toml::from_str("brightness = 50\n[devices.naga]\neffect = \"static\"\ncolor = \"#112233\"\n").unwrap();
        assert_eq!(old.brightness, 50);
        assert_eq!(old.devices["naga"].brightness, 100);
        assert_eq!(old.custom_colors.len(), 16);
    }

    #[test]
    fn sorted_by_model_then_name() {
        let mut cfg = Config::default();
        cfg.devices.insert("naga-B".into(), DeviceConfig { name: "b".into(), ..DeviceConfig::default() });
        cfg.devices.insert("naga-A".into(), DeviceConfig { name: "a".into(), ..DeviceConfig::default() });
        let v = cfg.sorted(ids(&["asus", "naga-B", "logitech-X", "naga-A"]));
        assert_eq!(v, ids(&["logitech-X", "naga-A", "naga-B", "asus"]));
    }

    #[test]
    fn hotkey_labels() {
        assert_eq!(Hotkey { mods: 0x2 | 0x4, vk: 0x7C }.label(), "Ctrl+Shift+F13");
        assert_eq!(Hotkey { mods: 0x1, vk: 0x41 }.label(), "Alt+A");
        assert_eq!(Hotkey { mods: 0x8, vk: 0x61 }.label(), "Win+Num1");
    }
}
