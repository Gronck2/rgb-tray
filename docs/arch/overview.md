# Architecture overview

RGB Tray (crate `rgbtray`) is a single Windows executable (Rust, Win32 + Direct2D, no UI framework) that sits in
the tray and drives RGB lighting of keyboards, mice and lights directly over HID, and Tuya bulbs over the LAN.

- Entry point and global state: `src/main.rs` (CLI tools, hidden window and message loop, apply/reapply on wake
  and hot-plug, hotkeys, battery poller).
- Shared hotkeys: `src/hotkey_hook.rs` (a hotkey another program owns, error 1409, is watched by an observing low-level keyboard hook that never suppresses keys; installed only while one exists).
- System batteries: `src/sysbat.rs` (Bluetooth devices Windows reports a level for, read-only, plus Logitech mice and headsets over HID++ battery reads while the user is active; shown on the overlay and covered by the low-battery alert).
- UI: `src/ui.rs` (flyout panel), `src/editor.rs` (per-key editor), `src/traymenu.rs`, `src/alert.rs`,
  `src/installer.rs`; shared drawing in `src/draw.rs`; strings via `tr!` in `src/i18n.rs`.
- Settings: `src/config.rs` (TOML in `%APPDATA%\rgbtray`, devices keyed `"<kind>-<serial>"`, backups).
- Devices: `src/devices/mod.rs` (the `Device` trait, `kinds()`, `DRIVERS`, `discover()`), native drivers
  (`logitech.rs`, `razer.rs`, `asus.rs`, `msi.rs`, `tuya.rs`), generic ones (`via.rs`, `lamparray.rs`, `rawhid.rs`),
  TOML profiles (`profile.rs`, `profiles/`), app-drawn effects (`animation.rs`); HID layer in `src/hid.rs`.
- Updates: `src/update.rs` (daily GitHub release check, download, quiet install).
- Landing page: `site/` (static, built by `site/build.py`, published by `.github/workflows/pages.yml`).

The detailed layout and the hardware rules are in `CLAUDE.md`.
