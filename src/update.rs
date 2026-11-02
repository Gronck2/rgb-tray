//! Update check: asks GitHub for the latest published release once a day; a newer one shows
//! a card on the panel, whose button downloads the installer and runs it quietly (the
//! installer closes this app, replaces the exe and starts the new one).

use serde::Deserialize;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::time::Duration;
use windows_sys::Win32::Networking::WinHttp::*;

use crate::wide;

/// The repository releases come from; pages and downloads must be on it.
const REPO: &str = "Gronck2/rgb-tray";
const API_URL: &str = "https://api.github.com/repos/Gronck2/rgb-tray/releases/latest";
const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
/// Downloaded installers are saved in %TEMP% under this name (it makes the exe start as the installer).
const SETUP_PREFIX: &str = "rgbtray-setup-";

#[derive(Clone, PartialEq, Debug)]
pub struct Release {
    /// "0.7.0", without the "v" of the tag.
    pub version: String,
    /// The release page, for when the download fails.
    pub page: String,
    /// The installer, only when it can be checked.
    setup: Option<Setup>,
}

#[derive(Clone, PartialEq, Debug)]
struct Setup {
    url: String,
    size: u64,
    sha256: [u8; 32],
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
    let setup = (kind != "nosetup").then(|| Setup { url: String::new(), size: 0, sha256: [0; 32] });
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
        remove_old_setups();
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
            // The state may have changed during the request: a download that started meanwhile
            // goes on, and a failed download of this same release stays failed.
            let mut s = STATE.lock().unwrap();
            match &*s {
                State::Downloading(_) => return,
                State::Available(old) | State::Failed(old) if *old == r => return,
                _ => {}
            }
            crate::log(&format!("update: {} available", r.version));
            *s = State::Available(r);
            drop(s);
            crate::post(crate::WM_APPLIED);
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
    let setup = r.setup.as_ref().ok_or("the release has no installer")?;
    let data = get(&setup.url)?;
    if data.len() as u64 != setup.size {
        return Err(format!("downloaded {} bytes of {}", data.len(), setup.size));
    }
    if sha256(&data).ok_or("SHA-256 failed")? != setup.sha256 {
        return Err("the installer does not match its SHA-256 digest".into());
    }
    // The name makes the exe start as the installer; the version is checked in `parse_release`.
    let path = std::env::temp_dir().join(format!("RGBTray-Setup-{}.exe", r.version));
    std::fs::write(&path, &data).map_err(|e| format!("{}: {e}", path.display()))?;
    crate::log(&format!("update: running {}", path.display()));
    std::process::Command::new(&path).arg("--quiet").spawn().map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// Removes installers left in %TEMP% by earlier updates (the running exe can't be removed and is skipped).
fn remove_old_setups() {
    let me = std::env::current_exe().ok();
    let Ok(dir) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if name.starts_with(SETUP_PREFIX) && name.ends_with(".exe") && Some(e.path()) != me && e.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// SHA-256 of `data` (Windows CNG).
fn sha256(data: &[u8]) -> Option<[u8; 32]> {
    use windows_sys::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let mut out = [0u8; 32];
    let ok = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, null(), 0, data.as_ptr(), data.len() as u32, out.as_mut_ptr(), 32) } == 0;
    ok.then_some(out)
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
    /// "sha256:<hex>"; older releases have none.
    #[serde(default)]
    digest: Option<String>,
}

/// The release; its installer only when it is on our repository and has a SHA-256 digest to check
/// (otherwise only its page is offered). The version goes into a file name: digits and dots only.
fn parse_release(body: &[u8]) -> Result<Release, String> {
    let r: ApiRelease = serde_json::from_slice(body).map_err(|e| format!("release JSON: {e}"))?;
    let version = r.tag_name.strip_prefix('v').unwrap_or(&r.tag_name).to_string();
    if !is_plain_version(&version) {
        return Err(format!("unexpected release tag {:?}", r.tag_name));
    }
    let site = format!("https://github.com/{REPO}/");
    let setup = r
        .assets
        .into_iter()
        .find(|a| a.name.to_lowercase().starts_with("rgbtray-setup") && a.name.to_lowercase().ends_with(".exe"))
        .filter(|a| a.browser_download_url.starts_with(&format!("{site}releases/download/")))
        .and_then(|a| Some(Setup { sha256: parse_digest(a.digest.as_deref()?)?, url: a.browser_download_url, size: a.size }));
    let page = if r.html_url.starts_with(&site) { r.html_url } else { format!("{site}releases/latest") };
    Ok(Release { version, page, setup })
}

/// "1.2.3": numbers separated by single dots.
fn is_plain_version(v: &str) -> bool {
    !v.is_empty() && v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The hash of GitHub's asset `digest`, "sha256:<64 hex digits>".
fn parse_digest(d: &str) -> Option<[u8; 32]> {
    let hex = d.strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
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
                {"name": "RGBTray-Setup-0.7.0.exe", "size": 1234567,
                 "browser_download_url": "https://github.com/Gronck2/rgb-tray/releases/download/v0.7.0/RGBTray-Setup-0.7.0.exe",
                 "digest": "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}
            ]
        }"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, "0.7.0");
        assert_eq!(r.page, "https://github.com/Gronck2/rgb-tray/releases/tag/v0.7.0");
        let setup = r.setup.unwrap();
        assert_eq!(setup.url, "https://github.com/Gronck2/rgb-tray/releases/download/v0.7.0/RGBTray-Setup-0.7.0.exe");
        assert_eq!(setup.size, 1234567);
        assert_eq!(Some(setup.sha256), sha256(b"abc"));
        // A release without an installer is still offered, with its page (a foreign one is replaced).
        let r = parse_release(br#"{"tag_name": "v0.7.1", "html_url": "https://p"}"#).unwrap();
        assert_eq!(r.setup, None);
        assert_eq!(r.page, "https://github.com/Gronck2/rgb-tray/releases/latest");
        assert!(parse_release(b"not json").is_err());
    }

    #[test]
    fn installer_only_with_digest_and_from_our_repository() {
        let release = |url: &str, digest: &str| {
            let json = format!(
                r#"{{"tag_name": "v0.7.0", "html_url": "https://github.com/Gronck2/rgb-tray/releases/tag/v0.7.0",
                    "assets": [{{"name": "RGBTray-Setup-0.7.0.exe", "browser_download_url": "{url}", "size": 1{digest}}}]}}"#
            );
            parse_release(json.as_bytes()).unwrap()
        };
        let ours = "https://github.com/Gronck2/rgb-tray/releases/download/v0.7.0/RGBTray-Setup-0.7.0.exe";
        let digest = r#", "digest": "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad""#;
        assert!(release(ours, digest).has_setup());
        assert!(!release(ours, "").has_setup());
        assert!(!release(ours, r#", "digest": "md5:900150983cd24fb0d6963f7d28e17f72""#).has_setup());
        assert!(!release("https://evil.example/RGBTray-Setup-0.7.0.exe", digest).has_setup());
        assert!(!release("https://github.com/someone/else/releases/download/v0.7.0/RGBTray-Setup-0.7.0.exe", digest).has_setup());
    }

    #[test]
    fn digests() {
        let abc = parse_digest("sha256:BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD").unwrap();
        assert_eq!(Some(abc), sha256(b"abc"));
        assert_ne!(Some(abc), sha256(b"abd"));
        assert_eq!(sha256(b"").map(|h| h[..4].to_vec()), Some(vec![0xe3, 0xb0, 0xc4, 0x42]));
        assert_eq!(parse_digest("sha256:ba78"), None);
        assert_eq!(parse_digest("sha512:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"), None);
        assert_eq!(parse_digest("sha256:+a7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"), None);
    }

    #[test]
    fn release_versions_are_plain() {
        assert!(is_plain_version("0.7.0"));
        assert!(is_plain_version("1"));
        for bad in ["", "0.7.0-beta", "0..7", "0.7.", "../../x", "0.7.0\\x", "v0.7"] {
            assert!(!is_plain_version(bad), "{bad}");
        }
        assert!(parse_release(br#"{"tag_name": "v0.7.0/../../x", "html_url": "https://p"}"#).is_err());
        assert_eq!(parse_release(br#"{"tag_name": "0.7.2", "html_url": "https://p"}"#).unwrap().version, "0.7.2");
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
