# 0003. Battery overlay: raise the plate only when the taskbar is above it

## Goal

The overlay plate re-asserts its z-order only when a taskbar window is really above it, and never lifts itself over
the app's own panel, tray menu or low-battery alert.

## Why

Found in code review (approved by the owner and the orchestrator, 2026-11-21). `keep_above_taskbar` in
`src/overlay.rs` (called every third tick of the 80 ms modifier timer, about every 240 ms) calls `raise()`
(`SetWindowPos(HWND_TOPMOST, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE)`) whenever the plate reaches outside the work
area of its monitor at all (`leaves_work`). That puts the plate above every always-on-top window, not only the taskbar:

- it covers the app's own flyout panel (`ui.rs`), tray menu (`traymenu.rs`) and low-battery alert (`alert.rs`), all
  `WS_EX_TOPMOST`, while they are open and the plate overlaps the taskbar area;
- it covers other apps' topmost windows (picture-in-picture, video overlays);
- another app that re-raises itself would swap places with the plate about four times a second.

The reason for the timer (0002 addendum) stays valid: the taskbar raises itself above topmost windows when clicked, and
the plate has to come back above it. The owner verified on 2026-11-21 that the plate stays above the taskbar after a
click on it; that must not regress.

## Scope

- In `keep_above_taskbar`, decide by z-order instead of geometry: walk upward from the plate with
  `GetWindow(hwnd, GW_HWNDPREV)`:
  - first a taskbar window (class `Shell_TrayWnd` or `Shell_SecondaryTrayWnd`) found: raise the plate;
  - first a window of this process found (panel, tray menu, alert): do nothing, the plate stays under it;
  - reached the end without either: do nothing.
- A pure helper over the list of windows above the plate (kind per window, in z-order, nearest first) that answers
  "raise or not", with unit tests.
- Keep the cheap early exit: a plate fully inside the work area never walks (no taskbar overlap, nothing to do).

## Out of scope

- Any change to dragging, clamping, `place`, the raise on show and at the end of a drag (they stay a plain `raise`).
- Positioning the plate just above the taskbar instead of above all topmost windows (see the plan, tradeoffs).
- A WinEvent hook instead of the timer (rejected in 0002).
- Other windows of the app (editor, installer) and anything outside `src/overlay.rs`.

## Acceptance

1. With the plate overlapping the taskbar and nothing else open, a click on the taskbar brings the plate back above
   it within about 240 ms (no regression of the 2026-11-21 behaviour), on the primary and, if present, a secondary
   monitor's taskbar.
2. With the plate overlapping the taskbar, opening the flyout panel, the tray menu or the low-battery alert (the demo or
   a real one) shows them above the plate and they stay there.
3. A topmost window of another app (for example picture-in-picture) is not covered by the plate and does not flicker
   against it, as long as no taskbar is above the plate.
4. A plate inside the work area does no z-order work at all (no `GetWindow` walk).
5. `cargo fmt --check`, `cargo clippy --release --all-targets -- -D warnings`, `cargo test --release` and
   `cargo build --release` pass; the new helper has unit tests (taskbar first, own window first, only others, empty,
   taskbar behind an own window).

## On-screen checklist for the owner

1. Drag the plate (Ctrl+Shift) so it overlaps the taskbar, click an empty spot of the taskbar: the plate is above the
   taskbar again almost at once.
2. With the plate still over the taskbar, open the tray menu and the panel: both are above the plate, not under it.
3. Play a video in a picture-in-picture window (or any other always-on-top window) near the plate: the plate does not
   cover it.

## Owner decisions

None expected. Defaults used: when an own window is above the plate the plate waits (a later tick raises it once that
window is closed and the taskbar is above again); other apps' topmost windows are skipped over while looking for the
taskbar, and a raise still goes above them (recommended: simplest, and it only happens when the taskbar is above).

## Notes

- No architectural decision, no ADR.
- Touches only `src/overlay.rs`: not a security-shaped path. Nothing is sent to a device.
