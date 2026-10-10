//! Windows services that fight the app over device lighting.

use std::mem::zeroed;
use std::ptr::{null, null_mut};

use windows_sys::Win32::System::Services::*;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// Logitech's bridge to Windows Dynamic Lighting. It draws on the G915 with per-key frames
/// (e.g. when an ASUS keyboard runs an animated effect), after which the keyboard ignores
/// effects until it is switched off and on.
pub const LOGI_LAMPARRAY: &str = "logi_lamparray_service";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The service is installed and running or set to start (so it would come back).
pub fn interferes(name: &str) -> bool {
    unsafe {
        let scm = OpenSCManagerW(null(), null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return false;
        }
        let svc = OpenServiceW(scm, wide(name).as_ptr(), SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS);
        if svc.is_null() {
            CloseServiceHandle(scm);
            return false;
        }
        let mut status: SERVICE_STATUS = zeroed();
        let running = QueryServiceStatus(svc, &mut status) != 0 && status.dwCurrentState != SERVICE_STOPPED;
        let mut needed = 0u32;
        QueryServiceConfigW(svc, null_mut(), 0, &mut needed);
        // u64 elements keep the structure aligned.
        let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(1)];
        let config = buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;
        let disabled = QueryServiceConfigW(svc, config, (buf.len() * 8) as u32, &mut needed) != 0 && (*config).dwStartType == SERVICE_DISABLED;
        CloseServiceHandle(svc);
        CloseServiceHandle(scm);
        running || !disabled
    }
}

/// Stops the service and turns off its autostart, asking for administrator rights (UAC).
/// Returns false when the user declined.
pub fn disable(name: &str) -> bool {
    let args = wide(&format!("/c sc stop {name} & sc config {name} start= disabled"));
    let r = unsafe { ShellExecuteW(null_mut(), wide("runas").as_ptr(), wide("cmd.exe").as_ptr(), args.as_ptr(), null(), SW_HIDE) };
    r as isize > 32
}
