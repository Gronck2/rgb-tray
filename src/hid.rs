//! Minimal Win32 HID layer: enumeration, feature reports, overlapped read/write.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
use windows_sys::Win32::Devices::HumanInterfaceDevice::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::System::IO::*;

pub struct HidInfo {
    /// NUL-terminated device interface path.
    pub path: Vec<u16>,
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub feature_len: u16,
    pub input_len: u16,
    pub output_len: u16,
}

impl HidInfo {
    /// The product name the device reports (empty if none or not readable).
    pub fn product(&self) -> String {
        self.string(HidD_GetProductString)
    }

    /// The manufacturer name the device reports (empty if none or not readable).
    pub fn manufacturer(&self) -> String {
        self.string(HidD_GetManufacturerString)
    }

    /// How the device is connected, from its interface path.
    pub fn transport(&self) -> &'static str {
        let path = String::from_utf16_lossy(&self.path).to_lowercase();
        // HID over Bluetooth Classic and Bluetooth LE (HOGP) services.
        if path.contains("{00001124-") || path.contains("{00001812-") {
            "Bluetooth"
        } else if path.contains("vid_") {
            "USB"
        } else {
            "internal"
        }
    }

    /// A string descriptor read with `get` (HidD_GetProductString etc.); empty if none.
    fn string(&self, get: unsafe extern "system" fn(HANDLE, *mut core::ffi::c_void, u32) -> BOOLEAN) -> String {
        unsafe {
            let h = CreateFileW(self.path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut());
            if h == INVALID_HANDLE_VALUE {
                return String::new();
            }
            let mut buf = [0u16; 128];
            let ok = get(h, buf.as_mut_ptr() as _, (buf.len() * 2) as u32) != 0;
            CloseHandle(h);
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            if ok {
                String::from_utf16_lossy(&buf[..len]).trim().to_string()
            } else {
                String::new()
            }
        }
    }
}

/// Lists present HID collections of the given vendor.
pub fn enumerate(vid: u16) -> Vec<HidInfo> {
    enumerate_where(Some(vid))
}

/// Lists every present HID collection (for diagnostics).
pub fn enumerate_all() -> Vec<HidInfo> {
    enumerate_where(None)
}

fn enumerate_where(vid: Option<u16>) -> Vec<HidInfo> {
    let mut out = Vec::new();
    unsafe {
        let mut guid: GUID = zeroed();
        HidD_GetHidGuid(&mut guid);
        let set = SetupDiGetClassDevsW(&guid, null(), null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if set as isize == -1 {
            return out;
        }
        let mut index = 0;
        loop {
            let mut ifd: SP_DEVICE_INTERFACE_DATA = zeroed();
            ifd.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(set, null(), &guid, index, &mut ifd) == 0 {
                break;
            }
            index += 1;
            let mut required = 0u32;
            SetupDiGetDeviceInterfaceDetailW(set, &ifd, null_mut(), 0, &mut required, null_mut());
            if required == 0 {
                continue;
            }
            let mut buf = vec![0u64; (required as usize).div_ceil(8)];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &ifd, detail, required, null_mut(), null_mut()) == 0 {
                continue;
            }
            let p = std::ptr::addr_of!((*detail).DevicePath) as *const u16;
            let mut len = 0;
            while *p.add(len) != 0 {
                len += 1;
            }
            let mut path = std::slice::from_raw_parts(p, len).to_vec();
            path.push(0);
            if let Some(vid) = vid {
                if !String::from_utf16_lossy(&path).to_lowercase().contains(&format!("vid_{vid:04x}")) {
                    continue;
                }
            }
            if let Some(info) = query(path) {
                out.push(info);
            }
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

unsafe fn query(path: Vec<u16>) -> Option<HidInfo> {
    // Zero access is enough for attributes/caps and works even for mouse/keyboard collections.
    let h = CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut());
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut attr: HIDD_ATTRIBUTES = zeroed();
    attr.Size = size_of::<HIDD_ATTRIBUTES>() as u32;
    let mut caps: HIDP_CAPS = zeroed();
    let mut ok = HidD_GetAttributes(h, &mut attr) != 0;
    if ok {
        let mut pp: PHIDP_PREPARSED_DATA = zeroed();
        ok = HidD_GetPreparsedData(h, &mut pp) != 0;
        if ok {
            HidP_GetCaps(pp, &mut caps);
            HidD_FreePreparsedData(pp);
        }
    }
    CloseHandle(h);
    ok.then_some(HidInfo {
        path,
        vid: attr.VendorID,
        pid: attr.ProductID,
        usage_page: caps.UsagePage,
        usage: caps.Usage,
        feature_len: caps.FeatureReportByteLength,
        input_len: caps.InputReportByteLength,
        output_len: caps.OutputReportByteLength,
    })
}

pub struct Hid {
    h: HANDLE,
    ev: HANDLE,
}

// Handles are plain kernel objects; each Hid is used by one thread at a time.
unsafe impl Send for Hid {}

impl Hid {
    /// `rw`: request read/write access (not possible for mouse/keyboard collections).
    /// `overlapped`: needed for read/write with timeouts; feature reports use a plain handle.
    pub fn open(path: &[u16], rw: bool, overlapped: bool) -> Option<Hid> {
        unsafe {
            let access = if rw { GENERIC_READ | GENERIC_WRITE } else { 0 };
            let flags = if overlapped { FILE_FLAG_OVERLAPPED } else { 0 };
            let h = CreateFileW(path.as_ptr(), access, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, flags, null_mut());
            if h == INVALID_HANDLE_VALUE {
                return None;
            }
            let ev = CreateEventW(null(), 1, 0, null());
            Some(Hid { h, ev })
        }
    }

    pub fn set_feature(&self, buf: &[u8]) -> bool {
        unsafe { HidD_SetFeature(self.h, buf.as_ptr() as _, buf.len() as u32) != 0 }
    }

    pub fn get_feature(&self, buf: &mut [u8]) -> bool {
        unsafe { HidD_GetFeature(self.h, buf.as_mut_ptr() as _, buf.len() as u32) != 0 }
    }

    pub fn write(&self, buf: &[u8]) -> bool {
        unsafe {
            let mut ov: OVERLAPPED = zeroed();
            ov.hEvent = self.ev;
            let mut n = 0u32;
            if WriteFile(self.h, buf.as_ptr(), buf.len() as u32, &mut n, &mut ov) != 0 {
                return true;
            }
            if GetLastError() != ERROR_IO_PENDING {
                return false;
            }
            if WaitForSingleObject(self.ev, 1000) != WAIT_OBJECT_0 {
                CancelIo(self.h);
                GetOverlappedResult(self.h, &ov, &mut n, 1);
                return false;
            }
            GetOverlappedResult(self.h, &ov, &mut n, 0) != 0
        }
    }

    /// Reads one input report. `Ok(None)` on timeout, `Err` if the device is gone.
    pub fn read(&self, buf: &mut [u8], timeout_ms: u32) -> Result<Option<usize>, ()> {
        unsafe {
            let mut ov: OVERLAPPED = zeroed();
            ov.hEvent = self.ev;
            let mut n = 0u32;
            if ReadFile(self.h, buf.as_mut_ptr(), buf.len() as u32, &mut n, &mut ov) == 0 {
                if GetLastError() != ERROR_IO_PENDING {
                    return Err(());
                }
                if WaitForSingleObject(self.ev, timeout_ms) != WAIT_OBJECT_0 {
                    CancelIo(self.h);
                    GetOverlappedResult(self.h, &ov, &mut n, 1);
                    return Ok(None);
                }
                if GetOverlappedResult(self.h, &ov, &mut n, 0) == 0 {
                    return Err(());
                }
            }
            Ok(Some(n as usize))
        }
    }
}

impl Drop for Hid {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.h);
            CloseHandle(self.ev);
        }
    }
}
