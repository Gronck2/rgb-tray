//! Right-click menu of the tray icon, drawn in the style of the panel.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows::core::w;
use windows::Win32::Foundation::HWND as WinHwnd;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE};
use windows_sys::Win32::UI::Shell::{SHAppBarMessage, ABM_GETTASKBARPOS, APPBARDATA};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::ui::{text_format, theme, Theme, R};

const W: f32 = 290.0;
const PAD: f32 = 4.0;
const ITEM_H: f32 = 36.0;
const GAP: f32 = 9.0;
const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Item {
    Open,
    Keys,
    Reapply,
    Autostart,
    Export,
    Import,
    Exit,
}

/// Rows of the menu: an item, a divider, or the version line.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Item(Item),
    Divider,
    Version,
}

const ROWS: [Row; 10] = [
    Row::Item(Item::Open),
    Row::Item(Item::Keys),
    Row::Item(Item::Reapply),
    Row::Item(Item::Autostart),
    Row::Divider,
    Row::Item(Item::Export),
    Row::Item(Item::Import),
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
    /// Keyboard whose key editor "Цвета клавиш…" opens (None: no such keyboard connected).
    keyboard: Option<String>,
    autostart: bool,
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
    let mut wc: WNDCLASSW = zeroed();
    wc.style = CS_DROPSHADOW;
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
    let (w, h) = ((W * scale).round() as i32, (height() * scale).round() as i32);
    let work = mi.rcWork;
    let mut abd: APPBARDATA = zeroed();
    abd.cbSize = size_of::<APPBARDATA>() as u32;
    let edge = if SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) != 0 { abd.uEdge } else { 3 };
    // Opens from the cursor towards the screen center, inside the work area.
    let x = match edge {
        0 => pt.x,
        2 => pt.x - w,
        _ => pt.x - w / 2,
    }
    .clamp(work.left, work.right - w);
    let y = match edge {
        1 => pt.y,
        3 => pt.y - h,
        _ => pt.y - h / 2,
    }
    .clamp(work.top, work.bottom - h);

    let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, class.as_ptr(), class.as_ptr(), WS_POPUP, x, y, w, h, null_mut(), null_mut(), hinst, null());
    let theme = theme();
    let round: u32 = 3; // DWMWCP_ROUNDSMALL, like Windows menus
    DwmSetWindowAttribute(hwnd, 33, &round as *const u32 as _, 4);
    let dark: u32 = theme.dark as u32;
    DwmSetWindowAttribute(hwnd, 20, &dark as *const u32 as _, 4);

    let keyboard = crate::PRESENT.lock().unwrap().iter().find(|id| crate::devices::kind_of(id).per_key).cloned();
    let menu = Menu {
        hwnd,
        scale,
        d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
        rt: None,
        brush: None,
        fmt: formats()?,
        theme,
        hover: None,
        tracking: false,
        keyboard,
        autostart: crate::autostart_enabled(),
    };
    MENU.with(|m| *m.borrow_mut() = Some(menu));
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    Ok(())
}

impl Menu {
    fn enabled(&self, item: Item) -> bool {
        item != Item::Keys || self.keyboard.is_some()
    }

    fn paint(&mut self) {
        unsafe {
            if self.rt.is_none() {
                let mut rc: RECT = zeroed();
                GetClientRect(self.hwnd, &mut rc);
                let dpi = 96.0 * self.scale;
                let props = D2D1_RENDER_TARGET_PROPERTIES { dpiX: dpi, dpiY: dpi, ..Default::default() };
                let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd: WinHwnd(self.hwnd),
                    pixelSize: D2D_SIZE_U { width: (rc.right - rc.left) as u32, height: (rc.bottom - rc.top) as u32 },
                    presentOptions: D2D1_PRESENT_OPTIONS_NONE,
                };
                let Ok(rt) = self.d2d.CreateHwndRenderTarget(&props, &hp) else { return };
                rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                self.brush = rt.CreateSolidColorBrush(&self.theme.text, None).ok();
                self.rt = Some(rt);
            }
            let rt = self.rt.clone().unwrap();
            rt.BeginDraw();
            self.draw(&rt);
            if rt.EndDraw(None, None).is_err() {
                self.rt = None;
            }
        }
    }

    unsafe fn fill(&self, rt: &ID2D1RenderTarget, r: R, radius: f32, c: D2D1_COLOR_F) {
        let b = self.brush.as_ref().unwrap();
        b.SetColor(&c);
        rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius }, b);
    }

    unsafe fn circle(&self, rt: &ID2D1RenderTarget, x: f32, y: f32, radius: f32, c: D2D1_COLOR_F) {
        let b = self.brush.as_ref().unwrap();
        b.SetColor(&c);
        rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x, y }, radiusX: radius, radiusY: radius }, b);
    }

    unsafe fn stroke(&self, rt: &ID2D1RenderTarget, r: R, radius: f32, c: D2D1_COLOR_F, width: f32) {
        let b = self.brush.as_ref().unwrap();
        b.SetColor(&c);
        rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.inset(width / 2.0).d2d(), radiusX: radius, radiusY: radius }, b, width, None);
    }

    unsafe fn text(&self, rt: &ID2D1RenderTarget, s: &str, fmt: &IDWriteTextFormat, r: R, c: D2D1_COLOR_F) {
        let b = self.brush.as_ref().unwrap();
        b.SetColor(&c);
        let s: Vec<u16> = s.encode_utf16().collect();
        rt.DrawText(&s, fmt, &r.d2d(), b, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
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
                        Item::Open => ("\u{E790}", "Открыть панель"),
                        Item::Keys => ("\u{E765}", "Цвета клавиш…"),
                        Item::Reapply => ("\u{E72C}", "Применить заново"),
                        Item::Autostart => ("\u{E823}", "Запускать с Windows"),
                        Item::Export => ("\u{E74E}", "Сохранить настройки в файл…"),
                        Item::Import => ("\u{E8E5}", "Загрузить настройки из файла…"),
                        Item::Exit => ("\u{E7E8}", "Выход"),
                    };
                    self.text(rt, icon, &self.fmt.icon, R { x: r.x + 6.0, y: r.y, w: 28.0, h: r.h }, fg);
                    // Only the autostart row needs room for its switch.
                    let label_w = if item == Item::Autostart { r.w - 100.0 } else { r.w - 44.0 };
                    self.text(rt, label, &self.fmt.text, R { x: r.x + 40.0, y: r.y, w: label_w, h: r.h }, fg);
                    if item == Item::Autostart {
                        // A small switch, like the one in the panel.
                        let sw = R { x: r.x + r.w - 46.0, y: r.cy() - 9.0, w: 36.0, h: 18.0 };
                        if self.autostart {
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
    let x = (lparam & 0xFFFF) as i16 as f32 / scale;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as f32 / scale;
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
        Item::Reapply => crate::apply_all(),
        Item::Autostart => crate::set_autostart(!crate::autostart_enabled()),
        Item::Export => crate::export_settings(),
        Item::Import => crate::import_settings(),
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
                        let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                        TrackMouseEvent(&mut tme);
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
        };
        m.draw(rt);
        Ok(())
    })
}
