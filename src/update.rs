//! Update check: asks GitHub for the latest published release once a day; a newer one shows
//! a card on the panel, whose button downloads the installer and runs it quietly (the
//! installer closes this app, replaces the exe and starts the new one).

use serde::Deserialize;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::time::Duration;
use windows_sys::Win32::Networking::WinHttp::*;

const API_URL: &str = "https://api.github.com/repos/Gronck2/rgb-tray/releases/latest";
const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

#[derive(Clone, PartialEq, Debug)]
pub struct Release {
    /// "0.7.0", without the "v" of the tag.
    pub version: String,
    /// The release page, for when the download fails.
    pub page: String,
    /// The installer: download URL and size in bytes.
    setup: Option<(String, u64)>,
}

impl Release {
    /// The release has an installer to download (otherwise only its page can be opened).
    pub fn has_setup(&self) -> bool {
        self.setup.is_some()
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Idle,
    Available(Release),
    Downloading(Release),
    Failed(Release),
}

static STATE: Mutex<State> = Mutex::new(State::Idle);

pub fn state() -> State {
    STATE.lock().unwrap().clone()
}

fn set_state(s: State) {
    *STATE.lock().unwrap() = s;
    crate::post(crate::WM_APPLIED);
}

/// The release the panel offers: a newer one the user didn't skip, while checking is on.
pub fn offered() -> Option<State> {
    let s = state();
    let cfg = crate::config();
    match &s {
        State::Idle => None,
        State::Available(r) | State::Downloading(r) | State::Failed(r) => {
            (cfg.check_updates && cfg.skipped_version.as_deref() != Some(r.version.as_str())).then_some(s)
        }
    }
}

/// `RGBTRAY_DEMO_UPDATE=0.7.0[,downloading|failed|nosetup]`: a made-up release for snapshots.
pub fn demo(spec: &str) {
    let mut parts = spec.split(',');
    let version = parts.next().unwrap_or("9.9.9").to_string();
    let kind = parts.next().unwrap_or("");
    let setup = (kind != "nosetup").then(|| (String::new(), 0));
    let r = Release { version, page: String::new(), setup };
    *STATE.lock().unwrap() = match kind {
        "downloading" => State::Downloading(r),
        "failed" => State::Failed(r),
        _ => State::Available(r),
    };
}

/// Checks a minute after start (not to slow it down), then once a day.
pub fn spawn_checker() {
    std::thread::spawn(|| {
        std::thread::sleep(FIRST_CHECK);
        loop {
            check_now();
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

/// Asks GitHub for the latest release (when checking is on); errors only go to the log.
pub fn check_now() {
    if !crate::config().check_updates || matches!(state(), State::Downloading(_)) {
        return;
    }
    match get(API_URL).and_then(|body| parse_release(&body)) {
        Ok(r) if is_newer(&r.version, env!("CARGO_PKG_VERSION")) => {
            crate::log(&format!("update: {} available", r.version));
            set_state(State::Available(r));
        }
        Ok(_) => {}
        Err(e) => crate::log(&format!("update check: {e}")),
    }
}

/// Downloads the installer of the offered release and runs it quietly.
pub fn install() {
    let Some(State::Available(r) | State::Failed(r)) = offered() else { return };
    set_state(State::Downloading(r.clone()));
    std::thread::spawn(move || {
        let res = download_and_run(&r);
        if let Err(e) = res {
            crate::log(&format!("update: {e}"));
            set_state(State::Failed(r));
        }
    });
}

fn download_and_run(r: &Release) -> Result<(), String> {
    let (url, size) = r.setup.as_ref().ok_or("the release has no installer")?;
    let data = get(url)?;
    if data.len() as u64 != *size {
        return Err(format!("downloaded {} bytes of {size}", data.len()));
    }
    // The name makes the exe start as the installer.
    let path = std::env::temp_dir().join(format!("RGBTray-Setup-{}.exe", r.version));
    std::fs::write(&path, &data).map_err(|e| format!("{}: {e}", path.display()))?;
    crate::log(&format!("update: running {}", path.display()));
    std::process::Command::new(&path).arg("--quiet").spawn().map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// Opens the release page in the browser (when the download failed).
pub fn open_page() {
    if let Some(State::Available(r) | State::Downloading(r) | State::Failed(r)) = offered() {
        crate::open_url(&r.page);
    }
}

/// Never offers this version again (a newer one is offered as usual).
pub fn skip() {
    if let Some(State::Available(r) | State::Failed(r)) = offered() {
        let mut cfg = crate::config();
        cfg.skipped_version = Some(r.version);
        cfg.save();
        drop(cfg);
        crate::post(crate::WM_APPLIED);
    }
}

// ---------------------------------------------------------------- GitHub release

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

fn parse_release(body: &[u8]) -> Result<Release, String> {
    let r: ApiRelease = serde_json::from_slice(body).map_err(|e| format!("release JSON: {e}"))?;
    let setup = r
        .assets
        .into_iter()
        .find(|a| a.name.to_lowercase().starts_with("rgbtray-setup") && a.name.to_lowercase().ends_with(".exe"))
        .map(|a| (a.browser_download_url, a.size));
    Ok(Release { version: r.tag_name.trim_start_matches('v').to_string(), page: r.html_url, setup })
}

/// "1.2.3" as numbers; anything after the numbers ("-beta") is ignored.
fn version_parts(v: &str) -> Vec<u32> {
    v.trim_start_matches('v').split('.').map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)).collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    let (mut a, mut b) = (version_parts(latest), version_parts(current));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

// ---------------------------------------------------------------- HTTPS GET (WinHTTP)

/// Closes a WinHTTP handle when dropped.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Body of an HTTPS GET; redirects are followed (GitHub sends downloads elsewhere), the
/// system proxy is used.
fn get(url: &str) -> Result<Vec<u8>, String> {
    let rest = url.strip_prefix("https://").ok_or("not an https URL")?;
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let path = if path.is_empty() { "/" } else { path };
    let agent = format!("rgbtray/{}", env!("CARGO_PKG_VERSION"));
    unsafe {
        let session = Handle(WinHttpOpen(wide(&agent).as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
        if session.0.is_null() {
            return Err("WinHttpOpen failed".into());
        }
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 30_000, 60_000);
        let connect = Handle(WinHttpConnect(session.0, wide(host).as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if connect.0.is_null() {
            return Err(format!("cannot connect to {host}"));
        }
        let request = Handle(WinHttpOpenRequest(connect.0, wide("GET").as_ptr(), wide(path).as_ptr(), null(), null(), null(), WINHTTP_FLAG_SECURE));
        if request.0.is_null() {
            return Err("WinHttpOpenRequest failed".into());
        }
        let headers = wide("Accept: application/vnd.github+json, application/octet-stream\r\n");
        if WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, null(), 0, 0, 0) == 0 || WinHttpReceiveResponse(request.0, null_mut()) == 0 {
            return Err(format!("request to {host} failed (error {})", windows_sys::Win32::Foundation::GetLastError()));
        }
        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, null(), &mut status as *mut u32 as _, &mut len, null_mut());
        if status != 200 {
            return Err(format!("HTTP {status} from {host}"));
        }
        let mut body = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            if WinHttpReadData(request.0, buf.as_mut_ptr() as _, buf.len() as u32, &mut read) == 0 {
                return Err("download interrupted".into());
            }
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions() {
        assert!(is_newer("0.6.13", "0.6.12"));
        assert!(is_newer("v0.7.0", "0.6.12"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(is_newer("0.6.12.1", "0.6.12"));
        assert!(!is_newer("0.6.12", "0.6.12"));
        assert!(!is_newer("v0.6.9", "0.6.12"));
        assert!(!is_newer("0.6.12-beta", "0.6.12"));
    }

    #[test]
    fn release_from_github_json() {
        let json = br#"{
            "tag_name": "v0.7.0",
            "html_url": "https://github.com/Gronck2/rgb-tray/releases/tag/v0.7.0",
            "draft": false,
            "assets": [
                {"name": "notes.txt", "browser_download_url": "https://x/notes.txt", "size": 10},
                {"name": "RGBTray-Setup-0.7.0.exe", "browser_download_url": "https://x/setup.exe", "size": 1234567}
            ]
        }"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, "0.7.0");
        assert_eq!(r.page, "https://github.com/Gronck2/rgb-tray/releases/tag/v0.7.0");
        assert_eq!(r.setup, Some(("https://x/setup.exe".to_string(), 1234567)));
        // A release without an installer is still offered, with its page.
        let r = parse_release(br#"{"tag_name": "v0.7.1", "html_url": "https://p"}"#).unwrap();
        assert_eq!(r.setup, None);
        assert!(parse_release(b"not json").is_err());
    }
    /// Talks to GitHub: `cargo test --release -- --ignored network`.
    #[test]
    #[ignore]
    fn network_release_and_download() {
        let body = get("https://api.github.com/repos/BurntSushi/ripgrep/releases/latest").unwrap();
        let r: ApiRelease = serde_json::from_slice(&body).unwrap();
        assert!(is_newer(&r.tag_name, "0.0.1"));
        // Downloads are redirected to another host.
        let a = r.assets.iter().find(|a| a.name.ends_with(".sha256")).unwrap();
        assert_eq!(get(&a.browser_download_url).unwrap().len() as u64, a.size);
        // Our repository: an error (it is private), not a hang or a crash.
        let _ = get(API_URL);
    }
}
