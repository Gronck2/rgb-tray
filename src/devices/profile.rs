//! Device profiles: a model of a known protocol described in TOML instead of code. The
//! built-in ones are in `profiles/` (compiled in); users add or replace models with files in
//! `%APPDATA%\rgbtray\devices\*.toml`. A profile becomes a `Kind`, like a model written in Rust.

use super::razer::RazerModel;
use super::{Class, Effect, Kind, Protocol, Speed};
use serde::Deserialize;

/// Built-in profiles, in panel order: (file name, contents).
const BUILT_IN: &[(&str, &str)] = &[
    ("razer-naga-v2-pro.toml", include_str!("../../profiles/razer-naga-v2-pro.toml")),
    ("razer-mouse-dock-pro.toml", include_str!("../../profiles/razer-mouse-dock-pro.toml")),
    ("razer-mice.toml", include_str!("../../profiles/razer-mice.toml")),
    ("razer-keyboards.toml", include_str!("../../profiles/razer-keyboards.toml")),
    ("razer-accessories.toml", include_str!("../../profiles/razer-accessories.toml")),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    /// Prefix of device ids ("naga" → "naga-<serial>"); settings are kept under it.
    key: String,
    /// Name in English, and in Russian if it differs.
    name: String,
    #[serde(default)]
    name_ru: Option<String>,
    class: ClassName,
    /// keyboard, mouse, dock, laptop, macropad, light; by default from the class.
    #[serde(default)]
    icon: Option<String>,
    /// Effects the device can show; all by default.
    #[serde(default)]
    effects: Option<Vec<Effect>>,
    #[serde(default)]
    zones: Vec<Zone>,
    /// The protocol and its parameters: exactly one section.
    #[serde(default)]
    razer: Option<RazerSection>,
    /// Someone checked the model on real hardware (the card says "untested" otherwise).
    #[serde(default)]
    tested: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ClassName {
    Keyboard,
    Mouse,
    Other,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Zone {
    led: u8,
    name: String,
    #[serde(default)]
    name_ru: Option<String>,
}

/// The 90-byte Razer report (as documented by OpenRazer).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RazerSection {
    /// USB product ids (vendor 1532), e.g. wireless and wired.
    pids: Vec<u16>,
    transaction_id: u8,
    /// LED ids that make up "the whole device".
    leds: Vec<u8>,
    #[serde(default)]
    battery: bool,
}

/// Profiles live as long as the app: their data is leaked into `'static`, once, at start.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn leak_slice<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
}

fn icon(name: &str) -> Option<&'static str> {
    Some(match name {
        "keyboard" => "\u{E765}",
        "mouse" => "\u{E962}",
        "dock" => "\u{E772}",
        "laptop" => "\u{E7F8}",
        "macropad" => "\u{E92E}",
        "light" => "\u{E82F}",
        _ => return None,
    })
}

/// Reads one profile into a model.
fn parse(text: &str) -> Result<Kind, String> {
    from_profile(toml::from_str(text).map_err(|e| e.message().to_string())?)
}

/// All models of a file: one profile, or several as `[[device]]` sections. A broken one
/// doesn't keep the others out.
fn parse_file(text: &str) -> Vec<Result<Kind, String>> {
    let table: toml::Table = match toml::from_str(text) {
        Ok(t) => t,
        Err(e) => return vec![Err(e.message().to_string())],
    };
    match table.get("device") {
        Some(toml::Value::Array(devices)) if table.len() == 1 => devices
            .iter()
            .enumerate()
            .map(|(n, d)| {
                let key = d.get("key").and_then(|k| k.as_str()).unwrap_or("?").to_string();
                d.clone()
                    .try_into()
                    .map_err(|e: toml::de::Error| e.message().to_string())
                    .and_then(from_profile)
                    .map_err(|e| format!("device {} ({key}): {e}", n + 1))
            })
            .collect(),
        _ => vec![parse(text)],
    }
}

fn from_profile(p: Profile) -> Result<Kind, String> {
    if p.key.is_empty() || !p.key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return Err(format!("key \"{}\": only small latin letters and digits", p.key));
    }
    let class = match p.class {
        ClassName::Keyboard => Class::Keyboard,
        ClassName::Mouse => Class::Mouse,
        ClassName::Other => Class::Other,
    };
    let icon = match &p.icon {
        Some(name) => icon(name).ok_or_else(|| format!("unknown icon \"{name}\""))?,
        None => match class {
            Class::Keyboard => "\u{E765}",
            Class::Mouse => "\u{E962}",
            Class::Other => "\u{E82F}",
        },
    };
    let effects = p.effects.unwrap_or_else(|| Effect::ALL.to_vec());
    if !effects.contains(&Effect::Off) {
        return Err("effects must include \"off\"".into());
    }
    let (protocol, speed) = match p.razer {
        Some(r) => {
            if r.pids.is_empty() || r.leds.is_empty() {
                return Err("[razer]: pids and leds must not be empty".into());
            }
            if let Some(z) = p.zones.iter().find(|z| !r.leds.contains(&z.led)) {
                return Err(format!("zone {:#04x} is not one of [razer] leds", z.led));
            }
            let model = RazerModel { pids: leak_slice(r.pids), transaction_id: r.transaction_id, leds: leak_slice(r.leds), battery: r.battery };
            // The Razer report has no effect speed.
            (Protocol::Razer(Box::leak(Box::new(model))), Speed::None)
        }
        None => return Err("no protocol section (e.g. [razer])".into()),
    };
    let name = leak(p.name);
    let zones = p
        .zones
        .into_iter()
        .map(|z| {
            let en = leak(z.name);
            (z.led, (z.name_ru.map_or(en, leak), en))
        })
        .collect();
    Ok(Kind {
        key: leak(p.key),
        names: (p.name_ru.map_or(name, leak), name),
        icon,
        zones: leak_slice(zones),
        layout: None,
        // The protocols profiles describe report a serial number.
        port_id: false,
        effects: leak_slice(effects),
        speed,
        class,
        protocol,
        tested: p.tested,
    })
}

/// Profile models: built-in ones in order, a user file with the same key replacing its
/// built-in one, other user files after them (by file name). Problems: (file, message).
pub fn load() -> (Vec<Kind>, Vec<(String, String)>) {
    let mut kinds: Vec<Kind> = Vec::new();
    let mut errors = Vec::new();
    let add = |file: &str, text: &str, kinds: &mut Vec<Kind>, errors: &mut Vec<(String, String)>| {
        for res in parse_file(text) {
            match res {
                Ok(k) => match kinds.iter().position(|o| o.key == k.key) {
                    Some(i) => kinds[i] = k,
                    None => kinds.push(k),
                },
                Err(e) => errors.push((file.to_string(), e)),
            }
        }
    };
    for (file, text) in BUILT_IN {
        add(file, text, &mut kinds, &mut errors);
    }
    // Tests see only the built-in profiles.
    if !cfg!(test) {
        let mut files: Vec<_> = std::fs::read_dir(user_dir()).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        files.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("toml")));
        files.sort();
        for path in files {
            let file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            match std::fs::read_to_string(&path) {
                Ok(text) => add(&format!("devices\\{file}"), &text, &mut kinds, &mut errors),
                Err(e) => errors.push((file, e.to_string())),
            }
        }
    }
    (kinds, errors)
}

/// Where users put their own profiles.
pub fn user_dir() -> std::path::PathBuf {
    crate::config::Config::dir().join("devices")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_profiles_load() {
        let (kinds, errors) = load();
        assert!(errors.is_empty(), "{errors:?}");
        let keys: Vec<&str> = kinds.iter().map(|k| k.key).collect();
        assert_eq!(keys[..2], ["naga", "dock"]);
        // Every key once, every PID in one model only.
        for (i, k) in kinds.iter().enumerate() {
            assert!(kinds[i + 1..].iter().all(|o| o.key != k.key), "{}", k.key);
            let Protocol::Razer(m) = k.protocol else { continue };
            for o in &kinds[i + 1..] {
                if let Protocol::Razer(om) = o.protocol {
                    assert!(m.pids.iter().all(|p| !om.pids.contains(p)), "{} and {} share a PID", k.key, o.key);
                }
            }
        }
        assert!(kinds[0].tested && kinds[1].tested);
        let naga = &kinds[0];
        assert_eq!(naga.names, ("Razer Naga V2 Pro", "Razer Naga V2 Pro"));
        assert_eq!(naga.zones, &[(0x04, ("Логотип", "Logo")), (0x05, ("Боковая панель", "Side panel"))]);
        assert_eq!(naga.class, Class::Mouse);
        assert_eq!(naga.speed, Speed::None);
        let Protocol::Razer(m) = naga.protocol else { panic!("naga is not razer") };
        assert_eq!((m.pids, m.transaction_id, m.leds, m.battery), (&[0x00A8, 0x00A7][..], 0x1F, &[0x01, 0x04, 0x05][..], true));
        let Protocol::Razer(d) = kinds[1].protocol else { panic!("dock is not razer") };
        assert_eq!((d.pids, d.transaction_id, d.leds, d.battery), (&[0x00A4][..], 0xFF, &[0x00][..], false));
        assert_eq!(kinds[1].icon, "\u{E772}");
    }

    #[test]
    fn several_models_in_one_file() {
        let text = "[[device]]\nkey = \"a\"\nname = \"A\"\nclass = \"mouse\"\n[device.razer]\npids = [1]\ntransaction_id = 0x1F\nleds = [4]\n\n\
                    [[device]]\nkey = \"b\"\nname = \"B\"\nclass = \"toaster\"\n[device.razer]\npids = [2]\ntransaction_id = 0x1F\nleds = [4]\n\n\
                    [[device]]\nkey = \"c\"\nname = \"C\"\nclass = \"other\"\n[device.razer]\npids = [3]\ntransaction_id = 0x3F\nleds = [0]\n";
        let all = parse_file(text);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].as_ref().unwrap().key, "a");
        assert!(all[1].as_ref().unwrap_err().starts_with("device 2 (b): "), "{:?}", all[1].as_ref().err());
        assert_eq!(all[2].as_ref().unwrap().key, "c");
        // A single profile is one model.
        assert_eq!(parse_file(&text.replace("[[device]]", "").split("\n\n").next().unwrap().replace("[device.razer]", "[razer]")).len(), 1);
    }

    #[test]
    fn every_profile_file_is_built_in() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles");
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(BUILT_IN.iter().any(|(f, _)| *f == name), "{name} is not in BUILT_IN");
        }
    }

    #[test]
    fn bad_profiles_say_why() {
        let ok = "key = \"basilisk\"\nname = \"Razer Basilisk\"\nclass = \"mouse\"\n[razer]\npids = [0x0086]\ntransaction_id = 0x1F\nleds = [0x01, 0x04]\n";
        let k = parse(ok).unwrap();
        assert_eq!(k.effects, &Effect::ALL[..]);
        assert_eq!(k.icon, "\u{E962}");
        let err = |text: &str| parse(text).unwrap_err();
        assert!(err(&ok.replace("basilisk", "bad-key")).contains("key"));
        assert!(err(&ok.replace("[razer]", "[razr]")).contains("razr"));
        assert!(err(&ok.replace("class = \"mouse\"", "class = \"toaster\"")).contains("toaster"));
        assert!(err(&ok.replace("leds = [0x01, 0x04]", "leds = []")).contains("leds"));
        assert!(err(&format!("effects = [\"static\"]\n{ok}")).contains("off"));
        assert!(err(&format!("{ok}\n[[zones]]\nled = 0x09\nname = \"Wheel\"\n")).contains("zone"));
        assert!(err("key = \"x\"\nname = \"X\"\nclass = \"other\"\n").contains("protocol"));
    }
}
