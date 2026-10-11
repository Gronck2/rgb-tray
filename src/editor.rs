//! Per-key color editor for keyboards with per-key lighting (Logitech G915, LampArray
//! keyboards whose lamps are bound to keys): a drawn TKL layout to paint on.
//! Changes go to the keyboard immediately through a dedicated worker thread.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;

use windows::core::w;
use windows::Win32::Foundation::HWND as WinHwnd;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::config::Scheme;
use crate::devices::Device;
use crate::devices::{Effect, Rgb};
use crate::keymap::{self, KeyDef};
use crate::ui::{col, rgb_col, text_format, theme, Theme, PALETTE, R};

// ---------------------------------------------------------------- layout (in DIPs)

const U: f32 = 40.0;
const MARGIN: f32 = 24.0;
const KB_X: f32 = MARGIN;
const KB_Y: f32 = 72.0;
/// Room for the keyboard: the G915 TKL at 40 px a key; other layouts are scaled to fit.
const KB_W: f32 = 18.25 * U;
const KB_H: f32 = 7.1 * U;
const W: f32 = KB_W + 2.0 * MARGIN;
const ROW_Y: f32 = KB_Y + KB_H + 28.0;
const ROW_STEP: f32 = 48.0;
const ROW_H: f32 = 32.0;
const LABEL_W: f32 = 84.0;
const BOTTOM_Y: f32 = ROW_Y + ROW_STEP * 4.0 + 8.0;
const H: f32 = BOTTOM_Y + 36.0 + MARGIN;

const PRESETS: [crate::i18n::Text; 4] = [("Радуга", "Rainbow"), ("Градиент", "Gradient"), ("WASD", "WASD"), ("Только буквы", "Letters only")];
const OFF: Rgb = Rgb(0, 0, 0);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Key(u8),
    Brush(usize),
    BrushOff,
    BrushCustom,
    Fill,
    Clear,
    Preset(usize),
    /// Color dot `k` of the two-color preset `i`.
    Slot(usize, usize),
    WordBox,
    WordApply,
    Layout(bool),
    SaveScheme,
    Scheme(usize),
    Done,
}

// ---------------------------------------------------------------- keyboard worker

enum Msg {
    /// Switch to per-key mode and send every key.
    Full,
    Keys(Vec<(u8, Rgb)>),
    Stop,
}

static WORKER: Mutex<Option<Sender<Msg>>> = Mutex::new(None);

/// Id of the keyboard being edited.
static TARGET: Mutex<String> = Mutex::new(String::new());

/// Key layout of the keyboard being edited.
/// `RGBTRAY_DEMO_LAYOUT=full|tkl`: a snapshot of another layout than the keyboard's.
static DEMO_LAYOUT: Mutex<Option<String>> = Mutex::new(None);

fn layout() -> &'static keymap::Layout {
    match DEMO_LAYOUT.lock().unwrap().as_deref() {
        Some(name) => keymap::get(name),
        None => keymap::get(crate::devices::caps(&target()).layout),
    }
}

fn target() -> String {
    TARGET.lock().unwrap().clone()
}

/// Settings of the keyboard being edited.
fn keyboard(cfg: &mut crate::config::Config) -> &mut crate::config::DeviceConfig {
    cfg.device_mut(&target())
}

fn send(msg: Msg) {
    let mut worker = WORKER.lock().unwrap();
    if worker.is_none() {
        let (tx, rx) = channel::<Msg>();
        std::thread::spawn(move || {
            let mut kb: Option<Box<dyn Device>> = None;
            while let Ok(first) = rx.recv() {
                // Coalesce everything queued while the previous batch was being sent.
                let mut full = false;
                let mut keys = Vec::new();
                for msg in std::iter::once(first).chain(rx.try_iter()) {
                    match msg {
                        Msg::Full => full = true,
                        Msg::Keys(k) => keys.extend(k),
                        Msg::Stop => return,
                    }
                }
                let _guard = crate::APPLY_LOCK.lock().unwrap();
                if kb.is_none() {
                    kb = crate::devices::open_for_keys(&target());
                    full = true;
                }
                let Some(k) = &kb else { continue };
                let (brightness, all) = {
                    let id = target();
                    let cfg = crate::config();
                    (cfg.effective_brightness(&id), cfg.device(&id).all_keys(layout().codes()))
                };
                let res = if full {
                    k.apply_keys(&all, brightness)
                } else {
                    // Last color of each key wins.
                    let mut latest: Vec<(u8, Rgb)> = Vec::new();
                    for (code, c) in keys.into_iter().rev() {
                        if !latest.iter().any(|(k, _)| *k == code) {
                            latest.push((code, c));
                        }
                    }
                    k.update_keys(&latest, brightness)
                };
                if let Err(e) = res {
                    crate::log(&format!("key editor: {e}"));
                    kb = None;
                }
            }
        });
        *worker = Some(tx);
    }
    if let Some(tx) = worker.as_ref() {
        let _ = tx.send(msg);
    }
}

fn stop_worker() {
    if let Some(tx) = WORKER.lock().unwrap().take() {
        let _ = tx.send(Msg::Stop);
    }
}

// ---------------------------------------------------------------- color helpers

fn luminance(c: Rgb) -> f32 {
    (0.299 * c.0 as f32 + 0.587 * c.1 as f32 + 0.114 * c.2 as f32) / 255.0
}

fn hsv(h: f32, s: f32, v: f32) -> Rgb {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let f = |t: f32| ((t + m) * 255.0).round() as u8;
    Rgb(f(r), f(g), f(b))
}

fn to_hsv(c: Rgb) -> (f32, f32, f32) {
    let (r, g, b) = (c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb(f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

// ---------------------------------------------------------------- per-key history

/// Previous colors of each key, newest last; right click rolls a key back one step.
static HISTORY: Mutex<Vec<(u8, Vec<Rgb>)>> = Mutex::new(Vec::new());
const HISTORY_DEPTH: usize = 50;

/// Sets a key color and remembers the old one for rollback.
fn set_tracked(cfg: &mut crate::config::Config, code: u8, c: Rgb) {
    let old = keyboard(cfg).key_color(code);
    if old == c {
        return;
    }
    let mut history = HISTORY.lock().unwrap();
    let pos = history.iter().position(|(k, _)| *k == code).unwrap_or_else(|| {
        history.push((code, Vec::new()));
        history.len() - 1
    });
    let stack = &mut history[pos].1;
    if stack.len() == HISTORY_DEPTH {
        stack.remove(0);
    }
    stack.push(old);
    keyboard(cfg).set_key(code, c);
}

/// Restores the previous color of a key; returns it, or None when there is no history.
fn rollback_key(code: u8) -> Option<Rgb> {
    let prev = HISTORY.lock().unwrap().iter_mut().find(|(k, _)| *k == code).and_then(|(_, s)| s.pop())?;
    keyboard(&mut crate::config()).set_key(code, prev);
    send(Msg::Keys(vec![(code, prev)]));
    Some(prev)
}

// ---------------------------------------------------------------- presets

/// Presets in PRESETS that use two chosen colors (shown as dots on their buttons).
const GRADIENT: usize = 1;
const WASD: usize = 2;
const LETTERS: usize = 3;

fn two_color(i: usize) -> bool {
    matches!(i, GRADIENT | WASD | LETTERS)
}

/// Starting colors of the two-color presets.
fn default_colors() -> [[Rgb; 2]; PRESETS.len()] {
    let start = PALETTE[0];
    let (h, s, v) = to_hsv(start);
    let mut c = [[start, OFF]; PRESETS.len()];
    // Gradient: the start color into a shifted hue. WASD: bright keys on a dim keyboard.
    c[GRADIENT] = [start, hsv(h + 150.0, s.max(0.6), v.max(0.8))];
    c[WASD] = [start, mix(OFF, start, 0.12)];

    c
}

/// Keys of the current layout plus those only the other layout has: the keyboard has lights
/// for both (e.g. an ANSI backslash lit by the ISO zone), and presets must cover them all.
fn preset_keys(iso: bool) -> Vec<KeyDef> {
    let mut keys = layout().keys(iso).to_vec();
    for &kd in layout().keys(!iso) {
        if !keys.iter().any(|k| k.code == kd.code) {
            keys.push(kd);
        }
    }
    keys
}

fn apply_preset(i: usize, colors: [Rgb; 2]) {
    let mut cfg = crate::config();
    let keys = preset_keys(cfg.iso_layout);
    let center = |kd: &KeyDef| (kd.x + kd.w / 2.0) / layout().width;
    match i {
        // Static rainbow across the keyboard: zero cost, the firmware just holds it.
        0 => {
            for kd in keys.iter().filter(|k| k.code != 0) {
                set_tracked(&mut cfg, kd.code, hsv(center(kd) * 300.0, 1.0, 1.0));
            }
        }
        // The two chosen colors across the keyboard, left to right.
        GRADIENT => {
            for kd in keys.iter().filter(|k| k.code != 0) {
                set_tracked(&mut cfg, kd.code, mix(colors[0], colors[1], center(kd)));
            }
        }
        // Gaming keys in one color, everything else in the other.
        WASD => {
            let hot = ["W", "A", "S", "D", "\u{2191}", "\u{2190}", "\u{2193}", "\u{2192}"];
            for kd in keys.iter().filter(|k| k.code != 0) {
                let on = kd.y > 0.0 && hot.contains(&kd.label);
                set_tracked(&mut cfg, kd.code, if on { colors[0] } else { colors[1] });
            }
        }

        // Letters (Latin, and Cyrillic legends in Russian) in one color, everything else in the other.
        _ => {
            for kd in keys.iter().filter(|k| k.code != 0) {
                let letter = kd.y > 0.0 && kd.label.len() == 1 && kd.label.chars().all(|c| c.is_ascii_alphabetic());
                set_tracked(&mut cfg, kd.code, if letter || !crate::i18n::en() && !kd.ru.is_empty() && kd.label.len() == 1 { colors[0] } else { colors[1] });
            }
        }
    }
    cfg.save();
    drop(cfg);
    send_all();
}

fn send_all() {
    send(Msg::Keys(crate::config().device(&target()).all_keys(layout().codes())));
}

fn fill_all(c: Rgb) {
    let mut cfg = crate::config();
    for &code in layout().codes() {
        set_tracked(&mut cfg, code, c);
    }
    cfg.save();
    drop(cfg);
    send_all();
}

fn write_word(word: &str, brush: Rgb) {
    let brush = if brush == OFF { PALETTE[0] } else { brush };
    let mut cfg = crate::config();
    for &code in layout().codes() {
        set_tracked(&mut cfg, code, OFF);
    }
    for ch in word.chars() {
        if let Some(code) = layout().code_for_char(ch) {
            set_tracked(&mut cfg, code, brush);
        }
    }
    if layout().codes().contains(&keymap::LOGO) {
        set_tracked(&mut cfg, keymap::LOGO, brush);
    }
    cfg.save();
    drop(cfg);
    send_all();
}

// ---------------------------------------------------------------- window state

struct Formats {
    title: IDWriteTextFormat,
    label: IDWriteTextFormat,
    body: IDWriteTextFormat,
    hint_right: IDWriteTextFormat,
    chip: IDWriteTextFormat,
    key: IDWriteTextFormat,
    key_small: IDWriteTextFormat,
    key_lead: IDWriteTextFormat,
    key_ru: IDWriteTextFormat,
    key_icon: IDWriteTextFormat,
}

struct Editor {
    hwnd: HWND,
    scale: f32,
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    fmt: Formats,
    theme: Theme,
    hits: Vec<(R, Hit)>,
    hover: Option<Hit>,
    tracking_leave: bool,
    painting: bool,
    /// Right button held: keys passed over are rolled back.
    rolling: bool,
    last_painted: Option<u8>,
    brush_color: Rgb,
    /// The two colors of each two-color preset.
    preset_colors: [[Rgb; 2]; PRESETS.len()],
    /// The preset color dot the palette currently sets (None: the palette sets the brush).
    slot: Option<(usize, usize)>,
    word: String,
    word_focus: bool,
}

thread_local! {
    static EDITOR: RefCell<Option<Editor>> = const { RefCell::new(None) };
}

fn with_editor<T>(f: impl FnOnce(&mut Editor) -> T) -> Option<T> {
    EDITOR.with(|cell| cell.borrow_mut().as_mut().map(f))
}

fn invalidate() {
    if let Some(h) = with_editor(|e| e.hwnd) {
        unsafe { InvalidateRect(h, null(), 0) };
    }
}

fn formats(dw: &IDWriteFactory) -> windows::core::Result<Formats> {
    let ui = w!("Segoe UI Variable Text");
    let icons = w!("Segoe Fluent Icons");
    let (lead, trail, center) = (DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_ALIGNMENT_CENTER);
    let normal = DWRITE_FONT_WEIGHT_NORMAL;
    let semi = DWRITE_FONT_WEIGHT_SEMI_BOLD;
    let key_lead = text_format(dw, ui, 12.0, semi, lead)?;
    let key_ru = text_format(dw, ui, 10.0, normal, trail)?;
    unsafe {
        key_lead.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
        key_ru.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_FAR)?;
    }
    Ok(Formats {
        title: text_format(dw, w!("Segoe UI Variable Display"), 20.0, semi, lead)?,
        label: text_format(dw, ui, 14.0, semi, lead)?,
        body: text_format(dw, ui, 13.0, normal, lead)?,
        hint_right: text_format(dw, ui, 12.0, normal, trail)?,
        chip: text_format(dw, ui, 13.0, normal, center)?,
        key: text_format(dw, ui, 12.0, semi, center)?,
        key_small: text_format(dw, ui, 10.0, semi, center)?,
        key_lead,
        key_ru,
        key_icon: text_format(dw, icons, 12.0, normal, center)?,
    })
}

/// Opens the editor for a keyboard (or focuses it, switching to that keyboard) and switches
/// the keyboard to per-key colors.
pub fn open(id: &str) {
    unsafe {
        let switching = target() != id;
        if switching {
            // Another keyboard: its own connection and its own rollback history.
            stop_worker();
            HISTORY.lock().unwrap().clear();
            *TARGET.lock().unwrap() = id.to_string();
        }
        {
            let mut cfg = crate::config();
            let d = cfg.device_mut(id);
            if !d.per_key {
                // Keys never set (all of them the first time, or keys added to the layout later)
                // start from what the keyboard shows now.
                let c = if d.effect == Effect::Off { OFF } else { d.rgb() };
                for &code in layout().codes() {
                    if !d.has_key(code) {
                        d.set_key(code, c);
                    }
                }
                d.per_key = true;
                cfg.save();
            }
        }
        send(Msg::Full);
        if let Some(h) = with_editor(|e| e.hwnd) {
            set_title(h);
            InvalidateRect(h, null(), 0);
            ShowWindow(h, SW_RESTORE);
            SetForegroundWindow(h);
            return;
        }
        if let Err(e) = create_window() {
            crate::log(&format!("key editor window: {e}"));
        }
    }
}

/// Redraws an open editor, e.g. after the interface language changed.
pub fn refresh() {
    if let Some(h) = with_editor(|e| e.hwnd) {
        unsafe {
            set_title(h);
            InvalidateRect(h, null(), 0);
        }
    }
}

/// "Цвета клавиш — <keyboard name>".
unsafe fn set_title(hwnd: HWND) {
    let name = crate::config().display_name(&target());
    let title: Vec<u16> = format!("{} — {name}\0", tr!("Цвета клавиш", "Key colors")).encode_utf16().collect();
    SetWindowTextW(hwnd, title.as_ptr());
}

unsafe fn create_window() -> windows::core::Result<()> {
    let hinst = GetModuleHandleW(null());
    let class: Vec<u16> = "RgbTrayKeyEditor\0".encode_utf16().collect();
    let mut wc: WNDCLASSW = zeroed();
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinst;
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.lpszClassName = class.as_ptr();
    RegisterClassW(&wc);

    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(monitor, &mut mi);
    let (mut dpi, mut dpi_y) = (96u32, 96u32);
    GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
    let scale = dpi as f32 / 96.0;
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    let mut rc = RECT { left: 0, top: 0, right: (W * scale).round() as i32, bottom: (H * scale).round() as i32 };
    AdjustWindowRectExForDpi(&mut rc, style, 0, 0, dpi);
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let work = mi.rcWork;
    let x = work.left + ((work.right - work.left) - w).max(0) / 2;
    let y = work.top + ((work.bottom - work.top) - h).max(0) / 2;

    let title: Vec<u16> = tr!("Цвета клавиш\0", "Key colors\0").encode_utf16().collect();
    let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), style, x, y, w, h, null_mut(), null_mut(), hinst, null());
    set_title(hwnd);
    let theme = theme();
    let dark: u32 = theme.dark as u32;
    DwmSetWindowAttribute(hwnd, 20, &dark as *const u32 as _, 4); // dark title bar
    let bg = theme.bg;
    let caption: u32 = (bg.r * 255.0) as u32 | ((bg.g * 255.0) as u32) << 8 | ((bg.b * 255.0) as u32) << 16;
    DwmSetWindowAttribute(hwnd, 35, &caption as *const u32 as _, 4); // caption color

    let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
    let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
    let fmt = formats(&dw)?;
    let ed = Editor {
        hwnd,
        scale,
        d2d,
        dw,
        rt: None,
        brush: None,
        fmt,
        theme,
        hits: Vec::new(),
        hover: None,
        tracking_leave: false,
        painting: false,
        rolling: false,
        last_painted: None,
        brush_color: PALETTE[0],
        preset_colors: default_colors(),
        slot: None,
        word: String::new(),
        word_focus: false,
    };
    EDITOR.with(|c| *c.borrow_mut() = Some(ed));
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    Ok(())
}

/// Renders the editor into a PNG file (no window needed), for checking the look.
pub fn snapshot(path: &str, dark: Option<bool>, scale: f32) -> windows::core::Result<()> {
    *DEMO_LAYOUT.lock().unwrap() = std::env::var("RGBTRAY_DEMO_LAYOUT").ok();
    if target().is_empty() {
        let first = crate::PRESENT.lock().unwrap().iter().find(|id| crate::devices::caps(id).per_key).cloned();
        *TARGET.lock().unwrap() = first.unwrap_or_else(|| "logitech".into());
    }

    crate::ui::render_png(path, W, H, scale, |rt, d2d| unsafe {
        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let theme = crate::ui::theme_for(dark);
        {
            // Demo colors in memory only (never saved) so the picture isn't all dark.
            let mut cfg = crate::config();
            let iso = cfg.iso_layout;
            let d = keyboard(&mut cfg);
            if d.keys.is_empty() {
                for kd in layout().keys(iso).iter().filter(|k| k.code != 0) {
                    d.set_key(kd.code, hsv((kd.x + kd.w / 2.0) / layout().width * 300.0, 1.0, 1.0));
                }
                for label in ["W", "A", "S", "D"] {
                    d.set_key(layout().code_by_label(label).unwrap_or(0), OFF);
                }
            }
        }
        let mut ed = Editor {
            hwnd: null_mut(),
            scale,
            d2d,
            fmt: formats(&dw)?,
            dw,
            rt: None,
            brush: Some(rt.CreateSolidColorBrush(&theme.text, None)?),
            theme,
            hits: Vec::new(),
            hover: Some(Hit::Key(layout().code_by_label("G").unwrap_or(0))),
            tracking_leave: false,
            painting: false,
            rolling: false,
            last_painted: None,
            brush_color: PALETTE[0],
            preset_colors: default_colors(),
            slot: None,
            word: tr!("ПРИВЕТ", "HELLO").into(),
            word_focus: true,
        };
        ed.draw(rt);
        Ok(())
    })
}

// ---------------------------------------------------------------- painting

impl Editor {
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
        unsafe {
            rt.BeginDraw();
            self.hits.clear();
            self.draw(&rt);
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

    fn text_width(&self, s: &str, fmt: &IDWriteTextFormat) -> f32 {
        let s: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            self.dw
                .CreateTextLayout(&s, fmt, 2000.0, 100.0)
                .and_then(|l| {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    l.GetMetrics(&mut m).map(|_| m.widthIncludingTrailingWhitespace)
                })
                .unwrap_or(s.len() as f32 * 7.0)
        }
    }

    /// Draws a pill button and returns its right edge.
    unsafe fn chip(&mut self, rt: &ID2D1RenderTarget, x: f32, y: f32, label: &str, hit: Hit, accent: bool) -> f32 {
        let t = &self.theme;
        let r = R { x, y, w: self.text_width(label, &self.fmt.chip) + 28.0, h: ROW_H };
        let hovered = self.hover == Some(hit);
        let (bg, fg) = if accent {
            (if hovered { D2D1_COLOR_F { a: 0.85, ..t.accent } } else { t.accent }, t.on_accent)
        } else {
            (if hovered { t.card_hover } else { t.card }, t.text)
        };
        self.fill(rt, r, 4.0, bg);
        if !accent {
            self.stroke(rt, r, 4.0, t.stroke, 1.0);
        }
        self.text(rt, label, &self.fmt.chip, r, fg);
        self.hits.push((r, hit));
        r.x + r.w
    }

    /// A two-color preset: its label plus its two color dots; returns its right edge.
    unsafe fn two_color_chip(&mut self, rt: &ID2D1RenderTarget, x: f32, y: f32, i: usize) -> f32 {
        let t = self.theme;
        let label = crate::i18n::pick(PRESETS[i]);
        let label_w = self.text_width(label, &self.fmt.chip) + 24.0;
        let r = R { x, y, w: label_w + 2.0 * 26.0 + 6.0, h: ROW_H };
        self.fill(rt, r, 4.0, if self.hover == Some(Hit::Preset(i)) { t.card_hover } else { t.card });
        self.stroke(rt, r, 4.0, t.stroke, 1.0);
        self.text(rt, label, &self.fmt.chip, R { w: label_w, ..r }, t.text);
        // Dots first in the hit list: they sit on top of the chip.
        for k in 0..2 {
            let d = R { x: r.x + label_w + k as f32 * 26.0, y: r.y + 3.0, w: 26.0, h: r.h - 6.0 };
            let active = self.slot == Some((i, k));
            if active {
                self.ring(rt, d.cx(), d.cy(), 11.0, t.accent, 2.0);
            } else if self.hover == Some(Hit::Slot(i, k)) {
                self.ring(rt, d.cx(), d.cy(), 11.0, t.text2, 1.5);
            }
            let c = self.preset_colors[i][k];
            if c == OFF {
                self.ring(rt, d.cx(), d.cy(), 7.5, t.text2, 1.5);
            } else {
                self.circle(rt, d.cx(), d.cy(), 8.0, rgb_col(c));
                self.ring(rt, d.cx(), d.cy(), 7.5, t.stroke, 1.0);
            }
            self.hits.push((d, Hit::Slot(i, k)));
        }
        self.hits.push((r, Hit::Preset(i)));
        r.x + r.w
    }

    unsafe fn draw(&mut self, rt: &ID2D1RenderTarget) {
        let (keys_cfg, iso, schemes) = {
            let cfg = crate::config();
            let colors: Vec<(u8, Rgb)> = cfg.device(&target()).all_keys(layout().codes());
            (colors, cfg.iso_layout, cfg.schemes.iter().map(|s| s.name.clone()).collect::<Vec<_>>())
        };
        let color_of = |code: u8| keys_cfg.iter().find(|(c, _)| *c == code).map(|(_, c)| *c).unwrap_or(OFF);
        let t_dark = self.theme.dark;
        rt.Clear(Some(&self.theme.bg));

        // Header.
        self.text(rt, tr!("Цвета клавиш", "Key colors"), &self.fmt.title, R { x: MARGIN, y: 18.0, w: 300.0, h: 32.0 }, self.theme.text);
        self.text(
            rt,
            tr!("ЛКМ — покрасить · ПКМ — откатить цвет · Ctrl+ЛКМ — пипетка", "Left click — paint · Right click — undo · Ctrl+click — eyedropper"),
            &self.fmt.hint_right,
            R { x: W - MARGIN - 460.0, y: 18.0, w: 460.0, h: 32.0 },
            self.theme.text2,
        );

        // Keyboard body: the real keyboard is black in any theme.
        let body = R { x: KB_X - 12.0, y: KB_Y - 12.0, w: KB_W + 24.0, h: KB_H + 24.0 };
        self.fill(rt, body, 12.0, col(0x14, 0x14, 0x14, 1.0));
        self.stroke(rt, body, 12.0, if t_dark { col(255, 255, 255, 0.10) } else { col(0, 0, 0, 0.25) }, 1.0);

        // A layout of another size is scaled to fit and centered.
        let lay = layout();
        let u = (KB_W / lay.width).min(KB_H / lay.height);
        let (ox, oy) = (KB_X + (KB_W - lay.width * u) / 2.0, KB_Y + (KB_H - lay.height * u) / 2.0);
        for &kd in lay.keys(iso) {
            let r = R { x: ox + kd.x * u + 2.0, y: oy + kd.y * u + 2.0, w: kd.w * u - 4.0, h: kd.h * u - 4.0 };
            let c = if kd.code == 0 { OFF } else { color_of(kd.code) };
            let lit = c != OFF;
            let fill = if lit { rgb_col(c) } else { col(0x2A, 0x2A, 0x2A, 1.0) };
            let fg = if !lit {
                col(0x8A, 0x8A, 0x8A, 1.0)
            } else if luminance(c) > 0.55 {
                col(0x10, 0x10, 0x10, 1.0)
            } else {
                col(255, 255, 255, 1.0)
            };
            let radius = if kd.h < 1.0 { 3.0 } else { 5.0 };
            self.fill(rt, r, radius, fill);
            if kd.code == 0 {
                self.stroke(rt, r, radius, col(255, 255, 255, 0.06), 1.0);
            }
            if kd.icon {
                self.text(rt, kd.label, &self.fmt.key_icon, r, fg);
            } else if !kd.ru.is_empty() && !crate::i18n::en() {
                self.text(rt, kd.label, &self.fmt.key_lead, r.inset(5.0), fg);
                self.text(rt, kd.ru, &self.fmt.key_ru, r.inset(5.0), fg);
            } else {
                let fmt = if kd.label.chars().count() > 3 { &self.fmt.key_small } else { &self.fmt.key };
                self.text(rt, kd.label, fmt, r, fg);
            }
            if kd.code != 0 {
                if self.hover == Some(Hit::Key(kd.code)) {
                    self.stroke(rt, r.inset(-1.0), radius + 1.0, col(255, 255, 255, 1.0), 2.0);
                }
                self.hits.push((r, Hit::Key(kd.code)));
            }
        }

        // Row 1: brush.
        let x0 = MARGIN + LABEL_W;
        let mut y = ROW_Y;
        self.text(rt, tr!("Кисть", "Brush"), &self.fmt.label, R { x: MARGIN, y, w: LABEL_W, h: ROW_H }, self.theme.text);
        let mut x = x0;
        // With a gradient dot selected, the palette marks (and sets) that dot's color.
        let brush = self.slot.map_or(self.brush_color, |(i, k)| self.preset_colors[i][k]);
        let swatches: Vec<(Hit, Option<Rgb>)> =
            PALETTE.iter().enumerate().map(|(i, c)| (Hit::Brush(i), Some(*c))).chain([(Hit::BrushOff, Some(OFF)), (Hit::BrushCustom, None)]).collect();
        for (hit, c) in swatches {
            let r = R { x, y: y + 2.0, w: 28.0, h: 28.0 };
            let chosen = match c {
                Some(c) => c == brush,
                None => brush != OFF && !PALETTE.contains(&brush),
            };
            if chosen || self.hover == Some(hit) {
                self.ring(rt, r.cx(), r.cy(), 14.0, if chosen { self.theme.text } else { self.theme.text2 }, 2.0);
            }
            match (hit, c) {
                (Hit::BrushOff, _) => {
                    self.circle(rt, r.cx(), r.cy(), 10.0, col(0x2A, 0x2A, 0x2A, 1.0));
                    self.text(rt, "\u{E711}", &self.fmt.key_icon, r, col(0xB0, 0xB0, 0xB0, 1.0));
                }
                (_, Some(c)) => {
                    self.circle(rt, r.cx(), r.cy(), 10.0, rgb_col(c));
                    self.ring(rt, r.cx(), r.cy(), 9.5, self.theme.stroke, 1.0);
                }
                _ => {
                    if chosen {
                        self.circle(rt, r.cx(), r.cy(), 10.0, rgb_col(brush));
                    } else {
                        self.circle(rt, r.cx(), r.cy(), 11.0, self.theme.card);
                        self.ring(rt, r.cx(), r.cy(), 10.5, self.theme.stroke, 1.0);
                        self.text(rt, "\u{E710}", &self.fmt.key_icon, r, self.theme.text);
                    }
                }
            }
            self.hits.push((r, hit));
            x += 36.0;
        }
        let (clear, fill) = (tr!("Очистить", "Clear"), tr!("Залить всё", "Fill all"));
        let clear_w = self.text_width(clear, &self.fmt.chip) + 28.0;
        let fill_w = self.text_width(fill, &self.fmt.chip) + 28.0;
        let right = W - MARGIN;
        self.chip(rt, right - clear_w - 8.0 - fill_w, y, fill, Hit::Fill, false);
        self.chip(rt, right - clear_w, y, clear, Hit::Clear, false);

        // Row 2: presets.
        y += ROW_STEP;
        self.text(rt, tr!("Пресеты", "Presets"), &self.fmt.label, R { x: MARGIN, y, w: LABEL_W, h: ROW_H }, self.theme.text);
        let mut x = x0;
        for (i, name) in PRESETS.iter().enumerate() {
            x = if two_color(i) { self.two_color_chip(rt, x, y, i) } else { self.chip(rt, x, y, crate::i18n::pick(*name), Hit::Preset(i), false) } + 8.0;
        }

        // Row 3: word + layout switch.
        y += ROW_STEP;
        self.text(rt, tr!("Слово", "Word"), &self.fmt.label, R { x: MARGIN, y, w: LABEL_W, h: ROW_H }, self.theme.text);
        let tb = R { x: x0, y, w: 260.0, h: ROW_H };
        self.fill(rt, tb, 4.0, self.theme.card);
        self.stroke(rt, tb, 4.0, if self.word_focus { self.theme.accent } else { self.theme.stroke }, if self.word_focus { 2.0 } else { 1.0 });
        let inner = R { x: tb.x + 10.0, y: tb.y, w: tb.w - 20.0, h: tb.h };
        if self.word.is_empty() && !self.word_focus {
            self.text(rt, tr!("Например: ПРИВЕТ", "For example: HELLO"), &self.fmt.body, inner, self.theme.text2);
        } else {
            self.text(rt, &self.word, &self.fmt.body, inner, self.theme.text);
        }
        if self.word_focus {
            let cx = inner.x + self.text_width(&self.word, &self.fmt.body) + 1.0;
            self.fill(rt, R { x: cx, y: tb.y + 8.0, w: 1.5, h: tb.h - 16.0 }, 0.0, self.theme.text);
        }
        self.hits.push((tb, Hit::WordBox));
        self.chip(rt, tb.x + tb.w + 8.0, y, tr!("Подсветить", "Light up"), Hit::WordApply, false);

        // ANSI / ISO segmented switch.
        let seg_w = 64.0;
        let seg = R { x: right - 2.0 * seg_w, y, w: 2.0 * seg_w, h: ROW_H };
        self.fill(rt, seg, 4.0, self.theme.card);
        self.stroke(rt, seg, 4.0, self.theme.stroke, 1.0);
        for (i, (name, value)) in [("ANSI", false), ("ISO", true)].into_iter().enumerate() {
            let r = R { x: seg.x + i as f32 * seg_w, y: seg.y, w: seg_w, h: seg.h }.inset(3.0);
            let on = iso == value;
            if on {
                self.fill(rt, r, 3.0, self.theme.accent);
            } else if self.hover == Some(Hit::Layout(value)) {
                self.fill(rt, r, 3.0, self.theme.card_hover);
            }
            self.text(rt, name, &self.fmt.chip, r, if on { self.theme.on_accent } else { self.theme.text });
            self.hits.push((r, Hit::Layout(value)));
        }
        self.text(rt, tr!("Раскладка", "Layout"), &self.fmt.hint_right, R { x: seg.x - 110.0, y, w: 100.0, h: ROW_H }, self.theme.text2);

        // Row 4: saved schemes.
        y += ROW_STEP;
        self.text(rt, tr!("Схемы", "Schemes"), &self.fmt.label, R { x: MARGIN, y, w: LABEL_W, h: ROW_H }, self.theme.text);
        let mut x = self.chip(rt, x0, y, tr!("+ Сохранить текущую", "+ Save current"), Hit::SaveScheme, false) + 8.0;
        for (i, name) in schemes.iter().enumerate() {
            let w = self.text_width(name, &self.fmt.chip) + 28.0;
            if x + w > right {
                break;
            }
            x = self.chip(rt, x, y, name, Hit::Scheme(i), false) + 8.0;
        }
        if schemes.is_empty() {
            self.text(rt, tr!("Сохранённых схем пока нет", "No saved schemes yet"), &self.fmt.body, R { x, y, w: 300.0, h: ROW_H }, self.theme.text2);
        }

        // Bottom.
        self.fill(rt, R { x: 0.0, y: BOTTOM_Y - 12.0, w: W, h: 1.0 }, 0.0, self.theme.stroke);
        self.text(
            rt,
            tr!(
                "Можно вести мышью с зажатой кнопкой. Изменения сразу на клавиатуре. ПКМ по схеме — удалить.",
                "Drag to paint many keys. Changes apply instantly. Right-click a scheme to delete."
            ),
            &self.fmt.body,
            R { x: MARGIN, y: BOTTOM_Y, w: 560.0, h: 36.0 },
            self.theme.text2,
        );
        let done = tr!("Готово", "Done");
        let done_w = self.text_width(done, &self.fmt.chip) + 48.0;
        let r = R { x: right - done_w, y: BOTTOM_Y + 2.0, w: done_w, h: ROW_H };
        let hovered = self.hover == Some(Hit::Done);
        self.fill(rt, r, 4.0, if hovered { D2D1_COLOR_F { a: 0.85, ..self.theme.accent } } else { self.theme.accent });
        self.text(rt, done, &self.fmt.chip, r, self.theme.on_accent);
        self.hits.push((r, Hit::Done));
    }

    fn hit_test(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits.iter().find(|(r, _)| r.contains(x, y)).map(|(_, h)| *h)
    }
}

// ---------------------------------------------------------------- input

fn paint_key(code: u8, c: Rgb) {
    set_tracked(&mut crate::config(), code, c);
    send(Msg::Keys(vec![(code, c)]));
}

/// Runs a click action. No RefCell borrow may be held: the color dialog re-enters the window procedure.
/// A color picked in the palette: sets the selected preset color dot (and redraws that
/// preset on the keyboard), or else the brush.
fn pick(c: Rgb) {
    let preset = with_editor(|e| match e.slot {
        Some((i, k)) => {
            e.preset_colors[i][k] = c;
            Some((i, e.preset_colors[i]))
        }
        None => {
            e.brush_color = c;
            None
        }
    })
    .flatten();
    if let Some((i, colors)) = preset {
        apply_preset(i, colors);
    }
}

unsafe fn activate(hwnd: HWND, hit: Hit) {
    let (brush, colors, slot) = with_editor(|e| (e.brush_color, e.preset_colors, e.slot)).unwrap_or((PALETTE[0], default_colors(), None));
    with_editor(|e| e.word_focus = hit == Hit::WordBox);
    match hit {
        Hit::Key(_) => {}
        Hit::Brush(i) => pick(PALETTE[i]),
        Hit::BrushOff => pick(OFF),
        Hit::BrushCustom => {
            let current = slot.map_or(brush, |(i, k)| colors[i][k]);
            if let Some(c) = crate::pick_color(hwnd, if current == OFF { PALETTE[0] } else { current }) {
                pick(c);
            }
        }
        Hit::Slot(i, k) => {
            with_editor(|e| e.slot = if e.slot == Some((i, k)) { None } else { Some((i, k)) });
        }
        Hit::Fill => fill_all(brush),
        Hit::Clear => fill_all(OFF),
        Hit::Preset(i) => apply_preset(i, colors[i]),
        Hit::WordBox => {}

        Hit::WordApply => {
            let word = with_editor(|e| e.word.clone()).unwrap_or_default();
            write_word(&word, brush);
        }
        Hit::Layout(iso) => {
            let mut cfg = crate::config();
            cfg.iso_layout = iso;
            cfg.save();
        }
        Hit::SaveScheme => {
            let word = with_editor(|e| e.word.trim().to_string()).unwrap_or_default();
            let mut cfg = crate::config();
            let name = if word.is_empty() { format!("{} {}", tr!("Схема", "Scheme"), cfg.schemes.len() + 1) } else { word };
            let keys = keyboard(&mut cfg).keys.clone();
            cfg.schemes.push(Scheme { name, keys });
            cfg.save();
        }
        Hit::Scheme(i) => {
            let mut cfg = crate::config();
            if let Some(s) = cfg.schemes.get(i) {
                let scheme = s.keys.clone();
                // Key by key, so every key can be rolled back to what it was before.
                for &code in layout().codes() {
                    let tmp = crate::config::DeviceConfig { keys: scheme.clone(), ..Default::default() };
                    let c = tmp.key_color(code);
                    set_tracked(&mut cfg, code, c);
                }
                cfg.save();
                drop(cfg);
                send_all();
            }
        }
        Hit::Done => {
            DestroyWindow(hwnd);
            return;
        }
    }
    invalidate();
}

fn mouse_pos(lparam: LPARAM, scale: f32) -> (f32, f32) {
    let x = (lparam & 0xFFFF) as i16 as f32;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as f32;
    (x / scale, y / scale)
}

const WM_MOUSELEAVE: u32 = 0x02A3;

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            BeginPaint(hwnd, &mut ps);
            with_editor(|e| e.paint());
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            let paint = with_editor(|e| {
                let (x, y) = mouse_pos(lparam, e.scale);
                if !e.tracking_leave {
                    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    TrackMouseEvent(&mut tme);
                    e.tracking_leave = true;
                }
                let h = e.hit_test(x, y);
                let changed = h != e.hover;
                e.hover = h;
                // Dragging paints (left button) or rolls back (right button) every key passed over.
                let stroke = match (e.painting || e.rolling, h) {
                    (true, Some(Hit::Key(code))) if e.last_painted != Some(code) => {
                        e.last_painted = Some(code);
                        Some((code, e.brush_color, e.rolling))
                    }
                    _ => None,
                };
                (changed, stroke)
            });
            if let Some((changed, stroke)) = paint {
                match stroke {
                    Some((code, _, true)) => {
                        rollback_key(code);
                    }
                    Some((code, c, false)) => paint_key(code, c),
                    None => {}
                }
                if changed || stroke.is_some() {
                    invalidate();
                }
            }
            0
        }
        WM_MOUSELEAVE => {
            with_editor(|e| {
                e.tracking_leave = false;
                e.hover = None;
            });
            invalidate();
            0
        }
        WM_LBUTTONDOWN => {
            let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
            let start = with_editor(|e| {
                let (x, y) = mouse_pos(lparam, e.scale);
                match e.hit_test(x, y) {
                    // Ctrl + click: eyedropper.
                    Some(Hit::Key(code)) if ctrl => {
                        e.brush_color = crate::config().device(&target()).key_color(code);
                        None
                    }
                    Some(Hit::Key(code)) => {
                        e.painting = true;
                        e.last_painted = Some(code);
                        e.word_focus = false;
                        Some((code, e.brush_color))
                    }
                    _ => None,
                }
            })
            .flatten();
            if let Some((code, c)) = start {
                SetCapture(hwnd);
                paint_key(code, c);
            }
            invalidate();
            0
        }
        WM_RBUTTONDOWN => {
            let start = with_editor(|e| {
                let (x, y) = mouse_pos(lparam, e.scale);
                match e.hit_test(x, y) {
                    Some(Hit::Key(code)) => {
                        e.rolling = true;
                        e.last_painted = Some(code);
                        Some(code)
                    }
                    _ => None,
                }
            })
            .flatten();
            if let Some(code) = start {
                SetCapture(hwnd);
                rollback_key(code);
                invalidate();
            }
            0
        }
        WM_LBUTTONUP => {
            let (was_painting, hit) = with_editor(|e| {
                let (x, y) = mouse_pos(lparam, e.scale);
                let p = e.painting;
                e.painting = false;
                e.last_painted = None;
                (p, e.hit_test(x, y))
            })
            .unwrap_or((false, None));
            if was_painting {
                ReleaseCapture();
                crate::config().save();
            } else if let Some(h) = hit {
                activate(hwnd, h);
            } else {
                with_editor(|e| e.word_focus = false);
                invalidate();
            }
            0
        }
        WM_RBUTTONUP => {
            let (was_rolling, hit) = with_editor(|e| {
                let (x, y) = mouse_pos(lparam, e.scale);
                let r = e.rolling;
                e.rolling = false;
                e.last_painted = None;
                (r, e.hit_test(x, y))
            })
            .unwrap_or((false, None));
            if was_rolling {
                ReleaseCapture();
                crate::config().save();
                invalidate();
                return 0;
            }
            if let Some(Hit::Scheme(i)) = hit {
                let mut cfg = crate::config();
                if i < cfg.schemes.len() {
                    cfg.schemes.remove(i);
                    cfg.save();
                }
            }
            invalidate();
            0
        }
        WM_CHAR => {
            let ch = char::from_u32(wparam as u32);
            let apply = with_editor(|e| {
                if !e.word_focus {
                    return None;
                }
                match ch {
                    Some('\u{8}') => {
                        e.word.pop();
                    }
                    Some('\r') => return Some((e.word.clone(), e.brush_color)),
                    Some('\u{1b}') => e.word_focus = false,
                    Some(c) if !c.is_control() && e.word.chars().count() < 24 => e.word.push(c),
                    _ => {}
                }
                None
            })
            .flatten();
            if let Some((word, brush)) = apply {
                write_word(&word, brush);
            }
            invalidate();
            0
        }
        WM_DPICHANGED => {
            let suggested = &*(lparam as *const RECT);
            with_editor(|e| {
                e.scale = GetDpiForWindow(hwnd) as f32 / 96.0;
                e.rt = None;
                e.brush = None;
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
            let ed = EDITOR.with(|c| c.borrow_mut().take());
            drop(ed);
            stop_worker();
            crate::ui::refresh();
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_colors() {
        assert_eq!(hsv(0.0, 1.0, 1.0), Rgb(255, 0, 0));
        assert_eq!(hsv(120.0, 1.0, 1.0), Rgb(0, 255, 0));
        assert_eq!(hsv(240.0, 1.0, 1.0), Rgb(0, 0, 255));
        assert_eq!(hsv(360.0, 1.0, 1.0), Rgb(255, 0, 0));
        assert_eq!(hsv(-120.0, 1.0, 1.0), Rgb(0, 0, 255));
        assert_eq!(hsv(77.0, 0.0, 0.5), Rgb(128, 128, 128));
    }

    #[test]
    fn hsv_round_trip() {
        for c in [Rgb(255, 0, 0), Rgb(10, 227, 239), Rgb(200, 100, 50), Rgb(17, 17, 17), Rgb(0, 0, 0), Rgb(255, 255, 255)] {
            let (h, s, v) = to_hsv(c);
            assert_eq!(hsv(h, s, v), c);
        }
    }

    #[test]
    fn mixing() {
        assert_eq!(mix(Rgb(0, 0, 0), Rgb(255, 255, 255), 0.5), Rgb(128, 128, 128));
        assert_eq!(mix(Rgb(10, 20, 30), Rgb(200, 0, 0), 0.0), Rgb(10, 20, 30));
        assert_eq!(mix(Rgb(10, 20, 30), Rgb(200, 0, 0), 1.0), Rgb(200, 0, 0));
        assert!((luminance(Rgb(255, 255, 255)) - 1.0).abs() < 1e-6);
    }
}
