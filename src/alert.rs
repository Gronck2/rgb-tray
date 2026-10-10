//! Low-battery alert: a small always-on-top window that stays until dismissed.
//! It does not take focus, so typing or a game is not interrupted.

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
use windows_sys::Win32::UI::Shell::{SHAppBarMessage, ABM_GETTASKBARPOS, APPBARDATA};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::devices::{Battery, CRITICAL_BATTERY};
use crate::ui::{col, text_format, theme, Theme, R};

const W: f32 = 380.0;
const H: f32 = 172.0;
const PAD: f32 = 20.0;
const BTN_Y: f32 = H - PAD - 32.0;

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Snooze,
    Dismiss,
}

struct Formats {
    title: IDWriteTextFormat,
    name: IDWriteTextFormat,
    big: IDWriteTextFormat,
    body: IDWriteTextFormat,
    button: IDWriteTextFormat,
    icon: IDWriteTextFormat,
}

struct Alert {
    hwnd: HWND,
    scale: f32,
    d2d: ID2D1Factory,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    fmt: Formats,
    theme: Theme,
    key: String,
    name: String,
    battery: Battery,
    hover: Option<Hit>,
}

thread_local! {
    static ALERT: RefCell<Option<Alert>> = const { RefCell::new(None) };
}

fn wrapping(f: IDWriteTextFormat) -> IDWriteTextFormat {
    unsafe {
        let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
        let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
    }
    f
}

/// The close cross in the top-right corner (the only way to dismiss) and the snooze button.
fn buttons() -> [(R, Hit, &'static str); 2] {
    let close = R { x: W - 8.0 - 36.0, y: 8.0, w: 36.0, h: 32.0 };
    let later = R { x: W - PAD - 200.0, y: BTN_Y, w: 200.0, h: 32.0 };
    [(later, Hit::Snooze, tr!("Напомнить через 30 мин", "Remind me in 30 min")), (close, Hit::Dismiss, "\u{E711}")]
}

/// Accent of the alert, in the app's own tone: the lighting color when all devices share
/// one, otherwise the system accent used for selections in the panel.
fn level_color(_b: Battery) -> D2D1_COLOR_F {
    // Ids first: active_ids() locks the config too.
    let ids = crate::active_ids();
    match crate::config().uniform_color(&ids) {
        Some(c) => crate::ui::rgb_col(c),
        None => theme().accent,
    }
}

/// Shows (or updates) the alert for a device.
pub fn show(key: String, name: &str, battery: Battery) {
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(MB_ICONWARNING);
        let existing = ALERT.with(|c| {
            c.borrow_mut().as_mut().map(|a| {
                a.key = key.clone();
                a.name = name.to_string();
                a.battery = battery;
                a.hwnd
            })
        });
        if let Some(h) = existing {
            InvalidateRect(h, null(), 0);
            return;
        }
        if let Err(e) = open(key, name, battery) {
            crate::log(&format!("battery alert: {e}"));
        }
    }
}

/// Closes the alert if it is about this device (e.g. it went on charge).
pub fn dismiss_for(key: &str) {
    let h = ALERT.with(|c| c.borrow().as_ref().filter(|a| a.key == key).map(|a| a.hwnd));
    if let Some(h) = h {
        unsafe { DestroyWindow(h) };
    }
}

unsafe fn open(key: String, name: &str, battery: Battery) -> windows::core::Result<()> {
    let hinst = GetModuleHandleW(null());
    let class: Vec<u16> = "RgbTrayBatteryAlert\0".encode_utf16().collect();
    let mut wc: WNDCLASSW = zeroed();
    wc.style = CS_DROPSHADOW;
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinst;
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.lpszClassName = class.as_ptr();
    RegisterClassW(&wc);

    // Same corner as the tray icon, next to the taskbar.
    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(monitor, &mut mi);
    let (mut dpi, mut dpi_y) = (96u32, 96u32);
    GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
    let scale = dpi as f32 / 96.0;
    let (w, h) = ((W * scale).round() as i32, (H * scale).round() as i32);
    let margin = (12.0 * scale) as i32;
    // Centered at the edge where the taskbar is (top or bottom), hard to miss.
    let work = mi.rcWork;
    let mut abd: APPBARDATA = zeroed();
    abd.cbSize = size_of::<APPBARDATA>() as u32;
    let edge = if SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) != 0 { abd.uEdge } else { 3 };
    let x = work.left + ((work.right - work.left) - w) / 2;
    let y = if edge == 3 { work.bottom - h - 4 * margin } else { work.top + 4 * margin };

    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
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
    let round: u32 = 2;
    DwmSetWindowAttribute(hwnd, 33, &round as *const u32 as _, 4);
    let dark: u32 = theme.dark as u32;
    DwmSetWindowAttribute(hwnd, 20, &dark as *const u32 as _, 4);
    // Colored window border so it stands out.
    let c = level_color(battery);
    let border: u32 = (c.r * 255.0) as u32 | ((c.g * 255.0) as u32) << 8 | ((c.b * 255.0) as u32) << 16;
    DwmSetWindowAttribute(hwnd, 34, &border as *const u32 as _, 4);

    let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
    let ui = w!("Segoe UI Variable Text");
    let (lead, center) = (DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_CENTER);
    let fmt = Formats {
        title: text_format(&dw, w!("Segoe UI Variable Display"), 16.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, lead)?,
        name: text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, lead)?,
        big: text_format(&dw, w!("Segoe UI Variable Display"), 28.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
        body: wrapping(text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, lead)?),
        button: text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, center)?,
        icon: text_format(&dw, w!("Segoe Fluent Icons"), 12.0, DWRITE_FONT_WEIGHT_NORMAL, center)?,
    };
    let alert = Alert {
        hwnd,
        scale,
        d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
        rt: None,
        brush: None,
        fmt,
        theme,
        key,
        name: name.to_string(),
        battery,
        hover: None,
    };
    ALERT.with(|c| *c.borrow_mut() = Some(alert));
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    Ok(())
}

impl Alert {
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
        let warn = level_color(self.battery);
        rt.Clear(Some(&t.bg));
        self.fill(rt, R { x: 0.0, y: 0.0, w: 5.0, h: H }, 0.0, warn);

        // Battery drawing: body, terminal nub and the remaining charge.
        let body = R { x: PAD, y: PAD + 4.0, w: 40.0, h: 22.0 };
        self.stroke(rt, body, 4.0, warn, 2.0);
        self.fill(rt, R { x: body.x + body.w + 1.0, y: body.y + 7.0, w: 3.0, h: 8.0 }, 1.0, warn);
        let level = (self.battery.percent as f32 / 100.0).clamp(0.08, 1.0);
        self.fill(rt, R { x: body.x + 4.0, y: body.y + 4.0, w: (body.w - 8.0) * level, h: body.h - 8.0 }, 1.5, warn);

        let title = if self.battery.percent <= CRITICAL_BATTERY { tr!("Батарея почти разряжена", "Battery almost empty") } else { tr!("Низкий заряд батареи", "Battery low") };
        self.text(rt, title, &self.fmt.title, R { x: PAD + 58.0, y: PAD - 2.0, w: W - 2.0 * PAD - 176.0, h: 24.0 }, t.text);
        self.text(rt, &self.name, &self.fmt.name, R { x: PAD + 58.0, y: PAD + 22.0, w: W - 2.0 * PAD - 176.0, h: 20.0 }, t.text2);
        self.text(rt, &format!("{}%", self.battery.percent), &self.fmt.big, R { x: W - PAD - 126.0, y: PAD - 4.0, w: 90.0, h: 44.0 }, warn);
        self.text(
            rt,
            tr!("Подключите кабель или поставьте на зарядку, чтобы устройство не отключилось.", "Plug in the cable or put it on the charger before the device turns off."),
            &self.fmt.body,
            R { x: PAD, y: PAD + 48.0, w: W - 2.0 * PAD, h: 40.0 },
            t.text,
        );

        for (r, hit, label) in buttons() {
            let hovered = self.hover == Some(hit);
            if hit == Hit::Dismiss {
                if hovered {
                    self.fill(rt, r, 4.0, t.card_hover);
                }
                self.text(rt, label, &self.fmt.icon, r, t.text);
            } else {
                self.fill(rt, r, 4.0, if hovered { D2D1_COLOR_F { a: 0.85, ..warn } } else { warn });
                // Dark text on light accents (e.g. white or turquoise lighting).
                let luminance = 0.299 * warn.r + 0.587 * warn.g + 0.114 * warn.b;
                let fg = if luminance > 0.6 { col(0x10, 0x10, 0x10, 1.0) } else { col(255, 255, 255, 1.0) };
                self.text(rt, label, &self.fmt.button, r, fg);
            }
        }
    }
}

/// Renders a sample alert into a PNG file, for checking the look.
pub fn snapshot(path: &str, dark: Option<bool>, scale: f32, percent: u8) -> windows::core::Result<()> {
    crate::ui::render_png(path, W, H, scale, |rt, d2d| unsafe {
        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let ui = w!("Segoe UI Variable Text");
        let (lead, center) = (DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_CENTER);
        let theme = crate::ui::theme_for(dark);
        let a = Alert {
            hwnd: null_mut(),
            scale,
            d2d,
            rt: None,
            brush: Some(rt.CreateSolidColorBrush(&theme.text, None)?),
            fmt: Formats {
                title: text_format(&dw, w!("Segoe UI Variable Display"), 16.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, lead)?,
                name: text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, lead)?,
                big: text_format(&dw, w!("Segoe UI Variable Display"), 28.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
                body: wrapping(text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, lead)?),
                button: text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, center)?,
                icon: text_format(&dw, w!("Segoe Fluent Icons"), 12.0, DWRITE_FONT_WEIGHT_NORMAL, center)?,
            },
            theme,
            key: "logitech".into(),
            name: "Logitech G915 TKL".into(),
            battery: Battery { percent, charging: false },
            hover: None,
        };
        a.draw(rt);
        Ok(())
    })
}

fn hit_at(lparam: LPARAM, scale: f32) -> Option<Hit> {
    let x = (lparam & 0xFFFF) as i16 as f32 / scale;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as f32 / scale;
    buttons().into_iter().find(|(r, _, _)| r.contains(x, y)).map(|(_, h, _)| h)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            BeginPaint(hwnd, &mut ps);
            ALERT.with(|c| c.borrow_mut().as_mut().map(|a| a.paint()));
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        // Clicks must work without the window ever being activated.
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_MOUSEMOVE => {
            let changed = ALERT.with(|c| {
                c.borrow_mut().as_mut().map(|a| {
                    let h = hit_at(lparam, a.scale);
                    let changed = h != a.hover;
                    a.hover = h;
                    changed
                })
            });
            if changed == Some(true) {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            let (hit, key) = ALERT.with(|c| c.borrow().as_ref().map(|a| (hit_at(lparam, a.scale), a.key.clone()))).unwrap_or_default();
            match hit {
                Some(Hit::Snooze) => {
                    crate::snooze_battery_alert(&key);
                    DestroyWindow(hwnd);
                }
                Some(Hit::Dismiss) => {
                    DestroyWindow(hwnd);
                }
                None => {}
            }
            0
        }
        WM_DESTROY => {
            let a = ALERT.with(|c| c.borrow_mut().take());
            drop(a);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
