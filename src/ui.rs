//! Windows 11 style flyout panel, drawn with Direct2D / DirectWrite.
//! The window exists only while the panel is open.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use windows::core::w;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::HWND as WinHwnd;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::devices::{kind_of, Effect, Rgb};

// ---------------------------------------------------------------- layout (in DIPs)

const W: f32 = 360.0;
const PAD: f32 = 16.0;
const CARD_Y: f32 = 56.0;
const CARD_H: f32 = 50.0;
const CARD_STEP: f32 = 54.0;
/// The "disconnected devices" row that folds the archive.
const ARCHIVE_H: f32 = 34.0;
const WARNING_H: f32 = 64.0;
// The section under the device list; its coordinates are relative to its top (`lower_y()`),
// since the list grows with the number of devices.
const COLOR_LABEL_Y: f32 = 8.0;
const SWATCH_Y: f32 = COLOR_LABEL_Y + 24.0;
const SWATCH: f32 = 28.0;
const EFFECT_LABEL_Y: f32 = SWATCH_Y + SWATCH + 16.0;
const SEG_Y: f32 = EFFECT_LABEL_Y + 24.0;
const SEG_H: f32 = 34.0;
const BRIGHT_LABEL_Y: f32 = SEG_Y + SEG_H + 16.0;
const SLIDER_Y: f32 = BRIGHT_LABEL_Y + 22.0;
const SLIDER_H: f32 = 24.0;
const PRESET_LABEL_Y: f32 = SLIDER_Y + SLIDER_H + 14.0;
const PRESET_Y: f32 = PRESET_LABEL_Y + 24.0;
const PRESET_H: f32 = 28.0;
const PRESET_STEP: f32 = 34.0;
const PRESET_ROWS: usize = 2;
const DIVIDER_Y: f32 = PRESET_Y + PRESET_STEP * PRESET_ROWS as f32 + 6.0;
const FOOTER_Y: f32 = DIVIDER_Y + 10.0;
const LOWER_H: f32 = FOOTER_Y + 50.0;
pub(crate) const PALETTE: [Rgb; 9] = [
    Rgb(0x0A, 0xE3, 0xEF),
    Rgb(0xFF, 0x1E, 0x1E),
    Rgb(0xFF, 0x7A, 0x00),
    Rgb(0xFF, 0xD4, 0x00),
    Rgb(0x20, 0xE0, 0x60),
    Rgb(0x1E, 0x64, 0xFF),
    Rgb(0x9B, 0x30, 0xFF),
    Rgb(0xFF, 0x3C, 0xAA),
    Rgb(0xFF, 0xFF, 0xFF),
];
const EFFECT_LABELS: [&str; 4] = ["Статичный", "Дыхание", "Спектр", "Выкл."];
/// Segoe Fluent Icons glyphs.
const ICON_ALL: &str = "\u{E82F}";
const ICON_CHEVRON_DOWN: &str = "\u{E70D}";
const ICON_CHEVRON_UP: &str = "\u{E70E}";
const ICON_WARNING: &str = "\u{E7BA}";
const ICON_REFRESH: &str = "\u{E72C}";
const ICON_PIN: &str = "\u{E718}";
const ICON_PINNED: &str = "\u{E841}";
const ICON_ADD: &str = "\u{E710}";
const ICON_POWER: &str = "\u{E7E8}";
const ICON_BRIGHTNESS: &str = "\u{E706}";
const ICON_SAVE: &str = "\u{E74E}";
const ICON_PLAY: &str = "\u{E768}";
const ICON_RENAME: &str = "\u{E70F}";
const ICON_DELETE: &str = "\u{E74D}";
const ICON_KEYBOARD: &str = "\u{E765}";

const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Card(usize),
    Swatch(usize),
    Custom,
    /// Global preset chip; right click opens its menu.
    Preset(usize),
    /// Small buttons on a preset chip: overwrite with the current lighting, record hotkey, rename.
    PresetOverwrite(usize),
    PresetHotkey(usize),
    PresetRename(usize),
    /// An item of the open preset menu; a click on the menu body; a click outside it.
    Menu(MenuItem),
    MenuBody,
    MenuOutside,
    SavePreset,
    /// Lighting zone of the selected device (0 = whole device).
    Zone(usize),
    Effect(usize),
    Slider,
    Autostart,
    Reapply,
    Exit,
    /// Opens the per-key editor of the keyboard on that row.
    Keys(usize),
    /// Forgets the disconnected device on that row (asks first).
    Forget(usize),
    /// The pencil after the device name on that row.
    RenameDevice(usize),
    /// "Отключить" on the warning about the Logitech LampArray service.
    DisableService,
    /// Keeps the panel open when it loses focus.
    Pin,
}

/// A row of the device list.
#[derive(Clone, PartialEq)]
enum Row {
    /// A Windows service that hangs the keyboard is active.
    ServiceWarning,
    All,
    /// Connected device, or one without a link.
    Device(String),
    /// Folds/unfolds the disconnected devices; their count.
    ArchiveHeader(usize),
    Archived(String),
}

/// The disconnected devices are listed.
static ARCHIVE_OPEN: AtomicBool = AtomicBool::new(false);
/// Disconnected device whose "forget" button was clicked once and waits for confirmation.
static FORGET_ARMED: Mutex<Option<String>> = Mutex::new(None);

/// Remembered devices that are neither connected nor without a link, last seen first.
fn archived_ids(active: &[String]) -> Vec<String> {
    let cfg = crate::config();
    let mut v: Vec<(&String, u64)> = cfg.devices.iter().filter(|(id, _)| !active.contains(id)).map(|(id, d)| (id, d.last_seen)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    v.into_iter().map(|(id, _)| id.clone()).collect()
}

/// The device list with each row's rectangle.
fn rows() -> Vec<(R, Row)> {
    let active = crate::active_ids();
    let archived = archived_ids(&active);
    let mut out = Vec::new();
    let mut y = CARD_Y;
    let mut push = |h: f32, row: Row| {
        out.push((R { x: 12.0, y, w: W - 24.0, h }, row));
        y += h + (CARD_STEP - CARD_H);
    };
    if crate::LAMPARRAY_WARN.load(Ordering::Relaxed) {
        push(WARNING_H, Row::ServiceWarning);
    }
    push(CARD_H, Row::All);
    for id in active {
        push(CARD_H, Row::Device(id));
    }
    if !archived.is_empty() {
        push(ARCHIVE_H, Row::ArchiveHeader(archived.len()));
        if ARCHIVE_OPEN.load(Ordering::Relaxed) {
            for id in archived {
                push(CARD_H, Row::Archived(id));
            }
        }
    }
    out
}

/// "Не подключено · вчера": when a disconnected device was last seen.
fn last_seen_label(last_seen: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if last_seen == 0 {
        return "Не подключено".into();
    }
    match now.saturating_sub(last_seen) / 86_400 {
        0 => "Не подключено · сегодня".into(),
        1 => "Не подключено · вчера".into(),
        n => format!("Не подключено · {n} дн. назад"),
    }
}

/// Top of the section under the device list.
fn lower_y_of(rows: &[(R, Row)]) -> f32 {
    rows.last().map_or(CARD_Y, |(r, _)| r.y + r.h + (CARD_STEP - CARD_H))
}

fn lower_y() -> f32 {
    lower_y_of(&rows())
}

fn panel_h() -> f32 {
    lower_y() + LOWER_H
}

#[derive(Clone, Copy)]
pub(crate) struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
    pub fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.h }
    }
    pub fn inset(&self, d: f32) -> R {
        R { x: self.x + d, y: self.y + d, w: self.w - 2.0 * d, h: self.h - 2.0 * d }
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

fn swatch_rect(i: usize) -> R {
    let count = PALETTE.len() + 1;
    let gap = (W - 2.0 * PAD - SWATCH * count as f32) / (count - 1) as f32;
    R { x: PAD + i as f32 * (SWATCH + gap), y: SWATCH_Y, w: SWATCH, h: SWATCH }
}

fn slider_rect() -> R {
    R { x: PAD, y: SLIDER_Y, w: W - 2.0 * PAD, h: SLIDER_H }
}

/// The pin button left of the refresh button in the header.
fn pin_rect() -> R {
    R { x: W - PAD - 32.0 - 36.0, y: 12.0, w: 32.0, h: 32.0 }
}

/// The "Клавиши" button of a keyboard card, on the line of the device name.
fn keys_button_rect(card: R) -> R {
    R { x: card.x + card.w - 44.0 - 82.0, y: card.y + 5.0, w: 82.0, h: 22.0 }
}

/// Width of a device name as drawn on its card (cached; measuring needs DirectWrite).
fn name_width(name: &str) -> f32 {
    thread_local! {
        static CACHE: RefCell<(Option<IDWriteTextFormat>, Vec<(String, f32)>)> = const { RefCell::new((None, Vec::new())) };
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((_, w)) = c.1.iter().find(|(n, _)| n == name) {
            return *w;
        }
        if c.0.is_none() {
            c.0 = unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) }
                .and_then(|dw| text_format(&dw, w!("Segoe UI Variable Text"), 14.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING))
                .ok();
        }
        let w = c.0.as_ref().map_or(name.chars().count() as f32 * 7.5, |f| text_width(name, f));
        c.1.push((name.to_string(), w));
        w
    })
}

/// Name area of a device card, up to its buttons.
fn name_rect(card: R, row: &Row) -> R {
    let buttons_x = match row {
        Row::Device(id) if kind_of(id).per_key => keys_button_rect(card).x,
        Row::Archived(_) => forget_button_rect(card).x,
        _ => card.x + card.w - 50.0,
    };
    // Room for the pencil after the name.
    R { x: card.x + 50.0, y: card.y + 6.0, w: buttons_x - card.x - 56.0 - 24.0, h: 20.0 }
}

/// The small pencil right after a device's name.
fn pencil_rect(card: R, row: &Row, name: &str) -> R {
    let n = name_rect(card, row);
    R { x: n.x + name_width(name).min(n.w) + 4.0, y: n.y, w: 20.0, h: 20.0 }
}

/// The "Отключить" button of the service warning.
fn disable_button_rect(card: R) -> R {
    R { x: card.x + card.w - 12.0 - 96.0, y: card.y + (card.h - 28.0) / 2.0, w: 96.0, h: 28.0 }
}

/// The "forget" button of a disconnected device's card, left of its color dot.
fn forget_button_rect(card: R) -> R {
    R { x: card.x + card.w - 44.0 - 82.0, y: card.y + 5.0, w: 82.0, h: 22.0 }
}

fn text_width(s: &str, fmt: &IDWriteTextFormat) -> f32 {
    let w: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)
            .and_then(|dw| dw.CreateTextLayout(&w, fmt, 2000.0, 100.0))
            .and_then(|l| {
                let mut m = DWRITE_TEXT_METRICS::default();
                l.GetMetrics(&mut m).map(|_| m.widthIncludingTrailingWhitespace)
            })
            .unwrap_or(s.chars().count() as f32 * 6.5)
    }
}

/// Clickable parts of the header and the device list (panel coordinates).
fn card_layout(rows: &[(R, Row)]) -> Vec<(R, Hit)> {
    let mut v = vec![(R { x: W - PAD - 32.0, y: 12.0, w: 32.0, h: 32.0 }, Hit::Reapply), (pin_rect(), Hit::Pin)];
    for (i, (r, row)) in rows.iter().enumerate() {
        // Buttons on a card come first, so hit-testing finds them before the card.
        match row {
            Row::Device(id) if kind_of(id).per_key => v.push((keys_button_rect(*r), Hit::Keys(i))),
            Row::Archived(_) => v.push((forget_button_rect(*r), Hit::Forget(i))),
            Row::ServiceWarning => v.push((disable_button_rect(*r), Hit::DisableService)),
            _ => {}
        }
        if let Row::Device(id) | Row::Archived(id) = row {
            if device_renaming().as_ref() != Some(id) {
                let name = crate::config().display_name(id);
                v.push((pencil_rect(*r, row, &name), Hit::RenameDevice(i)));
            }
        }
        v.push((*r, Hit::Card(i)));
    }
    v
}

/// Clickable parts of the section under the device list (relative to its top).
fn lower_layout() -> Vec<(R, Hit)> {
    let mut v = Vec::new();
    for i in 0..PALETTE.len() {
        v.push((swatch_rect(i), Hit::Swatch(i)));
    }
    v.push((swatch_rect(PALETTE.len()), Hit::Custom));
    let seg_w = (W - 2.0 * PAD) / 4.0;
    for i in 0..4 {
        v.push((R { x: PAD + i as f32 * seg_w, y: SEG_Y, w: seg_w, h: SEG_H }, Hit::Effect(i)));
    }
    v.push((slider_rect(), Hit::Slider));
    v.push((R { x: PAD - 4.0, y: FOOTER_Y, w: 250.0, h: 36.0 }, Hit::Autostart));
    v.push((R { x: W - PAD - 92.0, y: FOOTER_Y, w: 92.0, h: 36.0 }, Hit::Exit));
    v
}

fn hit_test(x: f32, y: f32) -> Option<Hit> {
    // An open menu takes every click.
    if let Some((panel, items)) = menu_layout() {
        return Some(match items.iter().find(|(r, _)| r.contains(x, y)) {
            Some(&(_, item)) => Hit::Menu(item),
            None if panel.contains(x, y) => Hit::MenuBody,
            None => Hit::MenuOutside,
        });
    }
    panel_hit_test(x, y)
}

fn panel_hit_test(x: f32, y: f32) -> Option<Hit> {
    let rows = rows();
    let top = lower_y_of(&rows);
    if y < top {
        return card_layout(&rows).into_iter().find(|(r, _)| r.contains(x, y)).map(|(_, h)| h);
    }
    let y = y - top;
    // The zone switch exists only for devices with several zones (whole device + each zone).
    let zone_count = selected_zones().len();
    let zones = zone_rects().into_iter().enumerate().take(if zone_count > 0 { zone_count + 1 } else { 0 });
    zones
        .map(|(k, r)| (r, Hit::Zone(k)))
        .chain(preset_chips().into_iter().map(|(r, h, _)| (r, h)))
        .chain(lower_layout())
        .find(|(r, _)| r.contains(x, y))
        .map(|(_, h)| h)
}
fn preset_label(p: &crate::config::Preset) -> String {
    match p.hotkey {
        Some(h) => format!("{} · {}", p.name, h.label()),
        None => p.name.clone(),
    }
}

/// Width of the three icon buttons at the right end of a preset chip.
const PRESET_ICONS_W: f32 = 73.0;

/// Preset chips flowing over up to two rows, then the "save" chip.
/// For each preset the icon buttons come before the chip itself, so hit-testing finds them first.
fn preset_chips() -> Vec<(R, Hit, String)> {
    let renaming = RENAME.load(Ordering::Relaxed);
    let labels: Vec<String> = crate::config()
        .presets
        .iter()
        .enumerate()
        .map(|(i, p)| if i == renaming { RENAME_TEXT.lock().unwrap().clone() } else { preset_label(p) })
        .collect();
    let items = labels.into_iter().enumerate().map(|(i, l)| (l, Some(i))).chain(std::iter::once(("+ Сохранить текущие".to_string(), None)));
    let mut out = Vec::new();
    let (mut x, mut row) = (PAD, 0);
    for (label, preset) in items {
        // Approximate width of 12px Segoe UI text; drawing uses the same rects.
        let mut w = label.chars().count() as f32 * 6.9 + 24.0;
        if preset == Some(renaming) {
            w = w.max(110.0);
        }
        if preset.is_some() {
            w += PRESET_ICONS_W;
        }
        if x + w > W - PAD && x > PAD {
            row += 1;
            x = PAD;
        }
        if row >= PRESET_ROWS {
            break;
        }
        let r = R { x, y: PRESET_Y + row as f32 * PRESET_STEP, w: w.min(W - 2.0 * PAD), h: PRESET_H };
        match preset {
            Some(i) => {
                let icon = |k: f32| R { x: r.x + r.w - PRESET_ICONS_W + 2.0 + k * 23.0, y: r.y + 3.0, w: 22.0, h: r.h - 6.0 };
                out.push((icon(0.0), Hit::PresetOverwrite(i), String::new()));
                out.push((icon(1.0), Hit::PresetHotkey(i), String::new()));
                out.push((icon(2.0), Hit::PresetRename(i), String::new()));
                out.push((r, Hit::Preset(i), label));
            }
            None => out.push((r, Hit::SavePreset, label)),
        }
        x += r.w + 6.0;
    }
    out
}

/// Preset whose hotkey is being recorded (usize::MAX = none).
static CAPTURE: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Preset being renamed in place (usize::MAX = none) and the text typed so far.
static RENAME: AtomicUsize = AtomicUsize::new(usize::MAX);
static RENAME_TEXT: Mutex<String> = Mutex::new(String::new());

/// Device whose name is being edited on its card (shares RENAME_TEXT with presets).
static DEVICE_RENAME: Mutex<Option<String>> = Mutex::new(None);

fn device_renaming() -> Option<String> {
    DEVICE_RENAME.lock().unwrap().clone()
}

/// Something is being renamed: a preset or a device.
fn renaming_any() -> bool {
    RENAME.load(Ordering::Relaxed) != usize::MAX || device_renaming().is_some()
}

fn start_device_rename(id: String) {
    finish_rename(true);
    CAPTURE.store(usize::MAX, Ordering::Relaxed);
    *RENAME_TEXT.lock().unwrap() = crate::config().display_name(&id);
    *DEVICE_RENAME.lock().unwrap() = Some(id);
}

fn start_rename(i: usize) {
    finish_rename(true);
    CAPTURE.store(usize::MAX, Ordering::Relaxed);
    *RENAME_TEXT.lock().unwrap() = crate::config().presets.get(i).map(|p| p.name.clone()).unwrap_or_default();
    RENAME.store(i, Ordering::Relaxed);
}

/// Ends in-place renaming, saving the typed name unless cancelled.
fn finish_rename(save: bool) {
    let i = RENAME.swap(usize::MAX, Ordering::Relaxed);
    if i != usize::MAX && save {
        let name = RENAME_TEXT.lock().unwrap().clone();
        crate::rename_preset(i, &name);
    }
    let device = DEVICE_RENAME.lock().unwrap().take();
    if let (Some(id), true) = (device, save) {
        let name = RENAME_TEXT.lock().unwrap().clone();
        crate::rename_device(&id, &name);
    }
}

const VK_F13: u32 = 0x7C;

/// Actions of the preset menu.
#[derive(Clone, Copy, PartialEq, Debug)]
enum MenuItem {
    Apply,
    Overwrite,
    Rename,
    /// F13 + n, or None to remove the hotkey.
    Hotkey(Option<u32>),
    /// Record any key combination.
    Combo,
    /// Modifier toggle for the F13–F24 buttons (MOD_* bit).
    Mod(u32),
    Delete,
}

/// The open preset menu, drawn inside the panel: (preset, anchor point, modifiers for F13–F24).
static MENU: Mutex<Option<(usize, f32, f32, u32)>> = Mutex::new(None);

/// MOD_* bits with their labels, in Windows order.
const MENU_MODS: [(u32, &str); 4] = [(0x2, "Ctrl"), (0x1, "Alt"), (0x4, "Shift"), (0x8, "Win")];

fn is_f13_24(vk: u32) -> bool {
    (VK_F13..VK_F13 + 12).contains(&vk)
}

const MENU_W: f32 = 240.0;
const MENU_ITEM_H: f32 = 32.0;
const MENU_HEADER_H: f32 = 34.0;
const MENU_GAP: f32 = 9.0;
const MENU_LABEL_H: f32 = 24.0;
const MENU_KEY_H: f32 = 26.0;
const MENU_KEY_STEP: f32 = 30.0;
const MENU_KEY_COLS: usize = 4;

fn open_menu(i: usize, x: f32, y: f32) {
    // Start from the modifiers of the current F13–F24 hotkey.
    let mods = crate::config().presets.get(i).and_then(|p| p.hotkey).filter(|h| is_f13_24(h.vk)).map_or(0, |h| h.mods);
    *MENU.lock().unwrap() = Some((i, x, y, mods));
}

fn menu_mods() -> u32 {
    MENU.lock().unwrap().map_or(0, |m| m.3)
}

fn close_menu() -> bool {
    MENU.lock().unwrap().take().is_some()
}

fn menu_preset() -> Option<usize> {
    MENU.lock().unwrap().map(|m| m.0)
}

/// The menu card and its items, placed next to the click and kept inside the panel.
fn menu_layout() -> Option<(R, Vec<(R, MenuItem)>)> {
    let (_, ax, ay, _) = (*MENU.lock().unwrap())?;
    let key_rows = 12 / MENU_KEY_COLS + 1;
    let h = 6.0 + MENU_HEADER_H + 3.0 * MENU_ITEM_H + MENU_GAP + MENU_LABEL_H + (key_rows + 1) as f32 * MENU_KEY_STEP + MENU_GAP + MENU_ITEM_H + 6.0;
    let x = ax.clamp(8.0, W - 8.0 - MENU_W);
    let y = if ay + h <= panel_h() - 8.0 { ay } else { (ay - h).max(8.0) };
    let panel = R { x, y, w: MENU_W, h };
    let mut items = Vec::new();
    let row = |y: f32| R { x: x + 4.0, y, w: MENU_W - 8.0, h: MENU_ITEM_H };
    let mut cy = y + 6.0 + MENU_HEADER_H;
    for item in [MenuItem::Apply, MenuItem::Overwrite, MenuItem::Rename] {
        items.push((row(cy), item));
        cy += MENU_ITEM_H;
    }
    cy += MENU_GAP + MENU_LABEL_H;
    let gx = x + 12.0;
    let cell = (MENU_W - 24.0 - (MENU_KEY_COLS - 1) as f32 * 4.0) / MENU_KEY_COLS as f32;
    for (k, &(bit, _)) in MENU_MODS.iter().enumerate() {
        items.push((R { x: gx + k as f32 * (cell + 4.0), y: cy, w: cell, h: MENU_KEY_H }, MenuItem::Mod(bit)));
    }
    cy += MENU_KEY_STEP;
    for n in 0..12u32 {
        let (c, r) = (n as usize % MENU_KEY_COLS, n as usize / MENU_KEY_COLS);
        let rect = R { x: gx + c as f32 * (cell + 4.0), y: cy + r as f32 * MENU_KEY_STEP, w: cell, h: MENU_KEY_H };
        items.push((rect, MenuItem::Hotkey(Some(n))));
    }
    let last = cy + (key_rows - 1) as f32 * MENU_KEY_STEP;
    items.push((R { x: gx, y: last, w: cell, h: MENU_KEY_H }, MenuItem::Hotkey(None)));
    items.push((R { x: gx + cell + 4.0, y: last, w: MENU_W - 24.0 - cell - 4.0, h: MENU_KEY_H }, MenuItem::Combo));
    cy = last + MENU_KEY_STEP + MENU_GAP;
    items.push((row(cy), MenuItem::Delete));
    Some((panel, items))
}

/// Overwriting makes sense only when the current lighting differs from the preset.
fn preset_matches(i: usize) -> bool {
    let cfg = crate::config();
    cfg.presets.get(i).is_some_and(|p| cfg.matches(p))
}

unsafe fn menu_action(hwnd: HWND, i: usize, item: MenuItem) {
    if let MenuItem::Mod(bit) = item {
        // Toggling a modifier keeps the menu open and updates an assigned F13–F24 hotkey.
        let mods = {
            let mut m = MENU.lock().unwrap();
            let Some(state) = m.as_mut() else { return };
            state.3 ^= bit;
            state.3
        };
        let current = crate::config().presets.get(i).and_then(|p| p.hotkey);
        if let Some(h) = current.filter(|h| is_f13_24(h.vk)) {
            crate::set_preset_hotkey(i, Some(crate::config::Hotkey { mods, vk: h.vk }));
        }
        return;
    }
    let mods = menu_mods();
    close_menu();
    match item {
        MenuItem::Apply => crate::apply_preset(i),
        MenuItem::Overwrite => {
            if !preset_matches(i) {
                crate::overwrite_preset(i);
            }
        }
        MenuItem::Rename => start_rename(i),
        MenuItem::Hotkey(None) => crate::set_preset_hotkey(i, None),
        MenuItem::Hotkey(Some(n)) => crate::set_preset_hotkey(i, Some(crate::config::Hotkey { mods, vk: VK_F13 + n })),
        MenuItem::Combo => {
            CAPTURE.store(i, Ordering::Relaxed);
            SetForegroundWindow(hwnd);
        }
        MenuItem::Delete => crate::delete_preset(i),
        MenuItem::Mod(_) => {}
    }
}
/// Records the pressed combination as the hotkey of the preset being captured.
/// Returns false when no capture is in progress.
unsafe fn capture_key(vk: u32) -> bool {
    let i = CAPTURE.load(Ordering::Relaxed);
    if i == usize::MAX {
        return false;
    }
    let down = |k: u16| GetKeyState(k as i32) < 0;
    let is_modifier = matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5);
    if is_modifier {
        return true;
    }
    let mut mods = 0;
    if down(VK_CONTROL) {
        mods |= 0x2;
    }
    if down(VK_MENU) {
        mods |= 0x1;
    }
    if down(VK_SHIFT) {
        mods |= 0x4;
    }
    if down(VK_LWIN) || down(VK_RWIN) {
        mods |= 0x8;
    }
    CAPTURE.store(usize::MAX, Ordering::Relaxed);
    match (vk, mods) {
        // Plain Esc cancels, Backspace/Delete remove the hotkey.
        (0x1B, 0) => {}
        (0x08 | 0x2E, 0) => crate::set_preset_hotkey(i, None),
        _ => crate::set_preset_hotkey(i, Some(crate::config::Hotkey { mods, vk })),
    }
    invalidate();
    true
}

/// Palette click: colors the selected zone, or the whole selected device(s).
fn paint_selected(c: Rgb) {
    let zone = ZONE.load(Ordering::Relaxed);
    match (selected(), selected_zones().get(zone.wrapping_sub(1))) {
        (Some(id), Some(&(led, _))) if zone > 0 => crate::set_zone_color(&id, led, c),
        _ => crate::set_color(&target_ids(), c),
    }
}

// ---------------------------------------------------------------- theme

pub(crate) fn col(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
}

pub(crate) fn rgb_col(c: Rgb) -> D2D1_COLOR_F {
    col(c.0, c.1, c.2, 1.0)
}

#[derive(Clone, Copy)]
pub(crate) struct Theme {
    pub dark: bool,
    pub bg: D2D1_COLOR_F,
    pub card: D2D1_COLOR_F,
    pub card_hover: D2D1_COLOR_F,
    pub stroke: D2D1_COLOR_F,
    pub text: D2D1_COLOR_F,
    pub text2: D2D1_COLOR_F,
    pub accent: D2D1_COLOR_F,
    pub on_accent: D2D1_COLOR_F,
}

fn read_dword(key: &str, value: &str) -> Option<u32> {
    let key: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let value: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let mut data = 0u32;
    let mut size = 4u32;
    let rc = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut data as *mut u32 as _, &mut size)
    };
    (rc == 0).then_some(data)
}

pub(crate) fn theme() -> Theme {
    let dark = read_dword("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize", "AppsUseLightTheme") == Some(0);
    // AccentColor is stored as 0xAABBGGRR.
    let a = read_dword("Software\\Microsoft\\Windows\\DWM", "AccentColor").unwrap_or(0xFFD47800);
    let (mut r, mut g, mut b) = (a as u8, (a >> 8) as u8, (a >> 16) as u8);
    if dark {
        // Lighter accent reads better on dark backgrounds, like Windows does.
        let mix = |c: u8| (c as f32 + (255.0 - c as f32) * 0.35) as u8;
        (r, g, b) = (mix(r), mix(g), mix(b));
    }
    if dark {
        dark_theme(col(r, g, b, 1.0))
    } else {
        light_theme(col(r, g, b, 1.0))
    }
}

fn on_accent(accent: D2D1_COLOR_F) -> D2D1_COLOR_F {
    let luminance = 0.299 * accent.r + 0.587 * accent.g + 0.114 * accent.b;
    if luminance > 0.6 { col(0, 0, 0, 1.0) } else { col(255, 255, 255, 1.0) }
}

fn dark_theme(accent: D2D1_COLOR_F) -> Theme {
    Theme {
        dark: true,
        bg: col(0x20, 0x20, 0x20, 1.0),
        card: col(0x2B, 0x2B, 0x2B, 1.0),
        card_hover: col(0x33, 0x33, 0x33, 1.0),
        stroke: col(255, 255, 255, 0.08),
        text: col(255, 255, 255, 1.0),
        text2: col(255, 255, 255, 0.6),
        accent,
        on_accent: on_accent(accent),
    }
}

fn light_theme(accent: D2D1_COLOR_F) -> Theme {
    Theme {
        dark: false,
        bg: col(0xF3, 0xF3, 0xF3, 1.0),
        card: col(0xFF, 0xFF, 0xFF, 1.0),
        card_hover: col(0xF9, 0xF9, 0xF9, 1.0),
        stroke: col(0, 0, 0, 0.08),
        text: col(0x1A, 0x1A, 0x1A, 1.0),
        text2: col(0, 0, 0, 0.6),
        accent,
        on_accent: on_accent(accent),
    }
}

// ---------------------------------------------------------------- state

/// Device of the selected card; None = "all devices".
static SELECTED: Mutex<Option<String>> = Mutex::new(None);
/// Zone of the selected device the palette paints (0 = whole device).
static ZONE: AtomicUsize = AtomicUsize::new(0);

fn selected() -> Option<String> {
    SELECTED.lock().unwrap().clone()
}

fn select(id: Option<String>) {
    *SELECTED.lock().unwrap() = id;
    ZONE.store(0, Ordering::Relaxed);
}

/// Zones of the device on the selected card.
fn selected_zones() -> &'static [(u8, &'static str)] {
    selected().map_or(&[], |id| kind_of(&id).zones)
}

/// Devices the controls change: the selected one, or every shown one.
fn target_ids() -> Vec<String> {
    selected().map_or_else(crate::active_ids, |id| vec![id])
}

const ZONE_SEG_W: [f32; 3] = [72.0, 72.0, 104.0];

fn zone_rects() -> Vec<R> {
    let total: f32 = ZONE_SEG_W.iter().sum();
    let mut x = W - PAD - total;
    ZONE_SEG_W
        .iter()
        .map(|w| {
            let r = R { x, y: COLOR_LABEL_Y - 5.0, w: *w, h: 28.0 };
            x += w;
            r
        })
        .collect()
}
static PICKING: AtomicBool = AtomicBool::new(false);
/// Keeps the panel open when it loses focus: the pin button (stays on for later opens) or `--show`.
pub static PINNED: AtomicBool = AtomicBool::new(false);
static LAST_CLOSE: AtomicU64 = AtomicU64::new(0);

struct Formats {
    title: IDWriteTextFormat,
    name: IDWriteTextFormat,
    sub: IDWriteTextFormat,
    label: IDWriteTextFormat,
    label_right: IDWriteTextFormat,
    center: IDWriteTextFormat,
    center_small: IDWriteTextFormat,
    sub_right: IDWriteTextFormat,
    icon_tiny: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_small: IDWriteTextFormat,
}

struct Flyout {
    hwnd: HWND,
    scale: f32,
    d2d: ID2D1Factory,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    fmt: Formats,
    theme: Theme,
    hover: Option<Hit>,
    /// Brightness while the slider is being dragged.
    drag: Option<u8>,
    tracking_leave: bool,
    autostart: bool,
    /// The panel has had keyboard focus at least once.
    was_active: bool,
    /// Docked to a bottom taskbar: the panel grows upwards when the device list gets longer.
    anchor_bottom: bool,
}

thread_local! {
    static FLYOUT: RefCell<Option<Flyout>> = const { RefCell::new(None) };
}

fn with_flyout<T>(f: impl FnOnce(&mut Flyout) -> T) -> Option<T> {
    FLYOUT.with(|cell| cell.borrow_mut().as_mut().map(f))
}

fn flyout_hwnd() -> Option<HWND> {
    with_flyout(|f| f.hwnd)
}

fn invalidate() {
    if let Some(h) = flyout_hwnd() {
        unsafe {
            fit_height(h);
            InvalidateRect(h, null(), 0);
        }
    }
}

/// Resizes the panel when the device list changed length, keeping the taskbar side in place.
unsafe fn fit_height(hwnd: HWND) {
    let Some((scale, anchor_bottom)) = with_flyout(|f| (f.scale, f.anchor_bottom)) else { return };
    let want = (panel_h() * scale).round() as i32;
    let mut rc: RECT = zeroed();
    GetWindowRect(hwnd, &mut rc);
    if rc.bottom - rc.top == want {
        return;
    }
    let y = if anchor_bottom { rc.bottom - want } else { rc.top };
    SetWindowPos(hwnd, null_mut(), rc.left, y, rc.right - rc.left, want, SWP_NOZORDER | SWP_NOACTIVATE);
    // The render target is created for the window size.
    with_flyout(|f| {
        f.rt = None;
        f.brush = None;
    });
}

/// Redraws the panel if it is open (e.g. after devices were re-applied).
pub fn refresh() {
    invalidate();
}

pub fn toggle() {
    unsafe {
        if let Some(h) = flyout_hwnd() {
            DestroyWindow(h);
            return;
        }
        // Clicking the tray icon first deactivates (closes) the panel; don't reopen it right away.
        if GetTickCount64().saturating_sub(LAST_CLOSE.load(Ordering::Relaxed)) < 300 {
            return;
        }
        if let Err(e) = open() {
            crate::log(&format!("panel: {e}"));
        }
    }
}

pub(crate) fn text_format(dw: &IDWriteFactory, family: windows::core::PCWSTR, size: f32, weight: DWRITE_FONT_WEIGHT, align: DWRITE_TEXT_ALIGNMENT) -> windows::core::Result<IDWriteTextFormat> {
    unsafe {
        let f = dw.CreateTextFormat(family, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("ru-RU"))?;
        f.SetTextAlignment(align)?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(f)
    }
}

fn formats() -> windows::core::Result<Formats> {
    let dw: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
    let ui = w!("Segoe UI Variable Text");
    let icons = w!("Segoe Fluent Icons");
    let (lead, trail, center) = (DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_ALIGNMENT_CENTER);
    Ok(Formats {
        title: text_format(&dw, w!("Segoe UI Variable Display"), 20.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, lead)?,
        name: text_format(&dw, ui, 14.0, DWRITE_FONT_WEIGHT_NORMAL, lead)?,
        sub: text_format(&dw, ui, 12.0, DWRITE_FONT_WEIGHT_NORMAL, lead)?,
        label: text_format(&dw, ui, 14.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, lead)?,
        label_right: text_format(&dw, ui, 14.0, DWRITE_FONT_WEIGHT_NORMAL, trail)?,
        center: text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, center)?,
        center_small: text_format(&dw, ui, 12.0, DWRITE_FONT_WEIGHT_NORMAL, center)?,
        sub_right: text_format(&dw, ui, 12.0, DWRITE_FONT_WEIGHT_NORMAL, trail)?,
        icon_tiny: text_format(&dw, icons, 12.0, DWRITE_FONT_WEIGHT_NORMAL, center)?,
        icon: text_format(&dw, icons, 18.0, DWRITE_FONT_WEIGHT_NORMAL, center)?,
        icon_small: text_format(&dw, icons, 14.0, DWRITE_FONT_WEIGHT_NORMAL, center)?,
    })
}

unsafe fn open() -> windows::core::Result<()> {
    let hinst = GetModuleHandleW(null());
    let class: Vec<u16> = "RgbTrayFlyout\0".encode_utf16().collect();
    let mut wc: WNDCLASSW = zeroed();
    wc.style = CS_DROPSHADOW;
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinst;
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.lpszClassName = class.as_ptr();
    RegisterClassW(&wc); // fails harmlessly if already registered

    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(monitor, &mut mi);
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    let scale = dpi_x as f32 / 96.0;
    let (w, h) = ((W * scale).round() as i32, (panel_h() * scale).round() as i32);
    let margin = (12.0 * scale) as i32;
    let work = mi.rcWork;
    // Open next to the taskbar, wherever it is docked.
    let mut abd: windows_sys::Win32::UI::Shell::APPBARDATA = zeroed();
    abd.cbSize = size_of::<windows_sys::Win32::UI::Shell::APPBARDATA>() as u32;
    let edge = if windows_sys::Win32::UI::Shell::SHAppBarMessage(windows_sys::Win32::UI::Shell::ABM_GETTASKBARPOS, &mut abd) != 0 {
        abd.uEdge
    } else {
        3 // ABE_BOTTOM
    };
    let near_x = (pt.x - w / 2).clamp(work.left + margin, work.right - w - margin);
    let near_y = (pt.y - h / 2).clamp(work.top + margin, work.bottom - h - margin);
    let (x, y) = match edge {
        0 => (work.left + margin, near_y),      // ABE_LEFT
        1 => (near_x, work.top + margin),       // ABE_TOP
        2 => (work.right - w - margin, near_y), // ABE_RIGHT
        _ => (near_x, work.bottom - h - margin),
    };

    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        class.as_ptr(),
        class.as_ptr(),
        WS_POPUP,
        x,
        y,
        w,
        h,
        null_mut(),
        null_mut(),
        hinst,
        null(),
    );
    let theme = theme();
    // Rounded corners and a dark border in dark mode (Windows 11).
    let round: u32 = 2; // DWMWCP_ROUND
    DwmSetWindowAttribute(hwnd, 33, &round as *const u32 as _, 4);
    let dark: u32 = theme.dark as u32;
    DwmSetWindowAttribute(hwnd, 20, &dark as *const u32 as _, 4);

    let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
    let fly = Flyout {
        hwnd,
        scale,
        d2d,
        rt: None,
        brush: None,
        fmt: formats()?,
        theme,
        hover: None,
        drag: None,
        tracking_leave: false,
        autostart: crate::autostart_enabled(),
        was_active: false,
        anchor_bottom: edge == 3,
    };
    FLYOUT.with(|c| *c.borrow_mut() = Some(fly));
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    SetTimer(hwnd, TIMER_FOCUS, 150, None);
    Ok(())
}

const TIMER_FOCUS: usize = 1;

/// Backup for WM_ACTIVATE: closes the panel once the user works elsewhere, even if
/// Windows refused to give it focus when it opened.
unsafe fn check_focus(hwnd: HWND) {
    if PICKING.load(Ordering::Relaxed) || PINNED.load(Ordering::Relaxed) {
        return;
    }
    let focused = GetForegroundWindow() == hwnd;
    let was_active = with_flyout(|f| {
        f.was_active |= focused;
        f.was_active
    })
    .unwrap_or(false);
    if focused {
        return;
    }
    let clicked = (GetAsyncKeyState(VK_LBUTTON as i32) as u16 & 0x8000) != 0 || (GetAsyncKeyState(VK_RBUTTON as i32) as u16 & 0x8000) != 0;
    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    let mut rc: RECT = zeroed();
    GetWindowRect(hwnd, &mut rc);
    let outside = pt.x < rc.left || pt.x >= rc.right || pt.y < rc.top || pt.y >= rc.bottom;
    if was_active || (clicked && outside) {
        DestroyWindow(hwnd);
    }
}

// ---------------------------------------------------------------- painting

/// What the panel shows about one device.
struct DeviceView {
    id: String,
    cfg: crate::config::DeviceConfig,
    name: String,
    present: bool,
    /// Battery, the last known level when there's no link.
    battery: Option<crate::devices::Battery>,
    no_link: bool,
}

/// Snapshot of everything the panel shows.
struct View {
    rows: Vec<(R, Row)>,
    /// Shown and archived devices.
    devices: Vec<DeviceView>,
    brightness: u8,
    selected: Option<String>,
    /// Top of the section under the device list.
    lower_y: f32,
}

fn view() -> View {
    let rows = rows();
    let lower_y = lower_y_of(&rows);
    let selected = selected();
    let cfg = crate::config();
    let present = crate::PRESENT.lock().unwrap().clone();
    let status = crate::STATUS.lock().unwrap().clone();
    let no_link = crate::NO_LINK.lock().unwrap().clone();
    let devices = rows
        .iter()
        .filter_map(|(_, row)| match row {
            Row::Device(id) | Row::Archived(id) => Some(id.clone()),
            _ => None,
        })
        .map(|id| DeviceView {
            cfg: cfg.device(&id),
            name: cfg.display_name(&id),
            present: present.contains(&id),
            battery: status.iter().find(|(s, _)| *s == id).map(|(_, b)| *b),
            no_link: no_link.contains(&id),
            id,
        })
        .collect();
    View {
        rows,
        devices,
        // The slider edits the overall brightness, or the selected device's share of it.
        brightness: selected.as_ref().map_or(cfg.brightness, |id| cfg.device(id).brightness),
        selected,
        lower_y,
    }
}

impl View {
    fn device(&self, id: &str) -> Option<&DeviceView> {
        self.devices.iter().find(|d| d.id == id)
    }

    /// Device configs the controls currently edit.
    fn targets(&self) -> Vec<&crate::config::DeviceConfig> {
        match &self.selected {
            Some(id) => self.device(id).map(|d| &d.cfg).into_iter().collect(),
            None => self.devices.iter().filter(|d| d.present || d.no_link).map(|d| &d.cfg).collect(),
        }
    }

    fn common_effect(&self) -> Option<Effect> {
        let t = self.targets();
        // Per-key colors are not one of the effects.
        if t.is_empty() || t.iter().any(|d| d.per_key) {
            return None;
        }
        t.iter().all(|d| d.effect == t[0].effect).then_some(t[0].effect)
    }

    fn common_color(&self) -> Option<Rgb> {
        let zone = ZONE.load(Ordering::Relaxed);
        if let (Some(id), true) = (&self.selected, zone > 0) {
            if let (Some(&(led, _)), Some(d)) = (selected_zones().get(zone - 1), self.device(id)) {
                return Some(d.cfg.zone_rgb(led));
            }
        }
        let t = self.targets();
        let first = t.first()?;
        t.iter().all(|d| d.color == first.color).then(|| first.rgb())
    }
}
impl Flyout {
    fn ensure_target(&mut self) -> windows::core::Result<()> {
        if self.rt.is_some() {
            return Ok(());
        }
        unsafe {
            let mut rc: RECT = zeroed();
            GetClientRect(self.hwnd, &mut rc);
            let dpi = 96.0 * self.scale;
            let props = D2D1_RENDER_TARGET_PROPERTIES { dpiX: dpi, dpiY: dpi, ..Default::default() };
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: WinHwnd(self.hwnd),
                pixelSize: D2D_SIZE_U { width: (rc.right - rc.left) as u32, height: (rc.bottom - rc.top) as u32 },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let rt = self.d2d.CreateHwndRenderTarget(&props, &hwnd_props)?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            self.brush = Some(rt.CreateSolidColorBrush(&self.theme.text, None)?);
            self.rt = Some(rt);
        }
        Ok(())
    }

    fn paint(&mut self) {
        if self.ensure_target().is_err() {
            return;
        }
        let rt = self.rt.clone().unwrap();
        let v = view();
        unsafe {
            rt.BeginDraw();
            self.draw(&rt, &v);
            if let Err(e) = rt.EndDraw(None, None) {
                if e.code() == windows::Win32::Foundation::D2DERR_RECREATE_TARGET {
                    self.rt = None;
                    self.brush = None;
                }
            }
        }
    }

    unsafe fn fill(&self, rt: &ID2D1RenderTarget, r: R, radius: f32, c: D2D1_COLOR_F) {
        let b = self.brush.as_ref().unwrap();
        b.SetColor(&c);
        rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius }, b);
    }

    unsafe fn stroke(&self, rt: &ID2D1RenderTarget, r: R, radius: f32, c: D2D1_COLOR_F, width: f32) {
        let b = self.brush.as_ref().unwrap();
        b.SetColor(&c);
        let r = r.inset(width / 2.0);
        rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius }, b, width, None);
    }

    unsafe fn circle(&self, rt: &ID2D1RenderTarget, x: f32, y: f32, radius: f32, c: D2D1_COLOR_F) {
        let b = self.brush.as_ref().unwrap();
        b.SetColor(&c);
        rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x, y }, radiusX: radius, radiusY: radius }, b);
    }

    unsafe fn ring(&self, rt: &ID2D1RenderTarget, x: f32, y: f32, radius: f32, c: D2D1_COLOR_F, width: f32) {
        let b = self.brush.as_ref().unwrap();
        b.SetColor(&c);
        rt.DrawEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x, y }, radiusX: radius, radiusY: radius }, b, width, None);
    }

    unsafe fn text(&self, rt: &ID2D1RenderTarget, s: &str, fmt: &IDWriteTextFormat, r: R, c: D2D1_COLOR_F) {
        let b = self.brush.as_ref().unwrap();
        b.SetColor(&c);
        let s: Vec<u16> = s.encode_utf16().collect();
        rt.DrawText(&s, fmt, &r.d2d(), b, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
    }

    /// Color dot: solid color, rainbow for the spectrum effect, hollow ring when off.
    unsafe fn dot(&self, rt: &ID2D1RenderTarget, x: f32, y: f32, radius: f32, d: &crate::config::DeviceConfig) {
        let t = &self.theme;
        match d.effect {
            Effect::Off => self.ring(rt, x, y, radius - 1.0, t.text2, 1.5),
            Effect::Cycle => {
                let stops: Vec<D2D1_GRADIENT_STOP> = [(0.0, (255, 40, 40)), (0.33, (255, 220, 0)), (0.66, (0, 200, 255)), (1.0, (180, 60, 255))]
                    .iter()
                    .map(|&(p, (r, g, b))| D2D1_GRADIENT_STOP { position: p, color: col(r, g, b, 1.0) })
                    .collect();
                if let Ok(coll) = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) {
                    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: D2D_POINT_2F { x: x - radius, y: y - radius },
                        endPoint: D2D_POINT_2F { x: x + radius, y: y + radius },
                    };
                    if let Ok(g) = rt.CreateLinearGradientBrush(&props, None, &coll) {
                        rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x, y }, radiusX: radius, radiusY: radius }, &g);
                    }
                }
            }
            _ => {
                self.circle(rt, x, y, radius, rgb_col(d.rgb()));
                self.ring(rt, x, y, radius - 0.5, t.stroke, 1.0);
            }
        }
    }

    unsafe fn draw(&self, rt: &ID2D1RenderTarget, v: &View) {
        let t = &self.theme;
        let f = &self.fmt;
        let hover = |h: Hit| self.hover == Some(h);
        rt.Clear(Some(&t.bg));

        // Header.
        self.text(rt, "Подсветка", &f.title, R { x: PAD, y: 12.0, w: 200.0, h: 32.0 }, t.text);
        let refresh = R { x: W - PAD - 32.0, y: 12.0, w: 32.0, h: 32.0 };
        if hover(Hit::Reapply) {
            self.fill(rt, refresh, 4.0, t.card_hover);
        }
        self.text(rt, ICON_REFRESH, &f.icon_small, refresh, t.text);
        // Pinned: the panel stays open and on top while working elsewhere.
        let pin = pin_rect();
        let pinned = PINNED.load(Ordering::Relaxed);
        if pinned {
            self.fill(rt, pin, 4.0, D2D1_COLOR_F { a: 0.2, ..t.accent });
        } else if hover(Hit::Pin) {
            self.fill(rt, pin, 4.0, t.card_hover);
        }
        self.text(rt, if pinned { ICON_PINNED } else { ICON_PIN }, &f.icon_small, pin, if pinned { t.accent } else { t.text });

        // Device list.
        for (i, (r, row)) in v.rows.iter().enumerate() {
            self.draw_row(rt, v, i, *r, row);
        }

        // Everything below the list is laid out relative to its top.
        rt.SetTransform(&Matrix3x2::translation(0.0, v.lower_y));
        // Color palette.
        self.text(rt, "Цвет", &f.label, R { x: PAD, y: COLOR_LABEL_Y, w: 200.0, h: 20.0 }, t.text);
        let zones = selected_zones();
        if !zones.is_empty() {
            // Which zone the palette paints: whole device or one zone.
            let zone = ZONE.load(Ordering::Relaxed);
            let rects = zone_rects();
            let all = R { x: rects[0].x, y: rects[0].y, w: rects.iter().map(|r| r.w).sum(), h: rects[0].h };
            self.fill(rt, all, 4.0, t.card);
            self.stroke(rt, all, 4.0, t.stroke, 1.0);
            let names = std::iter::once("Вся мышь").chain(zones.iter().map(|z| z.1));
            for (k, (name, r)) in names.zip(rects).enumerate() {
                let r = r.inset(3.0);
                if zone == k {
                    self.fill(rt, r, 3.0, t.accent);
                } else if hover(Hit::Zone(k)) {
                    self.fill(rt, r, 3.0, t.card_hover);
                }
                self.text(rt, name, &f.center_small, r, if zone == k { t.on_accent } else { t.text });
            }
        }
        let current = v.common_color();
        let static_like = matches!(v.common_effect(), Some(Effect::Static | Effect::Breathe));
        for (i, c) in PALETTE.iter().enumerate() {
            let r = swatch_rect(i);
            let chosen = static_like && current == Some(*c);
            if chosen || hover(Hit::Swatch(i)) {
                self.ring(rt, r.cx(), r.cy(), r.w / 2.0, if chosen { t.text } else { t.text2 }, 2.0);
            }
            self.circle(rt, r.cx(), r.cy(), r.w / 2.0 - 4.0, rgb_col(*c));
            self.ring(rt, r.cx(), r.cy(), r.w / 2.0 - 4.5, t.stroke, 1.0);
        }
        let r = swatch_rect(PALETTE.len());
        let custom = current.filter(|c| static_like && !PALETTE.contains(c));
        match custom {
            Some(c) => {
                self.ring(rt, r.cx(), r.cy(), r.w / 2.0, t.text, 2.0);
                self.circle(rt, r.cx(), r.cy(), r.w / 2.0 - 4.0, rgb_col(c));
            }
            None => {
                self.fill(rt, r.inset(2.0), r.w / 2.0 - 2.0, if hover(Hit::Custom) { t.card_hover } else { t.card });
                self.ring(rt, r.cx(), r.cy(), r.w / 2.0 - 2.5, t.stroke, 1.0);
                self.text(rt, ICON_ADD, &f.icon_small, r, t.text);
            }
        }

        // Effect segmented control.
        self.text(rt, "Эффект", &f.label, R { x: PAD, y: EFFECT_LABEL_Y, w: 200.0, h: 20.0 }, t.text);
        let seg = R { x: PAD, y: SEG_Y, w: W - 2.0 * PAD, h: SEG_H };
        self.fill(rt, seg, 6.0, t.card);
        self.stroke(rt, seg, 6.0, t.stroke, 1.0);
        let seg_w = seg.w / 4.0;
        let effect = v.common_effect();
        for (i, label) in EFFECT_LABELS.iter().enumerate() {
            let r = R { x: seg.x + i as f32 * seg_w, y: seg.y, w: seg_w, h: seg.h }.inset(3.0);
            let on = effect == Some(Effect::ALL[i]);
            if on {
                self.fill(rt, r, 4.0, t.accent);
            } else if hover(Hit::Effect(i)) {
                self.fill(rt, r, 4.0, t.card_hover);
            }
            self.text(rt, label, &f.center, r, if on { t.on_accent } else { t.text });
        }

        // Brightness slider.
        let brightness = self.drag.unwrap_or(v.brightness);
        let (label, value) = if v.selected.is_none() {
            ("Общая яркость", format!("{brightness}%"))
        } else {
            ("Яркость устройства", format!("{brightness}% от общей"))
        };
        self.text(rt, label, &f.label, R { x: PAD, y: BRIGHT_LABEL_Y, w: 200.0, h: 20.0 }, t.text);
        self.text(rt, &value, &f.label_right, R { x: W - PAD - 140.0, y: BRIGHT_LABEL_Y, w: 140.0, h: 20.0 }, t.text2);
        let s = slider_rect();
        let (x0, x1) = (s.x + 10.0, s.x + s.w - 10.0);
        let tx = x0 + (x1 - x0) * brightness as f32 / 100.0;
        self.fill(rt, R { x: x0, y: s.cy() - 2.0, w: x1 - x0, h: 4.0 }, 2.0, t.text2);
        self.fill(rt, R { x: x0, y: s.cy() - 2.0, w: tx - x0, h: 4.0 }, 2.0, t.accent);
        self.circle(rt, tx, s.cy(), 10.0, if t.dark { col(0x45, 0x45, 0x45, 1.0) } else { col(255, 255, 255, 1.0) });
        self.ring(rt, tx, s.cy(), 9.5, t.stroke, 1.0);
        let inner = if self.drag.is_some() { 4.5 } else if hover(Hit::Slider) { 6.5 } else { 5.5 };
        self.circle(rt, tx, s.cy(), inner, t.accent);

        // Global presets.
        self.text(rt, "Пресеты", &f.label, R { x: PAD, y: PRESET_LABEL_Y, w: 120.0, h: 20.0 }, t.text);
        let capture = CAPTURE.load(Ordering::Relaxed);
        if capture != usize::MAX {
            self.text(rt, "Нажмите сочетание · Del — убрать · Esc", &f.sub_right, R { x: W - PAD - 250.0, y: PRESET_LABEL_Y, w: 250.0, h: 20.0 }, t.accent);
        } else if renaming_any() {
            self.text(rt, "Enter — сохранить · Esc — отмена", &f.sub_right, R { x: W - PAD - 250.0, y: PRESET_LABEL_Y, w: 250.0, h: 20.0 }, t.accent);
        } else {
            self.text(rt, "ПКМ по пресету — меню", &f.sub_right, R { x: W - PAD - 200.0, y: PRESET_LABEL_Y, w: 200.0, h: 20.0 }, t.text2);
        }
        let active: Vec<bool> = {
            let cfg = crate::config();
            cfg.presets.iter().map(|p| cfg.matches(p)).collect()
        };
        let renaming = RENAME.load(Ordering::Relaxed);
        let chips = preset_chips();
        let is_on = |i: usize| active.get(i) == Some(&true) && renaming != i;
        // Pass 1: chips with their labels (or the rename field).
        for (r, hit, label) in chips.iter().filter(|c| matches!(c.1, Hit::Preset(_) | Hit::SavePreset)) {
            let (r, hit) = (*r, *hit);
            let i = match hit {
                Hit::Preset(i) => i,
                _ => usize::MAX,
            };
            let is_preset = i != usize::MAX;
            let editing = is_preset && i == renaming;
            let highlighted = is_preset && (i == capture || editing);
            if is_on(i) {
                self.fill(rt, r, 4.0, t.accent);
            } else {
                self.fill(rt, r, 4.0, if hover(hit) && !editing { t.card_hover } else { t.card });
                self.stroke(rt, r, 4.0, if highlighted { t.accent } else { t.stroke }, if highlighted { 2.0 } else { 1.0 });
            }
            let text_r = if i == usize::MAX { r } else { R { w: r.w - PRESET_ICONS_W + 6.0, ..r } };
            if editing {
                let inner = R { x: r.x + 10.0, w: r.w - PRESET_ICONS_W - 10.0, ..r };
                self.text(rt, label, &f.sub, inner, t.text);
                let caret_x = inner.x + (label.chars().count() as f32 * 6.9).min(inner.w - 2.0);
                self.fill(rt, R { x: caret_x, y: r.y + 6.0, w: 1.5, h: r.h - 12.0 }, 0.0, t.text);
            } else {
                let fg = if is_on(i) { t.on_accent } else if hit == Hit::SavePreset { t.text2 } else { t.text };
                self.text(rt, label, &f.center_small, text_r, fg);
            }
        }
        // Pass 2: the small icon buttons on top of their chips.
        for (r, hit, _) in chips.iter().filter(|c| matches!(c.1, Hit::PresetOverwrite(_) | Hit::PresetHotkey(_) | Hit::PresetRename(_))) {
            let (r, hit) = (*r, *hit);
            let (i, glyph) = match hit {
                Hit::PresetOverwrite(i) => (i, ICON_SAVE),
                Hit::PresetHotkey(i) => (i, "\u{E765}"),
                Hit::PresetRename(i) => (i, "\u{E70F}"),
                _ => continue,
            };
            let on = is_on(i);
            // Nothing to overwrite while the preset is exactly what is shown.
            let disabled = matches!(hit, Hit::PresetOverwrite(_)) && active.get(i) == Some(&true);
            if hover(hit) && !disabled {
                self.fill(rt, r, 3.0, if on { col(255, 255, 255, 0.25) } else { t.card_hover });
            }
            let active_btn = (hit == Hit::PresetHotkey(i) && capture == i) || (hit == Hit::PresetRename(i) && renaming == i);
            let mut fg = if active_btn { t.accent } else if on { t.on_accent } else { t.text2 };
            if disabled {
                fg.a *= 0.4;
            }
            self.text(rt, glyph, &f.icon_tiny, r, fg);
        }

        // Footer.
        self.fill(rt, R { x: 0.0, y: DIVIDER_Y, w: W, h: 1.0 }, 0.0, t.stroke);
        let sw = R { x: PAD, y: FOOTER_Y + 8.0, w: 40.0, h: 20.0 };
        if self.autostart {
            self.fill(rt, sw, 10.0, t.accent);
            self.circle(rt, sw.x + sw.w - 10.0, sw.cy(), if hover(Hit::Autostart) { 7.0 } else { 6.0 }, t.on_accent);
        } else {
            self.fill(rt, sw, 10.0, if hover(Hit::Autostart) { t.card_hover } else { t.card });
            self.stroke(rt, sw, 10.0, t.text2, 1.0);
            self.circle(rt, sw.x + 10.0, sw.cy(), if hover(Hit::Autostart) { 6.0 } else { 5.0 }, t.text2);
        }
        self.text(rt, "Запускать вместе с Windows", &f.sub, R { x: sw.x + 50.0, y: FOOTER_Y, w: 200.0, h: 36.0 }, t.text);
        let exit = R { x: W - PAD - 92.0, y: FOOTER_Y, w: 92.0, h: 36.0 };
        if hover(Hit::Exit) {
            self.fill(rt, exit, 4.0, t.card_hover);
        }
        self.text(rt, ICON_POWER, &f.icon_small, R { x: exit.x + 6.0, y: exit.y, w: 24.0, h: exit.h }, t.text);
        self.text(rt, "Выход", &f.sub, R { x: exit.x + 34.0, y: exit.y, w: 56.0, h: exit.h }, t.text);

        rt.SetTransform(&Matrix3x2::identity());
        self.draw_menu(rt);
    }

    /// One row of the device list.
    unsafe fn draw_row(&self, rt: &ID2D1RenderTarget, v: &View, i: usize, r: R, row: &Row) {
        let t = &self.theme;
        let f = &self.fmt;
        let hover = |h: Hit| self.hover == Some(h);

        if let Row::ServiceWarning = row {
            // Accent-tinted card: a Windows service is fighting over the keyboard.
            self.fill(rt, r, 6.0, t.card);
            self.fill(rt, r, 6.0, D2D1_COLOR_F { a: 0.14, ..t.accent });
            self.stroke(rt, r, 6.0, t.accent, 1.0);
            self.text(rt, ICON_WARNING, &f.icon, R { x: r.x + 10.0, y: r.y, w: 32.0, h: r.h }, t.accent);
            let text_w = disable_button_rect(r).x - r.x - 58.0;
            self.text(rt, "Мешает служба Logitech", &f.name, R { x: r.x + 50.0, y: r.y + 10.0, w: text_w, h: 20.0 }, t.text);
            self.text(rt, "Из-за неё G915 зависает", &f.sub, R { x: r.x + 50.0, y: r.y + 30.0, w: text_w, h: 16.0 }, t.text2);
            self.text(rt, "и не слушает эффекты", &f.sub, R { x: r.x + 50.0, y: r.y + 44.0, w: text_w, h: 16.0 }, t.text2);
            let b = disable_button_rect(r);
            self.fill(rt, b, 4.0, if hover(Hit::DisableService) { D2D1_COLOR_F { a: 0.85, ..t.accent } } else { t.accent });
            self.text(rt, "Отключить", &f.center_small, b, t.on_accent);
            return;
        }
        if let Row::ArchiveHeader(n) = row {
            // A plain foldable line, not a card.
            if hover(Hit::Card(i)) {
                self.fill(rt, r, 6.0, t.card_hover);
            }
            let open = ARCHIVE_OPEN.load(Ordering::Relaxed);
            self.text(rt, &format!("Отключённые устройства · {n}"), &f.sub, R { x: r.x + 14.0, y: r.y, w: r.w - 50.0, h: r.h }, t.text2);
            let chevron = if open { ICON_CHEVRON_UP } else { ICON_CHEVRON_DOWN };
            self.text(rt, chevron, &f.icon_tiny, R { x: r.x + r.w - 36.0, y: r.y, w: 24.0, h: r.h }, t.text2);
            return;
        }

        let id = match row {
            Row::Device(id) | Row::Archived(id) => Some(id.as_str()),
            _ => None,
        };
        let selected = v.selected.as_deref() == id;
        let mut bg = if hover(Hit::Card(i)) { t.card_hover } else { t.card };
        if selected {
            bg = D2D1_COLOR_F { a: 0.18, ..t.accent };
            self.fill(rt, r, 6.0, if hover(Hit::Card(i)) { t.card_hover } else { t.card });
        }
        self.fill(rt, r, 6.0, bg);
        self.stroke(rt, r, 6.0, if selected { t.accent } else { t.stroke }, 1.0);
        if selected {
            // Accent pill on the left edge, like Windows 11 navigation.
            self.fill(rt, R { x: r.x + 4.0, y: r.cy() - 8.0, w: 3.0, h: 16.0 }, 1.5, t.accent);
        }

        let (dx, dy) = (r.x + r.w - 26.0, r.cy());
        let Some(d) = id.and_then(|id| v.device(id)) else {
            // "All devices": small overlapping dots, one per shown device.
            let shown: Vec<&DeviceView> = v.devices.iter().filter(|d| d.present || d.no_link).collect();
            let n = shown.iter().filter(|d| d.present).count();
            self.text(rt, ICON_ALL, &f.icon, R { x: r.x + 10.0, y: r.y, w: 32.0, h: r.h }, t.text);
            self.text(rt, "Все устройства", &f.name, R { x: r.x + 50.0, y: r.y + 6.0, w: r.w - 100.0, h: 20.0 }, t.text);
            let sub = if n == shown.len() { format!("Подключено {n}") } else { format!("Подключено {n} из {}", shown.len()) };
            self.text(rt, &sub, &f.sub, R { x: r.x + 50.0, y: r.y + 26.0, w: r.w - 100.0, h: 16.0 }, t.text2);
            for (k, d) in shown.iter().enumerate().take(8).rev() {
                self.circle(rt, dx - k as f32 * 9.0, dy, 8.0, if selected { t.card } else { bg });
                self.dot(rt, dx - k as f32 * 9.0, dy, 7.0, &d.cfg);
            }
            return;
        };
        let kind = kind_of(&d.id);
        let archived = matches!(row, Row::Archived(_));

        // Subtitle parts, joined with " · "; (icon, text) where icon may be empty.
        let mut sub = vec![(
            "",
            if d.no_link {
                "Нет связи".to_string()
            } else if !d.present {
                last_seen_label(d.cfg.last_seen)
            } else if d.cfg.per_key {
                "Свои цвета клавиш".to_string()
            } else {
                d.cfg.effect.label().to_string()
            },
        )];
        if d.present && d.cfg.brightness < 100 && d.cfg.effect != Effect::Off {
            sub.push((ICON_BRIGHTNESS, format!("{}%", d.cfg.brightness)));
        }
        if let (true, Some(b)) = (d.present, d.battery) {
            sub.push(("", b.label()));
        }
        // The level from before the link was lost.
        if let (true, Some(b)) = (d.no_link, d.battery) {
            sub.push(("", format!("было {}%", b.percent)));
        }
        let low = d.present && d.battery.is_some_and(|b| b.is_low());
        let name_color = if d.present { t.text } else { t.text2 };
        self.text(rt, kind.icon, &f.icon, R { x: r.x + 10.0, y: r.y, w: 32.0, h: r.h }, name_color);
        // Buttons on the name line (keyboard editor, forget) leave less room for the name.
        let name_r = name_rect(r, row);
        if device_renaming().as_deref() == Some(d.id.as_str()) {
            // Inline name field with a caret.
            let field = R { x: name_r.x - 6.0, y: name_r.y - 2.0, w: name_r.w + 6.0, h: name_r.h + 4.0 };
            self.fill(rt, field, 4.0, t.bg);
            self.stroke(rt, field, 4.0, t.accent, 2.0);
            let text = RENAME_TEXT.lock().unwrap().clone();
            self.text(rt, &text, &f.name, name_r, t.text);
            let caret_x = name_r.x + text_width(&text, &f.name).min(name_r.w - 2.0);
            self.fill(rt, R { x: caret_x, y: field.y + 4.0, w: 1.5, h: field.h - 8.0 }, 0.0, t.text);
        } else {
            self.text(rt, &d.name, &f.name, name_r, name_color);
            let pencil = pencil_rect(r, row, &d.name);
            if hover(Hit::RenameDevice(i)) {
                self.fill(rt, pencil, 3.0, t.card_hover);
            }
            let fg = if hover(Hit::RenameDevice(i)) { t.text } else { t.text2 };
            self.text(rt, ICON_RENAME, &f.icon_tiny, pencil, fg);
        }
        let sub_color = if low { t.accent } else { t.text2 };
        let mut x = r.x + 50.0;
        for (k, (ic, s)) in sub.iter().enumerate() {
            let s = if k > 0 { format!(" · {s}") } else { s.clone() };
            let (sep, rest) = if !ic.is_empty() { s.split_at(s.len() - s.trim_start_matches([' ', '·']).len()) } else { ("", s.as_str()) };
            if !sep.is_empty() {
                self.text(rt, sep, &f.sub, R { x, y: r.y + 26.0, w: 60.0, h: 16.0 }, sub_color);
                x += text_width(sep, &f.sub);
            }
            if !ic.is_empty() {
                self.text(rt, ic, &f.icon_tiny, R { x, y: r.y + 27.0, w: 14.0, h: 16.0 }, sub_color);
                x += 16.0;
            }
            self.text(rt, rest, &f.sub, R { x, y: r.y + 26.0, w: r.w - 60.0, h: 16.0 }, sub_color);
            x += text_width(rest, &f.sub);
        }

        let mut dc = d.cfg.clone();
        if dc.per_key {
            dc.effect = Effect::Cycle; // multicolor dot
        }
        if !dc.zones.is_empty() && matches!(dc.effect, Effect::Static | Effect::Breathe) {
            // One small dot per zone.
            for (k, &(led, _)) in kind.zones.iter().enumerate().rev() {
                let mut z = dc.clone();
                z.set_rgb(dc.zone_rgb(led));
                self.circle(rt, dx - k as f32 * 9.0, dy, 8.0, if selected { t.card } else { bg });
                self.dot(rt, dx - k as f32 * 9.0, dy, 7.0, &z);
            }
        } else {
            self.dot(rt, dx, dy, 9.0, &dc);
        }

        match row {
            Row::Device(_) if kind.per_key => {
                let kb = keys_button_rect(r);
                self.fill(rt, kb, 4.0, if hover(Hit::Keys(i)) { t.card_hover } else { t.bg });
                self.stroke(rt, kb, 4.0, t.stroke, 1.0);
                self.text(rt, ICON_KEYBOARD, &f.icon_tiny, R { x: kb.x + 4.0, y: kb.y, w: 22.0, h: kb.h }, t.text);
                self.text(rt, "Клавиши", &f.sub, R { x: kb.x + 26.0, y: kb.y, w: kb.w - 28.0, h: kb.h }, t.text);
            }
            Row::Archived(id) if archived => {
                // Forgetting asks for a second click.
                let fb = forget_button_rect(r);
                let armed = FORGET_ARMED.lock().unwrap().as_deref() == Some(id.as_str());
                if armed {
                    self.fill(rt, fb, 4.0, t.accent);
                } else {
                    self.fill(rt, fb, 4.0, if hover(Hit::Forget(i)) { t.card_hover } else { t.bg });
                    self.stroke(rt, fb, 4.0, t.stroke, 1.0);
                }
                let fg = if armed { t.on_accent } else { t.text };
                let label = if armed { "Точно?" } else { "Забыть" };
                self.text(rt, ICON_DELETE, &f.icon_tiny, R { x: fb.x + 4.0, y: fb.y, w: 22.0, h: fb.h }, fg);
                self.text(rt, label, &f.sub, R { x: fb.x + 26.0, y: fb.y, w: fb.w - 28.0, h: fb.h }, fg);
            }
            _ => {}
        }
    }

    /// The preset menu: a raised card over a dimmed panel.
    unsafe fn draw_menu(&self, rt: &ID2D1RenderTarget) {
        let (Some((panel, items)), Some(i)) = (menu_layout(), menu_preset()) else { return };
        let Some(preset) = crate::config().presets.get(i).cloned() else { return };
        let t = &self.theme;
        let f = &self.fmt;
        let hover = |h: Hit| self.hover == Some(h);

        self.fill(rt, R { x: 0.0, y: 0.0, w: W, h: panel_h() }, 0.0, col(0, 0, 0, if t.dark { 0.35 } else { 0.18 }));
        // Soft shadow: a few widening translucent layers.
        for k in 1..=6 {
            let d = k as f32 * 2.0;
            self.fill(rt, R { x: panel.x - d + 1.0, y: panel.y - d + 4.0, w: panel.w + 2.0 * d - 2.0, h: panel.h + 2.0 * d - 2.0 }, 8.0 + d, col(0, 0, 0, 0.05));
        }
        self.fill(rt, panel, 8.0, t.bg);
        self.fill(rt, panel, 8.0, t.card);
        self.stroke(rt, panel, 8.0, t.stroke, 1.0);

        // Header: preset name and its hotkey.
        let head = R { x: panel.x + 14.0, y: panel.y + 6.0, w: panel.w - 28.0, h: MENU_HEADER_H };
        self.text(rt, &preset.name, &f.label, R { w: head.w - 90.0, ..head }, t.text);
        if let Some(hk) = preset.hotkey {
            self.text(rt, &hk.label(), &f.sub_right, head, t.text2);
        }

        let same = preset_matches(i);
        let mut line_after_actions = 0.0;
        for &(r, item) in &items {
            let h = Hit::Menu(item);
            match item {
                MenuItem::Apply | MenuItem::Overwrite | MenuItem::Rename | MenuItem::Delete => {
                    let disabled = item == MenuItem::Overwrite && same;
                    if hover(h) && !disabled {
                        self.fill(rt, r, 4.0, t.card_hover);
                    }
                    let (icon, label) = match item {
                        MenuItem::Apply => (ICON_PLAY, "Применить"),
                        MenuItem::Overwrite => (ICON_SAVE, if same { "Совпадает с текущими" } else { "Перезаписать текущими" }),
                        MenuItem::Rename => (ICON_RENAME, "Переименовать"),
                        _ => (ICON_DELETE, "Удалить"),
                    };
                    let mut fg = t.text;
                    if disabled {
                        fg = t.text2;
                        fg.a *= 0.6;
                    }
                    self.text(rt, icon, &f.icon_small, R { x: r.x + 6.0, y: r.y, w: 28.0, h: r.h }, fg);
                    self.text(rt, label, &f.sub, R { x: r.x + 40.0, y: r.y, w: r.w - 44.0, h: r.h }, fg);
                    if item == MenuItem::Rename {
                        line_after_actions = r.y + r.h;
                    }
                    if item == MenuItem::Delete {
                        self.fill(rt, R { x: panel.x, y: r.y - MENU_GAP / 2.0 - 0.5, w: panel.w, h: 1.0 }, 0.0, t.stroke);
                    }
                }
                MenuItem::Mod(bit) => {
                    let on = menu_mods() & bit != 0;
                    if on {
                        self.fill(rt, r, 4.0, D2D1_COLOR_F { a: 0.25, ..t.accent });
                        self.stroke(rt, r, 4.0, t.accent, 1.0);
                    } else {
                        self.fill(rt, r, 4.0, if hover(h) { t.card_hover } else { t.bg });
                        self.stroke(rt, r, 4.0, t.stroke, 1.0);
                    }
                    let label = MENU_MODS.iter().find(|m| m.0 == bit).map_or("", |m| m.1);
                    self.text(rt, label, &f.center_small, r, if on { t.accent } else { t.text2 });
                }
                MenuItem::Hotkey(_) | MenuItem::Combo => {
                    let current = match item {
                        MenuItem::Hotkey(Some(n)) => preset.hotkey.is_some_and(|h| h.vk == VK_F13 + n),
                        MenuItem::Hotkey(None) => preset.hotkey.is_none(),
                        _ => preset.hotkey.is_some_and(|h| !is_f13_24(h.vk)),
                    };
                    if current {
                        self.fill(rt, r, 4.0, t.accent);
                    } else {
                        self.fill(rt, r, 4.0, if hover(h) { t.card_hover } else { t.bg });
                        self.stroke(rt, r, 4.0, t.stroke, 1.0);
                    }
                    let fg = if current { t.on_accent } else { t.text };
                    let label = match item {
                        MenuItem::Hotkey(Some(n)) => format!("F{}", 13 + n),
                        MenuItem::Hotkey(None) => "Нет".to_string(),
                        _ => match preset.hotkey {
                            Some(hk) if current => hk.label(),
                            _ => "Сочетание…".to_string(),
                        },
                    };
                    if item == MenuItem::Combo {
                        self.text(rt, ICON_KEYBOARD, &f.icon_tiny, R { x: r.x + 6.0, y: r.y, w: 20.0, h: r.h }, fg);
                        self.text(rt, &label, &f.center_small, R { x: r.x + 22.0, w: r.w - 26.0, ..r }, fg);
                    } else {
                        self.text(rt, &label, &f.center_small, r, fg);
                    }
                }
            }
        }
        // Divider and the hotkey section title.
        self.fill(rt, R { x: panel.x, y: line_after_actions + MENU_GAP / 2.0 - 0.5, w: panel.w, h: 1.0 }, 0.0, t.stroke);
        self.text(
            rt,
            "Горячая клавиша",
            &f.sub,
            R { x: panel.x + 14.0, y: line_after_actions + MENU_GAP, w: panel.w - 28.0, h: MENU_LABEL_H - 4.0 },
            t.text2,
        );
    }
}

/// Renders the panel into a PNG file (no window needed), for checking the look.
/// `dark`: force the dark (`Some(true)`) or light theme instead of the system one.
pub fn snapshot(path: &str, dark: Option<bool>, scale: f32, card: usize, zone: usize) -> windows::core::Result<()> {
    if std::env::var_os("RGBTRAY_DEMO_ARCHIVE").is_some() {
        ARCHIVE_OPEN.store(true, Ordering::Relaxed);
    }
    match rows().get(card).map(|r| r.1.clone()) {
        Some(Row::Device(id) | Row::Archived(id)) => select(Some(id)),
        _ => select(None),
    }
    ZONE.store(zone, Ordering::Relaxed);
    {
        // Demo presets in memory only (never saved) so the row isn't empty.
        let mut cfg = crate::config();
        if std::env::var_os("RGBTRAY_DEMO_ARCHIVE").is_some() {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let d = crate::config::DeviceConfig { name: "Logitech G915 TKL 2".into(), last_seen: now - 2 * 86_400, ..cfg.device("x") };
            cfg.devices.insert("logitech-DEMO0001".into(), d);
        }
        if std::env::var_os("RGBTRAY_DEMO_DIM").is_some() {
            if let Some(d) = cfg.devices.iter_mut().find(|(id, _)| id.starts_with("logitech")) {
                d.1.brightness = 30;
            }
        }

        if cfg.presets.is_empty() {
            let mut a = cfg.snapshot("Работа".into());
            a.hotkey = Some(crate::config::Hotkey { mods: 0, vk: VK_F13 });
            let mut b = cfg.snapshot("Игра".into());
            b.brightness = 40;
            b.hotkey = Some(crate::config::Hotkey { mods: 0x2 | 0x1, vk: 0x31 });
            let c = cfg.snapshot("Ночь".into());
            cfg.presets = vec![a, b, c];
            cfg.presets[2].brightness = 10;
        }
    }
    if std::env::var_os("RGBTRAY_DEMO_MENU").is_some() {
        open_menu(0, PAD + 40.0, lower_y() + PRESET_Y + 20.0);
    }
    render_png(path, W, panel_h(), scale, |rt, d2d| unsafe {
        let fly = Flyout {
            hwnd: null_mut(),
            scale,
            d2d,
            rt: None,
            brush: Some(rt.CreateSolidColorBrush(&theme_for(dark).text, None)?),
            fmt: formats()?,
            theme: theme_for(dark),
            hover: None,
            drag: None,
            tracking_leave: false,
            autostart: crate::autostart_enabled(),
            was_active: false,
            anchor_bottom: false,
        };
        fly.draw(rt, &view());
        Ok(())
    })
}

/// System theme, or the dark/light variant with the system accent when forced.
pub(crate) fn theme_for(dark: Option<bool>) -> Theme {
    let theme = theme();
    match dark {
        Some(d) if d != theme.dark => {
            if d {
                dark_theme(theme.accent)
            } else {
                light_theme(theme.accent)
            }
        }
        _ => theme,
    }
}

/// Draws into an offscreen bitmap of `w`×`h` DIPs and saves it as PNG.
pub(crate) fn render_png(
    path: &str,
    w: f32,
    h: f32,
    scale: f32,
    draw: impl FnOnce(&ID2D1RenderTarget, ID2D1Factory) -> windows::core::Result<()>,
) -> windows::core::Result<()> {
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::*;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let (w, h) = ((w * scale) as u32, (h * scale) as u32);
        let bitmap = wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
        let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0 * scale,
            dpiY: 96.0 * scale,
            ..Default::default()
        };
        let rt = d2d.CreateWicBitmapRenderTarget(&bitmap, &props)?;
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        rt.BeginDraw();
        draw(&rt, d2d)?;
        rt.EndDraw(None, None)?;

        let stream = wic.CreateStream()?;
        let wpath: Vec<u16> = path.encode_utf16().chain([0]).collect();
        stream.InitializeFromFilename(windows::core::PCWSTR(wpath.as_ptr()), windows::Win32::Foundation::GENERIC_WRITE.0)?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.unwrap();
        frame.Initialize(None)?;
        frame.SetSize(w, h)?;
        let mut format = GUID_WICPixelFormat32bppPBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(&bitmap, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
    }
    Ok(())
}

// ---------------------------------------------------------------- input


fn slider_value(x: f32) -> u8 {
    let s = slider_rect();
    let (x0, x1) = (s.x + 10.0, s.x + s.w - 10.0);
    (((x - x0) / (x1 - x0)).clamp(0.0, 1.0) * 100.0).round().max(5.0) as u8
}

/// Runs a click action. No RefCell borrow may be held here: the color dialog and
/// DestroyWindow re-enter the window procedure.
unsafe fn activate(hwnd: HWND, hit: Hit) {
    // Any other click cancels a pending "forget".
    if !matches!(hit, Hit::Forget(_)) {
        FORGET_ARMED.lock().unwrap().take();
    }
    match hit {
        Hit::Card(i) => match rows().get(i).map(|r| r.1.clone()) {
            Some(Row::All) => select(None),
            Some(Row::Device(id) | Row::Archived(id)) => select(Some(id)),
            Some(Row::ArchiveHeader(_)) => {
                ARCHIVE_OPEN.fetch_xor(true, Ordering::Relaxed);
            }
            Some(Row::ServiceWarning) | None => {}
        },
        Hit::DisableService => {
            // The UAC prompt takes the focus; keep the panel open meanwhile.
            PICKING.store(true, Ordering::Relaxed);
            crate::disable_lamparray();
            PICKING.store(false, Ordering::Relaxed);
            SetForegroundWindow(hwnd);
        }
        Hit::Keys(i) => {
            if let Some((_, Row::Device(id))) = rows().get(i).cloned() {
                // A pinned panel stays open next to the editor.
                if !PINNED.load(Ordering::Relaxed) {
                    DestroyWindow(hwnd);
                    crate::editor::open(&id);
                    return;
                }
                crate::editor::open(&id);
            }
        }
        Hit::RenameDevice(i) => {
            if let Some((_, Row::Device(id) | Row::Archived(id))) = rows().get(i).cloned() {
                start_device_rename(id);
            }
        }
        Hit::Forget(i) => {
            if let Some((_, Row::Archived(id))) = rows().get(i).cloned() {
                let mut armed = FORGET_ARMED.lock().unwrap();
                if armed.as_deref() == Some(id.as_str()) {
                    armed.take();
                    drop(armed);
                    crate::forget_device(&id);
                    if selected().as_deref() == Some(id.as_str()) {
                        select(None);
                    }
                } else {
                    *armed = Some(id);
                }
            }
        }
        Hit::Zone(z) => ZONE.store(z, Ordering::Relaxed),
        Hit::Preset(i) => crate::apply_preset(i),
        Hit::PresetOverwrite(i) => {
            if !preset_matches(i) {
                crate::overwrite_preset(i);
            }
        }
        Hit::PresetHotkey(i) => {
            finish_rename(true);
            CAPTURE.store(i, Ordering::Relaxed);
        }
        Hit::Menu(item) => {
            if let Some(i) = menu_preset() {
                menu_action(hwnd, i, item);
            }
        }
        Hit::MenuOutside => {
            close_menu();
        }
        Hit::MenuBody => {}
        Hit::PresetRename(i) => {
            finish_rename(true);
            start_rename(i);
        }
        Hit::SavePreset => crate::save_preset(),
        Hit::Swatch(i) => paint_selected(PALETTE[i]),
        Hit::Custom => {
            let initial = view().common_color().unwrap_or(PALETTE[0]);
            PICKING.store(true, Ordering::Relaxed);
            let picked = crate::pick_color(hwnd, initial);
            PICKING.store(false, Ordering::Relaxed);
            SetForegroundWindow(hwnd);
            if let Some(c) = picked {
                paint_selected(c);
            }
        }
        Hit::Effect(i) => crate::set_effect(&target_ids(), Effect::ALL[i]),
        Hit::Autostart => {
            let on = !crate::autostart_enabled();
            crate::set_autostart(on);
            with_flyout(|f| f.autostart = on);
        }
        Hit::Reapply => crate::apply_all(),
        Hit::Pin => {
            PINNED.fetch_xor(true, Ordering::Relaxed);
        }
        Hit::Exit => {
            DestroyWindow(hwnd);
            crate::quit();
            return;
        }
        Hit::Slider => {}
    }
    invalidate();
}

fn mouse_pos(lparam: LPARAM, scale: f32) -> (f32, f32) {
    let x = (lparam & 0xFFFF) as i16 as f32;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as f32;
    (x / scale, y / scale)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            BeginPaint(hwnd, &mut ps);
            with_flyout(|f| f.paint());
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            let changed = with_flyout(|f| {
                let (x, y) = mouse_pos(lparam, f.scale);
                if !f.tracking_leave {
                    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    TrackMouseEvent(&mut tme);
                    f.tracking_leave = true;
                }
                if f.drag.is_some() {
                    f.drag = Some(slider_value(x));
                    return true;
                }
                let h = hit_test(x, y);
                let changed = h != f.hover;
                f.hover = h;
                changed
            });
            if changed == Some(true) {
                invalidate();
            }
            0
        }
        WM_MOUSELEAVE => {
            with_flyout(|f| {
                f.tracking_leave = false;
                f.hover = None;
            });
            invalidate();
            0
        }
        WM_LBUTTONDOWN => {
            let started = with_flyout(|f| {
                let (x, y) = mouse_pos(lparam, f.scale);
                (hit_test(x, y) == Some(Hit::Slider)).then(|| f.drag = Some(slider_value(x))).is_some()
            });
            if started == Some(true) {
                SetCapture(hwnd);
                invalidate();
            }
            0
        }
        WM_LBUTTONUP => {
            let (drag, hit) = with_flyout(|f| {
                let (x, y) = mouse_pos(lparam, f.scale);
                (f.drag.take(), hit_test(x, y))
            })
            .unwrap_or((None, None));
            // A click anywhere except the field being edited finishes renaming.
            let renaming = RENAME.load(Ordering::Relaxed);
            if renaming != usize::MAX && !matches!(hit, Some(Hit::Preset(i) | Hit::PresetRename(i)) if i == renaming) {
                finish_rename(true);
                invalidate();
            }
            // Clicking the card whose name is being edited keeps editing.
            let device_renaming = device_renaming();
            let on_renamed_card = match hit {
                Some(Hit::Card(i)) => matches!(rows().get(i), Some((_, Row::Device(id) | Row::Archived(id))) if Some(id) == device_renaming.as_ref()),
                _ => false,
            };
            if device_renaming.is_some() {
                if on_renamed_card {
                    return 0;
                }
                finish_rename(true);
                invalidate();
            }
            if let Some(b) = drag {
                ReleaseCapture();
                crate::set_brightness(selected().as_deref(), b);
                invalidate();
            } else if let Some(h) = hit {
                // Clicking the chip that is being renamed keeps editing instead of applying it.
                if !(matches!(h, Hit::Preset(i) | Hit::PresetRename(i) if i == renaming)) {
                    activate(hwnd, h);
                }
            }
            0
        }
        // While recording a preset hotkey every key press (incl. Alt/F10 combos) is captured.
        WM_KEYDOWN | WM_SYSKEYDOWN if CAPTURE.load(Ordering::Relaxed) != usize::MAX => {
            capture_key(wparam as u32);
            0
        }
        // Renaming a preset: Esc cancels (instead of closing the panel), text comes via WM_CHAR.
        WM_KEYDOWN if renaming_any() => {
            if wparam == VK_ESCAPE as usize {
                finish_rename(false);
                invalidate();
            }
            0
        }
        WM_CHAR if renaming_any() => {
            match char::from_u32(wparam as u32) {
                Some('\r') => finish_rename(true),
                Some('\u{8}') => {
                    RENAME_TEXT.lock().unwrap().pop();
                }
                Some(c) if !c.is_control() => {
                    let mut text = RENAME_TEXT.lock().unwrap();
                    if text.chars().count() < 24 {
                        text.push(c);
                    }
                }
                _ => {}
            }
            invalidate();
            0
        }
        WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
            // Esc closes the open menu first, then the panel.
            if close_menu() {
                invalidate();
            } else {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_RBUTTONUP => {
            let hit = with_flyout(|f| {
                let (x, y) = mouse_pos(lparam, f.scale);
                (x, y, panel_hit_test(x, y))
            });
            // Right click elsewhere just closes an open menu; on a preset it opens that preset's menu.
            close_menu();
            if let Some((x, y, Some(Hit::Preset(i)))) = hit {
                finish_rename(true);
                CAPTURE.store(usize::MAX, Ordering::Relaxed);
                open_menu(i, x, y);
            }
            invalidate();
            0
        }
        WM_TIMER if wparam == TIMER_FOCUS => {
            check_focus(hwnd);
            0
        }
        WM_ACTIVATE => {
            if (wparam & 0xFFFF) as u32 == WA_INACTIVE && !PICKING.load(Ordering::Relaxed) && !PINNED.load(Ordering::Relaxed) {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DPICHANGED => {
            let suggested = &*(lparam as *const RECT);
            with_flyout(|f| {
                f.scale = GetDpiForWindow(hwnd) as f32 / 96.0;
                f.rt = None;
                f.brush = None;
            });
            SetWindowPos(
                hwnd,
                null_mut(),
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            0
        }
        WM_DESTROY => {
            let fly = FLYOUT.with(|c| c.borrow_mut().take());
            drop(fly);
            CAPTURE.store(usize::MAX, Ordering::Relaxed);
            finish_rename(true);
            LAST_CLOSE.store(GetTickCount64(), Ordering::Relaxed);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
