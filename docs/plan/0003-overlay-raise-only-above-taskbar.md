# Plan 0003. Raise the plate only when the taskbar is above it

Task: `docs/tasks/0003-overlay-raise-only-above-taskbar.md`.

## Approach

Replace the geometric test in `keep_above_taskbar` with a z-order test. The plate is `WS_EX_TOPMOST`, so only windows
above it in the topmost band matter. Walk `GetWindow(hwnd, GW_HWNDPREV)` from the plate towards the top and classify
each window; stop at the first one that is not "other":

- `Taskbar`: class name `Shell_TrayWnd` or `Shell_SecondaryTrayWnd` (`GetClassNameW`): raise.
- `Own`: `GetWindowThreadProcessId` pid equals `GetCurrentProcessId()` (panel, tray menu, alert; the plate itself is not
  in the walk, it starts from its predecessor): do not raise. Checking the process avoids listing the window class
  names of `ui.rs`, `traymenu.rs` and `alert.rs`, which would go stale.
- `Other`: any other window: keep walking.
- End of chain (`GetWindow` returns null): do not raise.

The walk is bounded (for example 512 steps) so a chain that changes under us cannot loop.

Once the plate is raised it is the top window, so the next ticks find nothing above it and do nothing. Another topmost
app that raises itself above the plate is skipped over unless a taskbar is also above the plate, so no swapping.

Cost: the early exit stays (`leaves_work` false: no walk). Otherwise one walk every 240 ms over the topmost windows
(typically 5 to 30 on a desktop; the non-topmost ones are below the plate and never visited), each step a
`GetWindow` plus a pid and class lookup: microseconds, on the UI thread, no allocation beyond a small class buffer
(`[u16; 32]`). Fetch the class name only for windows of other processes (own windows stop the walk before it).

## Pure helper (tested)

```rust
enum Above { Taskbar, Own, Other }
/// `above`: windows over the plate, nearest first. Raise only when the first non-`Other` is a taskbar.
fn should_raise(above: impl IntoIterator<Item = Above>) -> bool
```

The Win32 walk is a lazy iterator mapping each hwnd to `Above`, so the walk stops at the first decisive element
(`.find(|a| !matches!(a, Above::Other)) == Some(Above::Taskbar)`). Class and pid reading stay in a thin
`unsafe fn classify(hwnd, own_pid) -> Above`; `Above` needs `PartialEq`/`Debug` for the tests.

Tests (inline `#[cfg(test)] mod tests` in `overlay.rs`): empty is false; `[Taskbar]` true; `[Other, Other, Taskbar]`
true; `[Own, Taskbar]` false; `[Other, Own, Taskbar]` false; `[Other, Other]` false; `[Taskbar, Own]` true.

## Steps

1. Add `Above`, `should_raise` and the tests.
2. Add `classify` and the walk (`GetWindow`, `GW_HWNDPREV`, `GetClassNameW`, `GetWindowThreadProcessId`,
   `GetCurrentProcessId`; check which `windows-sys` features are already on, add only what is missing).
3. In `keep_above_taskbar`, after the existing `leaves_work` early exit, call the walk and `raise` only when
   `should_raise` says so. Update the doc comments of `keep_above_taskbar` and the 0002 addendum remark ("re-asserts
   topmost every third tick") is superseded: say so in the task notes, do not edit 0002.
4. Leave `raise()` as is for show and drag end.
5. Gates: fmt, clippy, test, build. UI is unchanged, so no snapshot; the on-screen checklist is for the owner.

## Files likely touched

- `src/overlay.rs` (only).
- `Cargo.toml` only if a `windows-sys` feature is missing (`Win32_System_Threading` for `GetCurrentProcessId` is the
  likely one); then say so in the PR.
- `ROADMAP.md` is the owner's: update after the on-screen check.

## Tradeoffs considered

- Raise to just above the taskbar (`SetWindowPos` with `hWndInsertAfter` = the window preceding the taskbar) instead of
  `HWND_TOPMOST`: avoids covering other topmost windows that sit between, but is more code and the taskbar and other
  topmost windows are then ordered by whoever raised last. Rejected: a taskbar above the plate is rare and short, and
  `HWND_TOPMOST` after the walk already only happens in that case.
- WinEvent hook (`EVENT_OBJECT_REORDER`/foreground): no polling, but global state and it misses taskbar self-raises
  without a foreground change (0002 reasoning). Kept the timer.
- Identify own windows by class name list: brittle. Process id is one call and covers future windows.
- Own window above and taskbar above that: the plate stays under both until the own window closes. Accepted: those
  windows are short-lived, and covering them was the bug.
