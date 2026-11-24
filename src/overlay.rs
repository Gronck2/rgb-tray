//! Battery overlay: a small always-on-top plate with the charge of every device, shown and hidden
//! by a hotkey or the tray menu; left on, it comes back at the next start. It has no background: only the icons, percents and (full style) names and bars are drawn, with a
//! contrasting halo so they read on anything. It takes no focus and lets mouse clicks through, except while Ctrl and Shift are held: then it can be dragged
//! (the position is saved), and a double click puts it back in the default corner. It shows the last
//! known levels and never asks a device (that would wake sleeping wireless ones).

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::core::w;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_SHIFT};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::config::{OverlayPos, OverlayStyle};
use crate::devices::{kind_of, Battery, CRITICAL_BATTERY, LOW_BATTERY};
use crate::draw::{cursor_monitor, register_class, Paint};
use crate::sysbat::{self, SysDevice};
use crate::ui::{col, text_format, text_width, theme, Theme, R};

const W: f32 = 280.0;
const PAD: f32 = 14.0;
const VPAD: f32 = 8.0;
const ROW_H: f32 = 40.0;
const RIGHT_W: f32 = 96.0;
/// Width of the device icon in the full style (the name and the bar start after it).
const ICON_W: f32 = 28.0;
// The compact styles: a cell is the icon, the percent (right-aligned) and room for the charging mark.
const C_PAD: f32 = 12.0;
const C_ROW_H: f32 = 28.0;
const C_ICON_W: f32 = 20.0;
const C_PCT_W: f32 = 36.0;
const C_MARK_W: f32 = 14.0;
const C_CELL_W: f32 = C_ICON_W + C_PCT_W + C_MARK_W + 4.0;
/// Space between the cells of the row style.
const C_GAP: f32 = 12.0;
/// Width of the compact plate with its one empty-state line.
const C_EMPTY_W: f32 = 220.0;
/// Alpha of the faint backdrop drawn while the plate is movable, so that all of it can be grabbed
/// (pixels with alpha 0 get no mouse input).
const GRAB_ALPHA: f32 = 0.35;
/// Offset of the text halo, in DIPs.
const HALO: f32 = 1.0;
/// Timer that watches Ctrl+Shift.
const TIMER_MODIFIERS: usize = 1;
/// Timer that reads the system devices again (connecting or switching off headphones) while the plate is up.
const TIMER_SYSTEM: usize = 2;
const SYSTEM_EVERY_MS: u32 = 60_000;
/// Most rows on the plate, so that it fits the work area in every style (Row: 872 DIPs wide, Full: 416 high).
const MAX_ROWS: usize = 10;
/// System devices that did not fit, to log a change once.
static LEFT_OUT: AtomicUsize = AtomicUsize::new(0);
/// Ticks of the modifier timer, to run the taskbar check on some of them.
static TICKS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Level {
    Normal,
    Low,
    Critical,
}

/// One device with a battery on the plate.
#[derive(Clone, PartialEq, Debug)]
struct Row {
    name: String,
    /// The model's glyph of Segoe Fluent Icons, as on the panel's card.
    icon: &'static str,
    percent: u8,
    charging: bool,
    level: Level,
    /// No link: the percent is the last known one.
    asleep: bool,
}

fn level(b: Battery) -> Level {
    // A charging device is never low, as in `Battery::is_low`.
    if b.charging {
        Level::Normal
    } else if b.percent <= CRITICAL_BATTERY {
        Level::Critical
    } else if b.percent <= LOW_BATTERY {
        Level::Low
    } else {
        Level::Normal
    }
}

/// The rows, in the order of `ids` (the panel order); devices without a battery are left out. The system devices
/// (already filtered and sorted by `sysbat::listed`) follow, as many as fit under `MAX_ROWS`; the app's own devices
/// are always kept.
fn rows(ids: &[String], status: &[(String, Battery)], no_link: &[String], name: impl Fn(&str) -> String, system: &[SysDevice]) -> Vec<Row> {
    let mut rows: Vec<Row> = ids
        .iter()
        .filter_map(|id| {
            let b = status.iter().find(|(s, _)| s == id)?.1;
            Some(Row { name: name(id), icon: kind_of(id).icon, percent: b.percent, charging: b.charging, level: level(b), asleep: no_link.contains(id) })
        })
        .collect();
    let room = MAX_ROWS.saturating_sub(rows.len());
    // Windows gives no charging state for these, and a disconnected device is not listed, so none is asleep.
    rows.extend(system.iter().take(room).map(|d| Row {
        name: d.name.clone(),
        icon: sysbat::glyph(d.class),
        percent: d.percent,
        charging: false,
        level: level(Battery { percent: d.percent, charging: false }),
        asleep: false,
    }));
    rows
}

/// Window size in DIPs of `style` for `n` devices; no devices still show one line saying so.
fn size(style: OverlayStyle, n: usize) -> (f32, f32) {
    let k = n.max(1) as f32;
    match style {
        OverlayStyle::Full => (W, 2.0 * VPAD + k * ROW_H),
        _ if n == 0 => (C_EMPTY_W, 2.0 * VPAD + C_ROW_H),
        OverlayStyle::Row => (2.0 * C_PAD + k * C_CELL_W + (k - 1.0) * C_GAP, 2.0 * VPAD + C_ROW_H),
        OverlayStyle::Column => (2.0 * C_PAD + C_CELL_W, 2.0 * VPAD + k * C_ROW_H),
    }
}

fn current_rows() -> Vec<Row> {
    // Ids first: active_ids() locks the config too.
    let ids = crate::active_ids();
    let status = crate::STATUS.lock().unwrap().clone();
    let no_link = crate::NO_LINK.lock().unwrap().clone();
    let system = sysbat::listed();
    let cfg = crate::config();
    let rows = rows(&ids, &status, &no_link, |id| cfg.display_name(id), &system);
    let apps = ids.iter().filter(|id| status.iter().any(|(s, _)| s == *id)).count();
    let dropped = system.len().saturating_sub(MAX_ROWS.saturating_sub(apps));
    if LEFT_OUT.swap(dropped, Ordering::Relaxed) != dropped && dropped > 0 {
        crate::log(&format!("battery overlay: {dropped} system device(s) left out, the plate holds {MAX_ROWS} rows"));
    }
    rows
}

struct Formats {
    name: IDWriteTextFormat,
    percent: IDWriteTextFormat,
    /// The charging mark.
    icon: IDWriteTextFormat,
    /// The device icon.
    glyph: IDWriteTextFormat,
}

fn formats() -> windows::core::Result<Formats> {
    let dw: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
    let ui = w!("Segoe UI Variable Text");
    let (lead, trail) = (DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING);
    Ok(Formats {
        glyph: text_format(&dw, w!("Segoe Fluent Icons"), 16.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        name: text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, lead)?,
        percent: text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, trail)?,
        icon: text_format(&dw, w!("Segoe Fluent Icons"), 12.0, DWRITE_FONT_WEIGHT_NORMAL, trail)?,
    })
}

struct Overlay {
    hwnd: HWND,
    scale: f32,
    d2d: ID2D1Factory,
    surface: Option<Surface>,
    brush: Option<ID2D1SolidColorBrush>,
    fmt: Formats,
    theme: Theme,
    rows: Vec<Row>,
    style: OverlayStyle,
    /// Where the plate sits (see `OverlayPos`); kept when it is resized.
    pos: OverlayPos,
    /// Ctrl+Shift is held: the plate takes the mouse and can be dragged.
    movable: bool,
    /// The system move loop is running.
    dragging: bool,
}

/// The plate's pixels: a 32-bit top-down DIB in a memory DC with a premultiplied-alpha Direct2D target on it.
/// `UpdateLayeredWindow` shows it with per-pixel alpha.
struct Surface {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    w: i32,
    h: i32,
    rt: Option<ID2D1DCRenderTarget>,
}

impl Surface {
    unsafe fn new(d2d: &ID2D1Factory, w: i32, h: i32, scale: f32) -> windows::core::Result<Surface> {
        use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
        let mut bi: BITMAPINFO = zeroed();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        };
        let mut bits = null_mut();
        let bmp = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bmp.is_null() {
            return Err(windows::core::Error::from_win32());
        }
        let dc = CreateCompatibleDC(null_mut());
        let old = SelectObject(dc, bmp);
        let mut s = Surface { dc, bmp, old, w, h, rt: None };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0 * scale,
            dpiY: 96.0 * scale,
            ..Default::default()
        };
        // On error `s` is dropped and frees the GDI objects.
        let rt = d2d.CreateDCRenderTarget(&props)?;
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        s.rt = Some(rt);
        Ok(s)
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        self.rt = None;
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.dc);
        }
    }
}

/// Screen rectangle: left, top, right, bottom.
type Rc = (i32, i32, i32, i32);

fn rc(r: RECT) -> Rc {
    (r.left, r.top, r.right, r.bottom)
}

/// The position of `plate` to remember: its left edge (the plate always grows to the right), and the top or bottom edge
/// nearest the edge of `work` (it grows away from that one). `right` is only read from older configs, never set.
fn anchor_of(plate: Rc, work: Rc) -> OverlayPos {
    let bottom = plate.1 - work.1 > work.3 - plate.3;
    OverlayPos { x: plate.0, y: if bottom { plate.3 } else { plate.1 }, right: false, bottom }
}

/// `pos` of an older config (`right`: `x` is the plate's right edge) as a left-anchored one for a `w` wide plate.
fn left_anchored(pos: OverlayPos, w: i32) -> OverlayPos {
    OverlayPos { x: if pos.right { pos.x - w } else { pos.x }, right: false, ..pos }
}

/// Top-left of a `w` x `h` plate kept at `pos` (left-anchored): it grows to the right, and up or down away from the anchored edge.
fn origin(pos: OverlayPos, h: i32) -> (i32, i32) {
    (pos.x, if pos.bottom { pos.y - h } else { pos.y })
}

/// `origin` moved the least that puts the plate inside `work` (the top-left wins when it does not fit).
fn placed(pos: OverlayPos, w: i32, h: i32, work: Rc) -> (i32, i32) {
    let (l, t) = origin(pos, h);
    (l.min(work.2 - w).max(work.0), t.min(work.3 - h).max(work.1))
}

/// `plate` moved the least that puts it inside `work`.
fn clamped(plate: Rc, work: Rc) -> Rc {
    let (w, h) = (plate.2 - plate.0, plate.3 - plate.1);
    let (l, t) = placed(anchor_of(plate, work), w, h, work);
    (l, t, l + w, t + h)
}

/// A point inside the monitor a position belongs to (the anchor lies on the plate's edge, which is
/// outside a monitor on its right or bottom side).
fn probe(pos: OverlayPos) -> POINT {
    POINT { x: pos.x - pos.right as i32, y: pos.y - pos.bottom as i32 }
}

/// The default position of a `w` wide plate: top-right of `work`, `margin` pixels in (from there it grows to the right and is clamped).
fn default_pos(work: Rc, margin: i32, w: i32) -> OverlayPos {
    OverlayPos { x: work.2 - margin - w, y: work.1 + margin, right: false, bottom: false }
}

/// Full rectangle (the taskbar included: the plate may sit over it) and DPI of the monitor at `pt`; `None` when no monitor holds it and `flags` says not to guess.
unsafe fn monitor_at(pt: POINT, flags: MONITOR_FROM_FLAGS) -> Option<(Rc, u32)> {
    let monitor = MonitorFromPoint(pt, flags);
    if monitor.is_null() {
        return None;
    }
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(monitor, &mut mi);
    let (mut dpi, mut dpi_y) = (96u32, 96u32);
    GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
    Some((rc(mi.rcMonitor), dpi))
}

thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

pub fn is_visible() -> bool {
    OVERLAY.with(|c| c.borrow().is_some())
}

/// Shows the plate, or hides it when it is shown; the choice is remembered for the next start. The plate also goes away
/// when the app exits, but that is not the user's choice and is not remembered.
pub fn toggle() {
    let plate = OVERLAY.with(|c| c.borrow().as_ref().map(|o| o.hwnd));
    let shown = match plate {
        Some(h) => {
            unsafe { DestroyWindow(h) };
            false
        }
        // A plate that could not be opened changes nothing.
        None if !show() => return,
        None => true,
    };
    let mut cfg = crate::config();
    cfg.overlay_shown = shown;
    cfg.save();
}

/// At start: shows the plate when the user left it on.
pub fn restore() {
    let wanted = crate::config().overlay_shown;
    if wanted && !is_visible() {
        show();
    }
}

/// Opens the plate and reads the system devices; whether it opened.
fn show() -> bool {
    if let Err(e) = unsafe { open() } {
        crate::log(&format!("battery overlay: {e}"));
        return false;
    }
    // Windows' own levels ask no device; the Logitech HID++ reads are skipped when the user is idle or has system devices off.
    sysbat::refresh_async();
    true
}

/// Draws the plate again and puts it on screen; nothing when it is hidden.
fn redraw() {
    OVERLAY.with(|c| c.borrow_mut().as_mut().map(|o| o.redraw()));
}

/// Redraws the plate with the latest levels; nothing when it is hidden.
pub fn refresh() {
    if !is_visible() {
        return;
    }
    let rows = current_rows();
    let style = crate::config().overlay_style;
    unsafe {
        let moved = OVERLAY.with(|c| {
            c.borrow_mut().as_mut().map(|o| {
                let resized = size(o.style, o.rows.len()) != size(style, rows.len());
                o.rows = rows;
                o.style = style;
                let t = theme();
                if t.dark != o.theme.dark {
                    o.theme = t;
                    o.brush = o.surface.as_ref().and_then(|s| s.rt.as_ref()).and_then(|rt| rt.CreateSolidColorBrush(&t.text, None).ok());
                }
                if resized {
                    // The surface is made for the window size.
                    o.surface = None;
                    o.brush = None;
                }
                (o.hwnd, resized, o.scale, o.pos, size(style, o.rows.len()))
            })
        });
        if let Some((hwnd, resized, scale, pos, dip)) = moved {
            if resized {
                place(hwnd, scale, pos, dip);
            }
            redraw();
        }
    }
}

/// Puts the plate at `pos`, sized `dip` (width, height in DIPs), inside its monitor (the taskbar included).
unsafe fn place(hwnd: HWND, scale: f32, pos: OverlayPos, dip: (f32, f32)) {
    let (w, h) = ((dip.0 * scale).round() as i32, (dip.1 * scale).round() as i32);
    let bounds = monitor_at(probe(pos), MONITOR_DEFAULTTONEAREST).map(|m| m.0).unwrap_or((0, 0, i32::MAX, i32::MAX));
    let (l, t) = placed(pos, w, h, bounds);
    SetWindowPos(hwnd, HWND_TOPMOST, l, t, w, h, SWP_NOACTIVATE);
}

/// Puts the plate above the other topmost windows (the taskbar is one) without activating it.
unsafe fn raise(hwnd: HWND) {
    SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
}

/// Whether `plate` reaches outside `work` (the taskbar and other app bars are outside the work area).
fn leaves_work(plate: Rc, work: Rc) -> bool {
    plate.0 < work.0 || plate.1 < work.1 || plate.2 > work.2 || plate.3 > work.3
}

/// A window above the plate in z-order.
#[derive(Debug, PartialEq)]
enum Above {
    Taskbar,
    /// A window of this process (panel, tray menu, alert): the plate stays under it.
    Own,
    Other,
}

/// `above`: windows over the plate, nearest first. Raise only when the first one that is not `Other` is a taskbar.
fn should_raise(above: impl IntoIterator<Item = Above>) -> bool {
    above.into_iter().find(|a| !matches!(a, Above::Other)) == Some(Above::Taskbar)
}

unsafe fn classify(hwnd: HWND, own_pid: u32) -> Above {
    // A hidden window covers nothing (IME windows of our threads, a taskbar of a monitor that is gone).
    if IsWindowVisible(hwnd) == 0 {
        return Above::Other;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == own_pid {
        return Above::Own;
    }
    let mut buf = [0u16; 32];
    let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
    match String::from_utf16_lossy(&buf[..n]).as_str() {
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" => Above::Taskbar,
        _ => Above::Other,
    }
}

/// The taskbar raises itself above topmost windows when it is clicked: when the plate overlaps it and the nearest window above
/// the plate (own windows count, other apps' windows are skipped) is a taskbar, put the plate back on top. Called from the
/// modifier timer; a no-op for a plate inside the work area. Once the plate is on top nothing is above it, so there is no flicker.
unsafe fn keep_above_taskbar(hwnd: HWND) {
    let mut r: RECT = zeroed();
    GetWindowRect(hwnd, &mut r);
    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    if !monitor.is_null() && GetMonitorInfoW(monitor, &mut mi) != 0 && leaves_work(rc(r), rc(mi.rcWork)) {
        let own_pid = GetCurrentProcessId();
        // 512 steps at most: the chain can change while it is walked.
        let above = std::iter::successors(Some(hwnd), |&w| {
            let p = GetWindow(w, GW_HWNDPREV);
            (!p.is_null()).then_some(p)
        })
        .skip(1)
        .take(512)
        .map(|w| classify(w, own_pid));
        if should_raise(above) {
            raise(hwnd);
        }
    }
}

/// While Ctrl and Shift are both down the plate takes the mouse (so it can be dragged); otherwise
/// clicks pass through it. Checked by a timer: a click-through window gets no keyboard or mouse messages.
unsafe fn follow_modifiers(hwnd: HWND) {
    let down = |vk| GetAsyncKeyState(vk as i32) < 0;
    let want = down(VK_CONTROL) && down(VK_SHIFT);
    let changed = OVERLAY.with(|c| match c.borrow_mut().as_mut() {
        // A drag in progress keeps the plate movable even if a key is let go.
        Some(o) if !o.dragging && o.movable != want => {
            o.movable = want;
            true
        }
        _ => false,
    });
    if changed {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let ex = if want { ex & !WS_EX_TRANSPARENT } else { ex | WS_EX_TRANSPARENT };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex as isize);
        redraw(); // the grab backdrop and the border cue
    }
}

/// The drag ended: remember the plate's corner nearest the screen edges.
unsafe fn save_position(hwnd: HWND) {
    let mut r: RECT = zeroed();
    GetWindowRect(hwnd, &mut r);
    let plate = rc(r);
    let Some((bounds, dpi)) = monitor_at(POINT { x: (plate.0 + plate.2) / 2, y: (plate.1 + plate.3) / 2 }, MONITOR_DEFAULTTONEAREST) else { return };
    // The move loop lets the plate hang past an edge: pull it back in before remembering its corner.
    let inside = clamped(plate, bounds);
    if inside != plate {
        SetWindowPos(hwnd, null_mut(), inside.0, inside.1, 0, 0, SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE);
    }
    let pos = anchor_of(inside, bounds);
    let rescaled = OVERLAY.with(|c| {
        c.borrow_mut().as_mut().and_then(|o| {
            o.pos = pos;
            let scale = dpi as f32 / 96.0;
            // Dragged to a monitor with another DPI: size the plate for it.
            let rescaled = scale != o.scale;
            if rescaled {
                o.scale = scale;
                o.surface = None;
                o.brush = None;
            }
            rescaled.then(|| (scale, size(o.style, o.rows.len())))
        })
    });
    if let Some((scale, dip)) = rescaled {
        place(hwnd, scale, pos, dip);
    }
    let mut cfg = crate::config();
    cfg.overlay_pos = Some(pos);
    cfg.save();
}

/// Back to the default corner of the monitor under the cursor; forgets the saved position.
unsafe fn reset_position(hwnd: HWND) {
    let (_, mi, dpi) = cursor_monitor(MONITOR_DEFAULTTOPRIMARY);
    let scale = dpi as f32 / 96.0;
    let margin = (12.0 * scale).round() as i32;
    let moved = OVERLAY.with(|c| {
        c.borrow_mut().as_mut().map(|o| {
            if scale != o.scale {
                o.scale = scale;
                o.surface = None;
                o.brush = None;
            }
            let dip = size(o.style, o.rows.len());
            o.pos = default_pos(rc(mi.rcWork), margin, (dip.0 * scale).round() as i32);
            (o.pos, dip)
        })
    });
    if let Some((pos, dip)) = moved {
        place(hwnd, scale, pos, dip);
        redraw();
    }
    let mut cfg = crate::config();
    cfg.overlay_pos = None;
    cfg.save();
}

unsafe fn open() -> windows::core::Result<()> {
    let hinst = GetModuleHandleW(null());
    let class: Vec<u16> = "RgbTrayBatteryOverlay\0".encode_utf16().collect();
    register_class(&class, 0, wndproc);

    // The saved place if a monitor still holds it (clamped to the whole monitor: the plate may sit over the taskbar), else the
    // default corner of the work area of the monitor under the cursor now (it does not follow while the plate is up).
    let saved = crate::config().overlay_pos.and_then(|p| monitor_at(probe(p), MONITOR_DEFAULTTONULL).map(|m| (p, m)));
    let (saved_pos, bounds, dpi) = match saved {
        Some((p, (full, dpi))) => (Some(p), full, dpi),
        None => {
            let (_, mi, dpi) = cursor_monitor(MONITOR_DEFAULTTOPRIMARY);
            (None, rc(mi.rcWork), dpi)
        }
    };
    let scale = dpi as f32 / 96.0;
    let rows = current_rows();
    let style = crate::config().overlay_style;
    let dip = size(style, rows.len());
    let (w, h) = ((dip.0 * scale).round() as i32, (dip.1 * scale).round() as i32);
    let pos = match saved_pos {
        Some(p) => left_anchored(p, w),
        None => default_pos(bounds, (12.0 * scale).round() as i32, w),
    };
    let (left, top) = placed(pos, w, h, bounds);

    // What can fail comes before the window: a window without its state would be stuck.
    let d2d = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
    let fmt = formats()?;
    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
        class.as_ptr(),
        class.as_ptr(),
        WS_POPUP,
        left,
        top,
        w,
        h,
        null_mut(),
        null_mut(),
        hinst,
        null(),
    );
    if hwnd.is_null() {
        return Err(windows::core::Error::from_win32());
    }
    let theme = theme();

    let overlay = Overlay { hwnd, scale, d2d, surface: None, brush: None, fmt, theme, rows, style, pos, movable: false, dragging: false };
    OVERLAY.with(|c| *c.borrow_mut() = Some(overlay));
    SetTimer(hwnd, TIMER_MODIFIERS, 80, None);
    SetTimer(hwnd, TIMER_SYSTEM, SYSTEM_EVERY_MS, None);
    redraw(); // a layered window with per-pixel alpha shows nothing until it is updated
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    raise(hwnd);
    Ok(())
}

impl Paint for Overlay {
    fn brush(&self) -> &ID2D1SolidColorBrush {
        self.brush.as_ref().unwrap()
    }
}

/// The name cut with an ellipsis to fit `max_w`.
fn fit(name: &str, fmt: &IDWriteTextFormat, max_w: f32) -> String {
    if text_width(name, fmt) <= max_w {
        return name.to_string();
    }
    let mut s: Vec<char> = name.chars().collect();
    while s.pop().is_some() {
        let t: String = s.iter().collect::<String>().trim_end().to_string() + "…";
        if text_width(&t, fmt) <= max_w {
            return t;
        }
    }
    "…".into()
}

/// The 8 directions of the halo.
const HALO_DIRS: [(f32, f32); 8] = [(-1.0, -1.0), (0.0, -1.0), (1.0, -1.0), (-1.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (0.0, 1.0), (1.0, 1.0)];

/// Halo colour: black behind light text (dark theme), white behind dark text; `a` is the text's alpha (dimming).
fn halo_color(t: &Theme, a: f32) -> D2D1_COLOR_F {
    let (v, k) = if t.dark { (0.0, 0.3) } else { (1.0, 0.5) };
    D2D1_COLOR_F { r: v, g: v, b: v, a: k * a }
}

/// Warning colors that read on the plate's background.
fn level_color(level: Level, t: &Theme) -> D2D1_COLOR_F {
    match (level, t.dark) {
        (Level::Normal, _) => t.accent,
        (Level::Low, true) => col(255, 185, 40, 1.0),
        (Level::Low, false) => col(196, 112, 0, 1.0),
        (Level::Critical, true) => col(255, 99, 99, 1.0),
        (Level::Critical, false) => col(196, 32, 32, 1.0),
    }
}

impl Overlay {
    /// Draws the plate and puts it on screen. A lost render target is rebuilt and drawn once more; any other
    /// failure is logged and the plate stays as it was.
    fn redraw(&mut self) {
        let Err(e) = self.paint() else { return };
        self.surface = None;
        self.brush = None;
        let e = if e.code() == windows::Win32::Foundation::D2DERR_RECREATE_TARGET {
            match self.paint() {
                Ok(()) => return,
                Err(e) => {
                    self.surface = None;
                    self.brush = None;
                    e
                }
            }
        } else {
            e
        };
        crate::log(&format!("battery overlay: {e}"));
    }

    /// Draws the plate on its surface (made when missing) and puts it on screen.
    fn paint(&mut self) -> windows::core::Result<()> {
        unsafe {
            let (w, h) = size(self.style, self.rows.len());
            let (pw, ph) = ((w * self.scale).round() as i32, (h * self.scale).round() as i32);
            if self.surface.as_ref().is_some_and(|s| (s.w, s.h) != (pw, ph)) {
                self.surface = None;
                self.brush = None;
            }
            if self.surface.is_none() {
                let s = Surface::new(&self.d2d, pw, ph, self.scale)?;
                self.brush = s.rt.as_ref().and_then(|rt| rt.CreateSolidColorBrush(&self.theme.text, None).ok());
                self.surface = Some(s);
            }
            let s = self.surface.as_ref().unwrap();
            let rt = s.rt.clone().unwrap();
            let bound = windows::Win32::Foundation::RECT { left: 0, top: 0, right: pw, bottom: ph };
            rt.BindDC(windows::Win32::Graphics::Gdi::HDC(s.dc), &bound)?;
            rt.BeginDraw();
            rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            self.draw(&rt);
            rt.EndDraw(None, None)?;
            let size = SIZE { cx: pw, cy: ph };
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            UpdateLayeredWindow(self.hwnd, null_mut(), null(), &size, s.dc, &POINT { x: 0, y: 0 }, 0, &blend, ULW_ALPHA);
            Ok(())
        }
    }

    /// Text with a halo of the contrasting colour around it, so that it reads on any background.
    unsafe fn glow(&self, rt: &ID2D1RenderTarget, s: &str, fmt: &IDWriteTextFormat, r: R, c: D2D1_COLOR_F) {
        let halo = halo_color(&self.theme, c.a);
        for (dx, dy) in HALO_DIRS {
            self.text(rt, s, fmt, R { x: r.x + dx * HALO, y: r.y + dy * HALO, ..r }, halo);
        }
        self.text(rt, s, fmt, r, c);
    }

    unsafe fn draw(&self, rt: &ID2D1RenderTarget) {
        let t = &self.theme;
        let (w, h) = size(self.style, self.rows.len());
        if self.movable {
            // The plate can be dragged now: a faint backdrop so that all of it takes the mouse, and the accent border.
            self.fill(rt, R { x: 0.0, y: 0.0, w, h }, 0.0, D2D1_COLOR_F { a: GRAB_ALPHA, ..t.bg });
            self.stroke(rt, R { x: 1.0, y: 1.0, w: w - 2.0, h: h - 2.0 }, 0.0, t.accent, 2.0);
        }
        if self.rows.is_empty() {
            let line = tr!("Нет устройств с батареей", "No devices with a battery");
            let (pad, line_h) = if self.style == OverlayStyle::Full { (PAD, ROW_H) } else { (C_PAD, C_ROW_H) };
            self.glow(rt, line, &self.fmt.name, R { x: pad, y: VPAD, w: w - 2.0 * pad, h: line_h }, t.text2);
            return;
        }
        for (i, row) in self.rows.iter().enumerate() {
            match self.style {
                OverlayStyle::Full => self.full_row(rt, row, VPAD + i as f32 * ROW_H),
                OverlayStyle::Row => self.cell(rt, row, C_PAD + i as f32 * (C_CELL_W + C_GAP), VPAD),
                OverlayStyle::Column => self.cell(rt, row, C_PAD, VPAD + i as f32 * C_ROW_H),
            }
        }
    }

    /// Color of the percent and the icon: the warning one, grey when the device is asleep.
    fn value_color(&self, row: &Row) -> D2D1_COLOR_F {
        match (row.level, row.asleep) {
            (Level::Normal, false) => self.theme.text,
            (Level::Normal, true) => self.theme.text2,
            (level, _) => level_color(level, &self.theme),
        }
    }

    /// A device without link is dimmed: it keeps its last level.
    fn dim(row: &Row, mut c: D2D1_COLOR_F) -> D2D1_COLOR_F {
        if row.asleep {
            c.a *= 0.5;
        }
        c
    }

    /// The full style: icon, name, percent with the charging mark, and the level bar.
    unsafe fn full_row(&self, rt: &ID2D1RenderTarget, row: &Row, y: f32) {
        let t = &self.theme;
        let dim = |c| Self::dim(row, c);
        let accent = level_color(row.level, t);
        self.glow(rt, row.icon, &self.fmt.glyph, R { x: PAD, y: y + 2.0, w: ICON_W - 6.0, h: 29.0 }, dim(self.value_color(row)));
        let left = PAD + ICON_W;
        let name_w = W - PAD - left - RIGHT_W - 8.0;
        self.glow(rt, &fit(&row.name, &self.fmt.name, name_w), &self.fmt.name, R { x: left, y: y + 2.0, w: name_w, h: 20.0 }, dim(t.text));

        let right = R { x: W - PAD - RIGHT_W, y: y + 2.0, w: RIGHT_W, h: 20.0 };
        let percent = if row.asleep { format!("{} · {}%", tr!("спит", "asleep"), row.percent) } else { format!("{}%", row.percent) };
        self.glow(rt, &percent, &self.fmt.percent, right, dim(self.value_color(row)));
        if row.charging && !row.asleep {
            // The charging mark sits left of the percent.
            let mark = R { x: right.x, w: right.w - text_width(&percent, &self.fmt.percent) - 4.0, ..right };
            self.glow(rt, "\u{E83E}", &self.fmt.icon, mark, dim(accent));
        }

        let bar = R { x: left, y: y + 27.0, w: W - PAD - left, h: 4.0 };
        // An outline of the halo colour keeps the bar visible on any background.
        let halo = halo_color(t, if row.asleep { 0.5 } else { 1.0 });
        self.fill(rt, R { x: bar.x - HALO, y: bar.y - HALO, w: bar.w + 2.0 * HALO, h: bar.h + 2.0 * HALO }, 2.0 + HALO, halo);
        self.fill(rt, bar, 2.0, D2D1_COLOR_F { a: 0.45, ..t.text2 });
        let filled = (bar.w * row.percent.min(100) as f32 / 100.0).max(bar.h);
        self.fill(rt, R { w: filled, ..bar }, 2.0, dim(accent));
    }

    /// A compact cell at (`x`, `y`): icon, percent and the charging mark; no name, no bar.
    unsafe fn cell(&self, rt: &ID2D1RenderTarget, row: &Row, x: f32, y: f32) {
        let dim = |c| Self::dim(row, c);
        let color = dim(self.value_color(row));
        self.glow(rt, row.icon, &self.fmt.glyph, R { x, y, w: C_ICON_W, h: C_ROW_H }, color);
        self.glow(rt, &format!("{}%", row.percent), &self.fmt.percent, R { x: x + C_ICON_W, y, w: C_PCT_W, h: C_ROW_H }, color);
        if row.charging && !row.asleep {
            self.glow(
                rt,
                "\u{E83E}",
                &self.fmt.icon,
                R { x: x + C_ICON_W + C_PCT_W + 4.0, y, w: C_MARK_W, h: C_ROW_H },
                dim(level_color(row.level, &self.theme)),
            );
        }
    }
}

/// Renders the plate with the current levels into a PNG file, for checking the look: transparent, as on screen, or over a
/// backdrop with `RGBTRAY_DEMO_OVERLAY_BG=light|dark|split` (split: white left half, black right half). The style is the
/// saved one, or `RGBTRAY_DEMO_OVERLAY_STYLE=full|row|column`.
pub fn snapshot(path: &str, dark: Option<bool>, scale: f32) -> windows::core::Result<()> {
    let rows = current_rows();
    let style = std::env::var("RGBTRAY_DEMO_OVERLAY_STYLE").ok().and_then(|s| OverlayStyle::parse(&s)).unwrap_or(crate::config().overlay_style);
    let (w, h) = size(style, rows.len());
    crate::ui::render_png(path, w, h, scale, |rt, d2d| unsafe {
        let theme = crate::ui::theme_for(dark);
        let o = Overlay {
            hwnd: null_mut(),
            scale,
            d2d,
            surface: None,
            brush: Some(rt.CreateSolidColorBrush(&theme.text, None)?),
            fmt: formats()?,
            theme,
            rows,
            style,
            pos: default_pos((0, 0, 0, 0), 0, 0),
            movable: false,
            dragging: false,
        };
        let backdrop = std::env::var("RGBTRAY_DEMO_OVERLAY_BG").unwrap_or_default();
        let paint = |x: f32, w: f32, v: f32| o.fill(rt, R { x, y: 0.0, w, h }, 0.0, D2D1_COLOR_F { r: v, g: v, b: v, a: 1.0 });
        match backdrop.as_str() {
            "light" => paint(0.0, w, 1.0),
            "dark" => paint(0.0, w, 0.0),
            "split" => {
                paint(0.0, w / 2.0, 1.0);
                paint(w / 2.0, w / 2.0, 0.0);
            }
            _ => {}
        }
        o.draw(rt);
        Ok(())
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Mouse clicks go to the window under the plate, unless Ctrl+Shift is held: then the plate is a
        // caption and the system moves it. It is never activated.
        WM_NCHITTEST => {
            if OVERLAY.with(|c| c.borrow().as_ref().is_some_and(|o| o.movable)) {
                HTCAPTION as LRESULT
            } else {
                HTTRANSPARENT as LRESULT
            }
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_TIMER if wparam == TIMER_MODIFIERS => {
            follow_modifiers(hwnd);
            // Every third tick (about 240 ms).
            if TICKS.fetch_add(1, Ordering::Relaxed).is_multiple_of(3) {
                keep_above_taskbar(hwnd);
            }
            0
        }
        WM_TIMER if wparam == TIMER_SYSTEM => {
            sysbat::refresh_async();
            0
        }
        WM_ENTERSIZEMOVE => {
            OVERLAY.with(|c| c.borrow_mut().as_mut().map(|o| o.dragging = true));
            0
        }
        WM_EXITSIZEMOVE => {
            OVERLAY.with(|c| c.borrow_mut().as_mut().map(|o| o.dragging = false));
            save_position(hwnd);
            raise(hwnd);
            follow_modifiers(hwnd); // keys may be up by now
            redraw();
            0
        }
        WM_NCLBUTTONDBLCLK => {
            reset_position(hwnd);
            0
        }
        // No system menu from the drag area.
        WM_NCRBUTTONUP => 0,
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_MODIFIERS);
            KillTimer(hwnd, TIMER_SYSTEM);
            let o = OVERLAY.with(|c| c.borrow_mut().take());
            drop(o);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysbat::SysClass;

    #[test]
    fn plate_outside_work_area_overlaps_taskbar() {
        let work = (0, 0, 1920, 1040);
        assert!(!leaves_work((10, 10, 300, 100), work));
        assert!(!leaves_work((0, 0, 1920, 1040), work));
        assert!(leaves_work((10, 1000, 300, 1060), work));
        assert!(leaves_work((-5, 10, 300, 100), work));
    }

    #[test]
    fn raise_only_when_taskbar_is_nearest_decisive_window() {
        use Above::*;
        assert!(!should_raise([]));
        assert!(should_raise([Taskbar]));
        assert!(should_raise([Other, Other, Taskbar]));
        assert!(!should_raise([Own, Taskbar]));
        assert!(!should_raise([Other, Own, Taskbar]));
        assert!(!should_raise([Other, Other]));
        assert!(should_raise([Taskbar, Own]));
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn b(percent: u8, charging: bool) -> Battery {
        Battery { percent, charging }
    }

    fn run(order: &[&str], status: &[(&str, Battery)], no_link: &[&str]) -> Vec<Row> {
        run_with_system(order, status, no_link, &[])
    }

    fn run_with_system(order: &[&str], status: &[(&str, Battery)], no_link: &[&str], system: &[SysDevice]) -> Vec<Row> {
        let status: Vec<(String, Battery)> = status.iter().map(|(i, b)| (i.to_string(), *b)).collect();
        rows(&ids(order), &status, &ids(no_link), |id| format!("name-{id}"), system)
    }

    fn sys(name: &str, class: SysClass, percent: u8) -> SysDevice {
        SysDevice { id: format!("system-{name}"), name: name.into(), class, percent }
    }

    #[test]
    fn system_rows_follow_the_apps_with_their_own_icon_and_levels() {
        let system = [sys("Headphones", SysClass::Headphones, 94), sys("Pad", SysClass::Gamepad, 8), sys("Mouse", SysClass::Mouse, 4)];
        let r = run_with_system(&["a", "b"], &[("a", b(50, false)), ("b", b(60, true))], &["a"], &system);
        // The app's rows are as without system devices.
        assert_eq!(&r[..2], &run(&["a", "b"], &[("a", b(50, false)), ("b", b(60, true))], &["a"])[..]);
        assert_eq!(r.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["name-a", "name-b", "Headphones", "Pad", "Mouse"]);
        assert_eq!(r[2].icon, sysbat::glyph(SysClass::Headphones));
        assert_eq!(r[3].icon, sysbat::glyph(SysClass::Gamepad));
        // Never asleep, never charging; the usual colours.
        assert!(r[2..].iter().all(|r| !r.asleep && !r.charging));
        assert_eq!(r[2..].iter().map(|r| r.level).collect::<Vec<_>>(), [Level::Normal, Level::Low, Level::Critical]);
        assert_eq!(r[2].percent, 94);
    }

    #[test]
    fn system_devices_alone_fill_the_plate() {
        assert_eq!(run_with_system(&[], &[], &[], &[sys("X", SysClass::Other, 50)]).len(), 1);
        assert!(run_with_system(&[], &[], &[], &[]).is_empty());
    }

    #[test]
    fn the_cap_keeps_the_apps_devices_and_drops_system_ones_last() {
        let system: Vec<SysDevice> = (0..8).map(|i| sys(&format!("S{i}"), SysClass::Other, 50)).collect();
        let order: Vec<String> = (0..4).map(|i| format!("a{i}")).collect();
        let order: Vec<&str> = order.iter().map(String::as_str).collect();
        let status: Vec<(&str, Battery)> = order.iter().map(|id| (*id, b(50, false))).collect();
        let r = run_with_system(&order, &status, &[], &system);
        assert_eq!(r.len(), MAX_ROWS);
        assert_eq!(r[MAX_ROWS - 1].name, "S5");
        // More app devices than the cap: none is dropped, no system one is added.
        let many: Vec<String> = (0..12).map(|i| format!("a{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let status: Vec<(&str, Battery)> = many.iter().map(|id| (*id, b(50, false))).collect();
        assert_eq!(run_with_system(&many, &status, &[], &system).len(), 12);
    }

    #[test]
    fn the_plate_at_the_cap_fits_small_work_areas() {
        // 1366x768 at 100% and 1920x1080 at 200% (960x540 DIPs) leave room below 920 x 500.
        for style in STYLES {
            let (w, h) = size(style, MAX_ROWS);
            assert!(w < 920.0 && h < 500.0, "{style:?}: {w} x {h}");
        }
    }

    #[test]
    fn skips_devices_without_battery() {
        let r = run(&["a", "wired", "b"], &[("a", b(50, false)), ("b", b(60, false))], &[]);
        assert_eq!(r.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["name-a", "name-b"]);
    }

    #[test]
    fn levels_follow_the_thresholds() {
        let r = run(&["a", "b", "c", "d", "e"], &[("a", b(11, false)), ("b", b(10, false)), ("c", b(6, false)), ("d", b(5, false)), ("e", b(0, false))], &[]);
        let l: Vec<Level> = r.iter().map(|r| r.level).collect();
        assert_eq!(l, [Level::Normal, Level::Low, Level::Low, Level::Critical, Level::Critical]);
    }

    #[test]
    fn charging_is_never_low() {
        let r = run(&["a", "b"], &[("a", b(3, true)), ("b", b(10, true))], &[]);
        assert!(r.iter().all(|r| r.level == Level::Normal && r.charging));
    }

    #[test]
    fn no_link_is_asleep_and_keeps_the_percent() {
        let r = run(&["a", "b"], &[("a", b(42, false)), ("b", b(80, false))], &["a"]);
        assert!(r[0].asleep && r[0].percent == 42);
        assert!(!r[1].asleep);
    }

    #[test]
    fn order_is_the_given_one() {
        let status = [("a", b(1, false)), ("b", b(2, false)), ("c", b(3, false))];
        let r = run(&["c", "a", "b"], &status, &[]);
        assert_eq!(r.iter().map(|r| r.percent).collect::<Vec<_>>(), [3, 1, 2]);
        // A level of a device no longer listed is not shown.
        assert_eq!(run(&["a"], &status, &[]).len(), 1);
    }

    #[test]
    fn rows_carry_the_model_icon() {
        let r = run(&["logitech-1"], &[("logitech-1", b(50, false))], &[]);
        assert_eq!(r[0].icon, kind_of("logitech-1").icon);
        assert!(!r[0].icon.is_empty());
    }

    #[test]
    fn no_devices_gives_no_rows() {
        assert!(run(&[], &[("a", b(50, false))], &[]).is_empty());
        assert!(run(&["a"], &[], &[]).is_empty());
    }

    #[test]
    fn asleep_low_device_keeps_its_level() {
        let r = run(&["a"], &[("a", b(4, false))], &["a"]);
        assert!(r[0].asleep && r[0].level == Level::Critical && r[0].percent == 4);
    }

    #[test]
    fn halo_contrasts_with_the_text_and_follows_dimming() {
        let dark = theme_for_test(true);
        let light = theme_for_test(false);
        let (hd, hl) = (halo_color(&dark, 1.0), halo_color(&light, 1.0));
        assert!(hd.r == 0.0 && hl.r == 1.0);
        assert_eq!(halo_color(&dark, 0.5).a, hd.a / 2.0);
    }

    fn theme_for_test(dark: bool) -> Theme {
        crate::ui::theme_for(Some(dark))
    }

    const STYLES: [OverlayStyle; 3] = [OverlayStyle::Full, OverlayStyle::Row, OverlayStyle::Column];

    #[test]
    fn height_grows_with_rows_and_has_an_empty_state() {
        let height = |n| size(OverlayStyle::Full, n).1;
        assert!(height(0) > 2.0 * VPAD);
        assert_eq!(height(0), height(1));
        assert!(height(3) > height(2) && height(2) > height(1));
    }

    #[test]
    fn full_is_fixed_width_and_column_is_narrow() {
        for n in 0..5 {
            assert_eq!(size(OverlayStyle::Full, n).0, W);
        }
        for n in 1..5 {
            let (cw, _) = size(OverlayStyle::Column, n);
            assert!(cw < W && cw == size(OverlayStyle::Column, 1).0);
        }
    }

    #[test]
    fn row_grows_in_width_and_column_in_height() {
        let (w1, h1) = size(OverlayStyle::Row, 1);
        let (w3, h3) = size(OverlayStyle::Row, 3);
        assert!(w3 > w1 && h3 == h1);
        let (cw1, ch1) = size(OverlayStyle::Column, 1);
        let (cw3, ch3) = size(OverlayStyle::Column, 3);
        assert!(ch3 > ch1 && cw3 == cw1);
        // The compact plates are lower than the full one for the same devices.
        assert!(h3 < size(OverlayStyle::Full, 3).1 && ch3 < size(OverlayStyle::Full, 3).1);
    }

    #[test]
    fn empty_state_fits_one_line_in_every_style() {
        for style in STYLES {
            let (w, h) = size(style, 0);
            assert!(w >= C_EMPTY_W && h > 2.0 * VPAD);
            assert_eq!(size(style, 0).1, if style == OverlayStyle::Full { size(style, 1).1 } else { 2.0 * VPAD + C_ROW_H });
        }
    }

    #[test]
    fn rows_carry_the_icon_of_the_model() {
        let r = run(&["logitech-A", "naga-B", "unknown-C"], &[("logitech-A", b(50, false)), ("naga-B", b(60, true)), ("unknown-C", b(70, false))], &[]);
        for (row, id) in r.iter().zip(["logitech-A", "naga-B", "unknown-C"]) {
            assert!(!row.icon.is_empty());
            assert_eq!(row.icon, kind_of(id).icon);
        }
        // What the compact styles show survives in the row: the percent, the charging mark and the dimming.
        assert!(r[1].charging && r[1].percent == 60);
    }

    // Work area of 1920x1040 at the origin.
    const WORK: Rc = (0, 0, 1920, 1040);

    #[test]
    fn anchor_is_the_left_edge_and_the_vertical_edge_nearest_the_screen_edge() {
        let a = |l, t, w, h| anchor_of((l, t, l + w, t + h), WORK);
        // Near the right edge the anchor is still the left edge.
        let p = a(1600, 12, 300, 100);
        assert_eq!((p.x, p.y, p.right, p.bottom), (1600, 12, false, false));
        let p = a(20, 900, 300, 100);
        assert_eq!((p.x, p.y, p.right, p.bottom), (20, 1000, false, true));
        let p = a(5, 5, 300, 100);
        assert_eq!((p.x, p.y, p.right, p.bottom), (5, 5, false, false));
        // A plate across the middle: ties go to the top.
        let p = a(810, 470, 300, 100);
        assert!(!p.right && !p.bottom == (470 <= 1040 - 570));
    }

    #[test]
    fn plate_on_the_left_grows_right_keeping_x() {
        let pos = anchor_of((100, 12, 400, 112), WORK);
        assert_eq!(placed(pos, 300, 100, WORK), (100, 12));
        assert_eq!(placed(pos, 500, 100, WORK), (100, 12));
        assert_eq!(placed(pos, 200, 100, WORK), (100, 12));
    }

    #[test]
    fn plate_near_the_right_edge_grows_right_and_is_clamped_inside() {
        let pos = anchor_of((1600, 12, 1900, 112), WORK);
        assert_eq!(placed(pos, 300, 100, WORK), (1600, 12));
        // Wider: its left edge would stay, but the right would leave the work area: shifted left just enough.
        let (l, _) = placed(pos, 500, 100, WORK);
        assert_eq!((l, l + 500), (1420, 1920));
        // The saved x is kept, so it returns when the plate shrinks again.
        assert_eq!(placed(pos, 300, 100, WORK), (1600, 12));
        // Same on a monitor left of the origin.
        let work = (-1920, 0, 0, 1040);
        let pos = anchor_of((-400, 12, -100, 112), work);
        assert_eq!(placed(pos, 600, 100, work), (-600, 12));
    }

    #[test]
    fn older_right_anchored_position_becomes_left_anchored() {
        let old = OverlayPos { x: 1900, y: 12, right: true, bottom: false };
        let p = left_anchored(old, 300);
        assert_eq!((p.x, p.y, p.right, p.bottom), (1600, 12, false, false));
        assert_eq!(placed(p, 300, 100, WORK), (1600, 12));
        // A left-anchored position is unchanged.
        let q = OverlayPos { x: 20, y: 1000, right: false, bottom: true };
        assert_eq!(left_anchored(q, 300), q);
    }

    #[test]
    fn vertical_growth_is_away_from_the_nearest_edge() {
        let bottom_left = OverlayPos { x: 20, y: 1000, right: false, bottom: true };
        assert_eq!(origin(bottom_left, 100), (20, 900));
        assert_eq!(origin(bottom_left, 200), (20, 800));
        let top = OverlayPos { x: 20, y: 12, right: false, bottom: false };
        assert_eq!(origin(top, 200), (20, 12));
        // Same anchor after a re-anchor of the unchanged rectangle.
        let (l, t) = origin(bottom_left, 100);
        assert_eq!(anchor_of((l, t, l + 300, t + 100), WORK), bottom_left);
    }

    #[test]
    fn placement_is_clamped_into_the_work_area() {
        let at = |x, y, bottom, w, h| placed(OverlayPos { x, y, right: false, bottom }, w, h, WORK);
        assert_eq!(at(1600, 12, false, 300, 100), (1600, 12));
        // The plate is too wide for the room that is left on the right.
        assert_eq!(at(1800, 12, false, 300, 100), (1620, 12));
        // Anchored at the bottom, taller than the room above: pushed down from the top edge.
        assert_eq!(at(20, 50, true, 300, 100), (20, 0));
        // Off the screen on the left/top.
        assert_eq!(at(-50, -50, false, 300, 100), (0, 0));
        // Larger than the work area: the top-left corner stays visible.
        assert_eq!(at(1000, 500, true, 3000, 2000), (0, 0));
        // Work area not at the origin (second monitor on the left).
        let work = (-1920, 0, 0, 1040);
        assert_eq!(placed(OverlayPos { x: -300, y: 12, right: false, bottom: false }, 300, 100, work), (-300, 12));
    }

    #[test]
    fn default_is_top_right_and_probe_is_inside_the_monitor() {
        let d = default_pos(WORK, 12, 280);
        assert_eq!((d.x, d.y, d.right, d.bottom), (1628, 12, false, false));
        assert_eq!(placed(d, 280, 100, WORK), (1628, 12));
        // Then it grows right and is clamped to the margin-less edge.
        assert_eq!(placed(d, 400, 100, WORK), (1520, 12));
        // The anchor of a right/bottom plate sits on the exclusive edge: the probe steps inside.
        let p = probe(OverlayPos { x: 1920, y: 1040, right: true, bottom: true });
        assert_eq!((p.x, p.y), (1919, 1039));
        let p = probe(OverlayPos { x: 0, y: 0, right: false, bottom: false });
        assert_eq!((p.x, p.y), (0, 0));
    }

    #[test]
    fn anchor_then_place_is_stable_in_every_corner_and_on_negative_monitors() {
        // Primary, a monitor to the left (negative x), one above (negative y) and one up-left (both).
        for work in [WORK, (-1920, 0, 0, 1040), (0, -1080, 1920, 0), (-1600, -900, 0, -40)] {
            let (w, h) = (300, 100);
            let corners = [(work.0 + 10, work.1 + 10), (work.2 - w - 10, work.1 + 10), (work.0 + 10, work.3 - h - 10), (work.2 - w - 10, work.3 - h - 10)];
            for (i, (l, t)) in corners.into_iter().enumerate() {
                let pos = anchor_of((l, t, l + w, t + h), work);
                assert_eq!((pos.right, pos.bottom), (false, i >= 2), "{work:?} corner {i}");
                assert_eq!(placed(pos, w, h, work), (l, t), "{work:?} corner {i}");
                // A taller and wider plate grows (right, and away from the vertical anchor) and still fits.
                let (bl, bt) = placed(pos, w + 100, h + 100, work);
                assert!(bl >= work.0 && bl + w + 100 <= work.2 && bt >= work.1 && bt + h + 100 <= work.3, "{work:?} corner {i}");
                let p = probe(pos);
                assert!(p.x >= work.0 && p.x < work.2 && p.y >= work.1 && p.y < work.3, "{work:?} corner {i}");
            }
        }
    }

    #[test]
    fn dropped_plate_hanging_off_an_edge_is_pulled_in_before_saving() {
        let (w, h) = (300, 100);
        for work in [WORK, (-1920, 0, 0, 1040)] {
            // Half off each edge, then off two edges at once, then fully inside.
            let hangs = [
                (work.2 - w / 2, 200, true),
                (work.0 - w / 2, 200, true),
                (work.0 + 400, work.1 - h / 2, true),
                (work.0 + 400, work.3 - h / 2, true),
                (work.2 - w / 2, work.3 - h / 2, true),
                (work.0 + 10, work.1 + 10, false),
            ];
            for (l, t, moves) in hangs {
                let plate = (l, t, l + w, t + h);
                let inside = clamped(plate, work);
                assert_eq!(inside != plate, moves, "{work:?} {plate:?}");
                assert!(inside.0 >= work.0 && inside.2 <= work.2 && inside.1 >= work.1 && inside.3 <= work.3, "{work:?} {plate:?}");
                let p = probe(anchor_of(inside, work));
                assert!(p.x >= work.0 && p.x < work.2 && p.y >= work.1 && p.y < work.3, "{work:?} {plate:?}");
            }
        }
    }

    #[test]
    fn plate_dropped_over_the_taskbar_keeps_its_place_and_restores() {
        // Monitor 1920x1080 with a taskbar in the bottom 40 px (work area 1920x1040), or on the right (60 px).
        let monitor: Rc = (0, 0, 1920, 1080);
        let (w, h) = (300, 100);
        for plate in [(100, 960, 100 + w, 960 + h), (100, 980, 100 + w, 980 + h), (1620, 400, 1920, 400 + h), (1560, 0, 1560 + w, h)] {
            // The monitor rect keeps the plate where it was dropped (the work area would have moved it).
            assert_eq!(clamped(plate, monitor), plate, "{plate:?}");
            let pos = anchor_of(plate, monitor);
            // The probe is inside the monitor, so the saved place is found again, and it opens at the same spot.
            let p = probe(pos);
            assert!(p.x >= monitor.0 && p.x < monitor.2 && p.y >= monitor.1 && p.y < monitor.3, "{plate:?}");
            assert_eq!(placed(pos, w, h, monitor), (plate.0, plate.1), "{plate:?}");
        }
        // Dropped half below the monitor: pulled back to its bottom edge, not to the work area.
        assert_eq!(clamped((100, 1040, 400, 1140), monitor), (100, 980, 400, 1080));
        // The default place stays inside the work area.
        let d = default_pos(WORK, 12, w);
        assert!(placed(d, w, h, WORK).1 + h <= WORK.3);
    }
}
