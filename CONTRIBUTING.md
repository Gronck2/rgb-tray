# Contributing to RGB Tray

**English** · [Русский](CONTRIBUTING.ru.md)

Thanks for helping. RGB Tray is a small Windows tray app in Rust (Win32 + Direct2D, no UI framework) that drives RGB lighting directly over HID. You can help without writing code. The most useful things are device reports and testing on hardware we don't have.

## Ways to help

- **Ask for a device.** Right-click the tray icon → **Request device support…**. The app copies a report of your HID devices and opens the [request form](../../issues/new?template=device-request.yml). Paste the report and describe what lights up.
- **Test builds.** A device we add for you can only be checked on your hardware. We'll ask you to try a build and tell us what the lights actually did.
- **Report bugs** with the [bug form](../../issues/new?template=bug-report.yml). Attach `%APPDATA%\rgbtray\rgbtray.log` (and `rgbtray.old.log` if the problem was before the last restart).
- **Send a pull request:** a device, a fix, a translation fix.

## Build and check

Requires Windows 10/11 and stable [Rust](https://rustup.rs). Both the `x86_64-pc-windows-gnu` and the MSVC toolchains work. Development uses GNU and CI uses MSVC.

```powershell
cargo build --release                          # target\release\rgbtray.exe
cargo fmt --check                              # rustfmt.toml: max_width = 160
cargo clippy --release --all-targets -- -D warnings
cargo test --release
.\scripts\release.ps1 [-Install]               # installer in dist\, optionally install it
```

CI (`.github/workflows/ci.yml`) runs `fmt`, `clippy` and the tests on every push and pull request. Run them before you push.

### Seeing the panel without opening it

```powershell
target\release\rgbtray.exe --snapshot panel.png [dark|light] [card] [zone]
```

This renders a view to a PNG. The file name picks the view: `*editor*` is the key editor, `*setup*` the installer, `*traymenu*` the tray menu, `*alert*` the low-battery window, anything else the panel. `RGBTRAY_LANG=en|ru` picks the language. `RGBTRAY_DEMO_*` variables add made-up states (see `ui::snapshot` and the `--snapshot` code in `main.rs`). Please attach before/after snapshots to pull requests that change the UI.

### Debugging devices

- `rgbtray --diagnose report.txt`: all HID devices and interfaces, with the ones RGB Tray knows marked.
- `%APPDATA%\rgbtray\rgbtray.log`: what was applied and every error. Create an empty file `%APPDATA%\rgbtray\trace` to also log every HID++ request and reply.
- Close vendor software (G HUB, Synapse, Armoury Crate, SignalRGB, OpenRGB) while testing. Two programs writing to one device fight each other, and that looks like a bug in either one.

## Project layout

| Path | What it is |
|---|---|
| `src/main.rs` | Entry point, CLI, message loop, applying settings, sleep/wake and hot-plug, hotkeys, battery poller |
| `src/ui.rs`, `editor.rs`, `traymenu.rs`, `alert.rs` | The panel, the per-key editor, the tray menu, the low-battery window |
| `src/config.rs` | TOML settings in `%APPDATA%\rgbtray\config.toml`, presets |
| `src/i18n.rs` | UI strings: `tr!("русский", "English")` in place |
| `src/hid.rs` | Minimal Win32 HID layer: enumerate, open, feature/output/input reports |
| `src/devices/mod.rs` | The `Device` trait, `KINDS` (every model), `discover()`, `interface_note()` |
| `src/devices/*.rs` | Drivers: `logitech` (HID++), `razer`, `asus` (Aura), `via` (QMK/VIA), `lamparray` (HID LampArray), `animation` (effects drawn by the app) |

## Adding a device

Start with the cheapest route that works.

### 1. Maybe it already works

- **HID LampArray** (Windows Dynamic Lighting) devices are driven by the generic `lamparray` driver: static, breathing, spectrum, off, and zones and key colors described by the device itself.
- **QMK keyboards with VIA and RGB** are driven by the generic `via` driver.

If the report marks your device as one of these but something doesn't work, open a bug, not a device request.

### 2. A new model of a protocol we know

- **Razer:** write a profile: a small TOML file in `profiles/` (built in) or `%APPDATA%\rgbtray\devices\` (your own, no compiling). [docs/profiles.md](docs/profiles.md) describes the format; OpenRazer is a good source for the values. Add the file to `BUILT_IN` in `src/devices/profile.rs` to build it in.
- **QMK/VIA with a known firmware effect:** write a profile with a `[via]` section (see [docs/profiles.md](docs/profiles.md)). Without one the keyboard still works; the app draws breathing and spectrum itself.
- **Logitech HID++ 2.0** keyboards with the 0x8071/0x8081 features are close to the G915. The receiver ids and the layout (`layouts/*.toml`, see `src/keymap.rs`) are the parts that differ.

Logitech HID++ and ASUS Aura models are still in code: their drivers find what a device can do by themselves, and a profile would only matter once there's a second model to tell apart (see `ROADMAP.md`).

### 3. A new protocol: write a driver

1. Create `src/devices/<name>.rs` and implement `Device`:

   ```rust
   use super::{Device, Effect, Rgb};
   use crate::hid::{self, Hid};

   pub struct MyDevice {
       pub id: String,
       hid: Hid,
   }

   pub fn find_all() -> Vec<MyDevice> {
       hid::enumerate(0x1234)
           .into_iter()
           .filter(|i| i.usage_page == 0xFF00 && i.feature_len >= 64)
           .filter_map(|i| {
               let hid = Hid::open(&i.path, true, false)?;
               // "<kind key>-<serial>"; without a serial, super::path_hash(&i.path) and port_id: true.
               Some(MyDevice { id: format!("mydev-{}", super::path_hash(&i.path)), hid })
           })
           .collect()
   }

   impl Device for MyDevice {
       fn apply(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String> {
           let c = color.scaled(brightness);
           // super::speed() is the speed of breathing and spectrum for this device.
           let mut buf = [0u8; 64];
           buf[..4].copy_from_slice(&[0x00, 0x01, c.0, c.1]); // your protocol here
           let _ = effect;
           if self.hid.set_feature(&buf) { Ok(()) } else { Err("MyDevice: send failed".into()) }
       }
   }
   ```

   `apply_zones`, `apply_keys`/`update_keys` and `battery` have defaults; implement them if the device has zones, per-key lighting or a battery. If the firmware has no breathing or spectrum, play them with `animation::start` the way `lamparray.rs` and `via.rs` do. A wireless device that is off but whose receiver is plugged in returns `Err(NO_LINK)`.

2. Add `pub mod <name>;` and a `Kind` to `KINDS` in `src/devices/mod.rs`. The key is the id prefix and must not contain `-`. Add the name in both languages, a Segoe Fluent Icons glyph and what the model can do: zones, per-key colors, effects, speed (`Speed::None`, `Steps(n)` or `Smooth`) and class (keyboard, mouse, other). The panel shows only the controls these allow (`devices::caps`). What only the device itself can tell (like LampArray zones and keys) the driver registers when it finds the device.
3. Hook the driver into `discover()`. If the device is wireless, also hook it into `batteries()` and `answering()`. A native driver must come before the generic `lamparray`/`via` ones, because `discover()` keeps the first device with a given id.
4. Add a line to `interface_note()` so `--diagnose` marks the device as supported.
5. Add the device to the tables in `README.md` and `README.ru.md`.

Some things to keep in mind:

- **Device ids** are `"<kind>-<serial>"`, and settings are keyed by them. Use a serial the device reports. If there is none, use `path_hash` of the HID path and set `port_id: true`. Never change the id scheme of an existing model, or users lose their settings.
- **Unit tests** cover only logic that runs without hardware (config, presets, layouts, colors, zones, battery curves). Add tests next to such logic in `#[cfg(test)] mod tests` at the bottom of the module.
- **Hardware quirks** that you verified go in a comment next to the workaround, with what triggers them. Some workarounds look wrong but are deliberate, for example the G915 notes in the Logitech driver. Don't "simplify" them away.

### 4. A closed protocol

If nobody has documented the protocol, it can be learned by recording what the vendor software sends while you change the lighting (Wireshark + USBPcap). [docs/usb-capture.md](docs/usb-capture.md) explains it step by step. Attach the capture to the device request, together with a list of what you changed and when.

## Code style

- Code, comments, commit messages and `--diagnose` output are in English. UI strings are in Russian and English: `tr!("русский", "English")`, or a `Text` tuple `(ru, en)` in const tables. Don't edit files with Cyrillic in Windows PowerShell 5.1 `Get-Content`/`Set-Content`, because it breaks UTF-8.
- Match the code around you: short doc comments on what is not obvious, no new dependencies without a reason, no UI framework.
- `cargo fmt` decides formatting. Long one-line expressions up to 160 columns are normal here.

## Pull requests

- One topic per pull request. Describe what you changed and **which hardware you checked it on**. Say so if you couldn't check it.
- UI changes: attach `--snapshot` images (dark and light, and Russian if the strings changed).
- CI must pass.
- A new device or a behaviour change: update `README.md` and `README.ru.md`.

## License

RGB Tray is licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

Don't copy code from projects under other licenses (OpenRGB and OpenRazer are GPL). Learning a protocol from them is fine; RGB Tray's code is written from scratch.
