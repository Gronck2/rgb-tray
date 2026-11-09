# RGB Tray (crate `rgbtray`)

A small Windows tray app (Rust, Win32 + Direct2D, no UI framework) that drives RGB lighting
of keyboards, mice and lights directly over HID (and Tuya bulbs over the LAN), replacing SignalRGB / G HUB / Synapse / Armoury Crate.
Windows only.

Talk to the person in their language. Code, comments and commit messages are in English.
The person you work with may be the maintainer or anyone who cloned the repository to get their own device working.
If they want their device supported, use the `/add-device` skill (`.claude/skills/add-device/SKILL.md`).

**You cannot see the lights.** Anything that talks to a device has to be checked by the person: ask them what the device did.
Never send firmware update, bootloader or "save to flash" commands to a device.

## Build and check

- Toolchain: stable Rust, `x86_64-pc-windows-gnu` or `-msvc` (CI uses MSVC). `cargo` may be installed but not on PATH:
  look in `%USERPROFILE%\.cargo\bin`.
- Before committing, run what CI runs (`.github/workflows/ci.yml`):
  `cargo fmt --check`, `cargo clippy --release --all-targets -- -D warnings`, `cargo test --release`.
  `rustfmt.toml` sets `max_width = 160`, so long one-line expressions are normal.
- Unit tests live in `#[cfg(test)] mod tests` at the bottom of each module and cover only hardware-free logic:
  config, migration, presets, keymap, colors, battery curve, LampArray zones, profiles and report templates.
  When you change that logic, add a test there.
- Don't bump `version` in `Cargo.toml` or push tags: releases are made by the maintainer.

## The command line

The release build is a GUI program (`windows_subsystem = "windows"`): the CLI tools write to files, never to the console.

- `--diagnose [file]`: HID report (English only): every HID collection with usage page, report lengths and what the app makes of it.
- `--hid-send <out> <vid:pid[:page[:usage]]> <feature|output|get> "<hex>"...`: raw reports for working out a protocol; answers appended to `<out>`.
- `--profile-check <out> <profile.toml>`: what the app makes of each model of a profile file (or its errors), no device needed.
- `--profile-test <out> <profile.toml>`: plays a fixed sequence on the devices of a `[hid]` profile, step log in `<out>`.
- `--lamparray-test <out> <rrggbb> <sec>`, `--logitech-effect-test` (G915 firmware effect variants, 2 s dark between steps, steps logged),
  `--logitech-battery-led-test` (SWControl variants, 20 s each).
- `--show`, `--install`/`--quiet` (installer), `--uninstall`. `RGBTRAY_LAMPARRAY_ALL=1` makes the
  generic LampArray driver take devices that have a native driver.

## Seeing the UI without a screen

You cannot see windows from the sandbox. Render offline instead and look at the PNG:

```
target\release\rgbtray.exe --snapshot <out.png> [dark|light] [card] [zone]
```

The file name picks the view: `*editor*` = key editor, `*setup*` = installer, `*traymenu*`, `*alert*`; anything else = the panel.
Env vars: `RGBTRAY_LANG=en|ru`, `RGBTRAY_DEMO_MANY=n`, `RGBTRAY_DEMO_PAGE=k`, `RGBTRAY_DEMO_WARN`,
`RGBTRAY_DEMO_DYNLIGHT`, `RGBTRAY_DEMO_NOLINK`, `RGBTRAY_DEMO_ARCHIVE`, `RGBTRAY_DEMO_DIM`, `RGBTRAY_DEMO_SPEED`,
`RGBTRAY_DEMO_MENU` (`=device`: hotkey menu of the last device), `RGBTRAY_DEMO_WIN`, `RGBTRAY_DEMO_APPS=synapse,ghub` (conflict warnings),
`RGBTRAY_DEMO_UPDATE=0.7.0[,downloading|failed|nosetup]` (update card).
Snapshots still talk to real devices (discovery).

## Runtime files

In `%APPDATA%\rgbtray`: `config.toml`, `rgbtray.log` and `rgbtray.old.log`; `config.backup.toml` (last good settings)
and `backups/` (daily copies, last 14, and one before each version, last 10); `devices\*.toml` (the user's own profiles); `tuya.toml` (bulbs).
An unreadable config is kept as `config.unreadable-<time>.toml` and replaced by the newest copy that reads (`Config::load_from`). Never make a
settings value unreadable for older/newer versions: unknown effects read as static (`lenient_effect`).
Every G915 command is logged with milliseconds.

## Layout

- `main.rs`: entry point, CLI, hidden window and message loop, global state (statics), apply/reapply
  (wake from Modern Standby via `GUID_CONSOLE_DISPLAY_STATE`, hot-plug), hotkeys, battery poller,
  `--diagnose`, export/import.
- `ui.rs`: the flyout panel (Direct2D/DirectWrite). `editor.rs`: per-key editor. `draw.rs`: drawing and window helpers the windows share. `traymenu.rs`,
  `alert.rs` (low-battery window), `installer.rs` (the exe installs itself).
- `config.rs`: TOML config. Devices are keyed `"<kind>-<serial>"`, e.g. `[devices."logitech-1A2B3C4D"]`.
- `i18n.rs`: every UI string is written in place as `tr!("рус", "eng")`. `Text = (ru, en)` tuples are used for const tables.
  Driver errors and `--diagnose` are English only.
- `update.rs`: daily update check (GitHub `releases/latest` over WinHTTP), download and quiet install of the new setup.
  `cargo test --release -- --ignored network` checks it against a public repo.
- `conflicts.rs`: vendor programs that really drive lighting (by process name and device kind).
  Keep the list narrow: ASUS system components are not conflicts.
- `hid.rs`: minimal Win32 HID layer. `services.rs`: services that fight over devices
  (`logi_lamparray_service`). `keymap.rs`: key names and codes (G915 zone ids = shifted HID usages; saved key colors use them),
  layouts from `layouts/*.toml` (g915-tkl, tkl, full; ANSI/ISO in one file). `RGBTRAY_DEMO_LAYOUT=full` for editor snapshots.
- `devices/mod.rs`: the `Device` trait, `kinds()` (every model, in panel order: Rust ones and profiles), `caps()`, `DRIVERS` (each driver module has one `DRIVER`: discover, batteries, answering, key editor, diagnose note), `discover()`,
  `interface_note()`. Drivers: `logitech.rs` (HID++ 0x8071 effects, 0x8081
  per-key), `razer.rs` (91-byte feature report), `asus.rs` (Aura 0x5D/0x5A), `via.rs` (generic
  QMK/VIA), `lamparray.rs` (generic HID LampArray 0x59), `rawhid.rs` (`[hid]` profiles: report templates), `tuya.rs` (Tuya Wi-Fi bulbs, local protocol 3.3 over TCP 6668;
  bulbs listed in `%APPDATA%\rgbtray\tuya.toml`), `animation.rs` (60 fps app-drawn effects).
- `devices/profile.rs`: TOML profiles (`profiles/`, built in via `BUILT_IN`; users add theirs in `%APPDATA%\rgbtray\devices\`).

### Adding a device

The `/add-device` skill walks through it. In short, from cheapest:

1. LampArray devices and QMK/VIA keyboards already work through generic drivers.
2. A model of a protocol profiles support (Razer, VIA) is a TOML file in `profiles/`, listed in `BUILT_IN` in `devices/profile.rs` (format: `docs/profiles.md`).
3. A simple protocol (reports with the color in them, no answers needed) is a `[hid]` profile.
4. Anything else needs code: a driver that implements `Device`, a `Kind` in `NATIVE` in `devices/mod.rs`, a `DRIVER` at the end of its module
   (`discover`, and `batteries`/`answering` if wireless, `note` for `--diagnose`, `open_for_keys` for per-key) added to `DRIVERS`.

Then update every README: `README.md` and its translations `README.*.md` (ru, zh-CN, es, de, fr, pt-BR, ja). A native driver wins over the generic LampArray/VIA ones, and `discover()` dedups by id.
Never change the id scheme of an existing model: settings are keyed by it.

## Hardware gotchas (verified, do not "fix")

- G915 hangs and then ignores 0x8071 effects (0x8081 per-key still works) until it is power-cycled. Known triggers:
  - `logi_lamparray_service` reacting to an animated ASUS effect;
  - logo breathe or logo off;
  - switching directly from one animation to another;
  - most likely the undocumented zone effect 0x000C (11.10.2026: nothing animated after it until a power cycle). Never send it.
  The workarounds since v0.4.4: the logo is set to static instead, and static is sent before every animation.
  It also happens at random, for example on fast static/black switching. Every command is still acknowledged, so the hang cannot be seen from the protocol.
  Since v0.6.9, Static and Off go through 0x8081 per-key fill, which keeps working in the hung state.
  0x8071 is only used for breathe and cycle. Don't move steady colors back to 0x8071.
  A `trace` file in `%APPDATA%\rgbtray` turns on logging of every HID++ request and response.
- LampArray drops the first update after `AutonomousMode = 0`, so there is a 1 s wait on the first take.
  Virtual VHF devices are skipped.
- The ASUS keyboard resets to white on wake. Lighting is reapplied at +0.5 s, +3 s and +10 s.
  ASUS LampArray has 8 lamps: 4 keyboard zones and 4 body lamps.
- App-drawn breathing has an 8% floor. 30 fps, or breathing down to 0, looked choppy.
- Naga zones: logo 0x04, side 0x05. There is no per-button lighting.
- Tuya bulbs (checked on a LINUS A60): protocol 3.3 only. A bulb takes one LAN client at a time: while Smart Life is open
  on the phone, connections are reset. Writing DP 22 (white brightness) switches it to white mode, so colour sends only 20/21/24.
  Keys come from the Smart Life / Tuya Smart account; branded copies of the app can't hand them out.
  Bulbs are class `Light`: they don't go dark with the display.

## Windows shell pitfalls

- Never edit files with Cyrillic text through PowerShell 5.1 `Get-Content`/`Set-Content`, because it mangles UTF-8.
  Use the Edit/Write tools or Bash.
- Never run the setup exe with `Start-Process -Wait` or pipe its output: it launches the tray app and the command hangs.
