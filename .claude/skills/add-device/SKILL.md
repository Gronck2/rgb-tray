---
name: add-device
description: Get a keyboard, mouse, light or other RGB device working in RGB Tray, and optionally contribute it to the repository. Use when the user wants their device supported, asks why a device isn't found, or wants to add, test or submit a device profile or driver.
---

# Add a device to RGB Tray

You are helping someone get the RGB lighting of their own device working in RGB Tray. They may not know Rust or HID; you do the technical work, they are your eyes. **You cannot see the lights.** Every result comes from the person looking at the device, so ask them after each step that should change something.

Talk to the person in their language. Files, code and commit messages are in English.

## Safety rules (always)

Sending the wrong bytes to a device can, in rare cases, put it into a bad state. Follow these rules for the whole session:

- Never send firmware update, bootloader, DFU, factory reset, or "save to flash / EEPROM / onboard profile" commands. If a captured or documented command might do one of these, don't send it.
- Before sending reports you made up or guessed, say what you will send and why, and get a yes. Send one report (or one short sequence) at a time and ask what happened.
- Prefer reading first: the device report, `get` feature reports, documented protocols, then writing.
- Don't run the vendor's software and RGB Tray against the same device at once: they fight. Ask the person to quit the vendor app (tray icon → Exit) and to exit RGB Tray while probing a device RGB Tray already drives.
- If the device stops responding, ask the person to unplug it and plug it back in (or switch it off and on) before anything else.

## 0. Get the tools

- If the repository isn't built yet: check for `cargo` (`cargo --version`; on Windows it may be in `%USERPROFILE%\.cargo\bin`, not on PATH). Without it, the installed app works for steps 1–4: `%LOCALAPPDATA%\Programs\RGB Tray\rgbtray.exe` (it must be a version with `--hid-send`; if not, offer to install Rust from https://rustup.rs and build with `cargo build --release`).
- The app is a GUI program: its command-line tools write to a file, never to the console. Always pass an output file and read it afterwards.

## 1. Look at what is connected

Run `rgbtray --diagnose report.txt` and read the report. Find the device by the person's description (vendor and product names, `VID:PID`). Note:

- what the report says about each of its interfaces: `SUPPORTED: …` means a driver already takes it; `Razer control interface (protocol known)`, a LampArray collection (usage page `0059`), or a QMK/VIA one (`FF60:0061`) mean a known protocol; `vendor-specific` collections are candidates for a lighting protocol;
- the feature / output report lengths of those collections;
- the "Device profiles" line: profile files with mistakes are listed there.

If the device is marked SUPPORTED but doesn't work right, this is a bug, not a new device: check `%APPDATA%\rgbtray\rgbtray.log`, and if you can't fix it, help the person file a bug report.

## 2. Pick the cheapest route

1. **Already works:** a LampArray device or a QMK keyboard with VIA. Have them open the panel; at most write a profile for a nicer name (`docs/profiles.md`).
2. **A Razer model RGB Tray doesn't list:** a `[razer]` profile. Take the product ids, transaction id and LED ids from OpenRazer's source (`driver/razer*_driver.h` and `.c`), see `docs/profiles.md`.
3. **A simple protocol:** a `[hid]` profile, report templates with the color in them (`docs/profiles.md`, section `[hid]`). No compiling.
4. **Anything that needs answers, handshakes or state** (an XOR or sum checksum, "take control" reports and per-LED colors a `[hid]` profile can do): a Rust driver (`CONTRIBUTING.md`, "A new protocol: write a driver"). This needs the Rust toolchain.

## 3. Find out the protocol

In this order, stop as soon as you know enough:

1. **It is documented.** Search the web for the device and VID:PID together with OpenRGB, OpenRazer, liquidctl, SignalRGB plugins, or reverse engineering notes. OpenRGB's controller sources (`Controllers/<Vendor>…`) are the best single source: they show the report id, length, command bytes, and whether a commit / apply report follows the color. Read them to learn the protocol; don't copy code (RGB Tray is MIT/Apache, OpenRGB is GPL).
2. **The vendor software can drive it.** Ask the person to record a USB capture while they change colors in the vendor app (`docs/usb-capture.md`), and read it: find the report that changes when only the color changes.
3. **Neither.** Careful probing with `--hid-send` only with the person's yes for each step, starting with `get` reads of the vendor collection's feature reports. If nothing turns up, stop: suggest a device request issue with the report attached, and say why.

Send test reports with:

```
rgbtray --hid-send out.txt <vid:pid[:usage_page[:usage]]> <feature|output|get> "<hex bytes>" ["<hex bytes>"...]
```

Bytes start with the report id (`00` when the collection has none) and are padded with zeros to the report length. The file gets what was sent and any answer appended. After each send ask the person what the lights did.

## 4. Write and test the profile

Write the profile in a scratch location first, following `docs/profiles.md` exactly (fields, `key` rules, `tested`). Then:

```
rgbtray --profile-test out.txt <profile.toml>
```

plays red, green, blue, dim white, each zone alone, breathing, spectrum, and off, logging each step with its time. Ask the person to watch and tell you which steps looked right. Fix and repeat.

When it works, install it for them: copy it to `%APPDATA%\rgbtray\devices\`, then ask them to restart RGB Tray (tray icon → Exit, start it again) and check the device's card on the panel. A user file with the same `key` replaces a built-in profile.

Choose `key` carefully: settings are stored under it, so it must never change later.

## 5. A driver, if a profile can't do it

Follow `CONTRIBUTING.md` ("A new protocol: write a driver") and the repository's `CLAUDE.md`. In short: a module in `src/devices/`, a `Kind` in `NATIVE`, a `DRIVER` added to `DRIVERS` before the generic ones. Build with `cargo build --release`, check with `cargo fmt --check`, `cargo clippy --release --all-targets -- -D warnings`, `cargo test --release`, and have the person test each effect on the device. Unit tests go only to hardware-free logic.

## 6. Contribute it (ask first)

Ask whether the person wants to share the device with everyone. If yes:

- **Profile:** copy it to `profiles/<vendor>-<model>.toml`, add it to `BUILT_IN` in `src/devices/profile.rs` (the test `every_profile_file_is_built_in` checks this), set `tested = true` only if they checked it on the device, and add the device to the tables of `README.md` and `README.ru.md`.
- **Driver:** the steps in `CONTRIBUTING.md`, both READMEs too.
- Run the three checks above, commit on a new branch, and open a pull request with `gh pr create` if `gh` is set up (fork first if they can't push to the repository). Fill in the pull request template: device, route, what was checked on hardware.
- If they don't want to deal with git, help them open a device request issue instead (`.github/ISSUE_TEMPLATE/device-request.yml`) and attach the profile and the device report.

Serial numbers in device reports are shortened by the app; don't paste anything else personal into public issues.
