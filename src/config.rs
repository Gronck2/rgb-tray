use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::devices::{kind_of, kinds, Effect, Rgb, NORMAL_SPEED};

/// Settings of one device (by device id: "naga-123456H246", "logitech-1A2B3C4D", ...).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Name given by the user or to tell identical devices apart; empty = the model name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(deserialize_with = "lenient_effect")]
    pub effect: Effect,
    /// "#RRGGBB"
    pub color: String,
    /// Separate colors of lighting zones: LED id (decimal) -> "#RRGGBB". Empty = all zones use `color`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub zones: BTreeMap<String, String>,
    /// Brightness relative to the overall one, percent (devices differ a lot in how bright they are).
    #[serde(default = "full_brightness", skip_serializing_if = "is_full_brightness")]
    pub brightness: u8,
    /// Speed of breathing and spectrum relative to the overall one, 0..=100; NORMAL_SPEED = the same.
    #[serde(default = "normal_speed", skip_serializing_if = "is_normal_speed")]
    pub speed: u8,
    /// The keyboard shows `keys` instead of its effect.
    #[serde(default, skip_serializing_if = "is_false")]
    pub per_key: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: KeyColors,
    /// When the device was last connected (Unix seconds).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub last_seen: u64,
    /// Global hotkey that switches the device off and back on (not part of presets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<Hotkey>,
    /// Switched off by its hotkey: dark, with its lighting kept for the next press. Like the
    /// display going dark, it is not part of presets.
    #[serde(default, skip_serializing_if = "is_false")]
    pub switched_off: bool,
}

/// An effect this version doesn't know (saved by a newer or an older one) reads as steady:
/// one strange value must not make the whole settings file unreadable (0.6.24 lost the
/// owner's presets that way after 0.6.23 had saved "experimental").
fn lenient_effect<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Effect, D::Error> {
    Ok(effect_named(&String::deserialize(d)?))
}

fn effect_named(name: &str) -> Effect {
    use serde::de::IntoDeserializer;
    let known: Result<Effect, serde::de::value::Error> = Effect::deserialize(name.into_deserializer());
    known.unwrap_or(Effect::Static)
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
            speed: NORMAL_SPEED,
            per_key: false,
            keys: KeyColors::new(),
            last_seen: 0,
            hotkey: None,
            switched_off: false,
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

    /// Every key of a layout (`keymap::Layout::codes`) with its color (off keys included), ready to send.
    pub fn all_keys(&self, codes: &[u8]) -> Vec<(u8, Rgb)> {
        codes.iter().map(|&c| (c, self.key_color(c))).collect()
    }

    /// Codes of the keys stored, whatever the layout.
    fn stored_codes(&self) -> impl Iterator<Item = u8> + '_ {
        self.keys.keys().filter_map(|k| k.parse().ok())
    }

    /// Only the lighting (what presets save), without name and bookkeeping.
    fn lighting(&self) -> DeviceConfig {
        DeviceConfig { name: String::new(), last_seen: 0, hotkey: None, switched_off: false, ..self.clone() }
    }

    /// Takes over a preset's lighting; whether the hotkey switched the device off stays.
    fn set_lighting(&mut self, from: &DeviceConfig) {
        *self = DeviceConfig {
            name: std::mem::take(&mut self.name),
            last_seen: self.last_seen,
            hotkey: self.hotkey,
            switched_off: self.switched_off,
            ..from.lighting()
        };
    }

    /// The device hotkey: switches the device off, or back on as it was. A device set to "Off"
    /// in the panel comes on steady.
    pub fn toggle(&mut self) {
        if !self.switched_off && self.effect == Effect::Off {
            self.effect = Effect::Static;
        } else {
            self.switched_off = !self.switched_off;
        }
    }

    /// What the device shows: per-key colors that are all the same look exactly like that
    /// color shown steadily (e.g. right after opening the key editor or "fill all"), so the
    /// panel and preset matching treat them as that.
    pub fn visible(&self) -> DeviceConfig {
        // The stored keys: the editor stores every key of the keyboard's layout (off ones as black).
        let colors: Vec<Rgb> = self.stored_codes().map(|c| self.key_color(c)).collect();
        match colors.first().copied().unwrap_or(Rgb(0, 0, 0)) {
            c if self.per_key && colors.iter().all(|&k| k == c) => {
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

    /// Breathes or cycles: the speed matters.
    pub fn animated(&self) -> bool {
        !self.per_key && self.effect.animated()
    }

    fn same_lighting(&self, other: &DeviceConfig) -> bool {
        // The speed matters only while the device breathes or cycles.
        let norm = |d: &DeviceConfig| {
            let d = d.visible().lighting();
            DeviceConfig { speed: if d.animated() { d.speed } else { NORMAL_SPEED }, ..d }
        };
        let (a, b) = (norm(self), norm(other));
        // Per-key colors matter only while they are shown; compared key by key, since an off
        // key may be stored as black or (older versions) left out.
        let keys_equal = !a.per_key && !b.per_key || a.stored_codes().chain(b.stored_codes()).all(|c| a.key_color(c) == b.key_color(c));
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

/// A preset in a file of its own, to share (`Config::preset_file`).
#[derive(Serialize, Deserialize)]
struct PresetFile {
    rgbtray_preset: Preset,
}

/// What a file chosen to load holds (`Config::file_kind`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FileKind {
    Preset,
    Settings,
    Other,
}

/// A shared preset that was added (`Config::import_preset`).
#[derive(Debug, PartialEq)]
pub struct ImportedPreset {
    pub name: String,
    /// Devices of the file that found one here, of `total`.
    pub matched: usize,
    pub total: usize,
    /// Models of the file not here (names).
    pub missing: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum ImportError {
    /// Not a preset file this version can read.
    Unreadable(String),
    /// None of its devices is here: the models it has.
    NoDevices(Vec<String>),
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
    /// The lighting dims with the display and goes off while the display is off.
    pub follow_display: bool,
    /// Look for a new version on GitHub once a day.
    pub check_updates: bool,
    /// A version the user chose not to install (`update::skip`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_version: Option<String>,
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
            follow_display: true,
            check_updates: true,
            skipped_version: None,
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

/// Writes through a temporary file and a rename, so a crash or power loss in the middle
/// leaves the old file, never a cut one.
fn write_atomically(path: &std::path::Path, text: &str) {
    let tmp = path.with_extension("toml.tmp");
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::write(path, text);
    }
}

/// Daily copies kept in `backups/`, and copies made before each new version.
const DAILY_COPIES: usize = 14;
const VERSION_COPIES: usize = 10;

/// "2026-10-11" of Unix seconds (UTC), for copy names that sort by date.
fn date(secs: u64) -> String {
    // Days to civil date (Howard Hinnant's algorithm).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02}")
}

/// Copies of settings that read well: the last one, today's, and (at load, `before_version`)
/// this version's first. Text that doesn't read as settings is never copied.
fn keep_copies(dir: &std::path::Path, text: &str, before_version: bool) {
    if Config::file_kind(text) != FileKind::Settings {
        return;
    }
    write_atomically(&dir.join("config.backup.toml"), text);
    let backups = dir.join("backups");
    if std::fs::create_dir_all(&backups).is_err() {
        return;
    }
    let daily = backups.join(format!("config-{}.toml", date(now())));
    if !daily.exists() {
        let _ = std::fs::write(&daily, text);
    }
    // Before a new version first saves anything: what the previous one left.
    let version = backups.join(format!("config-before-{}.toml", env!("CARGO_PKG_VERSION")));
    if before_version && !version.exists() {
        let _ = std::fs::write(&version, text);
    }
    prune(&backups, "config-before-", VERSION_COPIES);
    prune(&backups, "config-2", DAILY_COPIES);
}

/// Files in `dir` whose names start with `prefix`, newest first.
fn newest_first(dir: &std::path::Path, prefix: &str) -> Vec<std::path::PathBuf> {
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .collect()
        })
        .unwrap_or_default();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().map(|f| f.1).collect()
}

/// Keeps the newest `keep` copies whose names start with `prefix`.
fn prune(dir: &std::path::Path, prefix: &str, keep: usize) {
    for old in newest_first(dir, prefix).into_iter().skip(keep) {
        let _ = std::fs::remove_file(old);
    }
}

/// Every copy to restore from, newest first: the last good one, then those in `backups/`.
fn copies_newest_first(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::iter::once(dir.join("config.backup.toml")).chain(newest_first(&dir.join("backups"), "")).collect()
}

impl Config {
    /// Preset `i` as a file to share: its lighting, without the hotkey (it would clash with
    /// the receiver's own).
    pub fn preset_file(&self, i: usize) -> Option<String> {
        let preset = Preset { hotkey: None, ..self.presets.get(i)?.clone() };
        let body = toml::to_string_pretty(&PresetFile { rgbtray_preset: preset }).ok()?;
        Some(format!("# RGB Tray preset. To add it: tray icon → Load settings or a preset…\n# Пресет RGB Tray. Чтобы добавить: значок в трее → Загрузить настройки или пресет…\n\n{body}"))
    }

    /// Adds a shared preset (`preset_file`). Devices of the file that aren't here go to the
    /// devices of the same model here (another person's G915 → this G915); with more of a model
    /// here than in the file, the others get the first one's lighting too. Models not here are
    /// left out, and a preset none of whose devices are here isn't added. A name already
    /// taken gets a number.
    pub fn import_preset(&mut self, text: &str) -> Result<ImportedPreset, ImportError> {
        let file: PresetFile = toml::from_str(text).map_err(|e| ImportError::Unreadable(e.message().to_string()))?;
        let mut preset = file.rgbtray_preset;
        let total = preset.devices.len();
        // Devices that are here keep theirs first.
        let (mine, foreign): (Vec<_>, Vec<_>) = std::mem::take(&mut preset.devices).into_iter().partition(|(id, _)| self.devices.contains_key(id));
        let mut matched = mine.len();
        let mut devices: BTreeMap<String, DeviceConfig> = mine.into_iter().collect();
        // By the id's model prefix itself: `kind_of` takes an unknown model for the first one.
        let model = |id: &str| id.split('-').next().unwrap_or(id).to_string();
        let mut missing: Vec<String> = Vec::new();
        let mut models: Vec<String> = foreign.iter().map(|(id, _)| model(id)).collect();
        models.sort();
        models.dedup();
        for m in models {
            let theirs: Vec<&DeviceConfig> = foreign.iter().filter(|(id, _)| model(id) == m).map(|(_, d)| d).collect();
            let free: Vec<String> = self.devices.keys().filter(|k| model(k) == m && !devices.contains_key(*k)).cloned().collect();
            if free.is_empty() {
                let name = kinds().iter().find(|k| k.key == m).map_or(m.clone(), |k| k.name().to_string());
                missing.extend(std::iter::repeat_n(name, theirs.len()));
                continue;
            }
            matched += theirs.len().min(free.len());
            // In order; devices left over here take the first one's lighting.
            for (k, local) in free.into_iter().enumerate() {
                devices.insert(local, theirs.get(k).unwrap_or(&theirs[0]).lighting());
            }
        }
        missing.dedup();
        if devices.is_empty() {
            return Err(ImportError::NoDevices(missing));
        }
        preset.devices = devices;
        preset.hotkey = None;
        let base = preset.name.clone();
        let mut n = 2;
        while self.presets.iter().any(|p| p.name == preset.name) {
            preset.name = format!("{base} ({n})");
            n += 1;
        }
        let name = preset.name.clone();
        self.presets.push(preset);
        Ok(ImportedPreset { name, matched, total, missing })
    }

    /// What a file to load is: a shared preset, all settings, or neither. A settings file must
    /// have some settings in it: almost any TOML reads as a `Config` of defaults.
    pub fn file_kind(text: &str) -> FileKind {
        let Ok(table) = toml::from_str::<toml::Table>(text) else { return FileKind::Other };
        if table.contains_key("rgbtray_preset") {
            FileKind::Preset
        } else if ["devices", "presets", "brightness"].iter().any(|k| table.contains_key(*k)) && toml::from_str::<Config>(text).is_ok() {
            FileKind::Settings
        } else {
            FileKind::Other
        }
    }

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
        let animated = self.devices.values().any(|d| d.visible().animated());
        self.brightness == p.brightness
            && (self.speed == p.speed || !animated)
            && self.devices.iter().all(|(id, d)| p.devices.get(id).is_none_or(|pd| d.same_lighting(pd)))
    }

    /// Saving over the preset would change nothing: it is the current lighting and knows every
    /// device (a device added after it was saved can still be written into it).
    pub fn saved_in(&self, p: &Preset) -> bool {
        self.matches(p) && self.devices.keys().all(|id| p.devices.contains_key(id))
    }

    /// The color all these devices show, when they all show the same single color.
    pub fn uniform_color(&self, ids: &[String]) -> Option<Rgb> {
        let devices: Vec<DeviceConfig> = ids.iter().map(|id| self.device(id).visible()).collect();
        let first = devices.first()?;
        let single = |d: &DeviceConfig| d.effect.colored() && !d.per_key && d.zones.is_empty() && d.color == first.color;
        devices.iter().all(single).then(|| first.rgb())
    }

    /// What a device is actually driven at: the overall brightness times its own share.
    pub fn effective_brightness(&self, id: &str) -> u8 {
        let b = self.brightness as u32 * self.device(id).brightness.min(100) as u32 / 100;
        b.max(1) as u8
    }

    /// How many times faster than normal a device's breathing and spectrum run: the overall
    /// factor times its own. Not cut at the overall range, so a device's own slider works
    /// both ways even with the overall one at an end; the hardware sets the real limits.
    pub fn effective_speed(&self, id: &str) -> f32 {
        crate::devices::speed_factor(self.speed) * crate::devices::speed_factor(self.device(id).speed)
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
                if kind.layout.is_some() && (self.per_key || !self.keys.is_empty()) {
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
        kinds()
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
        let rank = |id: &String| kinds().iter().position(|k| k.key == kind_of(id).key).unwrap_or(usize::MAX);
        v.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| self.display_name(a).cmp(&self.display_name(b))));
        v
    }
}

impl Config {
    pub fn dir() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("rgbtray")
    }

    /// Reads the settings (see `load_from`).
    pub fn load() -> Config {
        Self::load_from(&Self::dir())
    }

    /// Reads `config.toml` in `dir`, so that settings are never lost:
    /// - a good file is copied to `config.backup.toml` (the last settings that worked) and to
    ///   `backups/` (one copy a day, the last 14, and one before each new version, the last 10);
    /// - a file that can't be read is kept as `config.unreadable-<time>.toml`, and the newest
    ///   copy that reads takes its place; only without any the app starts from defaults.
    fn load_from(dir: &std::path::Path) -> Config {
        let path = dir.join("config.toml");
        let Ok(text) = std::fs::read_to_string(&path) else { return Config::default() };
        match toml::from_str(&text) {
            Ok(cfg) => {
                keep_copies(dir, &text, true);
                cfg
            }
            Err(e) => {
                let aside = dir.join(format!("config.unreadable-{}.toml", now()));
                let _ = std::fs::write(&aside, &text);
                crate::log(&format!("config: can't read it ({}); kept as {}", e.message(), aside.display()));
                for copy in copies_newest_first(dir) {
                    let Ok(text) = std::fs::read_to_string(&copy) else { continue };
                    if let Ok(cfg) = toml::from_str::<Config>(&text) {
                        crate::log(&format!("config: restored from {}", copy.display()));
                        write_atomically(&path, &text);
                        return cfg;
                    }
                }
                Config::default()
            }
        }
    }

    pub fn save(&self) {
        self.save_to(&Self::dir());
    }

    /// Writes `config.toml` in `dir` and refreshes the copies of good settings (the dated one
    /// at most once a day), so they don't wait for the next start.
    fn save_to(&self, dir: &std::path::Path) {
        let _ = std::fs::create_dir_all(dir);
        if let Ok(s) = toml::to_string_pretty(self) {
            write_atomically(&dir.join("config.toml"), &s);
            keep_copies(dir, &s, false);
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
        let codes = crate::keymap::get(crate::keymap::G915_TKL).codes();
        for &c in codes {
            d.set_key(c, Rgb(0, 255, 0));
        }
        let v = d.visible();
        assert!(!v.per_key && v.effect == Effect::Static && v.rgb() == Rgb(0, 255, 0));

        for &c in codes {
            d.set_key(c, Rgb(0, 0, 0));
        }
        assert!(d.visible().effect == Effect::Off);

        // One key different: still per-key.
        d.set_key(codes[0], Rgb(1, 2, 3));
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
    fn device_hotkey_switches_off_and_back() {
        let mut d = device(Effect::Breathe, "#112233");
        d.toggle();
        assert!(d.switched_off && d.effect == Effect::Breathe);
        d.toggle();
        assert!(!d.switched_off && d.effect == Effect::Breathe);
        // A device set to "Off" in the panel comes on steady.
        let mut off = device(Effect::Off, "#112233");
        off.toggle();
        assert!(off.effect == Effect::Static && !off.switched_off);
    }

    #[test]
    fn device_hotkey_does_not_touch_presets() {
        let mut cfg = Config::default();
        let mut d = device(Effect::Cycle, "#112233");
        d.hotkey = Some(Hotkey { mods: 0x2, vk: 0x4C });
        cfg.devices.insert("tuya-A".into(), d);
        let p = cfg.snapshot("P".into());
        // Switched off by the hotkey: the preset still is the current lighting.
        cfg.devices.get_mut("tuya-A").unwrap().toggle();
        assert!(cfg.saved_in(&p));
        assert!(!cfg.snapshot("Q".into()).devices["tuya-A"].switched_off);
        assert!(p.devices["tuya-A"].hotkey.is_none());
        // Loading a preset keeps the hotkey and leaves the device off.
        cfg.load_preset(&p);
        assert!(cfg.devices["tuya-A"].hotkey.is_some() && cfg.devices["tuya-A"].switched_off);
        let back: Config = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
        assert!(back.devices["tuya-A"].switched_off && back.devices["tuya-A"].hotkey == Some(Hotkey { mods: 0x2, vk: 0x4C }));
    }

    #[test]
    fn preset_ignores_devices_it_does_not_know() {
        let mut cfg = Config::default();
        cfg.devices.insert("naga-A".into(), device(Effect::Static, "#FF0000"));
        let p = cfg.snapshot("P".into());
        cfg.devices.insert("dock-B".into(), device(Effect::Cycle, "#00FF00"));
        assert!(cfg.matches(&p));
        // It is still shown as the current preset, but the new device can be saved into it.
        assert!(!cfg.saved_in(&p));
        assert!(cfg.saved_in(&cfg.snapshot("P".into())));
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
    fn device_speed_shifts_the_overall_one() {
        let mut cfg = Config::default();
        cfg.devices.insert("naga-A".into(), DeviceConfig { speed: 75, ..DeviceConfig::default() });
        assert_eq!(cfg.effective_speed("naga-A"), 2.0);
        assert_eq!(cfg.effective_speed("naga-B"), 1.0);
        cfg.speed = 25;
        assert_eq!(cfg.effective_speed("naga-A"), 1.0);
        // With the overall speed at its fastest, the device's own still makes it faster or slower.
        cfg.speed = 100;
        assert_eq!(cfg.effective_speed("naga-A"), 8.0);
        cfg.devices.get_mut("naga-A").unwrap().speed = 25;
        assert_eq!(cfg.effective_speed("naga-A"), 2.0);
        cfg.devices.get_mut("naga-A").unwrap().speed = 75;
        // A device's own speed is part of its lighting, and matters only while it animates.
        let p = cfg.snapshot("P".into());
        cfg.devices.get_mut("naga-A").unwrap().speed = 20;
        assert!(cfg.matches(&p));
        cfg.devices.get_mut("naga-A").unwrap().effect = Effect::Cycle;
        let p = cfg.snapshot("P".into());
        cfg.devices.get_mut("naga-A").unwrap().speed = 60;
        assert!(!cfg.matches(&p));
        cfg.load_preset(&p);
        assert_eq!(cfg.device("naga-A").speed, 20);
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
        // Files from before the option follow the display.
        assert!(old.follow_display);
        assert!(old.check_updates);
    }

    /// A fresh empty folder for files of one test.
    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rgbtray-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn broken_settings_come_back_from_the_last_good_copy() {
        let dir = temp_dir("restore");
        let good = "[[presets]]\nname = \"Night\"\nbrightness = 10\n[presets.devices]\n";
        std::fs::write(dir.join("config.toml"), good).unwrap();
        // A good start keeps copies: the last good one, today's and this version's.
        assert_eq!(Config::load_from(&dir).presets[0].name, "Night");
        assert!(dir.join("config.backup.toml").exists());
        let copies: Vec<String> = std::fs::read_dir(dir.join("backups")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(copies.len(), 2, "{copies:?}");

        // Broken (cut in the middle): the presets come back, the broken file is kept aside.
        std::fs::write(dir.join("config.toml"), "[[presets]]\nname = \"Ni").unwrap();
        assert_eq!(Config::load_from(&dir).presets[0].name, "Night");
        assert_eq!(std::fs::read_to_string(dir.join("config.toml")).unwrap(), good);
        assert!(std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("config.unreadable-")));

        // Nothing to restore from: defaults, without a crash.
        let empty = temp_dir("nothing");
        std::fs::write(empty.join("config.toml"), "not toml [").unwrap();
        assert!(Config::load_from(&empty).presets.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn saving_refreshes_the_copies() {
        let dir = temp_dir("save");
        let mut cfg = Config::default();
        cfg.presets.push(cfg.snapshot("Day".into()));
        cfg.save_to(&dir);
        let backup = || toml::from_str::<Config>(&std::fs::read_to_string(dir.join("config.backup.toml")).unwrap()).unwrap();
        assert_eq!(backup().presets[0].name, "Day");
        // Today's copy is made once; a save is no new version.
        let daily = dir.join("backups").join(format!("config-{}.toml", date(now())));
        let first = std::fs::read_to_string(&daily).unwrap();
        cfg.presets[0].name = "Night".into();
        cfg.save_to(&dir);
        assert_eq!(backup().presets[0].name, "Night");
        assert_eq!(std::fs::read_to_string(&daily).unwrap(), first);
        assert_eq!(std::fs::read_dir(dir.join("backups")).unwrap().count(), 1);
        assert_eq!(Config::load_from(&dir).presets[0].name, "Night");

        // Text that isn't settings is never kept as a copy.
        keep_copies(&dir, "[package]\nname = \"x\"\n", true);
        keep_copies(&dir, "not toml [", true);
        assert_eq!(backup().presets[0].name, "Night");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_copies_are_pruned() {
        let dir = temp_dir("prune");
        for d in 1..=20 {
            std::fs::write(dir.join(format!("config-2026-01-{d:02}.toml")), "").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        prune(&dir, "config-2", DAILY_COPIES);
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left.len(), DAILY_COPIES);
        assert_eq!(left[0], "config-2026-01-07.toml");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shared_preset_goes_to_the_same_models() {
        // The sender: a G915, a Naga and a mouse the receiver doesn't have, a preset with a hotkey.
        let mut theirs = Config::default();
        for id in ["logitech-AAAA", "naga-BBBB", "basiliskv3-CCCC"] {
            theirs.devices.insert(id.into(), device(Effect::Static, "#000000"));
        }
        theirs.devices.get_mut("logitech-AAAA").unwrap().effect = Effect::Wave;
        theirs.devices.get_mut("naga-BBBB").unwrap().set_rgb(Rgb(255, 0, 0));
        theirs.presets.push(Preset { hotkey: Some(Hotkey { mods: 0, vk: 0x7C }), ..theirs.snapshot("Night".into()) });
        let file = theirs.preset_file(0).unwrap();
        assert!(!file.contains("hotkey"));
        assert_eq!(Config::file_kind(&file), FileKind::Preset);

        // The receiver: their own G915 and Naga, and already a preset called "Night".
        let mut mine = Config::default();
        mine.devices.insert("logitech-1111".into(), device(Effect::Static, "#FFFFFF"));
        mine.devices.insert("naga-2222".into(), device(Effect::Static, "#FFFFFF"));
        mine.presets.push(mine.snapshot("Night".into()));
        let added = mine.import_preset(&file).unwrap();
        assert_eq!(added.name, "Night (2)");
        assert_eq!((added.matched, added.total), (2, 3));
        assert_eq!(added.missing, ["Razer Basilisk V3"]);
        let p = &mine.presets[1];
        assert!(p.hotkey.is_none());
        let ids: Vec<&String> = p.devices.keys().collect();
        assert_eq!(ids, ["logitech-1111", "naga-2222"]);
        assert_eq!(p.devices["logitech-1111"].effect, Effect::Wave);
        assert_eq!(p.devices["naga-2222"].rgb(), Rgb(255, 0, 0));
        mine.load_preset(&mine.presets[1].clone());
        assert_eq!(mine.devices["naga-2222"].rgb(), Rgb(255, 0, 0));
    }

    #[test]
    fn shared_preset_lights_every_device_of_a_model() {
        let mut theirs = Config::default();
        theirs.devices.insert("logitech-AAAA".into(), device(Effect::Breathe, "#00FF00"));
        theirs.presets.push(theirs.snapshot("Green".into()));
        let file = theirs.preset_file(0).unwrap();

        // Two G915s here, one in the file: both get its lighting.
        let mut mine = Config::default();
        for id in ["logitech-1111", "logitech-2222"] {
            mine.devices.insert(id.into(), device(Effect::Static, "#FFFFFF"));
        }
        let added = mine.import_preset(&file).unwrap();
        assert_eq!((added.matched, added.total), (1, 1));
        assert!(added.missing.is_empty());
        for id in ["logitech-1111", "logitech-2222"] {
            assert_eq!(mine.presets[0].devices[id].effect, Effect::Breathe);
            assert_eq!(mine.presets[0].devices[id].rgb(), Rgb(0, 255, 0));
        }
    }

    #[test]
    fn shared_preset_for_other_devices_is_not_added() {
        let mut theirs = Config::default();
        theirs.devices.insert("basiliskv3-CCCC".into(), device(Effect::Static, "#FF0000"));
        theirs.presets.push(theirs.snapshot("Red".into()));
        let file = theirs.preset_file(0).unwrap();

        let mut mine = Config::default();
        mine.devices.insert("logitech-1111".into(), device(Effect::Static, "#FFFFFF"));
        assert_eq!(mine.import_preset(&file), Err(ImportError::NoDevices(vec!["Razer Basilisk V3".into()])));
        assert!(mine.presets.is_empty());
        assert!(matches!(mine.import_preset("not toml ["), Err(ImportError::Unreadable(_))));
    }

    #[test]
    fn only_real_settings_replace_the_settings() {
        assert_eq!(Config::file_kind(&toml::to_string(&Config::default()).unwrap()), FileKind::Settings);
        // Any other TOML would read as a Config of defaults and wipe everything.
        assert_eq!(Config::file_kind("[package]\nname = \"x\"\n"), FileKind::Other);
        assert_eq!(Config::file_kind("not toml ["), FileKind::Other);
        assert_eq!(Config::file_kind(""), FileKind::Other);
    }

    #[test]
    fn dates_of_copies() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_791_720_000), "2026-10-11");
        assert_eq!(date(951_782_400), "2000-02-29");
    }

    #[test]
    fn unknown_effect_keeps_the_rest_of_the_file() {
        // An effect of another version, on a device and in a preset: the presets survive.
        let text = "[devices.logitech-A]\neffect = \"experimental\"\ncolor = \"#FF0000\"\n\n\
                    [[presets]]\nname = \"Night\"\nbrightness = 10\n[presets.devices.logitech-A]\neffect = \"sparkle\"\ncolor = \"#0000FF\"\n";
        let cfg: Config = toml::from_str(text).unwrap();
        assert_eq!(cfg.devices["logitech-A"].effect, Effect::Static);
        assert_eq!(cfg.devices["logitech-A"].rgb(), Rgb(255, 0, 0));
        assert_eq!(cfg.presets[0].name, "Night");
        assert_eq!(cfg.presets[0].devices["logitech-A"].effect, Effect::Static);
        // Known ones read as they are.
        let cfg: Config = toml::from_str("[devices.a]\neffect = \"ripple\"\ncolor = \"#000000\"\n").unwrap();
        assert_eq!(cfg.devices["a"].effect, Effect::Ripple);
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
