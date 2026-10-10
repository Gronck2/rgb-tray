# RGB Tray

**English** · [Русский](README.ru.md)

A tiny Windows tray app that controls RGB lighting of keyboards and mice directly over HID — no vendor software (G HUB, Synapse, Armoury Crate, SignalRGB) running in the background.

<p>
  <img src="docs/screenshots/panel-dark.png" width="300" alt="Tray panel">
  <img src="docs/screenshots/editor-dark.png" width="520" alt="Per-key editor">
</p>

> The interface is in Russian for now; an English UI is planned (see [ROADMAP.md](ROADMAP.md)).

## Supported devices

| Device | Connection | What works |
|---|---|---|
| Logitech G915 TKL | LIGHTSPEED receiver | Static, breathing, spectrum, off; per-key colors; battery |
| Razer Naga V2 Pro | HyperSpeed dongle / dock | Separate logo and scroll wheel zones; effects; battery |
| Razer Mouse Dock Pro | USB | Static, breathing, spectrum, off |
| ASUS laptop keyboard (Aura) | Built-in | Static, breathing, spectrum, off |
| Geekboard MacroPad v2 (QMK/VIA) | USB | Static, spectrum, off |

Other models that speak the same protocols may work but are untested. To ask for a new device, attach the output of `rgbtray --diagnose` (see below).

## Features

- One panel for all devices: color, effect, brightness — for everything at once or per device.
- Global brightness plus a per-device brightness on top of it.
- Per-key editor for the Logitech keyboard: brush, eyedropper, rainbow, two-color gradient, WASD and letters-only presets, highlighting a word, saved schemes, ANSI/ISO layouts.
- Presets with global hotkeys (including F13–F24 with Ctrl/Alt/Shift/Win).
- Multiple identical devices with separate settings; devices are recognised by serial number.
- Disconnected devices are kept in an archive with their settings and come back when reconnected.
- "No link" status when a wireless device is asleep or off; colors are re-applied when it wakes.
- Re-applies lighting after sleep (including Modern Standby) and when devices are plugged in.
- Low battery alerts at 10% and 5%.
- The tray icon shows the current color.
- Pinned mode: the panel stays open on top and can be dragged by its header.
- Export and import of all settings to a file.
- Light and dark theme following Windows.

## Install

1. Download `RGBTray-Setup-<version>.exe` from [Releases](../../releases).
2. Run it. The app installs itself to `%LOCALAPPDATA%\Programs\RGB Tray` for the current user, without administrator rights, and starts.
3. It can be removed from Windows Settings → Apps.

The installer is not code-signed yet, so Windows SmartScreen may show a warning (More info → Run anyway). Settings are stored in `%APPDATA%\rgbtray` and survive updates and reinstalls.

Close vendor lighting software before using RGB Tray: two programs writing to the same device fight each other. On Windows 11 the "Logitech LampArray" service (Dynamic Lighting) makes the G915 ignore effects; the app warns about it and offers to disable it.

## Command line

| Command | What it does |
|---|---|
| `rgbtray --diagnose [file.txt]` | Writes a report of the app and Windows versions, the devices found and all HID interfaces. Without a file name it is saved to `%APPDATA%\rgbtray\rgbtray-diagnose.txt` and opened in Notepad. Also available in the tray menu. |
| `rgbtray --show` | Starts with the panel open and pinned. |
| `rgbtray --uninstall` | Removes the app (settings are kept). |

## Build from source

Requires Windows 10/11 and stable [Rust](https://rustup.rs) (developed with the `x86_64-pc-windows-gnu` toolchain).

```powershell
cargo build --release            # target\release\rgbtray.exe
.\scripts\release.ps1            # installer in dist\RGBTray-Setup-<version>.exe
.\scripts\release.ps1 -Install   # build and install/update silently
```

The installer is the same executable: named `RGBTray-Setup-*.exe` (or started with `--install`) it installs itself.

## Acknowledgements

The device protocols were learned from these projects; RGB Tray contains no code from them:

- [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB) — Logitech, Razer and ASUS Aura lighting
- [OpenRazer](https://github.com/openrazer/openrazer) — Razer protocol
- [Solaar](https://github.com/pwr-Solaar/Solaar) — Logitech HID++
- [QMK](https://qmk.fm) and [VIA](https://www.caniusevia.com) — MacroPad lighting protocol

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

Not affiliated with Logitech, Razer, ASUS or Geekboard. Trademarks belong to their owners.
