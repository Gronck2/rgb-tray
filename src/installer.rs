//! Per-user installer, updater and uninstaller built into the app itself.
//!
//! The same exe, started as `RGBTray-Setup-<version>.exe` (or with `--install`), installs
//! itself into %LOCALAPPDATA%\Programs\RGB Tray, replacing and restarting a running older
//! version. Settings live in %APPDATA%\rgbtray and are kept across updates.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::HWND as WinHwnd;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::devices::{Effect, Rgb};
use crate::ui::{col, text_format, theme, Theme, R};
use crate::wide;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const DISPLAY_NAME: &str = "RGB Tray";
const EXE_NAME: &str = "rgbtray.exe";
const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\rgbtray";

fn install_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Programs").join(DISPLAY_NAME)
}

fn installed_exe() -> PathBuf {
    install_dir().join(EXE_NAME)
}

fn start_menu_link() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Microsoft\\Windows\\Start Menu\\Programs").join(format!("{DISPLAY_NAME}.lnk"))
}

/// Whether this exe was started as an installer (by file name or flag).
pub fn is_setup(args: &[String]) -> bool {
    let name = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase())).unwrap_or_default();
    name.starts_with("rgbtray-setup") || args.iter().any(|a| a == "--install")
}

fn read_reg_string(key: &str, value: &str) -> Option<String> {
    unsafe {
        let (k, v) = (wide(key), wide(value));
        let mut size = 0u32;
        if RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut size) != 0 {
            return None;
        }
        let mut buf = vec![0u16; size as usize / 2 + 1];
        if RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr() as _, &mut size) != 0 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

fn set_reg_string(key: &str, value: &str, data: &str) {
    let d = wide(data);
    unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(value).as_ptr(), REG_SZ, d.as_ptr() as _, (d.len() * 2) as u32);
    }
}

fn set_reg_dword(key: &str, value: &str, data: u32) {
    unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(value).as_ptr(), REG_DWORD, &data as *const u32 as _, 4);
    }
}

fn installed_version() -> Option<String> {
    installed_exe().exists().then(|| read_reg_string(UNINSTALL_KEY, "DisplayVersion").unwrap_or_else(|| "?".into()))
}

// ---------------------------------------------------------------- icon file

/// Pixels (top-down, premultiplied BGRA) of the app icon: a rainbow ring around a dark core.
fn icon_pixels(size: u32) -> Vec<u32> {
    let s = size as f32;
    let c = (s - 1.0) / 2.0;
    let (outer, inner) = (s * 0.47, s * 0.27);
    let mut px = vec![0u32; (size * size) as usize];
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let d = (dx * dx + dy * dy).sqrt();
            let alpha = (outer + 0.5 - d).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            let Rgb(r, g, b) = if d < inner { Rgb(0x1C, 0x1C, 0x1C) } else { crate::hue(dy.atan2(dx).to_degrees() + 180.0) };
            let a = (alpha * 255.0) as u32;
            let pm = |v: u8| v as u32 * a / 255;
            px[(y * size + x) as usize] = (a << 24) | (pm(r) << 16) | (pm(g) << 8) | pm(b);
        }
    }
    px
}

/// Writes a multi-size .ico (32-bit BMP entries) for shortcuts and the Apps list.
fn write_icon(path: &Path) -> std::io::Result<()> {
    let sizes = [16u32, 24, 32, 48, 64, 256];
    let mut images = Vec::new();
    for &s in &sizes {
        let px = icon_pixels(s);
        let mask_row = s.div_ceil(32) * 4;
        let mut img = Vec::new();
        // BITMAPINFOHEADER; height counts the color and the mask bitmaps.
        for v in [40u32, s, s * 2] {
            img.extend_from_slice(&v.to_le_bytes());
        }
        img.extend_from_slice(&1u16.to_le_bytes());
        img.extend_from_slice(&32u16.to_le_bytes());
        for _ in 0..6 {
            img.extend_from_slice(&0u32.to_le_bytes());
        }
        // Bottom-up rows, straight (not premultiplied) alpha.
        for y in (0..s).rev() {
            for x in 0..s {
                let p = px[(y * s + x) as usize];
                let a = p >> 24;
                let un = |v: u32| (v * 255).checked_div(a).map_or(0, |x| x.min(255));
                let (r, g, b) = (un((p >> 16) & 0xFF), un((p >> 8) & 0xFF), un(p & 0xFF));
                img.extend_from_slice(&[b as u8, g as u8, r as u8, a as u8]);
            }
        }
        img.extend(std::iter::repeat_n(0u8, (mask_row * s) as usize));
        images.push((s, img));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (s, img) in &images {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for (_, img) in &images {
        out.extend_from_slice(img);
    }
    std::fs::write(path, out)
}

// ---------------------------------------------------------------- install steps

/// Asks a running instance to quit and waits until its exe can be replaced.
fn stop_running_instance() {
    unsafe {
        let class = wide("RgbTrayWindow");
        let hwnd = FindWindowW(class.as_ptr(), null());
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !FindWindowW(class.as_ptr(), null()).is_null() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
            // Give the process a moment to exit after its window is gone.
            std::thread::sleep(Duration::from_millis(400));
        }
    }
}

fn copy_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match std::fs::copy(from, to) {
            Ok(_) => return Ok(()),
            Err(e) if Instant::now() < deadline => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(e),
        }
    }
}

fn create_shortcut(link: &Path, target: &Path, icon: &Path) -> windows::core::Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let t = wide(&target.display().to_string());
        let dir = wide(&target.parent().unwrap_or(Path::new(".")).display().to_string());
        let ic = wide(&icon.display().to_string());
        let desc = wide(tr!("Подсветка клавиатуры, мыши и ноутбука", "Keyboard, mouse and laptop lighting"));
        sl.SetPath(PCWSTR(t.as_ptr()))?;
        sl.SetWorkingDirectory(PCWSTR(dir.as_ptr()))?;
        sl.SetIconLocation(PCWSTR(ic.as_ptr()), 0)?;
        sl.SetDescription(PCWSTR(desc.as_ptr()))?;
        let pf: IPersistFile = sl.cast()?;
        let l = wide(&link.display().to_string());
        pf.Save(PCWSTR(l.as_ptr()), true)?;
    }
    Ok(())
}

pub struct Options {
    pub autostart: bool,
    pub shortcut: bool,
}

/// Installs or updates the app and starts it. Returns a user-facing error message on failure.
fn install(opts: &Options) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = install_dir();
    let exe = installed_exe();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{} {e}", tr!("Не удалось создать папку:", "Could not create the folder:")))?;
    stop_running_instance();
    if me != exe {
        copy_with_retry(&me, &exe).map_err(|e| {
            format!("{} {e}", tr!("Не удалось скопировать файл (приложение ещё запущено?):", "Could not copy the file (is the app still running?):"))
        })?;
    }
    let icon = dir.join("rgbtray.ico");
    let _ = write_icon(&icon);

    let link = start_menu_link();
    if opts.shortcut {
        if let Err(e) = create_shortcut(&link, &exe, &icon) {
            crate::log(&format!("installer: shortcut: {e}"));
        }
    } else {
        let _ = std::fs::remove_file(&link);
    }
    crate::set_autostart_for(opts.autostart.then_some(exe.as_path()));

    // "Apps" entry in Windows Settings, with uninstall.
    let k = UNINSTALL_KEY;
    set_reg_string(k, "DisplayName", DISPLAY_NAME);
    set_reg_string(k, "DisplayVersion", VERSION);
    set_reg_string(k, "Publisher", DISPLAY_NAME);
    set_reg_string(k, "DisplayIcon", &icon.display().to_string());
    set_reg_string(k, "InstallLocation", &dir.display().to_string());
    set_reg_string(k, "UninstallString", &format!("\"{}\" --uninstall", exe.display()));
    set_reg_dword(k, "NoModify", 1);
    set_reg_dword(k, "NoRepair", 1);
    let size_kb = std::fs::metadata(&exe).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
    set_reg_dword(k, "EstimatedSize", size_kb);

    std::process::Command::new(&exe)
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("{} {e}", tr!("Не удалось запустить:", "Could not start:")))?;
    Ok(())
}

/// `--uninstall`: removes the app (settings in %APPDATA%\rgbtray are kept).
pub fn uninstall() {
    unsafe {
        let text = wide(tr!(
            "Удалить RGB Tray?\n\nНастройки и пресеты останутся в %APPDATA%\\rgbtray и вернутся при повторной установке.",
            "Uninstall RGB Tray?\n\nSettings and presets stay in %APPDATA%\\rgbtray and come back if you install it again."
        ));
        let caption = wide(DISPLAY_NAME);
        if MessageBoxW(null_mut(), text.as_ptr(), caption.as_ptr(), MB_YESNO | MB_ICONQUESTION) != IDYES {
            return;
        }
    }
    stop_running_instance();
    crate::set_autostart_for(None);
    let _ = std::fs::remove_file(start_menu_link());
    unsafe {
        RegDeleteTreeW(HKEY_CURRENT_USER, wide(UNINSTALL_KEY).as_ptr());
    }
    // The running exe can't delete itself: a hidden cmd removes the folder after we exit.
    let dir = install_dir();
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("cmd")
        .args(["/c", &format!("ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{}\"", dir.display())])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

// ---------------------------------------------------------------- setup window

const W: f32 = 440.0;
const H: f32 = 268.0;
const PAD: f32 = 24.0;

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Autostart,
    Shortcut,
    Cancel,
    Install,
}

#[derive(Clone, PartialEq)]
enum Stage {
    Ready,
    Done,
    Failed(String),
}

struct Setup {
    hwnd: HWND,
    scale: f32,
    d2d: ID2D1Factory,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    title: IDWriteTextFormat,
    body: IDWriteTextFormat,
    button: IDWriteTextFormat,
    icon_font: IDWriteTextFormat,
    theme: Theme,
    opts: Options,
    previous: Option<String>,
    stage: Stage,
    hover: Option<Hit>,
}

thread_local! {
    static SETUP: RefCell<Option<Setup>> = const { RefCell::new(None) };
}

fn hits(stage: &Stage) -> Vec<(R, Hit)> {
    let btn_y = H - PAD - 34.0;
    let install = R { x: W - PAD - 130.0, y: btn_y, w: 130.0, h: 34.0 };
    let cancel = R { x: install.x - 8.0 - 100.0, y: btn_y, w: 100.0, h: 34.0 };
    match stage {
        Stage::Ready => vec![
            (R { x: PAD, y: 128.0, w: 300.0, h: 26.0 }, Hit::Autostart),
            (R { x: PAD, y: 158.0, w: 300.0, h: 26.0 }, Hit::Shortcut),
            (cancel, Hit::Cancel),
            (install, Hit::Install),
        ],
        // After installing only one "close" button remains.
        _ => vec![(install, Hit::Cancel)],
    }
}

/// Runs the installer: a window, or straight install with `--quiet`.
pub fn run(args: &[String]) {
    let previous = installed_version();
    let opts = Options {
        // Updates keep the current choice; a fresh install starts with Windows by default.
        autostart: previous.is_none() || crate::autostart_enabled(),
        shortcut: previous.is_none() || start_menu_link().exists(),
    };
    if args.iter().any(|a| a == "--quiet") {
        if let Err(e) = install(&opts) {
            crate::log(&format!("installer: {e}"));
        }
        return;
    }
    unsafe {
        if let Err(e) = open(opts, previous) {
            crate::log(&format!("installer window: {e}"));
            return;
        }
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn open(opts: Options, previous: Option<String>) -> windows::core::Result<()> {
    let hinst = GetModuleHandleW(null());
    let class = wide("RgbTraySetup");
    let mut wc: WNDCLASSW = zeroed();
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinst;
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.hIcon = crate::make_icon(Effect::Cycle, Rgb(0, 0, 0), false);
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
    let x = work.left + ((work.right - work.left) - w) / 2;
    let y = work.top + ((work.bottom - work.top) - h) / 2;
    let title = wide(&format!("{} {DISPLAY_NAME} {VERSION}", tr!("Установка", "Setup")));
    let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), style, x, y, w, h, null_mut(), null_mut(), hinst, null());

    let theme = theme();
    let dark: u32 = theme.dark as u32;
    DwmSetWindowAttribute(hwnd, 20, &dark as *const u32 as _, 4);
    let bg = theme.bg;
    let caption: u32 = (bg.r * 255.0) as u32 | ((bg.g * 255.0) as u32) << 8 | ((bg.b * 255.0) as u32) << 16;
    DwmSetWindowAttribute(hwnd, 35, &caption as *const u32 as _, 4);

    let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
    let ui = w!("Segoe UI Variable Text");
    let body = text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?;
    body.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
    body.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
    let setup = Setup {
        hwnd,
        scale,
        d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
        rt: None,
        brush: None,
        title: text_format(&dw, w!("Segoe UI Variable Display"), 20.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        body,
        button: text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        icon_font: text_format(&dw, w!("Segoe Fluent Icons"), 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        theme,
        opts,
        previous,
        stage: Stage::Ready,
        hover: None,
    };
    SETUP.with(|c| *c.borrow_mut() = Some(setup));
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    Ok(())
}

impl Setup {
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

    unsafe fn stroke(&self, rt: &ID2D1RenderTarget, r: R, radius: f32, c: D2D1_COLOR_F) {
        let b = self.brush.as_ref().unwrap();
        b.SetColor(&c);
        rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.inset(0.5).d2d(), radiusX: radius, radiusY: radius }, b, 1.0, None);
    }

    unsafe fn text(&self, rt: &ID2D1RenderTarget, s: &str, fmt: &IDWriteTextFormat, r: R, c: D2D1_COLOR_F) {
        let b = self.brush.as_ref().unwrap();
        b.SetColor(&c);
        let s: Vec<u16> = s.encode_utf16().collect();
        rt.DrawText(&s, fmt, &r.d2d(), b, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
    }

    /// The app icon (rainbow ring), drawn as small segments.
    unsafe fn logo(&self, rt: &ID2D1RenderTarget, cx: f32, cy: f32, radius: f32) {
        let b = self.brush.as_ref().unwrap();
        let steps = 72;
        for i in 0..steps {
            let a = i as f32 / steps as f32 * std::f32::consts::TAU;
            let Rgb(r, g, bb) = crate::hue(a.to_degrees() + 180.0);
            b.SetColor(&col(r, g, bb, 1.0));
            let (x, y) = (cx + a.cos() * radius * 0.74, cy + a.sin() * radius * 0.74);
            rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x, y }, radiusX: radius * 0.26, radiusY: radius * 0.26 }, b);
        }
        b.SetColor(&col(0x1C, 0x1C, 0x1C, 1.0));
        rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: radius * 0.52, radiusY: radius * 0.52 }, b);
    }

    unsafe fn checkbox(&self, rt: &ID2D1RenderTarget, r: R, on: bool, label: &str, hovered: bool) {
        let t = &self.theme;
        let bx = R { x: r.x, y: r.y + 3.0, w: 20.0, h: 20.0 };
        if on {
            self.fill(rt, bx, 4.0, t.accent);
            self.text(rt, "\u{E73E}", &self.icon_font, bx, t.on_accent);
        } else {
            self.fill(rt, bx, 4.0, if hovered { t.card_hover } else { t.card });
            self.stroke(rt, bx, 4.0, t.text2);
        }
        self.text(rt, label, &self.body, R { x: r.x + 30.0, y: r.y + 4.0, w: r.w - 30.0, h: 20.0 }, t.text);
    }

    unsafe fn draw(&self, rt: &ID2D1RenderTarget) {
        let t = &self.theme;
        rt.Clear(Some(&t.bg));
        self.logo(rt, PAD + 24.0, PAD + 24.0, 24.0);
        self.text(rt, &format!("{DISPLAY_NAME} {VERSION}"), &self.title, R { x: PAD + 64.0, y: PAD, w: 300.0, h: 28.0 }, t.text);
        let subtitle = match &self.previous {
            Some(v) if v == VERSION => tr!("Переустановка той же версии", "Reinstalling the same version").to_string(),
            Some(v) => format!("{} {v}", tr!("Обновление с версии", "Updating from version")),
            None => tr!("Подсветка клавиатуры, мыши и ноутбука", "Keyboard, mouse and laptop lighting").to_string(),
        };
        self.text(rt, &subtitle, &self.body, R { x: PAD + 64.0, y: PAD + 28.0, w: 320.0, h: 20.0 }, t.text2);

        let info = R { x: PAD, y: 84.0, w: W - 2.0 * PAD, h: 40.0 };
        match &self.stage {
            Stage::Ready => {
                let text = if self.previous.is_some() {
                    tr!(
                        "Работающая версия будет закрыта и заменена. Настройки, пресеты и цвета клавиш сохранятся.",
                        "The running version will be closed and replaced. Settings, presets and key colors are kept."
                    )
                    .to_string()
                } else {
                    format!("{} {}", tr!("Будет установлено в", "Installs to"), install_dir().display())
                };
                self.text(rt, &text, &self.body, info, t.text2);
                let hs = hits(&self.stage);
                self.checkbox(rt, hs[0].0, self.opts.autostart, tr!("Запускать вместе с Windows", "Start with Windows"), self.hover == Some(Hit::Autostart));
                self.checkbox(rt, hs[1].0, self.opts.shortcut, tr!("Ярлык в меню «Пуск»", "Start menu shortcut"), self.hover == Some(Hit::Shortcut));
            }
            Stage::Done => {
                self.text(
                    rt,
                    tr!("Готово! Приложение запущено — значок уже в трее.", "Done! The app is running — its icon is in the tray."),
                    &self.body,
                    info,
                    t.text,
                );
            }
            Stage::Failed(e) => {
                self.text(rt, &format!("{} {e}", tr!("Не получилось:", "Failed:")), &self.body, R { h: 90.0, ..info }, t.text);
            }
        }

        for (r, hit) in hits(&self.stage) {
            let hovered = self.hover == Some(hit);
            match (hit, &self.stage) {
                (Hit::Install, _) => {
                    let label = if self.previous.is_some() { tr!("Обновить", "Update") } else { tr!("Установить", "Install") };
                    self.fill(rt, r, 4.0, if hovered { D2D1_COLOR_F { a: 0.85, ..t.accent } } else { t.accent });
                    self.text(rt, label, &self.button, r, t.on_accent);
                }
                (Hit::Cancel, Stage::Ready) => {
                    self.fill(rt, r, 4.0, if hovered { t.card_hover } else { t.card });
                    self.stroke(rt, r, 4.0, t.stroke);
                    self.text(rt, tr!("Отмена", "Cancel"), &self.button, r, t.text);
                }
                (Hit::Cancel, _) => {
                    self.fill(rt, r, 4.0, if hovered { D2D1_COLOR_F { a: 0.85, ..t.accent } } else { t.accent });
                    self.text(rt, tr!("Закрыть", "Close"), &self.button, r, t.on_accent);
                }
                _ => {}
            }
        }
    }
}

/// Renders the setup window into a PNG file, for checking the look.
pub fn snapshot(path: &str, dark: Option<bool>, scale: f32, update: bool) -> windows::core::Result<()> {
    crate::ui::render_png(path, W, H, scale, |rt, d2d| unsafe {
        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let ui = w!("Segoe UI Variable Text");
        let body = text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?;
        body.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
        body.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
        let theme = crate::ui::theme_for(dark);
        let s = Setup {
            hwnd: null_mut(),
            scale,
            d2d,
            rt: None,
            brush: Some(rt.CreateSolidColorBrush(&theme.text, None)?),
            title: text_format(&dw, w!("Segoe UI Variable Display"), 20.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?,
            body,
            button: text_format(&dw, ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
            icon_font: text_format(&dw, w!("Segoe Fluent Icons"), 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
            theme,
            opts: Options { autostart: true, shortcut: true },
            previous: update.then(|| "0.1.0".to_string()),
            stage: Stage::Ready,
            hover: Some(Hit::Install),
        };
        s.draw(rt);
        Ok(())
    })
}

fn hit_at(lparam: LPARAM, scale: f32) -> Option<Hit> {
    let x = (lparam & 0xFFFF) as i16 as f32 / scale;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as f32 / scale;
    let stage = SETUP.with(|c| c.borrow().as_ref().map(|s| s.stage.clone()))?;
    hits(&stage).into_iter().find(|(r, _)| r.contains(x, y)).map(|(_, h)| h)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            BeginPaint(hwnd, &mut ps);
            SETUP.with(|c| c.borrow_mut().as_mut().map(|s| s.paint()));
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            let scale = SETUP.with(|c| c.borrow().as_ref().map(|s| s.scale)).unwrap_or(1.0);
            let h = hit_at(lparam, scale);
            let changed = SETUP.with(|c| {
                c.borrow_mut().as_mut().map(|s| {
                    let changed = s.hover != h;
                    s.hover = h;
                    changed
                })
            });
            if changed == Some(true) {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            let scale = SETUP.with(|c| c.borrow().as_ref().map(|s| s.scale)).unwrap_or(1.0);
            match hit_at(lparam, scale) {
                Some(Hit::Autostart) => {
                    SETUP.with(|c| c.borrow_mut().as_mut().map(|s| s.opts.autostart = !s.opts.autostart));
                }
                Some(Hit::Shortcut) => {
                    SETUP.with(|c| c.borrow_mut().as_mut().map(|s| s.opts.shortcut = !s.opts.shortcut));
                }
                Some(Hit::Cancel) => {
                    DestroyWindow(hwnd);
                    return 0;
                }
                Some(Hit::Install) => {
                    let opts = SETUP.with(|c| c.borrow().as_ref().map(|s| Options { autostart: s.opts.autostart, shortcut: s.opts.shortcut }));
                    if let Some(opts) = opts {
                        SetCursor(LoadCursorW(null_mut(), IDC_WAIT));
                        let stage = match install(&opts) {
                            Ok(()) => Stage::Done,
                            Err(e) => Stage::Failed(e),
                        };
                        SETUP.with(|c| c.borrow_mut().as_mut().map(|s| s.stage = stage));
                    }
                }
                None => {}
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_DESTROY => {
            let s = SETUP.with(|c| c.borrow_mut().take());
            drop(s);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
