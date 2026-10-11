#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod alert;
mod config;
mod conflicts;
mod devices;
mod editor;
mod hid;
mod installer;
mod keymap;
mod services;
mod traymenu;
mod ui;
mod update;

use std::io::Write as _;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use config::Config;
use devices::{Effect, Rgb};

const WM_TRAY: u32 = WM_APP + 1;
/// Posted by background threads: re-apply lighting soon (debounced).
const WM_REAPPLY_SOON: u32 = WM_APP + 2;
/// Posted after an apply finished: refresh icon, tooltip and panel.
const WM_APPLIED: u32 = WM_APP + 3;
/// Posted by the battery poller with fresh levels.
const WM_BATTERY: u32 = WM_APP + 4;

const TIMER_REAPPLY: usize = 1;
/// Second pass after wake: the ASUS keyboard may reset to white a few seconds later.
const TIMER_REAPPLY_LATE: usize = 2;
/// First pass right after wake, to cut the white flash of the ASUS keyboard short.
const TIMER_REAPPLY_EARLY: usize = 3;

/// GUID_CONSOLE_DISPLAY_STATE: 0 off, 1 on, 2 dimmed. Modern Standby (S0ix) laptops
/// send no resume broadcast, so the display turning back on is the wake signal.
const GUID_CONSOLE_DISPLAY_STATE: windows_sys::core::GUID = windows_sys::core::GUID::from_u128(0x6fe69556_704a_47a0_8f24_c28d936fda47);
const DISPLAY_OFF: u8 = 0;
const DISPLAY_ON: u8 = 1;
const DISPLAY_DIMMED: u8 = 2;
static DISPLAY: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(DISPLAY_ON);

/// Brightness of the lighting while the display is dimmed (`Config::follow_display`), percent of what it is set to.
const DIMMED_BRIGHTNESS: u8 = 30;

fn schedule_wake_reapply(hwnd: HWND) {
    unsafe {
        SetTimer(hwnd, TIMER_REAPPLY_EARLY, 500, None);
        SetTimer(hwnd, TIMER_REAPPLY, 3000, None);
        SetTimer(hwnd, TIMER_REAPPLY_LATE, 10000, None);
    }
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APP_NAME: &str = "rgbtray";

static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();
/// Ids of devices found (and answering) at the last apply.
static PRESENT: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Battery levels of wireless devices from the last check.
static STATUS: Mutex<Vec<(String, devices::Battery)>> = Mutex::new(Vec::new());
/// Devices whose receiver/dock is plugged in but which don't answer (off, asleep, out of range).
/// Their STATUS entry is the last known battery level.
static NO_LINK: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Logitech's Dynamic Lighting service is active and can hang the G915 (see `services`).
static LAMPARRAY_WARN: AtomicBool = AtomicBool::new(false);

/// Windows Dynamic Lighting ("Use Dynamic Lighting on my devices") is on and drives the
/// LampArray devices itself, fighting with this app.
static DYNAMIC_LIGHTING_WARN: AtomicBool = AtomicBool::new(false);

/// Vendor programs running now that drive the lighting of devices on the panel.
static CONFLICTS: Mutex<Vec<conflicts::Conflict>> = Mutex::new(Vec::new());

fn check_services(cfg: &Config) {
    let has_logitech = cfg.devices.keys().any(|id| devices::kind_of(id).key == "logitech");
    LAMPARRAY_WARN.store(has_logitech && services::interferes(services::LOGI_LAMPARRAY), Ordering::Relaxed);
    DYNAMIC_LIGHTING_WARN.store(dynamic_lighting_on(), Ordering::Relaxed);
    // Not `active_ids()`: callers may hold the config lock.
    let mut active = PRESENT.lock().unwrap().clone();
    active.extend(NO_LINK.lock().unwrap().iter().cloned());
    *CONFLICTS.lock().unwrap() = conflicts::find(&conflicts::running(), &active, &cfg.ignored_apps);
}

/// Hides the warning about a program for good ("I keep it running on purpose").
fn ignore_app(key: &str) {
    let cfg = {
        let mut cfg = config();
        cfg.ignored_apps.push(key.to_string());
        cfg.save();
        cfg.clone()
    };
    check_services(&cfg);
}

/// The Dynamic Lighting switch in Windows Settings (Personalization → Dynamic Lighting).
fn dynamic_lighting_on() -> bool {
    unsafe {
        let mut v = 0u32;
        let mut size = 4u32;
        let key = wide("Software\\Microsoft\\Lighting");
        let name = wide("AmbientLightingEnabled");
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut v as *mut u32 as _, &mut size) == ERROR_SUCCESS
            && v != 0
    }
}

/// Opens Windows Settings at Dynamic Lighting.
/// Opens a web page in the default browser.
fn open_url(url: &str) {
    unsafe {
        ShellExecuteW(null_mut(), wide("open").as_ptr(), wide(url).as_ptr(), null(), null(), SW_SHOWNORMAL);
    }
}

fn open_dynamic_lighting_settings() {
    unsafe {
        ShellExecuteW(null_mut(), wide("open").as_ptr(), wide("ms-settings:personalization-lighting").as_ptr(), null(), null(), SW_SHOWNORMAL);
    }
}

/// Turns the Logitech LampArray service off (asks for administrator rights), then refreshes
/// the panel once it has stopped.
fn disable_lamparray() {
    if !services::disable(services::LOGI_LAMPARRAY) {
        return;
    }
    std::thread::spawn(|| {
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if !services::interferes(services::LOGI_LAMPARRAY) {
                break;
            }
        }
        check_services(&config().clone());
        post(WM_APPLIED);
    });
}

/// Serializes applies coming from different threads.
static APPLY_LOCK: Mutex<()> = Mutex::new(());
static HWND_MAIN: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicIsize = AtomicIsize::new(0);

fn config() -> std::sync::MutexGuard<'static, Config> {
    CONFIG.get_or_init(|| Mutex::new(Config::load())).lock().unwrap()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn log(msg: &str) {
    #[cfg(debug_assertions)]
    eprintln!("{msg}");
    let path = Config::dir().join("rgbtray.log");
    let _ = std::fs::create_dir_all(Config::dir());
    // A full log becomes the previous one, so the lines before a problem survive.
    if std::fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(&path, Config::dir().join("rgbtray.old.log"));
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{}.{:03}] {msg}", now.as_secs(), now.subsec_millis());
    }
}

/// Logs every device command when a file named `trace` exists next to the config.
fn trace_enabled() -> bool {
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| Config::dir().join("trace").exists())
}

fn post(msg: u32) {
    unsafe { PostMessageW(HWND_MAIN.load(Ordering::Relaxed) as HWND, msg, 0, 0) };
}

/// Devices shown on the panel: connected ones and those without a link, in panel order.
fn active_ids() -> Vec<String> {
    let mut ids = PRESENT.lock().unwrap().clone();
    ids.extend(NO_LINK.lock().unwrap().iter().cloned());
    config().sorted(ids)
}

/// Applies the saved settings to every connected device on a background thread.
fn apply_all() {
    std::thread::spawn(|| {
        let _guard = APPLY_LOCK.lock().unwrap();
        let d = devices::discover();
        let ids: Vec<String> = d.found.iter().map(|f| f.id.clone()).collect();
        log(&format!("apply: {ids:?}"));
        let cfg = {
            let mut cfg = config();
            if cfg.register(&ids) {
                cfg.save();
            }
            cfg.clone()
        };
        check_services(&cfg);
        let mut no_link = d.no_link;
        // A wireless device without a link can't tell its serial: it is the most recently
        // seen absent device of its model.
        for kind in d.no_link_kinds {
            let guess = cfg
                .devices
                .iter()
                .filter(|(id, _)| devices::kind_of(id).key == kind && !ids.contains(id) && !no_link.contains(id))
                .max_by_key(|(_, dc)| dc.last_seen)
                .map(|(id, _)| id.clone());
            no_link.extend(guess);
        }
        let mut status = Vec::new();
        let mut present = Vec::new();
        // The lighting follows the display: dimmed with it, off while it is off (idle, locked or asleep).
        let display = if cfg.follow_display { DISPLAY.load(Ordering::Relaxed) } else { DISPLAY_ON };
        if display != DISPLAY_ON {
            log(if display == DISPLAY_OFF { "display off: lighting off" } else { "display dimmed: lighting dimmed" });
        }
        for f in &d.found {
            let dc = cfg.device(&f.id);
            let mut brightness = cfg.effective_brightness(&f.id);
            if display == DISPLAY_DIMMED {
                brightness = (brightness as u32 * DIMMED_BRIGHTNESS as u32 / 100).max(1) as u8;
            }
            // Devices are applied one at a time (APPLY_LOCK): the driver reads this device's speed.
            devices::set_speed(cfg.effective_speed(&f.id));
            let res = if display == DISPLAY_OFF {
                f.dev.apply(Effect::Off, dc.rgb(), brightness)
            } else if devices::caps(&f.id).per_key && dc.per_key {
                f.dev.apply_keys(&dc.all_keys(keymap::get(devices::caps(&f.id).layout).codes()), brightness)
            } else if !dc.zones.is_empty() && !devices::zones_of(&f.id).is_empty() {
                let colors: Vec<(u8, Rgb)> = devices::zones_of(&f.id).iter().map(|&(led, _)| (led, dc.zone_rgb(led))).collect();
                f.dev.apply_zones(dc.effect, &colors, brightness)
            } else {
                f.dev.apply(dc.effect, dc.rgb(), brightness)
            };
            match res {
                Err(e) if e == devices::NO_LINK => {
                    no_link.push(f.id.clone());
                    continue;
                }
                Err(e) => log(&format!("{}: {e}", f.id)),
                Ok(()) => {}
            }
            present.push(f.id.clone());
            if let Some(b) = f.dev.battery() {
                status.push((f.id.clone(), b));
            }
        }
        if !no_link.is_empty() {
            log(&format!("no link: {no_link:?}"));
        }
        // Unreachable devices keep their last known battery level.
        for (id, b) in STATUS.lock().unwrap().iter() {
            if no_link.contains(id) {
                status.push((id.clone(), *b));
            }
        }
        *PRESENT.lock().unwrap() = present;
        *STATUS.lock().unwrap() = status;
        *NO_LINK.lock().unwrap() = no_link;
        post(WM_APPLIED);
    });
}

// ---------------------------------------------------------------- actions shared by the panel

fn set_color(ids: &[String], c: Rgb) {
    let mut cfg = config();
    for id in ids {
        let dc = cfg.device_mut(id);
        dc.set_rgb(c);
        // One color for the whole device replaces per-zone and per-key colors.
        dc.zones.clear();
        dc.per_key = false;
        // Picking a color while the light is off or cycling means "show this color".
        if !dc.effect.colored() {
            dc.effect = Effect::Static;
        }
    }
    cfg.save();
    drop(cfg);
    apply_all();
}

/// Colors one lighting zone of a device (e.g. the side panel of the mouse).
fn set_zone_color(id: &str, led: u8, c: Rgb) {
    let mut cfg = config();
    let dc = cfg.device_mut(id);
    let leds: Vec<u8> = devices::zones_of(id).iter().map(|z| z.0).collect();
    dc.set_zone(led, c, &leds);
    if !dc.effect.colored() {
        dc.effect = Effect::Static;
    }
    cfg.save();
    drop(cfg);
    apply_all();
}

fn set_effect(ids: &[String], e: Effect) {
    let mut cfg = config();
    // Devices that can't show the effect keep theirs.
    for id in ids.iter().filter(|id| devices::caps(id).has_effect(e)) {
        let dc = cfg.device_mut(id);
        dc.effect = e;
        dc.per_key = false;
    }
    cfg.save();
    drop(cfg);
    apply_all();
}

/// `device`: None sets the overall speed of breathing and spectrum, Some the device's shift from it.
fn set_speed(device: Option<&str>, speed: u8) {
    let mut cfg = config();
    match device {
        None => cfg.speed = speed.min(100),
        Some(id) => cfg.device_mut(id).speed = speed.min(100),
    }
    cfg.save();
    drop(cfg);
    apply_all();
}

/// `device`: None sets the overall brightness, Some the device's share of it.
fn set_brightness(device: Option<&str>, pct: u8) {
    let mut cfg = config();
    match device {
        None => cfg.brightness = pct.min(100),
        Some(id) => cfg.device_mut(id).brightness = pct.min(100),
    }
    cfg.save();
    drop(cfg);
    apply_all();
}

/// Removes a disconnected device with its settings.
fn forget_device(id: &str) {
    let mut cfg = config();
    cfg.forget(id);
    cfg.save();
}

fn rename_device(id: &str, name: &str) {
    let mut cfg = config();
    let name = name.trim();
    let kind = devices::kind_of(id);
    let default = name == kind.names.0 || name == kind.names.1;
    cfg.device_mut(id).name = if default { String::new() } else { name.to_string() };
    cfg.save();
}
// ---------------------------------------------------------------- battery

const BATTERY_POLL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const SNOOZE: std::time::Duration = std::time::Duration::from_secs(30 * 60);
/// Above this level (or on charge) the warnings are re-armed.
const BATTERY_REARM: u8 = 15;

/// Per device: highest warning shown (1 = low, 2 = critical) and an optional snooze deadline.
static BATTERY_WARNED: Mutex<Vec<(String, u8, Option<std::time::Instant>)>> = Mutex::new(Vec::new());

/// Seconds since the last keyboard/mouse input in this session.
fn idle_seconds() -> u32 {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    unsafe {
        let mut li = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if GetLastInputInfo(&mut li) == 0 {
            return 0;
        }
        GetTickCount().wrapping_sub(li.dwTime) / 1000
    }
}

/// Re-reads battery levels every few minutes while the user is active. Asking a sleeping
/// wireless keyboard would wake it up and drain the battery, so idle periods are skipped.
fn spawn_battery_poller() {
    std::thread::spawn(|| loop {
        std::thread::sleep(BATTERY_POLL);
        if idle_seconds() > 120 {
            continue;
        }
        let found = {
            let _guard = APPLY_LOCK.lock().unwrap();
            devices::batteries()
        };
        // A device that had no link answers again: re-apply so it gets its lighting and status.
        let reconnected = found.iter().any(|(k, _)| NO_LINK.lock().unwrap().contains(k));
        if !found.is_empty() {
            let mut status = STATUS.lock().unwrap();
            for (id, b) in found {
                status.retain(|(k, _)| *k != id);
                status.push((id, b));
            }
        }
        post(if reconnected { WM_REAPPLY_SOON } else { WM_BATTERY });
    });
}

/// The receiver doesn't always announce a keyboard waking up, so while some device has no
/// link it is looked for every half minute; once it answers, everything is re-applied.
fn spawn_link_watch() {
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(30));
        let missing = NO_LINK.lock().unwrap().clone();
        if missing.is_empty() {
            continue;
        }
        let back = {
            let _guard = APPLY_LOCK.lock().unwrap();
            devices::answering()
        };
        if missing.iter().any(|id| back.contains(id)) {
            post(WM_REAPPLY_SOON);
        }
    });
}

/// Shows the low-battery alert when a device crosses 10% or 5%, once per threshold.
fn check_batteries() {
    let status = STATUS.lock().unwrap().clone();
    let no_link = NO_LINK.lock().unwrap().clone();
    let mut warned = BATTERY_WARNED.lock().unwrap();
    // A stale level of an unreachable device isn't news.
    for (id, b) in status.into_iter().filter(|(k, _)| !no_link.contains(k)) {
        let pos = warned.iter().position(|(k, _, _)| *k == id).unwrap_or_else(|| {
            warned.push((id.clone(), 0, None));
            warned.len() - 1
        });
        let entry = &mut warned[pos];
        if b.charging || b.percent > BATTERY_REARM {
            if entry.1 > 0 {
                alert::dismiss_for(&id);
            }
            *entry = (id, 0, None);
            continue;
        }
        let level = if b.percent <= devices::CRITICAL_BATTERY {
            2
        } else if b.percent <= devices::LOW_BATTERY {
            1
        } else {
            0
        };
        let snooze_over = entry.2.is_some_and(|until| std::time::Instant::now() >= until);
        if level > entry.1 || (level > 0 && snooze_over) {
            entry.1 = level.max(entry.1);
            entry.2 = None;
            let name = config().display_name(&id);
            alert::show(id, &name, b);
        }
    }
}

fn snooze_battery_alert(id: &str) {
    if let Some(entry) = BATTERY_WARNED.lock().unwrap().iter_mut().find(|e| e.0 == id) {
        entry.2 = Some(std::time::Instant::now() + SNOOZE);
    }
}

// ---------------------------------------------------------------- presets and hotkeys

const HOTKEY_BASE: i32 = 0x100;
const MAX_HOTKEYS: i32 = 64;

/// (Re)registers the global hotkeys of all presets.
fn register_hotkeys() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT};
    let hwnd = HWND_MAIN.load(Ordering::Relaxed) as HWND;
    unsafe {
        for id in 0..MAX_HOTKEYS {
            UnregisterHotKey(hwnd, HOTKEY_BASE + id);
        }
        let cfg = config();
        for (i, p) in cfg.presets.iter().enumerate().take(MAX_HOTKEYS as usize) {
            if let Some(h) = p.hotkey {
                if RegisterHotKey(hwnd, HOTKEY_BASE + i as i32, h.mods | MOD_NOREPEAT, h.vk) == 0 {
                    log(&format!("hotkey {} for \"{}\" is taken by another app", h.label(), p.name));
                }
            }
        }
    }
}

fn apply_preset(i: usize) {
    let mut cfg = config();
    let Some(p) = cfg.presets.get(i).cloned() else { return };
    cfg.load_preset(&p);
    cfg.save();
    drop(cfg);
    apply_all();
}

fn save_preset() {
    let mut cfg = config();
    let name = format!("{} {}", tr!("Пресет", "Preset"), cfg.presets.len() + 1);
    let p = cfg.snapshot(name);
    cfg.presets.push(p);
    cfg.save();
}

/// Replaces the preset's lighting with the current one; its name and hotkey stay.
fn overwrite_preset(i: usize) {
    let mut cfg = config();
    if let Some(old) = cfg.presets.get(i) {
        let mut p = cfg.snapshot(old.name.clone());
        p.hotkey = old.hotkey;
        cfg.presets[i] = p;
        cfg.save();
    }
}

fn delete_preset(i: usize) {
    let mut cfg = config();
    if i < cfg.presets.len() {
        cfg.presets.remove(i);
        cfg.save();
    }
    drop(cfg);
    register_hotkeys();
}

fn rename_preset(i: usize, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let mut cfg = config();
    if let Some(p) = cfg.presets.get_mut(i) {
        p.name = name.to_string();
    }
    cfg.save();
}

fn set_preset_hotkey(i: usize, hotkey: Option<config::Hotkey>) {
    let mut cfg = config();
    // A hotkey belongs to one preset only.
    if hotkey.is_some() {
        for p in cfg.presets.iter_mut() {
            if p.hotkey == hotkey {
                p.hotkey = None;
            }
        }
    }
    if let Some(p) = cfg.presets.get_mut(i) {
        p.hotkey = hotkey;
    }
    cfg.save();
    drop(cfg);
    register_hotkeys();
}

fn quit() {
    unsafe { DestroyWindow(HWND_MAIN.load(Ordering::Relaxed) as HWND) };
}

// ---------------------------------------------------------------- diagnostics

fn reg_string(key: &str, value: &str) -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let mut size = (buf.len() * 2) as u32;
        let ok = RegGetValueW(HKEY_LOCAL_MACHINE, wide(key).as_ptr(), wide(value).as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr() as _, &mut size)
            == ERROR_SUCCESS;
        let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
        if ok {
            String::from_utf16_lossy(&buf[..len])
        } else {
            String::new()
        }
    }
}

/// A text report for "please add my device" requests: app and Windows versions, supported
/// devices found, interfering services and every HID device with its interfaces, the ones
/// that can carry lighting first. Serial numbers are shortened: the report is meant for a
/// public issue.
fn diagnose_report() -> String {
    use std::fmt::Write as _;
    let nt = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
    let mut s = String::new();
    let _ = writeln!(s, "RGB Tray {} device report", installer::VERSION);
    let build = reg_string(nt, "CurrentBuild");
    let mut product = reg_string(nt, "ProductName");
    // The registry still says "Windows 10" on Windows 11; builds from 22000 on are 11.
    if build.parse::<u32>().is_ok_and(|b| b >= 22000) {
        product = product.replace("Windows 10", "Windows 11");
    }
    let _ = writeln!(s, "Windows: {product} {} (build {build})", reg_string(nt, "DisplayVersion"));
    let found = {
        let _guard = APPLY_LOCK.lock().unwrap();
        devices::discover()
    };
    let _ = writeln!(s, "\nDevices driven by RGB Tray:");
    for f in &found.found {
        let _ = writeln!(s, "  {} — {}", short_id(&f.id), devices::kind_of(&f.id).names.1);
    }
    for id in &found.no_link {
        let _ = writeln!(s, "  {} — no link", short_id(id));
    }
    for kind in &found.no_link_kinds {
        let _ = writeln!(s, "  {kind} — no link (serial unknown)");
    }
    if found.found.is_empty() && found.no_link.is_empty() && found.no_link_kinds.is_empty() {
        let _ = writeln!(s, "  none found");
    }
    let profiles = devices::kinds().iter().filter(|k| !matches!(k.protocol, devices::Protocol::Native)).count();
    let _ = writeln!(s, "Device profiles: {profiles} models (own profiles go in {})", devices::profile::user_dir().display());
    // Firmware effects of Logitech keyboards (0x8071 effect ids per zone): what more they could show.
    let logitech = {
        let _guard = APPLY_LOCK.lock().unwrap();
        devices::logitech::Logitech::find_all(devices::logitech::SW_ID_POLL)
    };
    for kb in &logitech {
        for (zone, ids) in kb.zones.iter().enumerate() {
            let ids: Vec<String> = ids.iter().map(|e| format!("{e:04X}")).collect();
            let _ = writeln!(s, "  {} zone {zone} effects: {}", short_id(&kb.id), ids.join(" "));
        }
    }
    for (file, e) in devices::PROFILE_ERRORS.lock().unwrap().iter() {
        let _ = writeln!(s, "  {file}: {e}");
    }
    let lamparray = if services::interferes(services::LOGI_LAMPARRAY) { "running (makes the G915 hang)" } else { "disabled or absent" };
    let _ = writeln!(s, "Logitech LampArray service: {lamparray}");

    // One entry per physical device: its interfaces share the USB ids and the connection.
    let mut hids = hid::enumerate_all();
    hids.sort_by_key(|h| (h.vid, h.pid, h.usage_page, h.usage));
    let mut groups: Vec<Vec<&hid::HidInfo>> = Vec::new();
    for h in &hids {
        match groups.iter_mut().find(|g| g[0].vid == h.vid && g[0].pid == h.pid && g[0].transport() == h.transport()) {
            Some(g) => g.push(h),
            None => groups.push(vec![h]),
        }
    }
    let rgb_vendor = |vid: u16| devices::RGB_VENDORS.iter().find(|v| v.0 == vid).map(|v| v.1);
    let notes = |g: &[&hid::HidInfo]| g.iter().filter_map(|h| devices::interface_note(h)).collect::<Vec<_>>();
    let supported = |g: &[&hid::HidInfo]| notes(g).iter().any(|n| n.starts_with("SUPPORTED"));
    let candidate = |g: &[&hid::HidInfo]| !notes(g).is_empty() || rgb_vendor(g[0].vid).is_some();

    let section = |g: &[&hid::HidInfo]| {
        if supported(g) {
            0
        } else if candidate(g) {
            1
        } else {
            2
        }
    };
    let titles = ["Supported devices", "Other devices with lighting or vendor interfaces", "Other HID devices"];
    for (k, title) in titles.into_iter().enumerate() {
        let list: Vec<&Vec<&hid::HidInfo>> = groups.iter().filter(|g| section(g) == k).collect();
        let _ = writeln!(s, "\n{title} ({}):", list.len());
        for g in list {
            let first = g[0];
            let maker = g.iter().map(|h| h.manufacturer()).find(|m| !m.is_empty()).unwrap_or_default();
            let name = g.iter().map(|h| h.product()).find(|p| !p.is_empty()).unwrap_or_default();
            let mut line = format!("{:04X}:{:04X}  {}", first.vid, first.pid, first.transport());
            for part in [maker.as_str(), name.as_str()].into_iter().filter(|p| !p.is_empty()) {
                let _ = write!(line, " — {part}");
            }
            if let Some(v) = rgb_vendor(first.vid) {
                let _ = write!(line, "  [RGB vendor: {v}]");
            }
            let _ = writeln!(s, "{line}");
            // Interface (MI_xx) and collection (COLxx) from the path, in that order.
            let part = |h: &hid::HidInfo, key: &str| {
                let path = String::from_utf16_lossy(&h.path).to_lowercase();
                path.find(key).map(|i| path[i..].chars().take(key.len() + 2).collect::<String>().to_uppercase()).unwrap_or_default()
            };
            let mut g = g.clone();
            g.sort_by_key(|h| (part(h, "mi_"), part(h, "col")));
            for h in g {
                let _ = write!(
                    s,
                    "    {:<6}{:<6} {:04X}:{:04X} {:<15} in {:>3}  out {:>3}  feature {:>3}",
                    part(h, "mi_"),
                    part(h, "col"),
                    h.usage_page,
                    h.usage,
                    devices::usage_name(h.usage_page, h.usage),
                    h.input_len,
                    h.output_len,
                    h.feature_len
                );
                if let Some(n) = devices::interface_note(h).filter(|n| *n != "vendor-specific") {
                    let _ = write!(s, "  <- {n}");
                }
                let _ = writeln!(s);
                if h.usage_page == 0x0059 && h.usage == 0x0001 {
                    let _ = writeln!(s, "        LampArray: {}", devices::lamparray::describe(h));
                }
            }
        }
    }
    s
}

/// `--logitech-effect-test`: plays firmware effect variants on the keys of the Logitech
/// keyboard, each after 2 s of dark, for someone watching it to say what each did: breathing
/// with its period in either byte order (does the speed change?) and the undocumented effect
/// 0x000C. Every step is logged with its time.
fn logitech_effect_test() {
    let Some(kb) = devices::logitech::Logitech::find_all(devices::logitech::SW_ID_POLL).into_iter().next() else {
        log("effect test: no Logitech keyboard answers");
        return;
    };
    const STATIC: u16 = 0x0001;
    const BREATHE: u16 = 0x000A;
    const UNKNOWN: u16 = 0x000C;
    let g = [0x00, 0xFF, 0x00];
    let steps: [(&str, u16, [u8; 10], u64); 6] = [
        ("1 breathe, period 1000 ms big-endian", BREATHE, [g[0], g[1], g[2], 0x03, 0xE8, 0, 100, 0, 0, 0], 12),
        ("2 breathe, period 10000 ms big-endian", BREATHE, [g[0], g[1], g[2], 0x27, 0x10, 0, 100, 0, 0, 0], 12),
        ("3 breathe, period 1000 ms little-endian", BREATHE, [g[0], g[1], g[2], 0xE8, 0x03, 0, 100, 0, 0, 0], 12),
        ("4 breathe, period 10000 ms little-endian", BREATHE, [g[0], g[1], g[2], 0x10, 0x27, 0, 100, 0, 0, 0], 12),
        ("5 effect 0x000C, all parameters 0", UNKNOWN, [0; 10], 15),
        ("6 effect 0x000C, red, 5000 ms, intensity 100", UNKNOWN, [0xFF, 0x00, 0x00, 0x13, 0x88, 0, 100, 0, 0, 0], 15),
    ];
    for (name, effect, params, secs) in steps {
        let _ = kb.raw_effect(1, STATIC, &[0; 10]);
        std::thread::sleep(std::time::Duration::from_secs(2));
        log(&format!("effect test: {name}"));
        if let Err(e) = kb.raw_effect(1, effect, &params) {
            log(&format!("effect test: {name}: {e}"));
        }
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
    let _ = kb.raw_effect(1, STATIC, &[0; 10]);
    log("effect test: done");
}

fn lamparray_test(args: &[String]) {
    use std::fmt::Write as _;
    let out = args.first().cloned().unwrap_or_else(|| "lamparray.txt".into());
    let hex = args.get(1).map(String::as_str).unwrap_or("00ff00");
    let color = config::parse_hex(hex);
    let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    let mut s = String::new();
    let mut held = Vec::new();
    for info in devices::lamparray::interfaces() {
        let _ = writeln!(s, "{:04X}:{:04X} {} — {}", info.vid, info.pid, info.product(), devices::lamparray::describe(&info));
        match devices::lamparray::LampArray::open(&info) {
            Ok(la) => {
                for l in la.lamps().iter().take(12) {
                    let _ = writeln!(s, "  lamp {:>3}: pos {:?} purposes {:#x} levels {:?} key {:#x}", l.id, l.pos, l.purposes, l.levels, l.key);
                }
                let control = la.set_autonomous(false);
                let fill = la.fill(color);
                let _ = writeln!(s, "  take control: {control}, fill {hex}: {fill}");
                held.push(la);
            }
            Err(e) => {
                let _ = writeln!(s, "  open failed: {e}");
            }
        }
    }
    if held.is_empty() && s.is_empty() {
        s.push_str("no LampArray devices\n");
    }
    let _ = std::fs::write(&out, &s);
    let pause = std::time::Duration::from_secs(seconds);
    std::thread::sleep(pause);
    // With "set <hold seconds> <lamp>=<rrggbb>...": those lamps in those colors, the others off.
    if args.get(3).map(String::as_str) == Some("set") {
        let hold: u64 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(20);
        let pairs: Vec<(u16, Rgb)> =
            args.iter().skip(5).filter_map(|p| p.split_once('=')).filter_map(|(id, hex)| Some((id.parse().ok()?, config::parse_hex(hex)))).collect();
        for la in &held {
            let ok = la.fill(Rgb(0, 0, 0)) & la.set_lamps(&pairs);
            let _ = writeln!(s, "set {pairs:?}: {ok}");
        }
        let _ = std::fs::write(&out, &s);
        std::thread::sleep(std::time::Duration::from_secs(hold));
    }
    // With "probe": a fixed sequence that shows which lamps are visible and whether the
    // first update after taking control is lost. Each step is logged with its result.
    if args.get(3).map(String::as_str) == Some("probe") {
        let (red, yellow, green, blue, purple, white, off) =
            (Rgb(255, 0, 0), Rgb(255, 200, 0), Rgb(0, 255, 0), Rgb(0, 0, 255), Rgb(160, 0, 255), Rgb(255, 255, 255), Rgb(0, 0, 0));
        let start = std::time::Instant::now();
        let mut step = |name: &str, f: &dyn Fn(&devices::lamparray::LampArray) -> bool| {
            let ok: Vec<bool> = held.iter().map(f).collect();
            let _ = writeln!(s, "{:>5.1}s {name}: {ok:?}", start.elapsed().as_secs_f32());
            let _ = std::fs::write(&out, &s);
            std::thread::sleep(pause);
        };
        let count = |la: &devices::lamparray::LampArray| la.attributes.lamp_count;
        step("1 retake control, wait 1 s, all green", &|la| {
            la.set_autonomous(true);
            std::thread::sleep(std::time::Duration::from_millis(300));
            let c = la.set_autonomous(false);
            std::thread::sleep(std::time::Duration::from_secs(1));
            c & la.fill(green)
        });
        step("2 every lamp blue, one by one", &|la| (0..count(la)).all(|i| la.fill_range(i, i, blue)));
        step("3 lamp 0 red, others off", &|la| la.fill(off) & la.fill_range(0, 0, red));
        step("4 lamp 3 yellow, others off", &|la| la.fill(off) & la.fill_range(3, 3, yellow));
        step("5 lamps 4-7 purple, keys off", &|la| la.fill(off) & la.fill_range(4, count(la).saturating_sub(1), purple));
        step("6 all white in one update", &|la| la.fill(white));
    }
    // Hand the lighting back to the devices.
    for la in &held {
        la.set_autonomous(true);
    }
}

/// "naga-123456H78900012" → "naga-…0012": enough to tell identical devices apart.
fn short_id(id: &str) -> String {
    match id.split_once('-') {
        Some((kind, serial)) if serial.chars().count() > 4 => {
            let tail: String = serial.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            format!("{kind}-…{tail}")
        }
        _ => id.to_string(),
    }
}
/// Writes the device report to `path`, or next to the settings and opens it in Notepad.
fn diagnose(path: Option<std::path::PathBuf>) {
    let open = path.is_none();
    let path = path.unwrap_or_else(|| Config::dir().join("rgbtray-diagnose.txt"));
    let _ = std::fs::create_dir_all(Config::dir());
    // A BOM so Notepad shows non-ASCII product names right on every Windows version.
    let text = format!("\u{FEFF}{}", diagnose_report().replace('\n', "\r\n"));
    match std::fs::write(&path, text) {
        Ok(()) if open => {
            let _ = std::process::Command::new("notepad.exe").arg(&path).spawn();
        }
        Ok(()) => {}
        Err(e) => unsafe { message(&format!("{}\n{e}", tr!("Не удалось сохранить отчёт:", "Could not save the report:")), true) },
    }
}

/// The project page; device requests and bug reports go to its issues.
const REPO_URL: &str = "https://github.com/Gronck2/rgb-tray";

/// Puts text on the clipboard.
unsafe fn copy_to_clipboard(text: &str) -> bool {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    const CF_UNICODETEXT: u32 = 13;
    let w: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    if OpenClipboard(HWND_MAIN.load(Ordering::Relaxed) as HWND) == 0 {
        return false;
    }
    EmptyClipboard();
    let mem = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2);
    let mut ok = false;
    if !mem.is_null() {
        let p = GlobalLock(mem) as *mut u16;
        if !p.is_null() {
            std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
            GlobalUnlock(mem);
            // On success the clipboard owns the memory.
            ok = !SetClipboardData(CF_UNICODETEXT, mem).is_null();
        }
        if !ok {
            GlobalFree(mem);
        }
    }
    CloseClipboard();
    ok
}

/// "Request device support": copies the device report and opens the request form on GitHub.
/// Runs on its own thread: finding the devices takes a moment.
fn request_device_support() {
    let report = diagnose_report();
    // Also kept as a file, in case the clipboard gets overwritten before pasting.
    let path = Config::dir().join("rgbtray-diagnose.txt");
    let _ = std::fs::create_dir_all(Config::dir());
    let _ = std::fs::write(&path, format!("\u{FEFF}{}", report.replace('\n', "\r\n")));
    unsafe {
        let copied = copy_to_clipboard(&report);
        let text = if copied {
            tr!(
                "Отчёт об устройствах скопирован в буфер обмена.\n\nСейчас откроется форма запроса на GitHub (нужен аккаунт GitHub). Укажите модель устройства и вставьте отчёт в поле «Device report» (Ctrl+V).",
                "The device report is copied to the clipboard.\n\nThe request form on GitHub opens next (a GitHub account is needed). Enter the device model and paste the report into the \"Device report\" field (Ctrl+V)."
            )
            .to_string()
        } else {
            format!("{}\n{}", tr!("Не удалось скопировать отчёт. Он сохранён в файл:", "Could not copy the report. It is saved to:"), path.display())
        };
        message(&text, !copied);
        let url = wide(&format!("{REPO_URL}/issues/new?template=device-request.yml"));
        ShellExecuteW(null_mut(), wide("open").as_ptr(), url.as_ptr(), null(), null(), SW_SHOWNORMAL);
    }
}

// ---------------------------------------------------------------- export / import

/// Asks for a file to save to (`save`, suggesting `name`) or to open; None when cancelled.
unsafe fn settings_file_dialog(save: bool, name: &str) -> Option<std::path::PathBuf> {
    let mut buf = [0u16; 1024];
    // Characters Windows doesn't allow in file names.
    let name: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let name = wide(&name);
    if save {
        buf[..name.len()].copy_from_slice(&name);
    }
    let filter: Vec<u16> =
        tr!("Настройки RGB Tray (*.toml)\0*.toml\0Все файлы\0*.*\0\0", "RGB Tray settings (*.toml)\0*.toml\0All files\0*.*\0\0").encode_utf16().collect();
    let ext = wide("toml");
    let mut ofn: OPENFILENAMEW = zeroed();
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = HWND_MAIN.load(Ordering::Relaxed) as HWND;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrDefExt = ext.as_ptr();
    ofn.Flags = OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST | if save { OFN_OVERWRITEPROMPT } else { OFN_FILEMUSTEXIST };
    let ok = if save { GetSaveFileNameW(&mut ofn) } else { GetOpenFileNameW(&mut ofn) } != 0;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    (ok && len > 0).then(|| std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

unsafe fn message(text: &str, error: bool) {
    let flags = if error { MB_ICONERROR } else { MB_ICONINFORMATION };
    MessageBoxW(HWND_MAIN.load(Ordering::Relaxed) as HWND, wide(text).as_ptr(), wide("RGB Tray").as_ptr(), MB_OK | flags | MB_SETFOREGROUND);
}

/// Saves all settings (colors, presets, key colors, devices) to a file of the user's choice.
fn export_settings() {
    unsafe {
        let Some(path) = settings_file_dialog(true, "rgbtray-settings.toml") else { return };
        let text = toml::to_string_pretty(&*config()).unwrap_or_default();
        if let Err(e) = std::fs::write(&path, text) {
            message(&format!("{}\n{e}", tr!("Не удалось сохранить настройки:", "Could not save the settings:")), true);
        }
    }
}

/// Saves preset `i` to a file to share; it goes without its hotkey.
fn export_preset(i: usize) {
    unsafe {
        let (name, text) = {
            let cfg = config();
            (cfg.presets.get(i).map(|p| p.name.clone()).unwrap_or_default(), cfg.preset_file(i))
        };
        let Some(text) = text else { return };
        let Some(path) = settings_file_dialog(true, &format!("{name}.toml")) else { return };
        if let Err(e) = std::fs::write(&path, text) {
            message(&format!("{}\n{e}", tr!("Не удалось сохранить пресет:", "Could not save the preset:")), true);
        }
    }
}

/// Loads a file: a shared preset is added to the presets; a settings file replaces all
/// settings (the current ones are kept next to the config); anything else changes nothing.
fn import_settings() {
    unsafe {
        let Some(path) = settings_file_dialog(false, "") else { return };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                message(&format!("{}\n{e}", tr!("Не удалось прочитать файл:", "Could not read the file:")), true);
                return;
            }
        };
        match Config::file_kind(&text) {
            config::FileKind::Preset => {
                let added = {
                    let mut cfg = config();
                    let added = cfg.import_preset(&text);
                    if added.is_ok() {
                        cfg.save();
                    }
                    added
                };
                ui::refresh();
                match added {
                    Ok(p) if p.matched == p.total => message(&format!("{} «{}».", tr!("Добавлен пресет", "Added the preset"), p.name), false),
                    Ok(p) => {
                        let mut text = format!(
                            "{} «{}»: {} {} {} {}.",
                            tr!("Добавлен пресет", "Added the preset"),
                            p.name,
                            p.matched,
                            tr!("из", "of"),
                            p.total,
                            tr!("устройств подошли к вашим", "devices match yours"),
                        );
                        if !p.missing.is_empty() {
                            text += &format!("\n{} {}.", tr!("Нет у вас:", "Not here:"), p.missing.join(", "));
                        }
                        message(&text, false)
                    }
                    Err(config::ImportError::NoDevices(models)) => message(
                        &format!(
                            "{}\n{} {}.",
                            tr!("Пресет не добавлен: ни одного его устройства у вас нет.", "The preset was not added: none of its devices is here."),
                            tr!("В нём:", "It is for:"),
                            models.join(", ")
                        ),
                        true,
                    ),
                    Err(config::ImportError::Unreadable(e)) => {
                        message(&format!("{}\n{e}", tr!("Не удалось прочитать пресет:", "Could not read the preset:")), true)
                    }
                }
            }
            config::FileKind::Settings => {
                let Ok(new) = toml::from_str::<Config>(&text) else { return };
                let _ = std::fs::copy(Config::dir().join("config.toml"), Config::dir().join("config.before-import.toml"));
                {
                    let mut cfg = config();
                    *cfg = new;
                    cfg.save();
                }
                ui::PINNED.store(config().pinned, Ordering::Relaxed);
                i18n::set(config().language.as_deref());
                register_hotkeys();
                apply_all();
                update_tray(HWND_MAIN.load(Ordering::Relaxed) as HWND, NIM_MODIFY);
                ui::refresh();
            }
            config::FileKind::Other => {
                message(tr!("Это не файл настроек и не пресет RGB Tray.", "This is neither an RGB Tray settings file nor a preset."), true);
            }
        }
    }
}

/// Switches the daily update check; switching it on checks right away.
fn toggle_check_updates() {
    let mut cfg = config();
    cfg.check_updates = !cfg.check_updates;
    let on = cfg.check_updates;
    cfg.save();
    drop(cfg);
    if on {
        std::thread::spawn(update::check_now);
    }
    post(WM_APPLIED);
}

/// Switches whether the lighting dims and goes off with the display.
fn toggle_follow_display() {
    let mut cfg = config();
    cfg.follow_display = !cfg.follow_display;
    cfg.save();
    drop(cfg);
    apply_all();
}

/// Switches the interface between Russian and English and remembers the choice.
fn toggle_language() {
    let lang = if i18n::en() { "ru" } else { "en" };
    {
        let mut cfg = config();
        cfg.language = Some(lang.into());
        cfg.save();
    }
    i18n::set(Some(lang));
    unsafe { update_tray(HWND_MAIN.load(Ordering::Relaxed) as HWND, NIM_MODIFY) };
    ui::refresh();
    editor::refresh();
}

unsafe fn pick_color(owner: HWND, initial: Rgb) -> Option<Rgb> {
    let mut custom = [0u32; 16];
    {
        let cfg = config();
        for (dst, src) in custom.iter_mut().zip(cfg.custom_colors.iter()) {
            *dst = *src;
        }
    }
    let mut cc: CHOOSECOLORW = zeroed();
    cc.lStructSize = size_of::<CHOOSECOLORW>() as u32;
    cc.hwndOwner = owner;
    cc.rgbResult = initial.0 as u32 | (initial.1 as u32) << 8 | (initial.2 as u32) << 16;
    cc.lpCustColors = custom.as_mut_ptr();
    cc.Flags = CC_RGBINIT | CC_FULLOPEN | CC_ANYCOLOR;
    let ok = ChooseColorW(&mut cc) != 0;
    config().custom_colors = custom.to_vec();
    ok.then(|| {
        let v = cc.rgbResult;
        Rgb(v as u8, (v >> 8) as u8, (v >> 16) as u8)
    })
}

// ---------------------------------------------------------------- tray icon

/// Draws a round 32x32 icon in the given color (rainbow for the cycle effect).
unsafe fn make_icon(effect: Effect, c: Rgb, low_battery: bool) -> HICON {
    let accent = ui::theme().accent;
    let badge = Rgb((accent.r * 255.0) as u8, (accent.g * 255.0) as u8, (accent.b * 255.0) as u8);
    const S: i32 = 32;
    let mut bi: BITMAPINFO = zeroed();
    bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = S;
    bi.bmiHeader.biHeight = -S;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB;
    let mut bits = null_mut();
    let color_bmp = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    let px = std::slice::from_raw_parts_mut(bits as *mut u32, (S * S) as usize);
    let center = (S as f32 - 1.0) / 2.0;
    for y in 0..S {
        for x in 0..S {
            let (dx, dy) = (x as f32 - center, y as f32 - center);
            let d = (dx * dx + dy * dy).sqrt();
            let alpha = (14.5 - d).clamp(0.0, 1.0);
            let Rgb(r, g, b) = match effect {
                Effect::Off => Rgb(90, 90, 90),
                Effect::Cycle => hue(dy.atan2(dx).to_degrees() + 180.0),
                _ => c,
            };
            // Off: just a ring.
            let alpha = if effect == Effect::Off && d < 10.5 { 0.0 } else { alpha };
            // Low battery: accent-colored badge in the bottom-right corner with a dark outline.
            let (bx, by) = (x as f32 - 24.5, y as f32 - 24.5);
            let bd = (bx * bx + by * by).sqrt();
            let (Rgb(r, g, b), alpha) = if low_battery && bd < 7.5 {
                (if bd < 5.8 { badge } else { Rgb(0x20, 0x20, 0x20) }, (7.5 - bd).clamp(0.0, 1.0))
            } else {
                (Rgb(r, g, b), alpha)
            };
            let a = (alpha * 255.0) as u32;
            let pm = |v: u8| v as u32 * a / 255;
            px[(y * S + x) as usize] = (a << 24) | (pm(r) << 16) | (pm(g) << 8) | pm(b);
        }
    }
    let mask_bits = vec![0u8; (S * S / 8) as usize];
    let mask_bmp = CreateBitmap(S, S, 1, 1, mask_bits.as_ptr() as _);
    let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask_bmp, hbmColor: color_bmp };
    let icon = CreateIconIndirect(&ii);
    DeleteObject(color_bmp);
    DeleteObject(mask_bmp);
    icon
}

fn hue(deg: f32) -> Rgb {
    let h = (deg.rem_euclid(360.0)) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    Rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

static ICON: AtomicIsize = AtomicIsize::new(0);

/// Adds (`NIM_ADD`) or refreshes (`NIM_MODIFY`) the tray icon from the current config.
unsafe fn update_tray(hwnd: HWND, action: u32) {
    let ids = active_ids();
    let (effect, color, tip, low) = {
        let cfg = config();
        let present = PRESENT.lock().unwrap();
        let status = STATUS.lock().unwrap();
        let no_link = NO_LINK.lock().unwrap();
        // The icon follows the first connected device.
        let dc = ids.iter().find(|id| present.contains(id)).map(|id| cfg.device(id).visible()).unwrap_or_default();
        let mut tip = String::from(tr!("Подсветка", "Lighting"));
        for id in &ids {
            let name = cfg.display_name(id);
            if no_link.contains(id) {
                tip.push_str(&format!("\n{name}: {}", tr!("нет связи", "no link")));
                continue;
            }
            tip.push_str(&format!("\n{}: {}", name, cfg.device(id).visible().effect.label().to_lowercase()));
            if let Some((_, b)) = status.iter().find(|(s, _)| s == id) {
                tip.push_str(&format!(", {}", b.label()));
            }
        }
        let low = status.iter().any(|(k, b)| b.is_low() && !no_link.contains(k));
        (dc.effect, dc.rgb(), tip, low)
    };
    let icon = make_icon(effect, color, low);
    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = icon;
    for (i, ch) in tip.encode_utf16().take(nid.szTip.len() - 1).enumerate() {
        nid.szTip[i] = ch;
    }
    Shell_NotifyIconW(action, &nid);
    let old = ICON.swap(icon as isize, Ordering::Relaxed);
    if old != 0 {
        DestroyIcon(old as HICON);
    }
}

unsafe fn remove_tray(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

// ---------------------------------------------------------------- autostart

fn autostart_enabled() -> bool {
    unsafe {
        let mut size = 0u32;
        RegGetValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(APP_NAME).as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut size) == ERROR_SUCCESS
    }
}

fn set_autostart(on: bool) {
    let exe = std::env::current_exe().unwrap_or_default();
    set_autostart_for(on.then_some(exe.as_path()));
}

/// Starts the given exe with Windows (`None` removes autostart).
fn set_autostart_for(exe: Option<&std::path::Path>) {
    unsafe {
        if let Some(exe) = exe {
            let value = wide(&format!("\"{}\"", exe.display()));
            RegSetKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(APP_NAME).as_ptr(), REG_SZ, value.as_ptr() as _, (value.len() * 2) as u32);
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(APP_NAME).as_ptr());
        }
    }
}

// ---------------------------------------------------------------- window

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match (lparam & 0xFFFF) as u32 {
                WM_LBUTTONUP => ui::toggle(),
                WM_RBUTTONUP => traymenu::show(),
                _ => {}
            }
            0
        }
        // Devices plugged/unplugged or the PC woke up: re-apply once things settle.
        WM_DEVICECHANGE if wparam == DBT_DEVNODES_CHANGED as usize => {
            SetTimer(hwnd, TIMER_REAPPLY, 2000, None);
            1
        }
        WM_POWERBROADCAST => {
            if wparam == PBT_APMRESUMEAUTOMATIC as usize {
                schedule_wake_reapply(hwnd);
            } else if wparam == PBT_POWERSETTINGCHANGE as usize && lparam != 0 {
                let s = &*(lparam as *const windows_sys::Win32::System::Power::POWERBROADCAST_SETTING);
                let g = &s.PowerSetting;
                let d = &GUID_CONSOLE_DISPLAY_STATE;
                let same = (g.data1, g.data2, g.data3, g.data4) == (d.data1, d.data2, d.data3, d.data4);
                if same && s.DataLength >= 1 {
                    let state = s.Data[0];
                    let was = DISPLAY.swap(state, Ordering::Relaxed);
                    if state == DISPLAY_ON && was == DISPLAY_OFF {
                        schedule_wake_reapply(hwnd);
                    } else if state != was && config().follow_display {
                        // Dimmed, off, or back on after being only dimmed: the lighting follows right away.
                        apply_all();
                    }
                }
            }
            1
        }
        WM_HOTKEY => {
            let id = wparam as i32 - HOTKEY_BASE;
            if (0..MAX_HOTKEYS).contains(&id) {
                apply_preset(id as usize);
                ui::refresh();
            }
            0
        }
        WM_REAPPLY_SOON => {
            SetTimer(hwnd, TIMER_REAPPLY, 1000, None);
            0
        }
        WM_TIMER if wparam == TIMER_REAPPLY_EARLY || wparam == TIMER_REAPPLY || wparam == TIMER_REAPPLY_LATE => {
            KillTimer(hwnd, wparam);
            apply_all();
            0
        }
        WM_APPLIED | WM_BATTERY => {
            check_batteries();
            update_tray(hwnd, NIM_MODIFY);
            ui::refresh();
            0
        }
        WM_DESTROY => {
            remove_tray(hwnd);
            PostQuitMessage(0);
            0
        }
        // Explorer restarted: the tray icon has to be added again.
        _ if msg != 0 && msg as isize == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            ui::PINNED.store(config().pinned, Ordering::Relaxed);
            update_tray(hwnd, NIM_ADD);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    // `--snapshot <file.png> [dark|light]`: detect devices, render the panel to a PNG and exit.
    let args: Vec<String> = std::env::args().collect();
    i18n::set(config().language.as_deref());
    // `--diagnose [file.txt]`: device report for adding support for new devices.
    if let Some(i) = args.iter().position(|a| a == "--diagnose") {
        diagnose(args.get(i + 1).map(std::path::PathBuf::from));
        return;
    }
    // `--lamparray-test <out.txt> <rrggbb> <seconds>`: takes over every LampArray device,
    // fills it with the color, writes what the devices report, and hands them back.
    if args.iter().any(|a| a == "--logitech-effect-test") {
        logitech_effect_test();
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--lamparray-test") {
        lamparray_test(&args[i + 1..]);
        return;
    }
    if args.iter().any(|a| a == "--uninstall") {
        installer::uninstall();
        return;
    }
    if installer::is_setup(&args) {
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        installer::run(&args);
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let path = args.get(i + 1).cloned().unwrap_or_else(|| "panel.png".into());
        let dark = args.get(i + 2).map(|m| m == "dark");
        let d = devices::discover();
        let mut present: Vec<String> = d.found.iter().map(|f| f.id.clone()).collect();
        // Extra made-up devices, to see the list pages (registered in memory only, not saved).
        if let Some(n) = std::env::var("RGBTRAY_DEMO_MANY").ok().and_then(|v| v.parse::<usize>().ok()) {
            present.extend((1..=n).map(|k| format!("lamparray-DEMO{k:04}")));
        }
        if let Some(p) = std::env::var("RGBTRAY_DEMO_PAGE").ok().and_then(|v| v.parse().ok()) {
            ui::set_page(p);
        }
        config().register(&present);
        check_services(&config().clone());
        if std::env::var_os("RGBTRAY_DEMO_WARN").is_some() {
            LAMPARRAY_WARN.store(true, Ordering::Relaxed);
        }
        if let Ok(spec) = std::env::var("RGBTRAY_DEMO_UPDATE") {
            update::demo(&spec);
        }
        if std::env::var_os("RGBTRAY_DEMO_DYNLIGHT").is_some() {
            DYNAMIC_LIGHTING_WARN.store(true, Ordering::Relaxed);
        }
        let mut no_link = d.no_link;
        if std::env::var_os("RGBTRAY_DEMO_NOLINK").is_some() {
            if let Some(i) = present.iter().position(|id| id.starts_with("naga")) {
                no_link.push(present.remove(i));
            }
        }
        *NO_LINK.lock().unwrap() = no_link;
        // RGBTRAY_DEMO_APPS=synapse,ghub: as if these programs were running.
        if let Ok(keys) = std::env::var("RGBTRAY_DEMO_APPS") {
            let running: Vec<String> = keys.split(',').filter_map(conflicts::by_key).map(|a| a.processes[0].to_string()).collect();
            *CONFLICTS.lock().unwrap() = conflicts::find(&running, &present, &[]);
        }
        *PRESENT.lock().unwrap() = present;
        *STATUS.lock().unwrap() = d.found.iter().filter_map(|f| f.dev.battery().map(|b| (f.id.clone(), b))).collect();
        // Optional: selected card and zone of the panel.
        let num = |k: usize| args.get(i + k).and_then(|s| s.parse().ok()).unwrap_or(0);
        let res = if path.contains("editor") {
            editor::snapshot(&path, dark, 1.5)
        } else if path.contains("setup") {
            installer::snapshot(&path, dark, 1.5, num(3) == 1)
        } else if path.contains("traymenu") {
            traymenu::snapshot(&path, dark, 1.5)
        } else if path.contains("alert") {
            alert::snapshot(&path, dark, 1.5, num(3) as u8)
        } else {
            ui::snapshot(&path, dark, 1.5, num(3), num(4))
        };
        if let Err(e) = res {
            log(&format!("snapshot: {e}"));
        }
        return;
    }
    unsafe {
        let mutex_name = wide("Local\\rgbtray-single-instance");
        CreateMutexW(null(), 0, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let hinst = GetModuleHandleW(null());
        let class = wide("RgbTrayWindow");
        let mut wc: WNDCLASSW = zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinst;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        // A hidden top-level window (not message-only) so it receives WM_DEVICECHANGE broadcasts.
        let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, null_mut(), null_mut(), hinst, null());
        HWND_MAIN.store(hwnd as isize, Ordering::Relaxed);
        windows_sys::Win32::System::Power::RegisterPowerSettingNotification(hwnd, &GUID_CONSOLE_DISPLAY_STATE, DEVICE_NOTIFY_WINDOW_HANDLE);
        TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) as isize, Ordering::Relaxed);

        ui::PINNED.store(config().pinned, Ordering::Relaxed);
        update_tray(hwnd, NIM_ADD);
        register_hotkeys();
        apply_all();
        devices::logitech::spawn_watcher(|| post(WM_REAPPLY_SOON));
        spawn_battery_poller();
        spawn_link_watch();
        update::spawn_checker();
        if std::env::args().any(|a| a == "--show") {
            ui::PINNED.store(true, Ordering::Relaxed);
            ui::toggle();
        }

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The config lock is held to the end of the statement that takes it, so a second
    /// `config()` in the same statement waits forever (the tray menu hung that way in 0.6.13).
    #[test]
    fn one_config_lock_per_statement() {
        let needle = concat!("config", "()");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![root.clone(), root.join("devices")];
        let mut checked = 0;
        while let Some(dir) = files.pop() {
            for path in std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "rs")) {
                let text = std::fs::read_to_string(&path).unwrap();
                for statement in text.split(';') {
                    assert!(statement.matches(needle).count() < 2, "{}: two config locks in one statement:\n{statement}", path.display());
                }
                checked += 1;
            }
        }
        assert!(checked > 10);
    }

    #[test]
    fn short_ids() {
        assert_eq!(short_id("naga-123456H78900012"), "naga-…0012");
        assert_eq!(short_id("dock-AB12"), "dock-AB12");
        assert_eq!(short_id("asus"), "asus");
    }
}
