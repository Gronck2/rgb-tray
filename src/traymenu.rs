//! Right-click menu of the tray icon, drawn in the style of the panel.

use std::cell::RefCell;
use std::mem::zeroed;
use std::ptr::{null, null_mut};

use windows::core::w;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::draw::{cursor_monitor, dwm_set, hwnd_target, mouse_pos, register_class, taskbar_edge, track_leave, Paint, WM_MOUSELEAVE};
use crate::ui::{text_format, theme, Theme, R};

const W: f32 = 290.0;
const PAD: f32 = 4.0;
const ITEM_H: f32 = 36.0;
const GAP: f32 = 9.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Item {
    Open,
    Keys,
    Reapply,
    Autostart,
    FollowDisplay,
    CheckUpdates,
    Language,
    Export,
    Import,
    Diagnose,
    RequestDevice,
    Exit,
}

/// Rows of the menu: an item, a divider, or the version line.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Item(Item),
    Divider,
    Version,
}

const ROWS: [Row; 15] = [
    Row::Item(Item::Open),
    Row::Item(Item::Keys),
    Row::Item(Item::Reapply),
    Row::Item(Item::Autostart),
    Row::Item(Item::FollowDisplay),
    Row::Item(Item::CheckUpdates),
    Row::Item(Item::Language),
    Row::Divider,
    Row::Item(Item::Export),
    Row::Item(Item::Import),
    Row::Item(Item::Diagnose),
    Row::Item(Item::RequestDevice),
    Row::Divider,
    Row::Version,
    Row::Item(Item::Exit),
];

fn row_h(row: Row) -> f32 {
    match row {
        Row::Divider => GAP,
        Row::Version => 28.0,
        Row::Item(_) => ITEM_H,
    }
}

fn height() -> f32 {
    2.0 * PAD + ROWS.iter().map(|&r| row_h(r)).sum::<f32>()
}

fn layout() -> Vec<(R, Row)> {
    let mut y = PAD;
    ROWS.iter()
        .map(|&row| {
            let r = R { x: PAD, y, w: W - 2.0 * PAD, h: row_h(row) };
            y += r.h;
            (r, row)
        })
        .collect()
}

struct Formats {
    text: IDWriteTextFormat,
    small: IDWriteTextFormat,
    icon: IDWriteTextFormat,
}

struct Menu {
    hwnd: HWND,
    scale: f32,
    d2d: ID2D1Factory,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    fmt: Formats,
    theme: Theme,
    hover: Option<Item>,
    tracking: bool,
    /// Keyboard whose key editor "Key colors…" opens (None: no such keyboard connected).
    keyboard: Option<String>,
    autostart: bool,
    follow_display: bool,
    check_updates: bool,
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}

fn formats() -> windows::core::Result<Formats> {
    let dw: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
    let ui = w!("Segoe UI Variable Text");
    let (lead, center) = (DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_CENTER);
    Ok(Formats {
        text: text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, lead)?,
        small: text_format(&dw, ui, 12.0, DWRITE_FONT_WEIGHT_NORMAL, lead)?,
        icon: text_format(&dw, w!("Segoe Fluent Icons"), 14.0, DWRITE_FONT_WEIGHT_NORMAL, center)?,
    })
}

/// Opens the menu at the mouse cursor, away from the taskbar.
pub fn show() {
    if MENU.with(|m| m.borrow().is_some()) {
        return;
    }
    if let Err(e) = unsafe { open() } {
        crate::log(&format!("tray menu: {e}"));
    }
}

unsafe fn open() -> windows::core::Result<()> {
    let hinst = GetModuleHandleW(null());
    let class: Vec<u16> = "RgbTrayMenu\0".encode_utf16().collect();
    register_class(&class, CS_DROPSHADOW, wndproc);

    let (pt, mi, dpi) = cursor_monitor(MONITOR_DEFAULTTONEAREST);
    let scale = dpi as f32 / 96.0;
    let (w, h) = ((W * scale).round() as i32, (height() * scale).round() as i32);
    let work = mi.rcWork;
    let edge = taskbar_edge();
    // Opens from the cursor towards the screen center, inside the work area.
    let x = match edge {
        0 => pt.x,
        2 => pt.x - w,
        _ => pt.x - w / 2,
    }
    .clamp(work.left, (work.right - w).max(work.left));
    let y = match edge {
        1 => pt.y,
        3 => pt.y - h,
        _ => pt.y - h / 2,
    }
    .clamp(work.top, (work.bottom - h).max(work.top));

    // What can fail comes before the window: a window without its state would be stuck.
    let d2d = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
    let fmt = formats()?;
    let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, class.as_ptr(), class.as_ptr(), WS_POPUP, x, y, w, h, null_mut(), null_mut(), hinst, null());
    if hwnd.is_null() {
        return Err(windows::core::Error::from_win32());
    }
    let theme = theme();
    dwm_set(hwnd, 33, 3); // DWMWCP_ROUNDSMALL, like Windows menus
    dwm_set(hwnd, 20, theme.dark as u32);

    let keyboard = crate::PRESENT.lock().unwrap().iter().find(|id| crate::devices::caps(id).per_key).cloned();
    // One lock of the config: a guard taken inside the struct literal below would live to the
    // end of the statement, and a second one there would wait for it forever.
    let (follow_display, check_updates) = {
        let cfg = crate::config();
        (cfg.follow_display, cfg.check_updates)
    };
    let menu = Menu {
        hwnd,
        scale,
        d2d,
        rt: None,
        brush: None,
        fmt,
        theme,
        hover: None,
        tracking: false,
        keyboard,
        autostart: crate::autostart_enabled(),
        follow_display,
        check_updates,
    };
    MENU.with(|m| *m.borrow_mut() = Some(menu));
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    Ok(())
}

impl Paint for Menu {
    fn brush(&self) -> &ID2D1SolidColorBrush {
        self.brush.as_ref().unwrap()
    }
}

impl Menu {
    fn enabled(&self, item: Item) -> bool {
        item != Item::Keys || self.keyboard.is_some()
    }

    fn paint(&mut self) {
        unsafe {
            if self.rt.is_none() {
                let Ok(rt) = hwnd_target(&self.d2d, self.hwnd, self.scale) else { return };
                self.brush = rt.CreateSolidColorBrush(&self.theme.text, None).ok();
                self.rt = Some(rt);
            }
            let rt = self.rt.clone().unwrap();
            rt.BeginDraw();
            self.draw(&rt);
            if let Err(e) = rt.EndDraw(None, None) {
                self.rt = None;
                if e.code() == windows::Win32::Foundation::D2DERR_RECREATE_TARGET {
                    // Nothing was shown: draw again on a new target.
                    InvalidateRect(self.hwnd, null(), 0);
                }
            }
        }
    }

    unsafe fn draw(&self, rt: &ID2D1RenderTarget) {
        let t = &self.theme;
        rt.Clear(Some(&t.card));
        for (r, row) in layout() {
            match row {
                Row::Divider => self.fill(rt, R { x: 0.0, y: r.y + r.h / 2.0 - 0.5, w: W, h: 1.0 }, 0.0, t.stroke),
                Row::Version => {
                    let label = format!("RGB Tray {}", crate::installer::VERSION);
                    self.text(rt, &label, &self.fmt.small, R { x: r.x + 40.0, y: r.y, w: r.w - 44.0, h: r.h }, t.text2);
                }
                Row::Item(item) => {
                    let enabled = self.enabled(item);
                    if enabled && self.hover == Some(item) {
                        self.fill(rt, r, 4.0, t.card_hover);
                    }
                    let mut fg = t.text;
                    if !enabled {
                        fg = t.text2;
                        fg.a *= 0.6;
                    }
                    let (icon, label) = match item {
                        Item::Open => ("\u{E790}", tr!("Открыть панель", "Open panel")),
                        Item::Keys => ("\u{E765}", tr!("Цвета клавиш…", "Key colors…")),
                        Item::Reapply => ("\u{E72C}", tr!("Применить заново", "Apply again")),
                        Item::Autostart => ("\u{E823}", tr!("Запускать с Windows", "Start with Windows")),
                        Item::FollowDisplay => ("\u{E708}", tr!("Гасить вместе с экраном", "Off with the display")),
                        Item::CheckUpdates => ("\u{E895}", tr!("Проверять обновления", "Check for updates")),
                        // Names the other language: that is what a click switches to.
                        Item::Language => ("\u{E774}", tr!("Switch to English", "Переключить на русский")),
                        Item::Export => ("\u{E74E}", tr!("Сохранить настройки в файл…", "Save settings to a file…")),
                        Item::Import => ("\u{E8E5}", tr!("Загрузить настройки или пресет…", "Load settings or a preset…")),
                        Item::Diagnose => ("\u{E9D9}", tr!("Отчёт об устройствах…", "Device report…")),
                        Item::RequestDevice => ("\u{E710}", tr!("Запросить поддержку устройства…", "Request device support…")),
                        Item::Exit => ("\u{E7E8}", tr!("Выход", "Exit")),
                    };
                    self.text(rt, icon, &self.fmt.icon, R { x: r.x + 6.0, y: r.y, w: 28.0, h: r.h }, fg);
                    // Only the rows with a switch need room for it.
                    let switch = match item {
                        Item::Autostart => Some(self.autostart),
                        Item::FollowDisplay => Some(self.follow_display),
                        Item::CheckUpdates => Some(self.check_updates),
                        _ => None,
                    };
                    let label_w = if switch.is_some() { r.w - 100.0 } else { r.w - 44.0 };
                    self.text(rt, label, &self.fmt.text, R { x: r.x + 40.0, y: r.y, w: label_w, h: r.h }, fg);
                    if let Some(on) = switch {
                        // A small switch, like the one in the panel.
                        let sw = R { x: r.x + r.w - 46.0, y: r.cy() - 9.0, w: 36.0, h: 18.0 };
                        if on {
                            self.fill(rt, sw, 9.0, t.accent);
                            self.circle(rt, sw.x + sw.w - 9.0, sw.cy(), 5.0, t.on_accent);
                        } else {
                            self.stroke(rt, sw, 9.0, t.text2, 1.0);
                            self.circle(rt, sw.x + 9.0, sw.cy(), 4.5, t.text2);
                        }
                    }
                }
            }
        }
    }
}

fn item_at(lparam: LPARAM, scale: f32) -> Option<Item> {
    let (x, y) = mouse_pos(lparam, scale);
    layout().into_iter().find(|(r, _)| r.contains(x, y)).and_then(|(_, row)| match row {
        Row::Item(i) => Some(i),
        _ => None,
    })
}

/// Runs a menu item after the menu has closed.
fn run(item: Item, keyboard: Option<String>) {
    match item {
        Item::Open => crate::ui::toggle(),
        Item::Keys => {
            if let Some(id) = keyboard {
                crate::editor::open(&id);
            }
        }
        Item::Reapply => crate::reapply_all(),
        Item::Autostart => crate::set_autostart(!crate::autostart_enabled()),
        Item::FollowDisplay => crate::toggle_follow_display(),
        Item::CheckUpdates => crate::toggle_check_updates(),
        Item::Language => crate::toggle_language(),
        Item::Export => crate::export_settings(),
        Item::Import => crate::import_settings(),
        // Searching devices takes a moment; the report opens in Notepad when ready.
        Item::Diagnose => {
            std::thread::spawn(|| crate::diagnose(None));
        }
        Item::RequestDevice => {
            std::thread::spawn(crate::request_device_support);
        }
        Item::Exit => crate::quit(),
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            BeginPaint(hwnd, &mut ps);
            MENU.with(|m| m.borrow_mut().as_mut().map(|m| m.paint()));
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            let changed = MENU.with(|m| {
                m.borrow_mut().as_mut().map(|m| {
                    if !m.tracking {
                        track_leave(hwnd);
                        m.tracking = true;
                    }
                    let h = item_at(lparam, m.scale);
                    let changed = h != m.hover;
                    m.hover = h;
                    changed
                })
            });
            if changed == Some(true) {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_MOUSELEAVE => {
            MENU.with(|m| {
                if let Some(m) = m.borrow_mut().as_mut() {
                    m.tracking = false;
                    m.hover = None;
                }
            });
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONUP => {
            let picked = MENU.with(|m| m.borrow().as_ref().and_then(|m| item_at(lparam, m.scale).filter(|&i| m.enabled(i)).map(|i| (i, m.keyboard.clone()))));
            if let Some((item, keyboard)) = picked {
                DestroyWindow(hwnd);
                run(item, keyboard);
            }
            0
        }
        WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
            DestroyWindow(hwnd);
            0
        }
        // Clicking anywhere else closes the menu, like a regular one.
        WM_ACTIVATE if (wparam & 0xFFFF) as u32 == WA_INACTIVE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            let m = MENU.with(|m| m.borrow_mut().take());
            drop(m);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Renders the menu into a PNG file, for checking the look.
pub fn snapshot(path: &str, dark: Option<bool>, scale: f32) -> windows::core::Result<()> {
    crate::ui::render_png(path, W, height(), scale, |rt, d2d| unsafe {
        let theme = crate::ui::theme_for(dark);
        let m = Menu {
            hwnd: null_mut(),
            scale,
            d2d,
            rt: None,
            brush: Some(rt.CreateSolidColorBrush(&theme.text, None)?),
            fmt: formats()?,
            theme,
            hover: Some(Item::Reapply),
            tracking: false,
            keyboard: None,
            autostart: true,
            follow_display: true,
            check_updates: true,
        };
        m.draw(rt);
        Ok(())
    })
}
