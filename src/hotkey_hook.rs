//! Hotkeys another program already owns (`RegisterHotKey` error 1409): a low-level keyboard hook only observes the keys
//! and posts `WM_SHARED_HOTKEY` to the main window. It never suppresses a key and keeps only "which vk is down".

use crate::config::Hotkey;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Mutex;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Hotkeys to watch: (id relative to `HOTKEY_BASE`, combination).
static SHARED: Mutex<Vec<(i32, Hotkey)>> = Mutex::new(Vec::new());
static MATCHER: Mutex<Matcher> = Mutex::new(Matcher { held: Vec::new() });
static HOOK: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(core::ptr::null_mut());

/// Which keys are down, so that auto-repeat fires once and a press fires only if it matched at its key down.
struct Matcher {
    held: Vec<u32>,
}

impl Matcher {
    /// `mods` is queried lazily: only when the vk is one of the watched ones.
    fn on_key(&mut self, vk: u32, down: bool, mods: impl FnOnce() -> u32, shared: &[(i32, Hotkey)]) -> Option<i32> {
        if !down {
            self.held.retain(|&k| k != vk);
            return None;
        }
        if self.held.contains(&vk) {
            return None;
        }
        // shortcut: a stuck key (missed key up) stays marked until it is pressed and released again; the list stays tiny.
        if self.held.len() < 16 {
            self.held.push(vk);
        }
        if !shared.iter().any(|(_, h)| h.vk == vk) {
            return None;
        }
        let mods = mods();
        shared.iter().find(|(_, h)| h.vk == vk && h.mods == mods).map(|(id, _)| *id)
    }
}

/// Replaces the watched list and installs or removes the hook accordingly. Call on the thread with the message loop.
pub fn set(list: Vec<(i32, Hotkey)>) {
    let empty = list.is_empty();
    *SHARED.lock().unwrap() = list;
    if empty {
        uninstall();
    } else if HOOK.load(Ordering::Relaxed).is_null() {
        unsafe {
            let h = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), GetModuleHandleW(core::ptr::null()), 0);
            if h.is_null() {
                crate::log(&format!("shared hotkeys: hook failed, error {}", GetLastError()));
            }
            HOOK.store(h as *mut _, Ordering::Relaxed);
        }
    }
}

pub fn is_shared(id: i32) -> bool {
    SHARED.lock().unwrap().iter().any(|(i, _)| *i == id)
}

pub fn uninstall() {
    let h = HOOK.swap(core::ptr::null_mut(), Ordering::Relaxed);
    if !h.is_null() {
        unsafe { UnhookWindowsHookEx(h as _) };
    }
    MATCHER.lock().unwrap().held.clear();
}

fn mods_now() -> u32 {
    let down = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) } < 0;
    let mut m = 0;
    if down(VK_MENU) {
        m |= MOD_ALT;
    }
    if down(VK_CONTROL) {
        m |= MOD_CONTROL;
    }
    if down(VK_SHIFT) {
        m |= MOD_SHIFT;
    }
    if down(VK_LWIN) || down(VK_RWIN) {
        m |= MOD_WIN;
    }
    m
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let down = match wparam as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
            WM_KEYUP | WM_SYSKEYUP => Some(false),
            _ => None,
        };
        if let Some(down) = down {
            let vk = (*(lparam as *const KBDLLHOOKSTRUCT)).vkCode;
            // try_lock: never block the system's input path.
            if let (Ok(mut m), Ok(shared)) = (MATCHER.try_lock(), SHARED.try_lock()) {
                if let Some(id) = m.on_key(vk, down, mods_now, &shared) {
                    PostMessageW(crate::HWND_MAIN.load(Ordering::Relaxed) as HWND, crate::WM_SHARED_HOTKEY, id as usize, 0);
                }
            }
        }
    }
    CallNextHookEx(core::ptr::null_mut(), code, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAS: u32 = 7;
    fn hk(mods: u32, vk: u32) -> Hotkey {
        Hotkey { mods, vk }
    }

    #[test]
    fn exact_modifiers_only() {
        let list = [(5, hk(CAS, 79))];
        let mut m = Matcher { held: Vec::new() };
        assert_eq!(m.on_key(79, true, || CAS, &list), Some(5));
        m.on_key(79, false, || 0, &list);
        assert_eq!(m.on_key(79, true, || CAS | 8, &list), None);
        m.on_key(79, false, || 0, &list);
        assert_eq!(m.on_key(79, true, || 3, &list), None);
    }

    #[test]
    fn once_per_press_and_rearm() {
        let list = [(1, hk(2, 65))];
        let mut m = Matcher { held: Vec::new() };
        assert_eq!(m.on_key(65, true, || 2, &list), Some(1));
        assert_eq!(m.on_key(65, true, || 2, &list), None);
        m.on_key(65, false, || 2, &list);
        assert_eq!(m.on_key(65, true, || 2, &list), Some(1));
    }

    #[test]
    fn modifier_added_while_held_does_not_fire() {
        let list = [(1, hk(2, 65))];
        let mut m = Matcher { held: Vec::new() };
        assert_eq!(m.on_key(65, true, || 0, &list), None);
        assert_eq!(m.on_key(65, true, || 2, &list), None);
    }

    #[test]
    fn modifier_released_first_still_rearms() {
        let list = [(1, hk(2 | 8, 65))];
        let mut m = Matcher { held: Vec::new() };
        assert_eq!(m.on_key(0x5B, true, || 8, &list), None); // Win down
        assert_eq!(m.on_key(0x11, true, || 10, &list), None); // Ctrl down
        assert_eq!(m.on_key(65, true, || 10, &list), Some(1));
        m.on_key(0x5B, false, || 2, &list); // Win up first
        m.on_key(0x11, false, || 0, &list);
        assert_eq!(m.on_key(65, true, || 0, &list), None); // still held: auto-repeat
        m.on_key(65, false, || 0, &list);
        assert_eq!(m.on_key(65, true, || 10, &list), Some(1));
    }

    #[test]
    fn each_vk_fires_its_own_id() {
        let list = [(1, hk(2, 65)), (2, hk(2, 66))];
        let mut m = Matcher { held: Vec::new() };
        assert_eq!(m.on_key(66, true, || 2, &list), Some(2));
        assert_eq!(m.on_key(65, true, || 2, &list), Some(1));
    }
}
