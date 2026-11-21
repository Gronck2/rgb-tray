# 0001. Battery overlay (show/hide by a hotkey)

## Goal

A small always-on-top plate, like the overlay in G-Helper, that shows the charge of every device the app can read a
battery from. A global hotkey shows and hides it. The hotkey is set by the user (record a key combination) and is
saved in `config.toml`.

## Why

Today the charge is visible only in the panel (open it from the tray) and as a warning at 10% and 5%. The owner wants
to glance at all levels without opening anything, from inside a game or a full-screen app, and to dismiss the plate
with the same key.

## Current state (found in the code)

- Hotkeys already exist: `Hotkey { mods, vk }` in `src/config.rs`, `register_hotkeys()` / `WM_HOTKEY` in `src/main.rs`
  (presets and devices, ids from `HOTKEY_BASE = 0x100`, `MAX_HOTKEYS = 64` each), recording in `src/ui.rs`
  (`MenuTarget`, `MenuItem::Combo`, `capture_key`). A hotkey belongs to one owner (`take_hotkey`).
  This task adds a third kind of owner instead of a new mechanism.
- Battery data is already there: `STATUS` (id, `Battery { percent, charging }`) filled by the poller every 5 min
  while the user is active (it skips idle periods so sleeping wireless devices are not woken), `NO_LINK` (device
  asleep or off), `PRESENT`, `Config::display_name`. `Battery::is_low()` and `LOW_BATTERY` / `CRITICAL_BATTERY` exist.
- `src/alert.rs` is the closest window: topmost, `WS_EX_NOACTIVATE`, Direct2D, DWM round corners, `render_png` snapshot.

## Scope

- A new overlay window (new module `src/overlay.rs`): borderless, rounded, semi-transparent, topmost, never takes
  focus, click-through. One row per device that has a battery: name, percent, charging mark, a thin level bar;
  low (<= 10%) and critical (<= 5%) in warning colour; a device with no link is dimmed and marked as asleep,
  with its last known percent.
- A global hotkey toggles it (show if hidden, hide if shown). The tray menu gets an item that does the same
  (so the feature works before any hotkey is set and for people whose key is taken).
- The hotkey is configured in the existing hotkey menu (same "record a combination" flow as presets and devices),
  with a place to open it for the overlay (see the plan). Stored in `config.toml`, one owner per combination.
- While visible, the plate redraws when new battery data arrives (`WM_BATTERY`) and when devices come and go.
- All UI strings through `tr!`. README files updated if they list features or hotkeys (`README.md` and all `README.*.md`).
- `--snapshot` view for the overlay, so it can be checked without a screen.
- Unit tests for the hardware-free logic (see the plan).

## Out of scope

- Dragging the plate, resizing, a settings page for it (see decisions: fixed corner, fixed look).
- Showing it automatically on low battery (the existing alert does that).
- Injecting into exclusive full-screen games or hooking graphics APIs.
- Polling devices on show (would wake sleeping wireless devices); only the last known levels are shown.
- Any new device command. Nothing is sent to a device by this task.
- Changing the existing hotkey ids or the format of existing settings.

## Acceptance

1. With the app running and a hotkey set, pressing it shows the plate; pressing it again hides it. Focus and typing in
   the foreground app are not affected, mouse clicks pass through the plate to what is under it.
2. The plate lists each device with a battery (name as in the panel, percent, charging state). Low and critical
   levels use the warning colour. A device without link is shown dimmed, not removed, not as 0%.
3. With no battery devices, the plate says so (one line) instead of being empty.
4. The hotkey can be recorded, changed and removed (Backspace/Delete, Esc cancels) like the other hotkeys; it survives
   restart; setting a combination used by a preset or a device moves it to the overlay (and vice versa).
5. If the combination is taken by another program, the app does not crash; it is logged, and the menu shows that the
   key is not available. The tray menu item still works.
6. `config.toml` written by this version is read by older versions (unknown field ignored), and a config without
   the new field loads with the overlay hotkey unset.
7. Works with Russian and English UI, dark and light theme, 100% to 200% display scaling, two monitors.
8. `cargo fmt --check`, `cargo clippy --release --all-targets -- -D warnings`, `cargo test --release`,
   `cargo build --release` pass. New tests cover the logic listed in the plan.
9. The owner confirmed on real hardware (see the plan, "Hardware check").
10. Each row shows the device's icon, the same glyph as the panel's card for that model.
11. Three styles are selectable from the tray menu item "Overlay style" (cycles Full, Row, Column; the current name is
    shown on the item), saved as `overlay_style` in `config.toml`; the plate re-lays-out at once when visible.
    An unknown value in the file reads as Full and never makes the config unreadable; Full is not written.

## Owner decisions

Decided by the owner on 2026-11-21: every point as recommended below (1-10). Point 4: no default hotkey; the
plate is turned on from the tray item "Battery overlay" until a key is set (no Ctrl+Alt+B default).

1. Position: which corner, draggable, remembered?
   Recommend: top-right of the monitor the cursor is on, inside the work area, fixed, not draggable. Click-through
   and dragging exclude each other, and a draggable plate needs a "move mode". Can be added later with a saved `overlay_pos`.
2. What to show besides the percent?
   Recommend: device name, percent, a charging mark, a level bar, warning colour at <= 10% / <= 5%, and a
   "asleep" dim for devices without link. No "last seen" time in the first version: `STATUS` keeps no timestamp,
   and adding one is a small separate change if wanted.
3. Behaviour over full-screen games?
   Recommend: a normal topmost window. It shows over windowed and borderless-fullscreen games, not over exclusive
   fullscreen (Windows does not allow it without injection). Say so in the README, do not try to work around it.
4. Default hotkey and conflicts?
   Recommend: no default (any default will collide for somebody); the tray item is the way in until a key is set.
   When `RegisterHotKey` fails: keep the saved combination, log it, show "key is taken by another program" in the
   hotkey menu. If the owner wants a default, suggest Ctrl+Alt+B, set only when the field is absent from the config.
5. Auto-show on low battery?
   Recommend: no. The alert window already does it and has a snooze.
6. Opacity and size? (Opacity superseded 2026-11-21: no background, see the addendum.)
   Recommend: fixed, about 85% opaque, width about 280 px at 100% scaling, scaled by DPI, text size as in the panel.
   No settings for them in the first version.
7. Stays until toggled or auto-hides?
   Recommend: stays until toggled off (tray item or hotkey), since it is click-through and cannot be closed by
   mouse. Optional later: auto-hide after N seconds if the owner prefers a "peek" feel.
8. Multi-monitor?
   Recommend: the monitor under the cursor at the moment of showing; it does not follow while visible.
9. Fresh data on show?
   Recommend: no, show the last known levels (at most 5 min old while the user is active). Polling on show would
   wake sleeping keyboards and mice. If levels look too stale to the owner, add a poll of only the awake devices.
10. Where to open the hotkey setting?
    Recommend: a tray menu item "Battery overlay" that toggles the plate and shows the current combination, and
    the panel's existing hotkey menu (keyboard button) opened for it. The plan describes the options; the owner
    picks if the simplest one is not acceptable.

### Addendum, 2026-11-21 (owner, after trying it): icon, percent and three styles

Show the device icon and the charge in percent; three layouts, all selectable, icons the same as in the app:

1. Full (default): icon + name + percent (+ charging mark) + level bar.
2. Row: one compact horizontal line, icon + percent per device side by side, no names, no bar; the plate fits its content.
3. Column: one line per device, icon + percent, no name, no bar; narrow plate.

Kept in every style: charging mark, low/critical colours, dimming of a device without link (the compact styles show
the last percent dimmed, no "asleep" word), the empty-state line, top-right placement, click-through, no activation.
This relaxes "a settings page for it" in Out of scope for this one setting only: the choice is a tray menu item
("Вид индикатора" / "Overlay style"), not a page. Opacity, size and position stay fixed.

### Addendum, 2026-11-21 (owner): the plate can be moved

This replaces decision 1 ("fixed, not draggable, not remembered"): the plate is dragged with Ctrl+Shift+left-button
and the place is remembered. Without both keys it stays click-through as before.

12. Holding Ctrl+Shift makes the plate take the mouse (accent border as a cue); dragging it moves it and focus is not
    stolen; releasing the keys makes it click-through again. A drag in progress is not cut by releasing the keys.
13. The place is saved as `overlay_pos` in `config.toml` (not written when unset; a garbled value reads as unset and
    never makes the config unreadable) and used the next time the plate is shown, if a monitor still holds it; else
    the default (top-right of the monitor under the cursor). The plate is always kept inside the work area.
14. The plate remembers its corner nearest the screen edges, so adding rows or switching to the wider Row style grows
    it away from that edge instead of off-screen.
15. Ctrl+Shift+double-click puts it back in the default corner and clears the saved place.

### Addendum, 2026-11-21 (owner): no background

The plate covered what was under it. This replaces decision 6 ("about 85% opaque"): the background is removed completely,
with no setting. Only the icons, the percents and, in the full style, the names, the charging mark and the level bar are
drawn, each with a contrasting halo (black behind light text, white behind dark text) so that they read on any background.
Low/critical colours and the dimming of a device without link stay.

16. The plate has no background or border: the pixels between the items are fully transparent (per-pixel alpha).
17. Text, icons and the level bar (a semi-transparent track with a halo outline) read on both a white and a black
    background, in the dark and the light theme.
18. While Ctrl+Shift is held the plate shows a faint translucent backdrop (about 35%) and the accent border over its
    whole rectangle, so that all of it can be grabbed (fully transparent pixels get no mouse input); on release the
    backdrop is gone again.
19. `--snapshot *overlay*` writes the transparent image (with alpha); `RGBTRAY_DEMO_OVERLAY_BG=light|dark|split` draws it
    over a white, a black, or a half white / half black backdrop so that it can be reviewed.

## Notes

- No significant architectural decision: a new window module next to `alert.rs`, one new config field. No ADR needed.
- `src/config.rs` and `src/main.rs` are security-shaped paths (settings compatibility, hotkeys): extra care in review.
