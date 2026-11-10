# RGB Tray

**English** · [Русский](README.ru.md) · [简体中文](README.zh-CN.md) · [Español](README.es.md) · [Deutsch](README.de.md) · [Français](README.fr.md) · [Português (Brasil)](README.pt-BR.md) · [日本語](README.ja.md)

**Website:** [rgb-tray.gronck.com](https://rgb-tray.gronck.com/)

A tiny Windows tray app that controls RGB lighting of keyboards, mice and lights directly over HID (and Tuya bulbs over the local network) — no vendor software (G HUB, Synapse, Armoury Crate, SignalRGB) running in the background.

**Your device isn't listed? Add it yourself.** Open this repository in [Claude Code](https://claude.com/claude-code) and type `/add-device`: Claude finds the protocol, writes a profile or a driver and tests it with you, no Rust or HID knowledge needed ([how it works](#adding-your-device-with-claude)).

<p>
  <img src="docs/screenshots/panel-dark.png" width="300" alt="Tray panel">
  <img src="docs/screenshots/editor-dark.png" width="520" alt="Per-key editor">
</p>

## Supported devices

| Device | Connection | What works |
|---|---|---|
| Logitech G915 TKL | LIGHTSPEED receiver | Static, breathing, spectrum, off; per-key colors; battery |
| Razer Naga V2 Pro | HyperSpeed dongle / dock | Separate logo and scroll wheel zones; effects; battery |
| Razer Mouse Dock Pro | USB | Static, breathing, spectrum, off |
| About 90 more Razer mice, keyboards and accessories (Basilisk, DeathAdder, Viper, Cobra, BlackWidow, Huntsman, Ornata, Firefly, Goliathus, Base Station…), from OpenRazer's data — [list](profiles) | USB / dongle | Static, breathing, spectrum, off; zones of mice; battery. **Untested**: the card says so, and a report is welcome. Wireless HyperSpeed keyboards only on cable for now. |
| ASUS laptop keyboard (Aura) | Built-in | Static, breathing, spectrum, off; keyboard zones and body lamps via LampArray |
| Geekboard MacroPad v2 (QMK/VIA) | USB | Static, breathing, spectrum, off |
| Other QMK keyboards with VIA and RGB (Keychron, custom boards…) | USB | Static, breathing, spectrum, off, brightness (breathing and spectrum drawn by the app) |
| Devices with the Windows Dynamic Lighting standard (HID LampArray) | USB / built-in | Static, breathing, spectrum, off, zones from the device (breathing and spectrum drawn by the app) |
| Tuya Wi-Fi bulbs (Smart Life), e.g. LINUS A60 | Wi-Fi, local network | Color, white (a white-ish color picks the white LEDs and their tint), breathing, spectrum, off; does not go dark with the display. Needs the bulb's local key, see below |

Other models that speak the same protocols may work but are untested.

### Tuya bulbs

RGB Tray drives a Tuya bulb over the local network (protocol 3.3) with the bulb's *local key*, which only the Tuya cloud gives out. Get it once, for example with [tinytuya](https://github.com/jasonacox/tinytuya) (`python -m tinytuya wizard`), or the Smart Life QR login of `tuya-device-sharing-sdk`. Then list the bulbs in `%APPDATA%\rgbtray\tuya.toml`:

```toml
[[device]]
name = "Desk lamp"
id = "bf1234567890abcdef12"
local_key = "0123456789abcdef"
```

The app finds the bulb on the local network by itself and remembers its address (`local_ip`). The bulb must be in **Smart Life** or **Tuya Smart**: accounts of branded copies of the app can't hand out keys. A bulb takes one local connection at a time: while the Smart Life app is open on a phone, RGB Tray can't reach it.

### Asking for a new device

Right-click the tray icon → **Request device support…**. RGB Tray lists your HID devices, marks the ones that may have controllable lighting, copies the report to the clipboard and opens the [request form](../../issues/new?template=device-request.yml). Enter the model and paste the report. Serial numbers in the report are shortened.

A Razer or QMK/VIA device, or one with a simple protocol, can often be added right away, without a new version: describe it in a small profile file ([docs/profiles.md](docs/profiles.md)).

### Adding your device with Claude

With [Claude Code](https://claude.com/claude-code) you can get a device working yourself, without knowing Rust or HID. Clone this repository, open Claude Code in it and type `/add-device`. Claude reads the device report, looks for the protocol (OpenRGB, OpenRazer, a USB capture of the vendor app), writes a profile or a driver and tests it with you: it can't see the lights, so it asks what the device did after each step. Commands that could harm a device (firmware updates, saving to flash) are off limits. When it works, Claude can send it as a pull request so that it is built in for everyone.

## Features

- One panel for all devices: color, effect, brightness — for everything at once or per device.
- Global brightness plus a per-device brightness on top of it.
- Speed of breathing and spectrum, global and per device.
- Firmware effects of a keyboard in its key editor, next to the presets: wave and ripple on the Logitech G915. Painting a key brings the key colors back.
- Per-key editor for the Logitech keyboard: brush, eyedropper, rainbow, two-color gradient, WASD and letters-only presets, highlighting a word, saved schemes, ANSI/ISO layouts.
- Presets with global hotkeys (including F13–F24 with Ctrl/Alt/Shift/Win).
- A global hotkey per device that switches it off and back on as it was (the keyboard button after the name on its card: F13–F24 or any shortcut), e.g. for a room lamp. Presets are not affected.
- Multiple identical devices with separate settings; devices are recognised by serial number.
- Disconnected devices are kept in an archive with their settings and come back when reconnected.
- "No link" status when a wireless device is asleep or off; colors are re-applied when it wakes.
- Re-applies lighting after sleep (including Modern Standby) and when devices are plugged in.
- Updates: once a day the app asks GitHub for the latest release; a newer version shows a card on the panel that installs it in one click. The check can be switched off in the tray menu.
- Follows the display: the lighting dims when Windows dims the screen and goes off while the screen is off (idle, locked, asleep). Can be switched off in the tray menu.
- Low battery alerts at 10% and 5%.
- The tray icon shows the current color.
- Pinned mode: the panel stays open on top and can be dragged by its header.
- Export and import of all settings to a file. Presets can be shared one by one: right-click a preset → Save to a file…, and the receiver loads it with Load settings or a preset… in the tray menu; it goes to their devices of the same models (all of them, if they have several), and the app says which models of the file they don't have.
- English and Russian interface: Russian when Windows or one of its keyboard layouts is Russian, English otherwise; switchable in the tray menu.
- Light and dark theme following Windows.

## Install

1. Download `RGBTray-Setup-<version>.exe` from [Releases](../../releases).
2. Run it. The app installs itself to `%LOCALAPPDATA%\Programs\RGB Tray` for the current user, without administrator rights, and starts.
3. It can be removed from Windows Settings → Apps.

The installer is not code-signed yet, so Windows SmartScreen may show a warning (More info → Run anyway). Settings are stored in `%APPDATA%\rgbtray` and survive updates and reinstalls. The `backups` folder there keeps copies of the settings from the last 14 days and from before each update; if the settings file gets damaged, the app takes the last copy that works by itself.

Close vendor lighting software before using RGB Tray: two programs writing to the same device fight each other. On Windows 11 the "Logitech LampArray" service (Dynamic Lighting) makes the G915 ignore effects; the app warns about it and offers to disable it.

## Command line

| Command | What it does |
|---|---|
| `rgbtray --diagnose [file.txt]` | Writes the device report: app and Windows versions, supported devices, and all HID devices grouped with their interfaces, likely lighting controllers first. Without a file name it is saved to `%APPDATA%\rgbtray\rgbtray-diagnose.txt` and opened in Notepad. Also available in the tray menu. |
| `rgbtray --hid-send <out.txt> <vid:pid[:page[:usage]]> <feature|output|get> "<hex>"…` | Sends raw HID reports and appends the answers to the file, for working out a device's protocol ([docs/profiles.md](docs/profiles.md#hid-any-device-with-a-simple-protocol)). |
| `rgbtray --profile-check <out.txt> <profile.toml>` | Reads a profile file without touching any device and writes what the app makes of each model, or what is wrong with it. |
| `rgbtray --profile-test <out.txt> <profile.toml>` | Plays colors and effects on the devices of a `[hid]` profile that isn't installed yet and logs each step. |
| `rgbtray --show` | Starts with the panel open and pinned. |
| `rgbtray --uninstall` | Removes the app (settings are kept). |

## Build from source

Requires Windows 10/11 and stable [Rust](https://rustup.rs), the `x86_64-pc-windows-gnu` or `-msvc` toolchain. After installing rustup, open a new terminal so that `cargo` is on PATH.

```powershell
cargo build --release            # target\release\rgbtray.exe
.\scripts\release.ps1            # installer in dist\RGBTray-Setup-<version>.exe
.\scripts\release.ps1 -Install   # build and install/update silently
```

The installer is the same executable: named `RGBTray-Setup-*.exe` (or started with `--install`) it installs itself.

## Contributing

Device reports, testing on your hardware, bug reports and pull requests are welcome. [CONTRIBUTING.md](CONTRIBUTING.md) explains how to build and check the app, how to add a device and what a pull request needs.

## Acknowledgements

The device protocols were learned from these projects; RGB Tray contains no code from them:

- [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB) — Logitech, Razer and ASUS Aura lighting
- [OpenRazer](https://github.com/openrazer/openrazer) — Razer protocol
- [Solaar](https://github.com/pwr-Solaar/Solaar) — Logitech HID++
- [QMK](https://qmk.fm) and [VIA](https://www.caniusevia.com) — MacroPad lighting protocol

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

Not affiliated with Logitech, Razer, ASUS, Geekboard or Tuya. Trademarks belong to their owners.
