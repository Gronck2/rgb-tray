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

use crate::devices::{kind_of, speed_factor, Class, Effect, Rgb, Speed, NORMAL_SPEED};

// ---------------------------------------------------------------- layout (in DIPs)

const W: f32 = 360.0;
const PAD: f32 = 16.0;
const CARD_Y: f32 = 56.0;
const CARD_H: f32 = 50.0;
const CARD_STEP: f32 = 54.0;
/// The "disconnected devices" row that folds the archive.
const ARCHIVE_H: f32 = 34.0;
const WARNING_H: f32 = 64.0;
/// Device cards on one page of the list; more go to further pages.
const PAGE_SIZE: usize = 5;
/// The page switch under "All devices": ‹ dots ›.
const PAGER_H: f32 = 22.0;
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
const EFFECT_LABELS: [crate::i18n::Text; 4] = [("Статичный", "Static"), ("Дыхание", "Breathing"), ("Спектр", "Spectrum"), ("Выкл.", "Off")];
/// Segoe Fluent Icons glyphs.
const ICON_ALL: &str = "\u{E82F}";
const ICON_CHEVRON_DOWN: &str = "\u{E70D}";
const ICON_CHEVRON_UP: &str = "\u{E70E}";
const ICON_CHEVRON_LEFT: &str = "\u{E76B}";
const ICON_CHEVRON_RIGHT: &str = "\u{E76C}";
const ICON_WARNING: &str = "\u{E7BA}";
const ICON_SETTINGS: &str = "\u{E713}";
const ICON_REFRESH: &str = "\u{E72C}";
const ICON_PIN: &str = "\u{E718}";
const ICON_PINNED: &str = "\u{E841}";
const ICON_ADD: &str = "\u{E710}";
const ICON_POWER: &str = "\u{E7E8}";
const ICON_CLOSE: &str = "\u{E711}";
const ICON_BRIGHTNESS: &str = "\u{E706}";
const ICON_SPEED: &str = "\u{EC4A}";
const ICON_SAVE: &str = "\u{E74E}";
const ICON_PLAY: &str = "\u{E768}";
const ICON_RENAME: &str = "\u{E70F}";
const ICON_SHARE: &str = "\u{E72D}";
const ICON_DELETE: &str = "\u{E74D}";
const ICON_KEYBOARD: &str = "\u{E765}";
const ICON_DOWNLOAD: &str = "\u{E896}";
const ICON_OPEN: &str = "\u{E8A7}";

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
    /// The speed slider of breathing and spectrum.
    Speed,
    Autostart,
    Reapply,
    Exit,
    /// Opens the per-key editor of the keyboard on that row.
    Keys(usize),
    /// Forgets the disconnected device on that row (asks first).
    Forget(usize),
    /// The pencil after the device name on that row.
    RenameDevice(usize),
    /// The keyboard button after the pencil: records the hotkey that switches the device off and on.
    DeviceHotkey(usize),
    /// The power button on the warning about the Logitech LampArray service.
    DisableService,
    /// The close button on the warning about a program on that row: never show it again.
    IgnoreApp(usize),
    /// The gear button on the warning about Windows Dynamic Lighting.
    LightingSettings,
    /// The main button of the update card: download and install (or open the release page).
    UpdateInstall,
    /// The close button of the update card: skip this version.
    UpdateSkip,
    /// Keeps the panel open when it loses focus.
    Pin,
    /// The page switch of the device list: previous, next, a page's dot.
    PagePrev,
    PageNext,
    Page(usize),
}

/// A row of the device list.
#[derive(Clone, PartialEq)]
enum Row {
    /// A Windows service that hangs the keyboard is active.
    ServiceWarning,
    /// Windows Dynamic Lighting is on and drives the lighting itself.
    LightingWarning,
    /// A vendor program drives the lighting of these devices too (`conflicts::App::key`).
    AppWarning(&'static str),
    /// A newer version is out (`update::offered`).
    Update,
    All,
    /// Connected device, or one without a link.
    Device(String),
    /// Folds/unfolds the disconnected devices; their count.
    ArchiveHeader(usize),
    Archived(String),
    /// The page switch: current page and page count.
    Pager(usize, usize),
    /// Empty room at the end of a short page, so every page is as tall as the tallest.
    Spacer,
}

/// The page of the device list shown.
static PAGE: AtomicUsize = AtomicUsize::new(0);

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
    if crate::DYNAMIC_LIGHTING_WARN.load(Ordering::Relaxed) {
        push(WARNING_H, Row::LightingWarning);
    }
    for c in crate::CONFLICTS.lock().unwrap().iter() {
        push(WARNING_H, Row::AppWarning(c.app.key));
    }
    if crate::update::offered().is_some() {
        push(WARNING_H, Row::Update);
    }
    push(CARD_H, Row::All);
    let pages = list_pages(active, archived);
    let page = PAGE.load(Ordering::Relaxed).min(pages.len() - 1);
    if pages.len() > 1 {
        push(PAGER_H, Row::Pager(page, pages.len()));
    }
    let tallest = pages.iter().map(|p| p.iter().map(|r| row_h(r) + (CARD_STEP - CARD_H)).sum::<f32>()).fold(0.0, f32::max);
    let mut used = 0.0;
    for row in pages.into_iter().nth(page).unwrap_or_default() {
        used += row_h(&row) + (CARD_STEP - CARD_H);
        push(row_h(&row), row);
    }
    if tallest - used > 0.5 {
        push(tallest - used - (CARD_STEP - CARD_H), Row::Spacer);
    }
    out
}

fn row_h(row: &Row) -> f32 {
    if matches!(row, Row::ArchiveHeader(_)) {
        ARCHIVE_H
    } else {
        CARD_H
    }
}

/// Splits the device list into pages of at most PAGE_SIZE cards. The archive header is not a
/// card: it goes on the page of the first disconnected device it folds, or ends the last page.
fn list_pages(active: Vec<String>, archived: Vec<String>) -> Vec<Vec<Row>> {
    let mut entries: Vec<Row> = active.into_iter().map(Row::Device).collect();
    if !archived.is_empty() {
        entries.push(Row::ArchiveHeader(archived.len()));
        if ARCHIVE_OPEN.load(Ordering::Relaxed) {
            entries.extend(archived.into_iter().map(Row::Archived));
        }
    }
    let mut pages: Vec<Vec<Row>> = vec![Vec::new()];
    let mut cards = 0;
    let mut entries = entries.into_iter().peekable();
    while let Some(row) = entries.next() {
        let header = matches!(row, Row::ArchiveHeader(_));
        // A card that doesn't fit, or the header whose next card doesn't, starts a new page.
        let next_is_card = entries.peek().is_some();
        if cards == PAGE_SIZE && (!header || next_is_card) {
            pages.push(Vec::new());
            cards = 0;
        }
        if !header {
            cards += 1;
        }
        pages.last_mut().unwrap().push(row);
    }
    pages
}

/// The page holding the archive header.
fn header_page() -> Option<usize> {
    let active = crate::active_ids();
    let archived = archived_ids(&active);
    list_pages(active, archived).iter().position(|p| p.iter().any(|r| matches!(r, Row::ArchiveHeader(_))))
}

/// Shows a page of the device list (for snapshots).
pub fn set_page(page: usize) {
    PAGE.store(page, Ordering::Relaxed);
}

/// Shows another page (clamped to the existing ones).
fn turn_page(delta: isize) {
    let pages = rows().iter().find_map(|(_, r)| if let Row::Pager(_, n) = r { Some(*n) } else { None }).unwrap_or(1);
    let page = (PAGE.load(Ordering::Relaxed).min(pages - 1) as isize + delta).clamp(0, pages as isize - 1);
    PAGE.store(page as usize, Ordering::Relaxed);
}

/// The ‹ › arrows and the dots of the page switch.
fn pager_parts(r: R, pages: usize) -> Vec<(R, Hit)> {
    let mut v = vec![(R { x: r.x, y: r.y, w: 32.0, h: r.h }, Hit::PagePrev), (R { x: r.x + r.w - 32.0, y: r.y, w: 32.0, h: r.h }, Hit::PageNext)];
    let step = 16.0;
    let x0 = r.cx() - step * pages as f32 / 2.0;
    for k in 0..pages {
        v.push((R { x: x0 + k as f32 * step, y: r.y, w: step, h: r.h }, Hit::Page(k)));
    }
    v
}

/// "Не подключено · вчера": when a disconnected device was last seen.
fn last_seen_label(last_seen: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if last_seen == 0 {
        return tr!("Не подключено", "Not connected").into();
    }
    match now.saturating_sub(last_seen) / 86_400 {
        0 => tr!("Не подключено · сегодня", "Not connected · today").into(),
        1 => tr!("Не подключено · вчера", "Not connected · yesterday").into(),
        n => format!("{} · {n} {}", tr!("Не подключено", "Not connected"), tr!("дн. назад", "days ago")),
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

/// The small speed slider at the right end of the "Effect" label line.
fn speed_rect() -> R {
    R { x: W - PAD - 130.0, y: EFFECT_LABEL_Y - 1.0, w: 130.0, h: 22.0 }
}

/// Margin between a slider's ends and its track: the thumb's radius.
const SPEED_THUMB: f32 = 7.0;

/// The speed slider is shown while the selected device (or, with "All devices", any device on
/// the panel) breathes or cycles.
fn speed_shown() -> bool {
    let ids = selected().map_or_else(crate::active_ids, |id| vec![id]);
    let cfg = crate::config();
    ids.iter().any(|id| cfg.device(id).visible().animated() && crate::devices::caps(id).has_speed())
}

/// Height of the header strip by which a pinned panel can be dragged.
const HEADER_DRAG_H: f32 = 52.0;

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
    /// The text format used for measuring, and measured (name, width) pairs.
    type Widths = (Option<IDWriteTextFormat>, Vec<(String, f32)>);
    thread_local! {
        static CACHE: RefCell<Widths> = const { RefCell::new((None, Vec::new())) };
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
        Row::Device(id) if crate::devices::caps(id).per_key => keys_button_rect(card).x,
        Row::Archived(_) => forget_button_rect(card).x,
        _ => card.x + card.w - 50.0,
    };
    // Room for the pencil after the name, and the hotkey button of a connected device.
    let room = if matches!(row, Row::Device(_)) { 46.0 } else { 24.0 };
    R { x: card.x + 50.0, y: card.y + 6.0, w: buttons_x - card.x - 56.0 - room, h: 20.0 }
}

/// The small pencil right after a device's name.
fn pencil_rect(card: R, row: &Row, name: &str) -> R {
    let n = name_rect(card, row);
    R { x: n.x + name_width(name).min(n.w) + 4.0, y: n.y, w: 20.0, h: 20.0 }
}

/// The hotkey button after the pencil (shown while the card is hovered or a hotkey is recorded).
fn hotkey_rect(card: R, row: &Row, name: &str) -> R {
    let p = pencil_rect(card, row, name);
    R { x: p.x + 22.0, ..p }
}

/// The square icon button of a warning card: power (disable the service) or gear (Settings).
fn warning_button_rect(card: R) -> R {
    R { x: card.x + card.w - 12.0 - 36.0, y: card.y + (card.h - 32.0) / 2.0, w: 36.0, h: 32.0 }
}

/// The "skip this version" button of the update card, left of its main button.
fn update_skip_rect(card: R) -> R {
    let b = warning_button_rect(card);
    R { x: b.x - 6.0 - b.w, ..b }
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
        match row {
            Row::Pager(_, pages) => {
                v.extend(pager_parts(*r, *pages));
                continue;
            }
            Row::Spacer => continue,
            _ => {}
        }
        // Buttons on a card come first, so hit-testing finds them before the card.
        match row {
            Row::Device(id) if crate::devices::caps(id).per_key => v.push((keys_button_rect(*r), Hit::Keys(i))),
            Row::Archived(_) => v.push((forget_button_rect(*r), Hit::Forget(i))),
            Row::ServiceWarning => v.push((warning_button_rect(*r), Hit::DisableService)),
            Row::LightingWarning => v.push((warning_button_rect(*r), Hit::LightingSettings)),
            Row::AppWarning(_) => v.push((warning_button_rect(*r), Hit::IgnoreApp(i))),
            Row::Update if !matches!(crate::update::offered(), Some(crate::update::State::Downloading(_))) => {
                v.push((warning_button_rect(*r), Hit::UpdateInstall));
                v.push((update_skip_rect(*r), Hit::UpdateSkip));
            }
            _ => {}
        }
        if let Row::Device(id) | Row::Archived(id) = row {
            if device_renaming().as_ref() != Some(id) {
                let name = crate::config().display_name(id);
                v.push((pencil_rect(*r, row, &name), Hit::RenameDevice(i)));
                if matches!(row, Row::Device(_)) {
                    v.push((hotkey_rect(*r, row, &name), Hit::DeviceHotkey(i)));
                }
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
    for i in (0..4).filter(|&i| effect_enabled(Effect::ALL[i])) {
        v.push((R { x: PAD + i as f32 * seg_w, y: SEG_Y, w: seg_w, h: SEG_H }, Hit::Effect(i)));
    }
    v.push((slider_rect(), Hit::Slider));
    if speed_shown() {
        v.push((speed_rect(), Hit::Speed));
    }
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
    let zones = if zone_count > 0 { zone_rects() } else { Vec::new() }.into_iter().enumerate();
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
    let labels: Vec<String> =
        crate::config().presets.iter().enumerate().map(|(i, p)| if i == renaming { RENAME_TEXT.lock().unwrap().clone() } else { preset_label(p) }).collect();
    let items =
        labels.into_iter().enumerate().map(|(i, l)| (l, Some(i))).chain(std::iter::once((tr!("+ Сохранить текущие", "+ Save current").to_string(), None)));
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
/// Device whose on/off hotkey is being recorded.
static CAPTURE_DEVICE: Mutex<Option<String>> = Mutex::new(None);

fn capturing_device() -> Option<String> {
    CAPTURE_DEVICE.lock().unwrap().clone()
}

/// A hotkey of a preset or a device is being recorded.
fn capturing() -> bool {
    CAPTURE.load(Ordering::Relaxed) != usize::MAX || capturing_device().is_some()
}

fn stop_capture() {
    CAPTURE.store(usize::MAX, Ordering::Relaxed);
    CAPTURE_DEVICE.lock().unwrap().take();
}
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
    stop_capture();
    *RENAME_TEXT.lock().unwrap() = crate::config().display_name(&id);
    *DEVICE_RENAME.lock().unwrap() = Some(id);
}

fn start_rename(i: usize) {
    finish_rename(true);
    stop_capture();
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
    /// Save the preset to a file to share.
    Export,
    /// F13 + n, or None to remove the hotkey.
    Hotkey(Option<u32>),
    /// Record any key combination.
    Combo,
    /// Modifier toggle for the F13–F24 buttons (MOD_* bit).
    Mod(u32),
    Delete,
}

/// What the open menu is for: a preset (all its actions and its hotkey) or a device (its on/off hotkey).
#[derive(Clone, PartialEq)]
enum MenuTarget {
    Preset(usize),
    Device(String),
}

impl MenuTarget {
    fn hotkey(&self) -> Option<crate::config::Hotkey> {
        let cfg = crate::config();
        match self {
            MenuTarget::Preset(i) => cfg.presets.get(*i).and_then(|p| p.hotkey),
            MenuTarget::Device(id) => cfg.devices.get(id).and_then(|d| d.hotkey),
        }
    }

    fn set_hotkey(&self, hotkey: Option<crate::config::Hotkey>) {
        match self {
            MenuTarget::Preset(i) => crate::set_preset_hotkey(*i, hotkey),
            MenuTarget::Device(id) => crate::set_device_hotkey(id, hotkey),
        }
    }
}

/// The open menu, drawn inside the panel: (what for, anchor point, modifiers for F13–F24).
static MENU: Mutex<Option<(MenuTarget, f32, f32, u32)>> = Mutex::new(None);

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

fn open_menu(target: MenuTarget, x: f32, y: f32) {
    // Start from the modifiers of the current F13–F24 hotkey.
    let mods = target.hotkey().filter(|h| is_f13_24(h.vk)).map_or(0, |h| h.mods);
    *MENU.lock().unwrap() = Some((target, x, y, mods));
}

fn menu_mods() -> u32 {
    MENU.lock().unwrap().as_ref().map_or(0, |m| m.3)
}

fn close_menu() -> bool {
    MENU.lock().unwrap().take().is_some()
}

fn menu_target() -> Option<MenuTarget> {
    MENU.lock().unwrap().as_ref().map(|m| m.0.clone())
}

/// The menu card and its items, placed next to the click and kept inside the panel. A device's
/// menu has only the hotkey part.
fn menu_layout() -> Option<(R, Vec<(R, MenuItem)>)> {
    let (target, ax, ay, _) = MENU.lock().unwrap().clone()?;
    let preset = matches!(target, MenuTarget::Preset(_));
    let key_rows = 12 / MENU_KEY_COLS + 1;
    let actions = if preset { 4.0 * MENU_ITEM_H + MENU_GAP + MENU_ITEM_H } else { 0.0 };
    let h = 6.0 + MENU_HEADER_H + actions + MENU_GAP + MENU_LABEL_H + (key_rows + 1) as f32 * MENU_KEY_STEP + 6.0;
    let x = ax.clamp(8.0, W - 8.0 - MENU_W);
    let y = if ay + h <= panel_h() - 8.0 { ay } else { (ay - h).max(8.0) };
    let panel = R { x, y, w: MENU_W, h };
    let mut items = Vec::new();
    let row = |y: f32| R { x: x + 4.0, y, w: MENU_W - 8.0, h: MENU_ITEM_H };
    let mut cy = y + 6.0 + MENU_HEADER_H;
    if preset {
        for item in [MenuItem::Apply, MenuItem::Overwrite, MenuItem::Rename, MenuItem::Export] {
            items.push((row(cy), item));
            cy += MENU_ITEM_H;
        }
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
    if preset {
        cy = last + MENU_KEY_STEP + MENU_GAP;
        items.push((row(cy), MenuItem::Delete));
    }
    Some((panel, items))
}

/// Overwriting makes sense only when the current lighting differs from the preset or it lacks a device.
fn preset_saved(i: usize) -> bool {
    let cfg = crate::config();
    cfg.presets.get(i).is_some_and(|p| cfg.saved_in(p))
}

unsafe fn menu_action(hwnd: HWND, target: MenuTarget, item: MenuItem) {
    if let MenuItem::Mod(bit) = item {
        // Toggling a modifier keeps the menu open and updates an assigned F13–F24 hotkey.
        let mods = {
            let mut m = MENU.lock().unwrap();
            let Some(state) = m.as_mut() else { return };
            state.3 ^= bit;
            state.3
        };
        if let Some(h) = target.hotkey().filter(|h| is_f13_24(h.vk)) {
            target.set_hotkey(Some(crate::config::Hotkey { mods, vk: h.vk }));
        }
        return;
    }
    let mods = menu_mods();
    close_menu();
    match item {
        MenuItem::Hotkey(None) => target.set_hotkey(None),
        MenuItem::Hotkey(Some(n)) => target.set_hotkey(Some(crate::config::Hotkey { mods, vk: VK_F13 + n })),
        MenuItem::Combo => {
            stop_capture();
            match target {
                MenuTarget::Preset(i) => CAPTURE.store(i, Ordering::Relaxed),
                MenuTarget::Device(id) => *CAPTURE_DEVICE.lock().unwrap() = Some(id),
            }
            SetForegroundWindow(hwnd);
        }
        MenuItem::Mod(_) => {}
        // The other items are a preset's.
        _ => {
            let MenuTarget::Preset(i) = target else { return };
            match item {
                MenuItem::Apply => crate::apply_preset(i),
                MenuItem::Overwrite => {
                    if !preset_saved(i) {
                        crate::overwrite_preset(i);
                    }
                }
                MenuItem::Rename => start_rename(i),
                MenuItem::Export => crate::export_preset(i),
                MenuItem::Delete => crate::delete_preset(i),
                _ => {}
            }
        }
    }
}
/// Records the pressed combination as the hotkey of the preset or device being captured.
/// Returns false when no capture is in progress.
unsafe fn capture_key(vk: u32) -> bool {
    let i = CAPTURE.load(Ordering::Relaxed);
    let device = capturing_device();
    if i == usize::MAX && device.is_none() {
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
    stop_capture();
    // Plain Esc cancels, Backspace/Delete remove the hotkey.
    let hotkey = match (vk, mods) {
        (0x1B, 0) => {
            invalidate();
            return true;
        }
        (0x08 | 0x2E, 0) => None,
        _ => Some(crate::config::Hotkey { mods, vk }),
    };
    match device {
        Some(id) => crate::set_device_hotkey(&id, hotkey),
        None => crate::set_preset_hotkey(i, hotkey),
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
    let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut data as *mut u32 as _, &mut size) };
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
    if luminance > 0.6 {
        col(0, 0, 0, 1.0)
    } else {
        col(255, 255, 255, 1.0)
    }
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
fn selected_zones() -> Vec<(u8, String)> {
    selected().map_or(Vec::new(), |id| crate::devices::zones_of(&id))
}

/// Devices the controls change: the selected one, or every shown one.
fn target_ids() -> Vec<String> {
    selected().map_or_else(crate::active_ids, |id| vec![id])
}

/// Some device the controls change can show this effect.
fn effect_enabled(e: Effect) -> bool {
    target_ids().iter().any(|id| crate::devices::caps(id).has_effect(e))
}

/// Labels of the zone switch: the whole device, then each zone.
fn zone_labels() -> Vec<String> {
    let whole = if selected().is_some_and(|id| crate::devices::caps(&id).class == Class::Mouse) { tr!("Вся мышь", "All") } else { tr!("Все", "All") };
    std::iter::once(whole.to_string()).chain(selected_zones().into_iter().map(|z| z.1)).collect()
}

/// Segments of the zone switch, right-aligned on the "Color" line, sized to their labels.
fn zone_rects() -> Vec<R> {
    let labels = zone_labels();
    // Room right of the "Color" label; segments shrink to fit.
    let room = W - 2.0 * PAD - 70.0;
    let mut widths: Vec<f32> = labels.iter().map(|l| (l.chars().count() as f32 * 7.2 + 22.0).max(32.0)).collect();
    let total: f32 = widths.iter().sum();
    if total > room {
        widths.iter_mut().for_each(|w| *w *= room / total);
    }
    let mut x = W - PAD - widths.iter().sum::<f32>();
    widths
        .into_iter()
        .map(|w| {
            let r = R { x, y: COLOR_LABEL_Y - 5.0, w, h: 28.0 };
            x += w;
            r
        })
        .collect()
}
static PICKING: AtomicBool = AtomicBool::new(false);
/// Keeps the panel open when it loses focus: the pin button (saved in the config) or `--show`.
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
    /// Speed while its slider is being dragged.
    drag_speed: Option<u8>,
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
    let mut y = if anchor_bottom { rc.bottom - want } else { rc.top };
    // Keep a panel that was dragged near the bottom of the screen fully visible.
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi) != 0 {
        y = y.min(mi.rcWork.bottom - want).max(mi.rcWork.top);
    }
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

/// Whether a font family is installed (checked once per name).
fn font_installed(dw: &IDWriteFactory, family: &str) -> bool {
    static KNOWN: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
    // Snapshots can pretend to run on Windows 10.
    if std::env::var_os("RGBTRAY_DEMO_WIN10").is_some() {
        return false;
    }
    if let Some(&(_, ok)) = KNOWN.lock().unwrap().iter().find(|(n, _)| n == family) {
        return ok;
    }
    let ok = unsafe {
        let mut fonts = None;
        let name: Vec<u16> = family.encode_utf16().chain([0]).collect();
        let (mut index, mut exists) = (0u32, windows::Win32::Foundation::BOOL(0));
        dw.GetSystemFontCollection(&mut fonts, false).is_ok()
            && fonts.is_some_and(|f| f.FindFamilyName(windows::core::PCWSTR(name.as_ptr()), &mut index, &mut exists).is_ok() && exists.as_bool())
    };
    KNOWN.lock().unwrap().push((family.to_string(), ok));
    ok
}

/// Windows 10 lacks the Windows 11 fonts: icons fall back to Segoe MDL2 Assets (same code
/// points) and Segoe UI Variable to Segoe UI.
fn available_family(dw: &IDWriteFactory, family: &str) -> String {
    let fallback = match family {
        "Segoe Fluent Icons" => "Segoe MDL2 Assets",
        f if f.starts_with("Segoe UI Variable") => "Segoe UI",
        _ => return family.to_string(),
    };
    if font_installed(dw, family) {
        family.to_string()
    } else {
        fallback.to_string()
    }
}

pub(crate) fn text_format(
    dw: &IDWriteFactory,
    family: windows::core::PCWSTR,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
    align: DWRITE_TEXT_ALIGNMENT,
) -> windows::core::Result<IDWriteTextFormat> {
    unsafe {
        let family: Vec<u16> = available_family(dw, &family.to_string().unwrap_or_default()).encode_utf16().chain([0]).collect();
        let family = windows::core::PCWSTR(family.as_ptr());
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
    // Settings may have changed while the panel was closed (e.g. Dynamic Lighting turned off).
    let cfg = crate::config().clone();
    crate::check_services(&cfg);
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
    let (mut x, mut y) = match edge {
        0 => (work.left + margin, near_y),      // ABE_LEFT
        1 => (near_x, work.top + margin),       // ABE_TOP
        2 => (work.right - w - margin, near_y), // ABE_RIGHT
        _ => (near_x, work.bottom - h - margin),
    };
    // A pinned panel that was dragged away opens where it was left, if that is still on a screen.
    let saved = if PINNED.load(Ordering::Relaxed) { crate::config().panel_pos } else { None };
    let mut moved = false;
    if let Some((sx, sy)) = saved {
        let rc = RECT { left: sx, top: sy, right: sx + w, bottom: sy + h };
        let mon = MonitorFromRect(&rc, MONITOR_DEFAULTTONULL);
        if !mon.is_null() {
            let mut smi: MONITORINFO = zeroed();
            smi.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(mon, &mut smi);
            let wk = smi.rcWork;
            x = sx.clamp(wk.left, (wk.right - w).max(wk.left));
            y = sy.clamp(wk.top, (wk.bottom - h).max(wk.top));
            moved = true;
        }
    }

    let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, class.as_ptr(), class.as_ptr(), WS_POPUP, x, y, w, h, null_mut(), null_mut(), hinst, null());
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
        drag_speed: None,
        tracking_leave: false,
        autostart: crate::autostart_enabled(),
        was_active: false,
        anchor_bottom: edge == 3 && !moved,
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
    /// Connected, unlinked and archived devices (on every page of the list).
    devices: Vec<DeviceView>,
    brightness: u8,
    speed: u8,
    speed_shown: bool,
    selected: Option<String>,
    /// Top of the section under the device list.
    lower_y: f32,
}

fn view() -> View {
    let speed_shown = speed_shown();
    let rows = rows();
    let lower_y = lower_y_of(&rows);
    let selected = selected();
    // Every device, not only the page shown: "All devices" and the controls cover them all.
    let active = crate::active_ids();
    let archived = archived_ids(&active);
    let cfg = crate::config();
    let present = crate::PRESENT.lock().unwrap().clone();
    let status = crate::STATUS.lock().unwrap().clone();
    let no_link = crate::NO_LINK.lock().unwrap().clone();
    let devices = active
        .into_iter()
        .chain(archived)
        .map(|id| DeviceView {
            // One-color per-key lighting is shown as the steady color it looks like.
            cfg: cfg.device(&id).visible(),
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
        // Like brightness: the overall speed, or the selected device's shift from it.
        speed: selected.as_ref().map_or(cfg.speed, |id| cfg.device(id).speed),
        speed_shown,
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

    /// A Windows 11 slider: track filled up to `value` (0..=100) and a round thumb of radius
    /// `thumb`, whose dot shrinks while dragged and grows under the mouse.
    unsafe fn slider(&self, rt: &ID2D1RenderTarget, s: R, value: u8, thumb: f32, dragging: bool, hovered: bool) {
        let t = &self.theme;
        let (x0, x1) = (s.x + thumb, s.x + s.w - thumb);
        let tx = x0 + (x1 - x0) * value.min(100) as f32 / 100.0;
        self.fill(rt, R { x: x0, y: s.cy() - 2.0, w: x1 - x0, h: 4.0 }, 2.0, t.text2);
        self.fill(rt, R { x: x0, y: s.cy() - 2.0, w: tx - x0, h: 4.0 }, 2.0, t.accent);
        self.circle(rt, tx, s.cy(), thumb, if t.dark { col(0x45, 0x45, 0x45, 1.0) } else { col(255, 255, 255, 1.0) });
        self.ring(rt, tx, s.cy(), thumb - 0.5, t.stroke, 1.0);
        let inner = if dragging {
            0.45
        } else if hovered {
            0.65
        } else {
            0.55
        };
        self.circle(rt, tx, s.cy(), thumb * inner, t.accent);
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
        self.text(rt, tr!("Подсветка", "Lighting"), &f.title, R { x: PAD, y: 12.0, w: 200.0, h: 32.0 }, t.text);
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
        self.text(rt, tr!("Цвет", "Color"), &f.label, R { x: PAD, y: COLOR_LABEL_Y, w: 200.0, h: 20.0 }, t.text);
        let zones = selected_zones();
        if !zones.is_empty() {
            // Which zone the palette paints: whole device or one zone.
            let zone = ZONE.load(Ordering::Relaxed);
            let rects = zone_rects();
            let all = R { x: rects[0].x, y: rects[0].y, w: rects.iter().map(|r| r.w).sum(), h: rects[0].h };
            self.fill(rt, all, 4.0, t.card);
            self.stroke(rt, all, 4.0, t.stroke, 1.0);
            for (k, (name, r)) in zone_labels().iter().zip(rects).enumerate() {
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
        let static_like = v.common_effect().is_some_and(Effect::colored);
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
        self.text(rt, tr!("Эффект", "Effect"), &f.label, R { x: PAD, y: EFFECT_LABEL_Y, w: 200.0, h: 20.0 }, t.text);
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
            // An effect none of the devices can show is pale and can't be clicked.
            let fg = if on {
                t.on_accent
            } else if effect_enabled(Effect::ALL[i]) {
                t.text
            } else {
                D2D1_COLOR_F { a: t.text2.a * 0.5, ..t.text2 }
            };
            self.text(rt, crate::i18n::pick(*label), &f.center, r, fg);
        }
        if v.speed_shown {
            let s = speed_rect();
            let label = if v.selected.is_none() { tr!("Скорость", "Speed") } else { tr!("Скорость устройства", "Device speed") };
            self.text(rt, label, &f.sub_right, R { x: s.x - 166.0, y: EFFECT_LABEL_Y, w: 160.0, h: 20.0 }, t.text2);
            let speed = self.drag_speed.unwrap_or(v.speed);
            // Normal speed (for a device: the same as the overall one) in the middle.
            self.fill(rt, R { x: s.x + s.w / 2.0 - 1.0, y: s.y + s.h / 2.0 - 5.0, w: 2.0, h: 10.0 }, 1.0, t.text2);
            self.slider(rt, s, speed, SPEED_THUMB, self.drag_speed.is_some(), hover(Hit::Speed));
        }

        // Brightness slider.
        let brightness = self.drag.unwrap_or(v.brightness);
        let (label, value) = if v.selected.is_none() {
            (tr!("Общая яркость", "Brightness"), format!("{brightness}%"))
        } else {
            (tr!("Яркость устройства", "Device brightness"), format!("{brightness}% {}", tr!("от общей", "of global")))
        };
        self.text(rt, label, &f.label, R { x: PAD, y: BRIGHT_LABEL_Y, w: 200.0, h: 20.0 }, t.text);
        self.text(rt, &value, &f.label_right, R { x: W - PAD - 140.0, y: BRIGHT_LABEL_Y, w: 140.0, h: 20.0 }, t.text2);
        self.slider(rt, slider_rect(), brightness, 10.0, self.drag.is_some(), hover(Hit::Slider));

        // Global presets.
        self.text(rt, tr!("Пресеты", "Presets"), &f.label, R { x: PAD, y: PRESET_LABEL_Y, w: 120.0, h: 20.0 }, t.text);
        let capture = CAPTURE.load(Ordering::Relaxed);
        if capture != usize::MAX {
            self.text(
                rt,
                tr!("Нажмите сочетание · Del — убрать · Esc", "Press a shortcut · Del — remove · Esc"),
                &f.sub_right,
                R { x: W - PAD - 250.0, y: PRESET_LABEL_Y, w: 250.0, h: 20.0 },
                t.accent,
            );
        } else if renaming_any() {
            self.text(
                rt,
                tr!("Enter — сохранить · Esc — отмена", "Enter — save · Esc — cancel"),
                &f.sub_right,
                R { x: W - PAD - 250.0, y: PRESET_LABEL_Y, w: 250.0, h: 20.0 },
                t.accent,
            );
        } else {
            self.text(
                rt,
                tr!("ПКМ по пресету — меню", "Right-click a preset for menu"),
                &f.sub_right,
                R { x: W - PAD - 200.0, y: PRESET_LABEL_Y, w: 200.0, h: 20.0 },
                t.text2,
            );
        }
        let (active, saved): (Vec<bool>, Vec<bool>) = {
            let cfg = crate::config();
            cfg.presets.iter().map(|p| (cfg.matches(p), cfg.saved_in(p))).unzip()
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
                let fg = if is_on(i) {
                    t.on_accent
                } else if hit == Hit::SavePreset {
                    t.text2
                } else {
                    t.text
                };
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
            let disabled = matches!(hit, Hit::PresetOverwrite(_)) && saved.get(i) == Some(&true);
            if hover(hit) && !disabled {
                self.fill(rt, r, 3.0, if on { col(255, 255, 255, 0.25) } else { t.card_hover });
            }
            let active_btn = (hit == Hit::PresetHotkey(i) && capture == i) || (hit == Hit::PresetRename(i) && renaming == i);
            let mut fg = if active_btn {
                t.accent
            } else if on {
                t.on_accent
            } else {
                t.text2
            };
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
        self.text(rt, tr!("Запускать вместе с Windows", "Start with Windows"), &f.sub, R { x: sw.x + 50.0, y: FOOTER_Y, w: 200.0, h: 36.0 }, t.text);
        let exit = R { x: W - PAD - 92.0, y: FOOTER_Y, w: 92.0, h: 36.0 };
        if hover(Hit::Exit) {
            self.fill(rt, exit, 4.0, t.card_hover);
        }
        self.text(rt, ICON_POWER, &f.icon_small, R { x: exit.x + 6.0, y: exit.y, w: 24.0, h: exit.h }, t.text);
        self.text(rt, tr!("Выход", "Exit"), &f.sub, R { x: exit.x + 34.0, y: exit.y, w: 56.0, h: exit.h }, t.text);

        rt.SetTransform(&Matrix3x2::identity());
        self.draw_menu(rt);
    }

    /// One row of the device list.
    unsafe fn draw_row(&self, rt: &ID2D1RenderTarget, v: &View, i: usize, r: R, row: &Row) {
        let t = &self.theme;
        let f = &self.fmt;
        let hover = |h: Hit| self.hover == Some(h);

        if let Row::Update = row {
            use crate::update::State;
            let current = env!("CARGO_PKG_VERSION");
            // (title, line 1, line 2, main button icon); no buttons while downloading.
            let (title, line1, line2, button) = match crate::update::offered() {
                Some(State::Downloading(r)) => (
                    format!("{} {}…", tr!("Загружается версия", "Downloading version"), r.version),
                    tr!("Приложение закроется и", "The app will close and").to_string(),
                    tr!("запустится уже новым", "start again updated").to_string(),
                    None,
                ),
                Some(State::Failed(r)) => (
                    format!("{} {}", tr!("Не удалось скачать", "Could not download"), r.version),
                    tr!("Скачайте установщик", "Download the installer").to_string(),
                    tr!("на странице релиза", "from the release page").to_string(),
                    Some(ICON_OPEN),
                ),
                Some(State::Available(r)) => (
                    format!("{} {}", tr!("Вышла версия", "Version"), r.version) + tr!("", " is out"),
                    format!("{} {current}", tr!("Сейчас установлена", "Installed:")),
                    if r.has_setup() {
                        tr!("Обновление — пара секунд", "Updating takes seconds")
                    } else {
                        tr!("Скачайте на странице релиза", "Get it from the release page")
                    }
                    .to_string(),
                    Some(if r.has_setup() { ICON_DOWNLOAD } else { ICON_OPEN }),
                ),
                _ => return,
            };
            self.fill(rt, r, 6.0, t.card);
            self.fill(rt, r, 6.0, D2D1_COLOR_F { a: 0.14, ..t.accent });
            self.stroke(rt, r, 6.0, t.accent, 1.0);
            self.text(rt, ICON_DOWNLOAD, &f.icon, R { x: r.x + 10.0, y: r.y, w: 32.0, h: r.h }, t.accent);
            let b = warning_button_rect(r);
            let text_w = if button.is_some() { update_skip_rect(r).x } else { b.x + b.w } - r.x - 58.0;
            self.text(rt, &title, &f.name, R { x: r.x + 50.0, y: r.y + 10.0, w: text_w, h: 20.0 }, t.text);
            self.text(rt, &line1, &f.sub, R { x: r.x + 50.0, y: r.y + 30.0, w: text_w, h: 16.0 }, t.text2);
            self.text(rt, &line2, &f.sub, R { x: r.x + 50.0, y: r.y + 44.0, w: text_w, h: 16.0 }, t.text2);
            if let Some(icon) = button {
                self.fill(rt, b, 4.0, if hover(Hit::UpdateInstall) { D2D1_COLOR_F { a: 0.85, ..t.accent } } else { t.accent });
                self.text(rt, icon, &f.icon_small, b, t.on_accent);
                let s = update_skip_rect(r);
                if hover(Hit::UpdateSkip) {
                    self.fill(rt, s, 4.0, t.card_hover);
                }
                self.text(rt, ICON_CLOSE, &f.icon_small, s, t.text2);
            }
            return;
        }

        // Accent-tinted card: something in Windows is fighting over the lighting.
        let warning: Option<(String, String, String, &str, Hit)> = match row {
            Row::ServiceWarning => Some((
                tr!("Мешает служба Logitech", "A Logitech service interferes").into(),
                tr!("Из-за неё G915 зависает", "It makes the G915 hang").into(),
                tr!("и не слушает эффекты", "and ignore effects").into(),
                ICON_POWER,
                Hit::DisableService,
            )),
            Row::LightingWarning => Some((
                tr!("Динамическое освещение", "Dynamic Lighting is on").into(),
                tr!("Windows сама управляет подсветкой", "Windows drives the lighting itself").into(),
                tr!("и спорит с приложением", "and overrides this app").into(),
                ICON_SETTINGS,
                Hit::LightingSettings,
            )),
            Row::AppWarning(key) => crate::CONFLICTS.lock().unwrap().iter().find(|c| c.app.key == *key).map(|c| {
                let name = |id: &String| v.devices.iter().find(|d| d.id == *id).map(|d| d.name.clone()).unwrap_or_else(|| id.clone());
                let more = if c.devices.len() > 1 { format!(" +{}", c.devices.len() - 1) } else { String::new() };
                let line = |n: &str| format!("{} {n}{more}", tr!("Тоже управляет:", "Also drives:"));
                // The first device's name is shortened to keep the line clear of the button.
                let max_w = warning_button_rect(r).x - r.x - 62.0;
                let mut first: Vec<char> = c.devices.first().map(name).unwrap_or_default().chars().collect();
                let mut line1 = line(&first.iter().collect::<String>());
                while text_width(&line1, &f.sub) > max_w && first.len() > 1 {
                    first.pop();
                    line1 = line(&format!("{}…", first.iter().collect::<String>().trim_end()));
                }
                (format!("{} {}", tr!("Запущен", "Running:"), c.app.name), line1, crate::i18n::pick(c.app.remedy).to_string(), ICON_CLOSE, Hit::IgnoreApp(i))
            }),
            _ => None,
        };
        if let Some((title, line1, line2, button, hit)) = warning {
            let (title, line1, line2) = (title.as_str(), line1.as_str(), line2.as_str());
            self.fill(rt, r, 6.0, t.card);
            self.fill(rt, r, 6.0, D2D1_COLOR_F { a: 0.14, ..t.accent });
            self.stroke(rt, r, 6.0, t.accent, 1.0);
            self.text(rt, ICON_WARNING, &f.icon, R { x: r.x + 10.0, y: r.y, w: 32.0, h: r.h }, t.accent);
            let b = warning_button_rect(r);
            let text_w = b.x - r.x - 58.0;
            self.text(rt, title, &f.name, R { x: r.x + 50.0, y: r.y + 10.0, w: text_w, h: 20.0 }, t.text);
            self.text(rt, line1, &f.sub, R { x: r.x + 50.0, y: r.y + 30.0, w: text_w, h: 16.0 }, t.text2);
            self.text(rt, line2, &f.sub, R { x: r.x + 50.0, y: r.y + 44.0, w: text_w, h: 16.0 }, t.text2);
            self.fill(rt, b, 4.0, if hover(hit) { D2D1_COLOR_F { a: 0.85, ..t.accent } } else { t.accent });
            self.text(rt, button, &f.icon_small, b, t.on_accent);
            return;
        }
        if let Row::Spacer = row {
            return;
        }
        if let Row::Pager(page, pages) = *row {
            let parts = pager_parts(r, pages);
            for (b, hit) in parts {
                let hovered = hover(hit);
                match hit {
                    Hit::PagePrev | Hit::PageNext => {
                        let enabled = if hit == Hit::PagePrev { page > 0 } else { page + 1 < pages };
                        let mut c = if hovered && enabled { t.text } else { t.text2 };
                        if !enabled {
                            c.a *= 0.35;
                        }
                        let icon = if hit == Hit::PagePrev { ICON_CHEVRON_LEFT } else { ICON_CHEVRON_RIGHT };
                        self.text(rt, icon, &f.icon_tiny, b, c);
                    }
                    Hit::Page(k) if k == page => self.circle(rt, b.cx(), b.cy(), 4.0, t.accent),
                    _ => {
                        let mut c = t.text2;
                        c.a *= if hovered { 0.9 } else { 0.45 };
                        self.circle(rt, b.cx(), b.cy(), 3.0, c);
                    }
                }
            }
            return;
        }
        if let Row::ArchiveHeader(n) = row {
            // A plain foldable line, not a card.
            if hover(Hit::Card(i)) {
                self.fill(rt, r, 6.0, t.card_hover);
            }
            let open = ARCHIVE_OPEN.load(Ordering::Relaxed);
            self.text(
                rt,
                &format!("{} · {n}", tr!("Отключённые устройства", "Disconnected devices")),
                &f.sub,
                R { x: r.x + 14.0, y: r.y, w: r.w - 50.0, h: r.h },
                t.text2,
            );
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
            self.text(rt, tr!("Все устройства", "All devices"), &f.name, R { x: r.x + 50.0, y: r.y + 6.0, w: r.w - 100.0, h: 20.0 }, t.text);
            let sub = if n == shown.len() {
                format!("{} {n}", tr!("Подключено", "Connected"))
            } else {
                format!("{} {n} {} {}", tr!("Подключено", "Connected"), tr!("из", "of"), shown.len())
            };
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
                tr!("Нет связи", "No link").to_string()
            } else if !d.present {
                last_seen_label(d.cfg.last_seen)
            } else if d.cfg.per_key {
                tr!("Свои цвета клавиш", "Custom key colors").to_string()
            } else {
                d.cfg.effect.label().to_string()
            },
        )];
        if d.present && d.cfg.brightness < 100 && d.cfg.effect != Effect::Off {
            sub.push((ICON_BRIGHTNESS, format!("{}%", d.cfg.brightness)));
        }
        if d.present && d.cfg.speed != NORMAL_SPEED && d.cfg.animated() && crate::devices::caps(&d.id).has_speed() {
            sub.push((ICON_SPEED, speed_label(d.cfg.speed)));
        }
        // A model nobody has checked yet: a report from its owner is welcome.
        if d.present && !kind.tested {
            sub.push(("", tr!("не проверено", "untested").to_string()));
        }
        if let (true, Some(b)) = (d.present, d.battery) {
            sub.push(("", b.label()));
        }
        // The level from before the link was lost.
        if let (true, Some(b)) = (d.no_link, d.battery) {
            sub.push(("", format!("{} {}%", tr!("было", "was"), b.percent)));
        }
        if let Some(h) = d.cfg.hotkey {
            sub.push((ICON_KEYBOARD, h.label()));
        }
        if capturing_device().as_deref() == Some(d.id.as_str()) {
            sub = vec![("", tr!("Сочетание для вкл./выкл. · Del — убрать · Esc", "On/off shortcut · Del — remove · Esc").to_string())];
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
            // The on/off hotkey button, while the card is pointed at or the hotkey recorded.
            let recording = capturing_device().as_deref() == Some(d.id.as_str());
            if !archived && (recording || [Hit::Card(i), Hit::RenameDevice(i), Hit::DeviceHotkey(i)].into_iter().any(hover)) {
                let hk = hotkey_rect(r, row, &d.name);
                if recording {
                    self.fill(rt, hk, 3.0, t.accent);
                } else if hover(Hit::DeviceHotkey(i)) {
                    self.fill(rt, hk, 3.0, t.card_hover);
                }
                let fg = if recording {
                    col(255, 255, 255, 1.0)
                } else if hover(Hit::DeviceHotkey(i)) {
                    t.text
                } else {
                    t.text2
                };
                self.text(rt, ICON_KEYBOARD, &f.icon_tiny, hk, fg);
            }
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
            for (k, &(led, _)) in crate::devices::zones_of(&d.id).iter().enumerate().rev() {
                let mut z = dc.clone();
                z.set_rgb(dc.zone_rgb(led));
                self.circle(rt, dx - k as f32 * 9.0, dy, 8.0, if selected { t.card } else { bg });
                self.dot(rt, dx - k as f32 * 9.0, dy, 7.0, &z);
            }
        } else {
            self.dot(rt, dx, dy, 9.0, &dc);
        }

        match row {
            Row::Device(id) if crate::devices::caps(id).per_key => {
                let kb = keys_button_rect(r);
                self.fill(rt, kb, 4.0, if hover(Hit::Keys(i)) { t.card_hover } else { t.bg });
                self.stroke(rt, kb, 4.0, t.stroke, 1.0);
                self.text(rt, ICON_KEYBOARD, &f.icon_tiny, R { x: kb.x + 4.0, y: kb.y, w: 22.0, h: kb.h }, t.text);
                self.text(rt, tr!("Клавиши", "Keys"), &f.sub, R { x: kb.x + 26.0, y: kb.y, w: kb.w - 28.0, h: kb.h }, t.text);
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
                let label = if armed { tr!("Точно?", "Sure?") } else { tr!("Забыть", "Forget") };
                self.text(rt, ICON_DELETE, &f.icon_tiny, R { x: fb.x + 4.0, y: fb.y, w: 22.0, h: fb.h }, fg);
                self.text(rt, label, &f.sub, R { x: fb.x + 26.0, y: fb.y, w: fb.w - 28.0, h: fb.h }, fg);
            }
            _ => {}
        }
    }

    /// The preset menu: a raised card over a dimmed panel.
    unsafe fn draw_menu(&self, rt: &ID2D1RenderTarget) {
        let (Some((panel, items)), Some(target)) = (menu_layout(), menu_target()) else { return };
        let (name, hotkey, same) = match &target {
            MenuTarget::Preset(i) => {
                let Some(preset) = crate::config().presets.get(*i).cloned() else { return };
                (preset.name, preset.hotkey, preset_saved(*i))
            }
            MenuTarget::Device(id) => {
                let name = crate::config().display_name(id);
                (name, target.hotkey(), false)
            }
        };
        let t = &self.theme;
        let f = &self.fmt;
        let hover = |h: Hit| self.hover == Some(h);

        self.fill(rt, R { x: 0.0, y: 0.0, w: W, h: panel_h() }, 0.0, col(0, 0, 0, if t.dark { 0.35 } else { 0.18 }));
        // Soft shadow: a few widening translucent layers.
        for k in 1..=6 {
            let d = k as f32 * 2.0;
            self.fill(
                rt,
                R { x: panel.x - d + 1.0, y: panel.y - d + 4.0, w: panel.w + 2.0 * d - 2.0, h: panel.h + 2.0 * d - 2.0 },
                8.0 + d,
                col(0, 0, 0, 0.05),
            );
        }
        self.fill(rt, panel, 8.0, t.bg);
        self.fill(rt, panel, 8.0, t.card);
        self.stroke(rt, panel, 8.0, t.stroke, 1.0);

        // Header: preset name and its hotkey.
        let head = R { x: panel.x + 14.0, y: panel.y + 6.0, w: panel.w - 28.0, h: MENU_HEADER_H };
        self.text(rt, &name, &f.label, R { w: head.w - 90.0, ..head }, t.text);
        if let Some(hk) = hotkey {
            self.text(rt, &hk.label(), &f.sub_right, head, t.text2);
        }

        // A device menu has no actions: the hotkey part starts under the header.
        let mut line_after_actions = panel.y + 6.0 + MENU_HEADER_H;
        for &(r, item) in &items {
            let h = Hit::Menu(item);
            match item {
                MenuItem::Apply | MenuItem::Overwrite | MenuItem::Rename | MenuItem::Export | MenuItem::Delete => {
                    let disabled = item == MenuItem::Overwrite && same;
                    if hover(h) && !disabled {
                        self.fill(rt, r, 4.0, t.card_hover);
                    }
                    let (icon, label) = match item {
                        MenuItem::Apply => (ICON_PLAY, tr!("Применить", "Apply")),
                        MenuItem::Overwrite => (
                            ICON_SAVE,
                            if same {
                                tr!("Совпадает с текущими", "Same as current")
                            } else {
                                tr!("Перезаписать текущими", "Overwrite with current")
                            },
                        ),
                        MenuItem::Rename => (ICON_RENAME, tr!("Переименовать", "Rename")),
                        MenuItem::Export => (ICON_SHARE, tr!("Сохранить в файл…", "Save to a file…")),
                        _ => (ICON_DELETE, tr!("Удалить", "Delete")),
                    };
                    let mut fg = t.text;
                    if disabled {
                        fg = t.text2;
                        fg.a *= 0.6;
                    }
                    self.text(rt, icon, &f.icon_small, R { x: r.x + 6.0, y: r.y, w: 28.0, h: r.h }, fg);
                    self.text(rt, label, &f.sub, R { x: r.x + 40.0, y: r.y, w: r.w - 44.0, h: r.h }, fg);
                    if item == MenuItem::Export {
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
                        MenuItem::Hotkey(Some(n)) => hotkey.is_some_and(|h| h.vk == VK_F13 + n),
                        MenuItem::Hotkey(None) => hotkey.is_none(),
                        _ => hotkey.is_some_and(|h| !is_f13_24(h.vk)),
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
                        MenuItem::Hotkey(None) => tr!("Нет", "None").to_string(),
                        _ => match hotkey {
                            Some(hk) if current => hk.label(),
                            _ => tr!("Сочетание…", "Shortcut…").to_string(),
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
            tr!("Горячая клавиша", "Hotkey"),
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
        if std::env::var_os("RGBTRAY_DEMO_SPEED").is_some() {
            // The Naga too: its protocol has no speed, so it shows neither the slider nor the factor.
            for (_, d) in cfg.devices.iter_mut().filter(|(id, _)| id.starts_with("logitech") || id.starts_with("naga")) {
                d.effect = Effect::Breathe;
                d.per_key = false;
                d.speed = 75;
            }
        }

        if cfg.presets.is_empty() {
            let mut a = cfg.snapshot(tr!("Работа", "Work").into());
            a.hotkey = Some(crate::config::Hotkey { mods: 0, vk: VK_F13 });
            let mut b = cfg.snapshot(tr!("Игра", "Gaming").into());
            b.brightness = 40;
            b.hotkey = Some(crate::config::Hotkey { mods: 0x2 | 0x1, vk: 0x31 });
            let c = cfg.snapshot(tr!("Ночь", "Night").into());
            cfg.presets = vec![a, b, c];
            cfg.presets[2].brightness = 10;
        }
    }
    // RGBTRAY_DEMO_MENU=device: the hotkey menu of the last device instead of the first preset.
    match std::env::var("RGBTRAY_DEMO_MENU").ok().as_deref() {
        Some("device") => {
            if let Some(id) = crate::active_ids().last().cloned() {
                open_menu(MenuTarget::Device(id), PAD + 40.0, 160.0);
            }
        }
        Some(_) => open_menu(MenuTarget::Preset(0), PAD + 40.0, lower_y() + PRESET_Y + 20.0),
        None => {}
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
            drag_speed: None,
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

/// A device's own speed as a factor of the overall one: "×2", "×0.5", "×1.3".
fn speed_label(speed: u8) -> String {
    let f = speed_factor(speed);
    let s = if f < 1.0 { format!("{f:.2}") } else { format!("{f:.1}") };
    format!("×{}", s.trim_end_matches('0').trim_end_matches('.'))
}

fn speed_value(x: f32) -> u8 {
    let s = speed_rect();
    let (x0, x1) = (s.x + SPEED_THUMB, s.x + s.w - SPEED_THUMB);
    let v = (((x - x0) / (x1 - x0)).clamp(0.0, 1.0) * 100.0).round() as u8;
    snap_speed(v, selected().map_or(Speed::Smooth, |id| crate::devices::caps(&id).speed))
}

/// A device with a few fixed speeds gets only those; elsewhere the normal speed is easy to hit again.
fn snap_speed(v: u8, speed: Speed) -> u8 {
    match speed {
        Speed::Steps(n) if n >= 2 => {
            let step = 100.0 / (n - 1) as f32;
            ((v as f32 / step).round() * step).round() as u8
        }
        Speed::Steps(_) | Speed::None => NORMAL_SPEED,
        Speed::Smooth if v.abs_diff(NORMAL_SPEED) <= 3 => NORMAL_SPEED,
        Speed::Smooth => v,
    }
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
                // Opening may move the header to the page of the devices it unfolds: follow it.
                if let Some(p) = header_page() {
                    PAGE.store(p, Ordering::Relaxed);
                }
            }
            Some(Row::ServiceWarning | Row::LightingWarning | Row::AppWarning(_) | Row::Update | Row::Pager(..) | Row::Spacer) | None => {}
        },
        Hit::PagePrev => turn_page(-1),
        Hit::PageNext => turn_page(1),
        Hit::Page(k) => PAGE.store(k, Ordering::Relaxed),
        Hit::LightingSettings => crate::open_dynamic_lighting_settings(),
        Hit::UpdateInstall => match crate::update::offered() {
            Some(crate::update::State::Available(r)) if r.has_setup() => crate::update::install(),
            _ => crate::update::open_page(),
        },
        Hit::UpdateSkip => crate::update::skip(),
        Hit::IgnoreApp(i) => {
            if let Some((_, Row::AppWarning(key))) = rows().get(i).cloned() {
                crate::ignore_app(key);
            }
        }
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
        Hit::DeviceHotkey(i) => {
            if let Some((r, ref row @ Row::Device(ref id))) = rows().get(i).cloned() {
                finish_rename(true);
                stop_capture();
                let name = crate::config().display_name(id);
                let b = hotkey_rect(r, row, &name);
                open_menu(MenuTarget::Device(id.clone()), b.x, b.y + b.h + 4.0);
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
            if !preset_saved(i) {
                crate::overwrite_preset(i);
            }
        }
        Hit::PresetHotkey(i) => {
            finish_rename(true);
            CAPTURE.store(i, Ordering::Relaxed);
        }
        Hit::Menu(item) => {
            if let Some(target) = menu_target() {
                menu_action(hwnd, target, item);
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
        Hit::Reapply => crate::reapply_all(),
        Hit::Pin => {
            let pinned = !PINNED.fetch_xor(true, Ordering::Relaxed);
            let mut cfg = crate::config();
            cfg.pinned = pinned;
            cfg.save();
        }
        Hit::Exit => {
            DestroyWindow(hwnd);
            crate::quit();
            return;
        }
        Hit::Slider | Hit::Speed => {}
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
                if f.drag_speed.is_some() {
                    f.drag_speed = Some(speed_value(x));
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
                match hit_test(x, y) {
                    Some(Hit::Slider) => f.drag = Some(slider_value(x)),
                    Some(Hit::Speed) => f.drag_speed = Some(speed_value(x)),
                    _ => return false,
                }
                true
            });
            if started == Some(true) {
                SetCapture(hwnd);
                invalidate();
            }
            0
        }
        WM_LBUTTONUP => {
            let (drag, drag_speed, hit) = with_flyout(|f| {
                let (x, y) = mouse_pos(lparam, f.scale);
                (f.drag.take(), f.drag_speed.take(), hit_test(x, y))
            })
            .unwrap_or((None, None, None));
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
            } else if let Some(s) = drag_speed {
                ReleaseCapture();
                crate::set_speed(selected().as_deref(), s);
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
        WM_KEYDOWN | WM_SYSKEYDOWN if capturing() => {
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
        // The wheel over the device list turns its pages.
        WM_MOUSEWHEEL => {
            let mut pt = POINT { x: (lparam & 0xFFFF) as i16 as i32, y: ((lparam >> 16) & 0xFFFF) as i16 as i32 };
            ScreenToClient(hwnd, &mut pt);
            let over_list = with_flyout(|f| (pt.y as f32 / f.scale) < lower_y()).unwrap_or(false);
            let delta = ((wparam >> 16) & 0xFFFF) as i16;
            if over_list && delta != 0 {
                turn_page(if delta > 0 { -1 } else { 1 });
                invalidate();
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
                stop_capture();
                open_menu(MenuTarget::Preset(i), x, y);
            }
            invalidate();
            0
        }
        // A pinned panel is dragged by the empty part of its header.
        WM_NCHITTEST if PINNED.load(Ordering::Relaxed) => {
            let mut pt = POINT { x: (lparam & 0xFFFF) as i16 as i32, y: ((lparam >> 16) & 0xFFFF) as i16 as i32 };
            ScreenToClient(hwnd, &mut pt);
            let caption = with_flyout(|f| {
                let (x, y) = (pt.x as f32 / f.scale, pt.y as f32 / f.scale);
                (0.0..HEADER_DRAG_H).contains(&y) && menu_target().is_none() && hit_test(x, y).is_none()
            });
            if caption == Some(true) {
                HTCAPTION as LRESULT
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        // No maximizing or system menu from the drag area.
        WM_NCLBUTTONDBLCLK | WM_NCRBUTTONUP => 0,
        WM_EXITSIZEMOVE => {
            let mut rc: RECT = zeroed();
            GetWindowRect(hwnd, &mut rc);
            // From now on the panel grows downwards from where it was put.
            with_flyout(|f| f.anchor_bottom = false);
            let mut cfg = crate::config();
            cfg.panel_pos = Some((rc.left, rc.top));
            cfg.save();
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
            stop_capture();
            finish_rename(true);
            LAST_CLOSE.store(GetTickCount64(), Ordering::Relaxed);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_labels() {
        assert_eq!(speed_label(100), "×4");
        assert_eq!(speed_label(75), "×2");
        assert_eq!(speed_label(25), "×0.5");
        assert_eq!(speed_label(0), "×0.25");
        assert_eq!(speed_label(60), "×1.3");
    }

    #[test]
    fn speed_snaps_to_steps() {
        assert_eq!(snap_speed(20, Speed::Steps(3)), 0);
        assert_eq!(snap_speed(30, Speed::Steps(3)), 50);
        assert_eq!(snap_speed(80, Speed::Steps(3)), 100);
        assert_eq!(snap_speed(40, Speed::Steps(5)), 50);
        assert_eq!(snap_speed(10, Speed::Steps(5)), 0);
        assert_eq!(snap_speed(90, Speed::None), NORMAL_SPEED);
        assert_eq!(snap_speed(52, Speed::Smooth), NORMAL_SPEED);
        assert_eq!(snap_speed(60, Speed::Smooth), 60);
    }
}
