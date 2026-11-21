//! Battery overlay: a small always-on-top plate with the charge of every device, shown and hidden
//! by a hotkey or the tray menu. It takes no focus and lets mouse clicks through. It shows the last
//! known levels and never asks a device (that would wake sleeping wireless ones).

use std::cell::RefCell;
use std::mem::zeroed;
use std::ptr::{null, null_mut};

use windows::core::w;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::config::OverlayStyle;
use crate::devices::{kind_of, Battery, CRITICAL_BATTERY, LOW_BATTERY};
use crate::draw::{cursor_monitor, dwm_set, hwnd_target, register_class, Paint};
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
/// Uniform window opacity, about 85%.
const ALPHA: u8 = 217;

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

/// The rows, in the order of `ids` (the panel order); devices without a battery are left out.
fn rows(ids: &[String], status: &[(String, Battery)], no_link: &[String], name: impl Fn(&str) -> String) -> Vec<Row> {
    ids.iter()
        .filter_map(|id| {
            let b = status.iter().find(|(s, _)| s == id)?.1;
            Some(Row { name: name(id), icon: kind_of(id).icon, percent: b.percent, charging: b.charging, level: level(b), asleep: no_link.contains(id) })
        })
        .collect()
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
    let cfg = crate::config();
    rows(&ids, &status, &no_link, |id| cfg.display_name(id))
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
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    fmt: Formats,
    theme: Theme,
    rows: Vec<Row>,
    style: OverlayStyle,
    /// Right edge and top of the plate on the screen (pixels), kept when it is resized.
    right: i32,
    top: i32,
}

thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

pub fn is_visible() -> bool {
    OVERLAY.with(|c| c.borrow().is_some())
}

/// Shows the plate, or hides it when it is shown.
pub fn toggle() {
    let shown = OVERLAY.with(|c| c.borrow().as_ref().map(|o| o.hwnd));
    match shown {
        Some(h) => unsafe {
            DestroyWindow(h);
        },
        None => {
            if let Err(e) = unsafe { open() } {
                crate::log(&format!("battery overlay: {e}"));
            }
        }
    }
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
                if resized {
                    // The render target is made for the window size.
                    o.rt = None;
                    o.brush = None;
                }
                (o.hwnd, resized, o.scale, o.right, o.top, size(style, o.rows.len()))
            })
        });
        if let Some((hwnd, resized, scale, right, top, dip)) = moved {
            if resized {
                place(hwnd, scale, right, top, dip);
            }
            InvalidateRect(hwnd, null(), 0);
        }
    }
}

/// Puts the plate with its right edge at `right` and its top at `top`, sized `dip` (width, height in DIPs).
unsafe fn place(hwnd: HWND, scale: f32, right: i32, top: i32, dip: (f32, f32)) {
    let (w, h) = ((dip.0 * scale).round() as i32, (dip.1 * scale).round() as i32);
    SetWindowPos(hwnd, HWND_TOPMOST, right - w, top, w, h, SWP_NOACTIVATE);
}

unsafe fn open() -> windows::core::Result<()> {
    let hinst = GetModuleHandleW(null());
    let class: Vec<u16> = "RgbTrayBatteryOverlay\0".encode_utf16().collect();
    register_class(&class, 0, wndproc);

    // The monitor under the cursor now; it does not follow while the plate is up.
    let (_, mi, dpi) = cursor_monitor(MONITOR_DEFAULTTOPRIMARY);
    let scale = dpi as f32 / 96.0;
    let margin = (12.0 * scale).round() as i32;
    let (right, top) = (mi.rcWork.right - margin, mi.rcWork.top + margin);
    let rows = current_rows();
    let style = crate::config().overlay_style;
    let dip = size(style, rows.len());
    let (w, h) = ((dip.0 * scale).round() as i32, (dip.1 * scale).round() as i32);

    // What can fail comes before the window: a window without its state would be stuck.
    let d2d = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
    let fmt = formats()?;
    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
        class.as_ptr(),
        class.as_ptr(),
        WS_POPUP,
        right - w,
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
    SetLayeredWindowAttributes(hwnd, 0, ALPHA, LWA_ALPHA);
    let theme = theme();
    dwm_set(hwnd, 33, 2); // DWMWCP_ROUND
    dwm_set(hwnd, 20, theme.dark as u32);

    let overlay = Overlay { hwnd, scale, d2d, rt: None, brush: None, fmt, theme, rows, style, right, top };
    OVERLAY.with(|c| *c.borrow_mut() = Some(overlay));
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
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
        let (w, h) = size(self.style, self.rows.len());
        rt.Clear(Some(&t.bg));
        self.stroke(rt, R { x: 0.0, y: 0.0, w, h }, 0.0, t.stroke, 1.0);
        if self.rows.is_empty() {
            let line = tr!("Нет устройств с батареей", "No devices with a battery");
            let (pad, line_h) = if self.style == OverlayStyle::Full { (PAD, ROW_H) } else { (C_PAD, C_ROW_H) };
            self.text(rt, line, &self.fmt.name, R { x: pad, y: VPAD, w: w - 2.0 * pad, h: line_h }, t.text2);
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
        self.text(rt, row.icon, &self.fmt.glyph, R { x: PAD, y: y + 2.0, w: ICON_W - 6.0, h: 29.0 }, dim(self.value_color(row)));
        let left = PAD + ICON_W;
        let name_w = W - PAD - left - RIGHT_W - 8.0;
        self.text(rt, &fit(&row.name, &self.fmt.name, name_w), &self.fmt.name, R { x: left, y: y + 2.0, w: name_w, h: 20.0 }, dim(t.text));

        let right = R { x: W - PAD - RIGHT_W, y: y + 2.0, w: RIGHT_W, h: 20.0 };
        let percent = if row.asleep { format!("{} · {}%", tr!("спит", "asleep"), row.percent) } else { format!("{}%", row.percent) };
        self.text(rt, &percent, &self.fmt.percent, right, dim(self.value_color(row)));
        if row.charging && !row.asleep {
            // The charging mark sits left of the percent.
            let mark = R { x: right.x, w: right.w - text_width(&percent, &self.fmt.percent) - 4.0, ..right };
            self.text(rt, "\u{E83E}", &self.fmt.icon, mark, dim(accent));
        }

        let bar = R { x: left, y: y + 27.0, w: W - PAD - left, h: 4.0 };
        self.fill(rt, bar, 2.0, D2D1_COLOR_F { a: 0.22, ..t.text2 });
        let filled = (bar.w * row.percent.min(100) as f32 / 100.0).max(bar.h);
        self.fill(rt, R { w: filled, ..bar }, 2.0, dim(accent));
    }

    /// A compact cell at (`x`, `y`): icon, percent and the charging mark; no name, no bar.
    unsafe fn cell(&self, rt: &ID2D1RenderTarget, row: &Row, x: f32, y: f32) {
        let dim = |c| Self::dim(row, c);
        let color = dim(self.value_color(row));
        self.text(rt, row.icon, &self.fmt.glyph, R { x, y, w: C_ICON_W, h: C_ROW_H }, color);
        self.text(rt, &format!("{}%", row.percent), &self.fmt.percent, R { x: x + C_ICON_W, y, w: C_PCT_W, h: C_ROW_H }, color);
        if row.charging && !row.asleep {
            self.text(
                rt,
                "\u{E83E}",
                &self.fmt.icon,
                R { x: x + C_ICON_W + C_PCT_W + 4.0, y, w: C_MARK_W, h: C_ROW_H },
                dim(level_color(row.level, &self.theme)),
            );
        }
    }
}

/// Renders the plate with the current levels into a PNG file, for checking the look. The style is the
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
            rt: None,
            brush: Some(rt.CreateSolidColorBrush(&theme.text, None)?),
            fmt: formats()?,
            theme,
            rows,
            style,
            right: 0,
            top: 0,
        };
        o.draw(rt);
        Ok(())
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            BeginPaint(hwnd, &mut ps);
            OVERLAY.with(|c| c.borrow_mut().as_mut().map(|o| o.paint()));
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        // Mouse clicks go to the window under the plate; it is never activated.
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_DESTROY => {
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

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn b(percent: u8, charging: bool) -> Battery {
        Battery { percent, charging }
    }

    fn run(order: &[&str], status: &[(&str, Battery)], no_link: &[&str]) -> Vec<Row> {
        let status: Vec<(String, Battery)> = status.iter().map(|(i, b)| (i.to_string(), *b)).collect();
        rows(&ids(order), &status, &ids(no_link), |id| format!("name-{id}"))
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
    fn no_devices_gives_no_rows() {
        assert!(run(&[], &[("a", b(50, false))], &[]).is_empty());
        assert!(run(&["a"], &[], &[]).is_empty());
    }

    #[test]
    fn asleep_low_device_keeps_its_level() {
        let r = run(&["a"], &[("a", b(4, false))], &["a"]);
        assert!(r[0].asleep && r[0].level == Level::Critical && r[0].percent == 4);
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
}
