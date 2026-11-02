//! Vendor programs that drive the same devices' lighting and so fight with this app.
//!
//! Only programs that actually send colors to devices are listed — not every process of a
//! vendor. ASUS system components (Armoury Crate Control Interface, AsusHotkey, ASUS
//! Optimization...) handle Fn keys, fans and power and are left alone; Armoury Crate's
//! lighting lives in its separate LightingService.

use crate::i18n::Text;

/// A program known to drive lighting.
pub struct App {
    /// Stable key, stored in the config when the user hides the warning.
    pub key: &'static str,
    pub name: &'static str,
    /// Executable names (any case) of the parts that write to devices.
    pub processes: &'static [&'static str],
    /// Devices it fights over.
    devices: Devices,
    /// What to do about it, short enough for one line of a warning card.
    pub remedy: Text,
}

/// Which devices a program drives.
enum Devices {
    Any,
    /// These device kinds (`Kind::key`).
    Kinds(&'static [&'static str]),
    /// Every model of the Razer protocol, whichever profile it comes from.
    Razer,
}

impl App {
    /// The program drives devices of this model.
    fn drives(&self, kind: &crate::devices::Kind) -> bool {
        match self.devices {
            Devices::Any => true,
            Devices::Kinds(keys) => keys.contains(&kind.key),
            Devices::Razer => matches!(kind.protocol, crate::devices::Protocol::Razer(_)),
        }
    }
}

pub const APPS: &[App] = &[
    App {
        key: "ghub",
        name: "Logitech G HUB",
        processes: &["lghub_agent.exe", "lghub.exe", "lghub_system_tray.exe"],
        devices: Devices::Kinds(&["logitech"]),
        remedy: ("Закройте G HUB в трее", "Quit G HUB from the tray"),
    },
    App {
        key: "synapse",
        name: "Razer Synapse",
        processes: &["Razer Synapse Service.exe", "Razer Synapse 3.exe", "Razer Synapse.exe", "RazerAppEngine.exe"],
        devices: Devices::Razer,
        remedy: ("Закройте Synapse в трее", "Quit Synapse from the tray"),
    },
    App {
        key: "aura",
        name: "ASUS Aura (Armoury Crate)",
        processes: &["LightingService.exe"],
        devices: Devices::Kinds(&["asus"]),
        remedy: ("Остановите службу LightingService", "Stop the LightingService service"),
    },
    App {
        key: "signalrgb",
        name: "SignalRGB",
        processes: &["SignalRgb.exe", "SignalRgbLauncher.exe"],
        devices: Devices::Any,
        remedy: ("Закройте SignalRGB в трее", "Quit SignalRGB from the tray"),
    },
    App { key: "openrgb", name: "OpenRGB", processes: &["OpenRGB.exe"], devices: Devices::Any, remedy: ("Закройте OpenRGB", "Quit OpenRGB") },
];

/// A running program and the devices it fights over.
#[derive(Clone)]
pub struct Conflict {
    pub app: &'static App,
    /// Ids of the affected devices, in the order given.
    pub devices: Vec<String>,
}

/// Programs among `running` (executable names) that fight over any of the `devices` (ids),
/// except the ones the user chose to ignore.
pub fn find(running: &[String], devices: &[String], ignored: &[String]) -> Vec<Conflict> {
    APPS.iter()
        .filter(|app| !ignored.iter().any(|k| k == app.key))
        .filter(|app| app.processes.iter().any(|p| running.iter().any(|r| r.eq_ignore_ascii_case(p))))
        .filter_map(|app| {
            let affected: Vec<String> = devices.iter().filter(|id| app.drives(crate::devices::kind_of(id))).cloned().collect();
            (!affected.is_empty()).then_some(Conflict { app, devices: affected })
        })
        .collect()
}

pub fn by_key(key: &str) -> Option<&'static App> {
    APPS.iter().find(|a| a.key == key)
}

/// Executable names of all running processes.
pub fn running() -> Vec<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.push(String::from_utf16_lossy(&e.szExeFile[..len]));
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn keys(c: &[Conflict]) -> Vec<&str> {
        c.iter().map(|c| c.app.key).collect()
    }

    const MINE: &[&str] = &["logitech-1A2B3C4D", "dock-AB12", "naga-1234", "asus", "macropad-9F3E"];

    #[test]
    fn asus_system_components_are_not_a_conflict() {
        let running = s(&["ArmouryCrateControlInterface.exe", "ArmouryCrateKeyControl.exe", "AsusHotkey.exe", "AsusOptimization.exe", "AsusAppService.exe"]);
        assert!(find(&running, &s(MINE), &[]).is_empty());
    }

    #[test]
    fn a_program_matters_only_with_its_devices() {
        let running = s(&["razer synapse service.exe"]);
        let c = find(&running, &s(MINE), &[]);
        assert_eq!(keys(&c), ["synapse"]);
        assert_eq!(c[0].devices, s(&["dock-AB12", "naga-1234"]));
        assert!(find(&running, &s(&["logitech-1A2B3C4D", "asus"]), &[]).is_empty());
    }

    #[test]
    fn synapse_fights_over_every_razer_model() {
        let running = s(&["Razer Synapse 3.exe"]);
        let c = find(&running, &s(&["nagax-1234", "nagalefthanded-5678", "logitech-1A2B3C4D", "macropad-9F3E"]), &[]);
        assert_eq!(keys(&c), ["synapse"]);
        assert_eq!(c[0].devices, s(&["nagax-1234", "nagalefthanded-5678"]));
    }

    #[test]
    fn programs_for_any_device() {
        let c = find(&s(&["SignalRgb.exe", "OpenRGB.exe"]), &s(&["via-1234"]), &[]);
        assert_eq!(keys(&c), ["signalrgb", "openrgb"]);
        assert!(find(&s(&["SignalRgb.exe"]), &[], &[]).is_empty());
    }

    #[test]
    fn ignored_programs_are_skipped() {
        let running = s(&["lghub_agent.exe", "LightingService.exe"]);
        assert_eq!(keys(&find(&running, &s(MINE), &[])), ["ghub", "aura"]);
        assert_eq!(keys(&find(&running, &s(MINE), &s(&["ghub"]))), ["aura"]);
    }

    #[test]
    fn keys_are_unique() {
        for (i, a) in APPS.iter().enumerate() {
            assert!(APPS[i + 1..].iter().all(|b| b.key != a.key));
            assert!(by_key(a.key).is_some());
        }
    }
}
