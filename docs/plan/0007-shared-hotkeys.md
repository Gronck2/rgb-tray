# Plan 0007. Hotkeys shared with another program

Task: `docs/tasks/0007-shared-hotkeys.md`.

## Approach

`register_hotkeys` (`src/main.rs`, "presets and hotkeys") has one local closure `register(id, hotkey, what)` used for the overlay,
presets and devices. Make that closure the single place that decides: if `RegisterHotKey` returns 0 and `GetLastError() == 1409`,
push `(HOTKEY_BASE-relative id, Hotkey)` to a static `SHARED_HOTKEYS: Mutex<Vec<(i32, config::Hotkey)>>` (cleared at the top of
`register_hotkeys`, next to the `UnregisterHotKey` loop). After all registrations, install the hook if the list is non-empty and
none is installed, uninstall it if the list is empty.

The hook (`WH_KEYBOARD_LL`, `SetWindowsHookExW` with the module handle, 0 thread id) needs a message loop on the installing
thread. `register_hotkeys` already runs on the window thread (`WM_REGISTER_HOTKEYS` exists for that reason), whose loop is the main
message loop, so the hook runs there.

## Steps

1. New small module `src/hotkey_hook.rs` (or a section in `main.rs` if it stays short; prefer the module for the tests):
   - Pure part, no Win32: `struct Matcher { down: Option<u32> }` (or a small set of vks currently held and fired) with
     `fn on_key(&mut self, vk: u32, down: bool, mods: u32, shared: &[(i32, Hotkey)]) -> Option<i32>`.
     Key down: if `vk` is already marked held, return None (auto-repeat); mark held; return the id of the first shared hotkey with
     `h.vk == vk && h.mods == mods` (mods as in `Hotkey`: Alt 1, Ctrl 2, Shift 4, Win 8; no `MOD_NOREPEAT` bit in the comparison).
     Key up: clear the mark. A held key that matched nothing is still marked, so the same press never fires later when a modifier
     is added.
   - Win32 part: `install(hwnd)` / `uninstall()` keeping the `HHOOK` in an `AtomicPtr`; `hook_proc(code, wparam, lparam)`:
     for `code >= 0` and `WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP` read `KBDLLHOOKSTRUCT.vkCode`, skip events with the
     injected flag only if needed (do not skip: the pad is a real device, and injected keys are harmless), compute `mods` with
     `GetAsyncKeyState` for `VK_CONTROL`, `VK_MENU`, `VK_SHIFT`, `VK_LWIN`/`VK_RWIN` (the LL hook runs before the key state is updated, so
     the modifier state is of the keys already down, which is what is wanted for the trigger key), call the matcher, and on a hit
     `PostMessageW(hwnd, WM_SHARED_HOTKEY, id, 0)`. Always `return CallNextHookEx(null, code, wparam, lparam)`. Lock the matcher and
     the list with `try_lock`/short critical sections: no blocking in the callback.
   - Modifier keys themselves as the trigger vk are not supported (hotkeys always have a normal key).
2. `src/main.rs`:
   - `const WM_SHARED_HOTKEY: u32 = WM_APP + 7;` (check that `WM_APP + 7` is free).
   - Factor the body of the `WM_HOTKEY` arm (overlay / preset / device dispatch) into `fn dispatch_hotkey(id: i32)` and call it from
     `WM_HOTKEY` (`wparam as i32 - HOTKEY_BASE`) and `WM_SHARED_HOTKEY` (`wparam as i32`, already relative). Same ids, same actions.
   - `register_hotkeys`: change `register` as above; replace `OVERLAY_HOTKEY_TAKEN` by a lookup of `OVERLAY_HOTKEY_ID` in the shared
     list (rename to what it now means, e.g. `OVERLAY_HOTKEY_SHARED`, or let `ui.rs` call a `hotkey_shared(id)` helper). A failure
     with another error code keeps the log line "taken by another app" and is not shared. Log "shared with another program, observing"
     for the fallback.
   - `WM_DESTROY`: `hotkey_hook::uninstall()` before `PostQuitMessage`.
3. `src/ui.rs` (~line 2220): the title for the overlay target when the key is shared becomes
   `tr!("Общая с другой программой", "Shared with another program")` (the coder may refine the wording; keep it short, it sits in
   the section title line). Keep the accent colour. If the menu of presets or devices has the same title line, show it there too
   through the same helper (one mechanism for all); if they have none, leave them.
4. Tests (`#[cfg(test)] mod tests` in the new module): exact modifiers match; extra modifier and missing modifier do not match;
   auto-repeat (two downs without up) fires once; up then down fires again; a key that matched nothing then gets a modifier while held
   does not fire; two shared hotkeys on different vks each fire their own id.
5. Docs: README and translations only if hotkeys are described there (grep "hotkey"); `ROADMAP.md` is the owner's.
   `docs/arch/overview.md` gets one line on the shared-hotkey hook if it covers hotkeys.
6. Checks: fmt, clippy `-D warnings`, `cargo test --release`; `--snapshot` of the views that show the hotkey menu (`RGBTRAY_DEMO_MENU=overlay`);
   the shared state can only be seen live (config `overlay_hotkey = {mods=7, vk=79}` with G-Helper running), so the owner checks it.

## Files likely touched

- `src/main.rs` (`register_hotkeys`, `WM_HOTKEY` arm, new message, `WM_DESTROY`, mod declaration)
- `src/hotkey_hook.rs` (new)
- `src/ui.rs` (title text)
- `Cargo.toml` features of `windows-sys` only if `Win32_UI_WindowsAndMessaging`/`KeyboardAndMouse` lack `SetWindowsHookExW`,
  `KBDLLHOOKSTRUCT`, `GetAsyncKeyState` (probably already enabled; check, no new dependency)

## Tradeoffs

- Observing hook against `RegisterHotKey` alone: the only way to see a combination another program owns; cost is a global hook
  that every key press passes through, so it exists only while needed and does nothing but a table compare and a post.
- `GetAsyncKeyState` for modifiers against tracking modifier up/down in the hook: simpler, no stuck-modifier state after focus
  changes or a missed key up; the cost is a few syscalls per press of a key that matches a shared vk. Compare the vk first, query
  modifiers only on a vk match.
- Both programs fire on the same key press by design; ordering is not defined and not needed.
- Windows removes a slow LL hook silently (timeout), so the callback must stay trivial; the dispatch happens in the window procedure.
- Per-device hotkey ids depend on `DEVICE_HOTKEYS` order; the shared list stores the relative id of the same scheme, rebuilt on
  every `register_hotkeys`, so it stays in step with it.
