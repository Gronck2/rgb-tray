# Plan 0001. Battery overlay

Task: `docs/tasks/0001-battery-overlay.md`. Defaults below follow the recommendations in its "Owner decisions"; if the
owner answers differently, adjust the matching step.

## Approach

Add `src/overlay.rs`, modelled on `src/alert.rs` (Direct2D on an HWND target, a thread-local state, DPI scale,
`render_png` for snapshots). Reuse the existing hotkey machinery: one more hotkey owner, registered in the same
`register_hotkeys()`, recorded with the same `capture_key` flow. Data comes from the globals that already exist
(`STATUS`, `NO_LINK`, `PRESENT`, config names); no new device I/O.

## Config

In `src/config.rs`, `Config`:

```rust
/// Hotkey that shows and hides the battery overlay.
#[serde(skip_serializing_if = "Option::is_none")]
pub overlay_hotkey: Option<Hotkey>,
```

Default `None` (also in `impl Default`). `Config` is `#[serde(default)]`, so a file without the field loads. Check that
`Config` has no `deny_unknown_fields`, so older versions ignore the new one (acceptance 6). Whether the overlay is
currently visible is not saved: it starts hidden.

Move the "one owner per combination" rule from `take_hotkey` in `main.rs` into a `Config` method (e.g.
`Config::take_hotkey(&mut self, Option<Hotkey>)`) so it can be unit tested, and make it clear `overlay_hotkey` as well.
`main.rs::take_hotkey` becomes a thin call or is removed; behaviour for presets and devices must not change.

## Steps

1. `config.rs`: the field, the `Config::take_hotkey` method including the overlay, tests (below).
2. `main.rs`, hotkeys:
   - `const OVERLAY_HOTKEY_ID: i32 = 2 * MAX_HOTKEYS;` (id `HOTKEY_BASE + 128`). Extend the unregister loop in
     `register_hotkeys()` to cover it (`0..=2 * MAX_HOTKEYS`).
   - Register `cfg.overlay_hotkey` there. Make `register` return success; on failure store it in a static
     `OVERLAY_HOTKEY_TAKEN: AtomicBool` for the menu, and keep logging as now.
   - `WM_HOTKEY`: `id == OVERLAY_HOTKEY_ID` goes to `overlay::toggle()` before the preset/device branches.
   - `set_overlay_hotkey(hotkey)`: same shape as `set_device_hotkey` (take, set, save, `register_hotkeys()`).
   - Where `WM_BATTERY` is handled (and on device list changes, `WM_APPLIED`): call `overlay::refresh()`, a no-op when hidden.
   - On quit, destroy the overlay window with the other windows (see the function that destroys "the app's other
     windows" near `quit()`), so Direct2D objects are released.
3. `overlay.rs`:
   - Pure part, testable: `rows(status, no_link, names) -> Vec<Row>` where `Row { name, percent, charging, level, asleep }`
     and `level` is Normal / Low / Critical (via `LOW_BATTERY`, `CRITICAL_BATTERY`; charging is never low, as in
     `Battery::is_low`). Stable order: the panel order of devices (check how the panel orders `PRESENT`; use the same).
     Devices without a battery are not rows. `height(rows)` computes the window height from the row count.
   - Window: class `RgbTrayBatteryOverlay`, ex-styles `WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE |
     WS_EX_LAYERED | WS_EX_TRANSPARENT`, style `WS_POPUP`. Uniform opacity with `SetLayeredWindowAttributes(LWA_ALPHA, ~217)`
     (a Direct2D HWND target does not do per-pixel alpha; uniform alpha is enough). `WM_NCHITTEST` returns
     `HTTRANSPARENT`. DWM round corners as in `alert.rs`. `SW_SHOWNOACTIVATE`.
   - Position: `cursor_monitor(MONITOR_DEFAULTTOPRIMARY)` as in `alert.rs`, then top-right of `rcWork` with a margin
     of 12 dip. Size from `height(rows)` and DPI scale. Re-position and resize on `refresh()` if the row count changed.
   - Drawing: theme from `ui::theme()`, rows as in the panel's battery label style; the level bar like the battery
     drawing of the alert; warning colours: reuse what the alert uses for low levels. Empty state: one line
     "No devices with a battery" / "Нет устройств с батареей".
   - API: `toggle()`, `refresh()`, `is_visible()`, `destroy()`, `snapshot(path, dark, scale)`.
   - Keep the thread-local state pattern of `alert.rs` (create what can fail before the window; `WM_DESTROY` drops the state).
4. Setting the hotkey (`ui.rs`, `traymenu.rs`):
   - `MenuTarget::Overlay` with `hotkey()` / `set_hotkey()` arms (`cfg.overlay_hotkey`, `crate::set_overlay_hotkey`).
     `capture_key` currently chooses between preset index and device id; add a third state for the overlay
     (`CAPTURE_OVERLAY: AtomicBool`, or fold the three into one enum) and make `stop_capture` / `capturing()` aware of it.
     Do not break the existing `CAPTURE` / `CAPTURE_DEVICE` behaviour.
   - Entry point (simplest first): a tray menu item "Battery overlay" (`traymenu.rs`, new `Item::Overlay`) whose click
     toggles the plate, with the current combination as its right-hand label; a second item "Overlay hotkey…" that opens
     the panel and the hotkey menu for `MenuTarget::Overlay` (see how `Item::Keys` opens things in `run()`, and
     `open_menu` calls at ui.rs ~2250). The menu for the overlay shows only the combo/modifier part, no preset actions
     (`preset = matches!(target, MenuTarget::Preset(_))` already branches on this). If it turns out awkward, fall back
     to a small keyboard button in the panel's footer; keep the choice minimal and tell the owner.
   - In the menu, when `OVERLAY_HOTKEY_TAKEN` is set, show a one-line note "This key is taken by another program".
5. `i18n`: new strings in place with `tr!`; check that the Russian and English strings fit the plate and menu width.
6. `--snapshot` (`main.rs`): add `else if path.contains("overlay") { overlay::snapshot(&path, dark, 1.5) }` in the
   file-name chain (the existing keywords are editor, setup, traymenu, alert; the panel is the default). The snapshot
   needs batteries, but only really discovered devices have them, so add `RGBTRAY_DEMO_BATTERY=<id>:<percent>[:c],...`
   (made-up levels, `c` = charging, an id listed also in `NO_LINK` by the existing `RGBTRAY_DEMO_NOLINK` shows the dimmed
   row) that fills `STATUS` in the snapshot branch only. Document the keyword and the variable in `CLAUDE.md`
   ("Seeing the UI without a screen").
7. README: add the feature and the hotkey in `README.md` and all translations (ru, zh-CN, es, de, fr, pt-BR, ja); note the
   exclusive-fullscreen limit. `ROADMAP.md` is the owner's; the main thread updates it on the owner's confirmation.

## Addendum 2026-11-21: icon and overlay styles

- `config.rs`: `enum OverlayStyle { Full, Row, Column }` (default Full), field `overlay_style` with a lenient
  deserialize (unknown or garbled value reads as Full, like `lenient_effect`), not written when Full. `next()` cycles.
- `overlay.rs`: `Row` gets `icon` from `devices::kind_of(id).icon` (the panel's glyph, Segoe Fluent Icons). A pure
  `size(style, n) -> (w, h)` replaces `height()`: Full is 280 wide with 40 per row; Row and Column use fixed-width
  cells (icon, right-aligned percent, charging mark), Row grows in width, Column in height; the empty state is one line.
  `refresh()` also re-lays-out when the style changed.
- `traymenu.rs`: item "Overlay style" under "Overlay hotkey..." with the style name on the right; click calls
  `main::cycle_overlay_style` (next style, save, `overlay::refresh()`).
- Snapshots: `RGBTRAY_DEMO_OVERLAY_STYLE=full|row|column` (else the saved style); documented in `CLAUDE.md`.
- README feature line mentions the three styles. Tests: style parse/round trip/absent, `size()` per style, rows carry the icon.

## Addendum 2026-11-21: movable plate

- `config.rs`: `OverlayPos { x, y, right, bottom }` (the plate corner nearest the work-area edges and its screen point),
  field `overlay_pos: Option<_>` with a lenient deserialize (garbled reads as unset), not written when unset.
- `overlay.rs`: pure `anchor_of` (nearest corner), `origin` (top-left of a resized plate from the anchor), `placed`
  (clamped into the work area), `default_pos`, `probe`. A 80 ms timer (`follow_modifiers`) reads `GetAsyncKeyState`
  for Ctrl and Shift: both down drops `WS_EX_TRANSPARENT`, `WM_NCHITTEST` answers `HTCAPTION` (the panel does the
  same) so the system move loop drags it; `WM_MOUSEACTIVATE` stays `MA_NOACTIVATE`. `WM_EXITSIZEMOVE` saves the
  position (and rescales when the new monitor has another DPI). `WM_NCLBUTTONDBLCLK` resets to the default.
  `open()` uses the saved position if `MonitorFromPoint(.., DEFAULTTONULL)` finds a monitor, else the default.
- READMEs: feature line mentions Ctrl+Shift+drag and double-click.
- Tests: config round trip/absent/garbled; anchor choice, resize growth, clamping, default and probe.

## Addendum 2026-11-21: no background

- Rendering: `LWA_ALPHA` is gone. `Surface` in `overlay.rs` is a 32-bit top-down DIB section in a memory DC with an
  `ID2D1DCRenderTarget` (premultiplied BGRA, grayscale text AA) bound to it; `Overlay::redraw()` clears it to (0,0,0,0),
  draws, and presents with `UpdateLayeredWindow(ULW_ALPHA)`. `redraw()` replaces the `WM_PAINT` path and is called
  from open, `refresh`, a size or DPI change, `follow_modifiers`, `reset_position` and `WM_EXITSIZEMOVE`. The surface is
  dropped (DIB, DC and target freed in `Drop`) when the pixel size changes or a draw fails. The DWM round-corner
  attribute is dropped. `Cargo.toml` gets the `windows` feature `Win32_Graphics_Gdi` (needed for `BindDC`).
- Legibility: `glow()` draws each text 8 times offset by 1 DIP in the halo colour (black for the dark theme, white for
  the light one, alpha 0.3 / 0.5 times the text alpha), then the text itself. The level bar track is semi-transparent
  with a halo-coloured outline.
- Dragging: while `movable`, `draw()` adds a theme-bg backdrop at 35% alpha plus the accent border over the whole rect.
- Snapshot: transparent PNG; `RGBTRAY_DEMO_OVERLAY_BG=light|dark|split` paints a backdrop first.
- Tests: `halo_color` contrast and dimming. Look is checked with the snapshots over white, black and split backdrops.
- Hardware check: 12. The plate has no background on a bright and on a dark window; text readable on both. Ctrl+Shift
  shows the faint backdrop and the whole plate can be dragged, including between the items. Show/hide, style cycling
  and a drag to a monitor with other scaling leave no leaked handles (watch GDI objects in Task Manager).

## Files likely touched

`src/overlay.rs` (new), `src/main.rs` (mod, hotkey id, WM_HOTKEY, refresh, quit, snapshot, demo var),
`src/config.rs`, `src/ui.rs` (menu target, capture), `src/traymenu.rs`, `CLAUDE.md` (snapshot section), `README*.md`.
Not touched: `src/devices/**`, `src/hid.rs`.

## Tests (hardware-free, inline `#[cfg(test)]`)

- `config.rs`: round trip of `overlay_hotkey` through TOML; a config without the field loads with `None`; `None` is not
  written; `Config::take_hotkey` moves a combination from a preset or a device to the overlay and from the overlay to a
  device, and does nothing for `None`; the existing `device_hotkey_*` tests keep passing.
- `overlay.rs`: `rows()` skips devices without battery; marks Low at 10 and Critical at 5, not when charging; marks
  asleep for ids in `no_link` and keeps their percent; order is stable; `height()` grows with the row count and has an
  empty-state height.
- `Hotkey::label` already has a test; add a case only if a new key label is introduced.

## How to verify without a screen

```
$env:RGBTRAY_DEMO_IDS="logitech-DEMO1,naga-DEMO2,asus-DEMO3"
$env:RGBTRAY_DEMO_BATTERY="logitech-DEMO1:82,naga-DEMO2:8,asus-DEMO3:55:c"
target\release\rgbtray.exe --snapshot <scratch>\overlay-dark.png dark
target\release\rgbtray.exe --snapshot <scratch>\overlay-light.png light
RGBTRAY_LANG=ru and en; also with RGBTRAY_DEMO_NOLINK=1; and with no battery devices (empty state).
```

Look at the PNGs: row alignment, long names truncated not overflowing, warning colours readable on both themes, the
opacity is not visible in a snapshot (note it for the hardware check). Also snapshot the tray menu
(`*traymenu*`) and the hotkey menu of the overlay if it is reachable offline.

## Hardware check (the owner, the agent cannot see it)

1. Set a combination in the app; press it: the plate appears top-right on the monitor with the cursor, press again: it goes.
2. Type in a text editor while the plate is up: focus and typing unaffected; click on the plate area: the click reaches
   the window under it.
3. Levels match the panel for the G915 and the Naga V2 Pro; the Naga on the dock shows charging.
4. Put the keyboard to sleep or switch the mouse off: the row dims, no 0%, no crash; wake it: the row returns to normal
   after the next poll.
5. Over a game: windowed, borderless (should show), exclusive fullscreen (expected not to).
6. A combination another program holds (e.g. one that G HUB or Armoury Crate uses): the note appears, the tray item works.
7. Restart the app: the hotkey is kept; the plate starts hidden.
8. 150% scaling and a second monitor: sharp text, correct monitor.

9. Hold Ctrl+Shift: a coloured border appears and the plate can be dragged; typing in the editor underneath keeps
   working (focus not stolen). Let go of the keys: clicks pass through again.
10. Restart the app, show the plate: it comes back where it was left. Change the style to Row with the plate at the
    right edge: it grows to the left, stays on screen. Drag to the second monitor (other scaling): size follows.
11. Ctrl+Shift+double-click: back to the top-right corner.

## Tradeoffs considered

- Superseded 2026-11-21: per-pixel alpha with `UpdateLayeredWindow` was chosen after all (the owner wants no background).
- Layered window with uniform alpha vs per-pixel alpha (DirectComposition / `UpdateLayeredWindow`): uniform alpha keeps the
  same `HwndRenderTarget` code as `alert.rs`; per-pixel alpha gives a see-through background and soft edges but needs a new
  rendering path. Chosen: uniform alpha.
- Click-through vs draggable: click-through chosen (no focus, no blocking clicks); dragging would need a move mode. Superseded 2026-11-21: a Ctrl+Shift "move mode" (see the addendum).
- Reuse the hotkey menu vs a new settings control: reuse, since there is no settings window in the app (the global
  switches live in the tray menu) and a second recording UI would duplicate `capture_key`.
- Polling on show vs last known data: last known, to avoid waking sleeping devices (see the poller comment in `main.rs`).
- Saving the visible state: not saved, so the plate never reappears unexpectedly after a restart.
