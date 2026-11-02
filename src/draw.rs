//! Helpers shared by the Direct2D windows: the panel, the key editor, the tray menu and the
//! battery alert.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows::Win32::Foundation::HWND as WinHwnd;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::Shell::{SHAppBarMessage, ABM_GETTASKBARPOS, APPBARDATA};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::ui::R;

pub const WM_MOUSELEAVE: u32 = 0x02A3;

/// Shapes and text in one solid brush whose color is set per call.
pub trait Paint {
    fn brush(&self) -> &ID2D1SolidColorBrush;

    unsafe fn fill(&self, rt: &ID2D1RenderTarget, r: R, radius: f32, c: D2D1_COLOR_F) {
        let b = self.brush();
        b.SetColor(&c);
        rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius }, b);
    }

    /// Outline drawn inside `r`.
    unsafe fn stroke(&self, rt: &ID2D1RenderTarget, r: R, radius: f32, c: D2D1_COLOR_F, width: f32) {
        let b = self.brush();
        b.SetColor(&c);
        let r = r.inset(width / 2.0);
        rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius }, b, width, None);
    }

    unsafe fn circle(&self, rt: &ID2D1RenderTarget, x: f32, y: f32, radius: f32, c: D2D1_COLOR_F) {
        let b = self.brush();
        b.SetColor(&c);
        rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x, y }, radiusX: radius, radiusY: radius }, b);
    }

    unsafe fn ring(&self, rt: &ID2D1RenderTarget, x: f32, y: f32, radius: f32, c: D2D1_COLOR_F, width: f32) {
        let b = self.brush();
        b.SetColor(&c);
        rt.DrawEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x, y }, radiusX: radius, radiusY: radius }, b, width, None);
    }

    unsafe fn text(&self, rt: &ID2D1RenderTarget, s: &str, fmt: &IDWriteTextFormat, r: R, c: D2D1_COLOR_F) {
        let b = self.brush();
        b.SetColor(&c);
        let s: Vec<u16> = s.encode_utf16().collect();
        rt.DrawText(&s, fmt, &r.d2d(), b, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
    }
}

/// Render target over the whole client area of `hwnd`, `scale` × 96 DPI.
pub unsafe fn hwnd_target(d2d: &ID2D1Factory, hwnd: HWND, scale: f32) -> windows::core::Result<ID2D1HwndRenderTarget> {
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let dpi = 96.0 * scale;
    let props = D2D1_RENDER_TARGET_PROPERTIES { dpiX: dpi, dpiY: dpi, ..Default::default() };
    let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
        hwnd: WinHwnd(hwnd),
        pixelSize: D2D_SIZE_U { width: (rc.right - rc.left) as u32, height: (rc.bottom - rc.top) as u32 },
        presentOptions: D2D1_PRESENT_OPTIONS_NONE,
    };
    let rt = d2d.CreateHwndRenderTarget(&props, &hwnd_props)?;
    rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    Ok(rt)
}

/// Registers a window class with an arrow cursor. `name` ends with a NUL.
/// Registering it again fails harmlessly.
pub unsafe fn register_class(name: &[u16], style: WNDCLASS_STYLES, wndproc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT) {
    let mut wc: WNDCLASSW = zeroed();
    wc.style = style;
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = GetModuleHandleW(null());
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.lpszClassName = name.as_ptr();
    RegisterClassW(&wc);
}

/// The cursor position, the monitor under it (`flags`: which one when none is) and its DPI.
pub unsafe fn cursor_monitor(flags: MONITOR_FROM_FLAGS) -> (POINT, MONITORINFO, u32) {
    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    let monitor = MonitorFromPoint(pt, flags);
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(monitor, &mut mi);
    let (mut dpi, mut dpi_y) = (96u32, 96u32);
    GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
    (pt, mi, dpi)
}

/// Screen edge the taskbar is docked to (ABE_LEFT 0, TOP 1, RIGHT 2, BOTTOM 3); bottom if unknown.
pub unsafe fn taskbar_edge() -> u32 {
    let mut abd: APPBARDATA = zeroed();
    abd.cbSize = size_of::<APPBARDATA>() as u32;
    if SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) != 0 {
        abd.uEdge
    } else {
        3 // ABE_BOTTOM
    }
}

/// Sets a DWM window attribute that takes a u32.
pub unsafe fn dwm_set(hwnd: HWND, attr: u32, value: u32) {
    DwmSetWindowAttribute(hwnd, attr, &value as *const u32 as _, 4);
}

/// A COLORREF (0x00BBGGRR) for DWM border and caption colors.
pub fn colorref(c: D2D1_COLOR_F) -> u32 {
    (c.r * 255.0) as u32 | ((c.g * 255.0) as u32) << 8 | ((c.b * 255.0) as u32) << 16
}

/// Asks for WM_MOUSELEAVE once the mouse leaves `hwnd`.
pub unsafe fn track_leave(hwnd: HWND) {
    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
    TrackMouseEvent(&mut tme);
}

/// Mouse position of a mouse message in DIPs.
pub fn mouse_pos(lparam: LPARAM, scale: f32) -> (f32, f32) {
    let x = (lparam & 0xFFFF) as i16 as f32;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as f32;
    (x / scale, y / scale)
}
