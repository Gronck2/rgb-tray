# 0002. System devices (Bluetooth headphones and so on) on the battery overlay

## Goal

The battery overlay (task 0001) also lists devices whose battery level Windows itself knows and the app does not
drive: today the owner's Sony WH-1000XM6 headphones, tomorrow any Bluetooth headset, mouse, keyboard or gamepad.
They appear next to the app's own devices with their Windows name, a fitting icon and the percent, and the overlay
(all three styles) adapts to however many there are.

## Why

The owner wants one glance at every battery he cares about. The headphones are not an RGB device, so the app has no
driver for them, but Windows already shows their level (Settings > Bluetooth & devices). Reading what the OS
already knows needs no protocol work and no device command.

## Current state (found in the code)

- Battery data of the app's devices: drivers return `(id, Battery)` from `Driver::batteries`; `devices::batteries()`
  is polled by `spawn_battery_poller` in `src/main.rs` every 5 min while the user is active (idle > 120 s is
  skipped, because asking a sleeping Logitech keyboard wakes it). Results go to `STATUS`; devices asleep are in
  `NO_LINK`; `check_batteries()` raises the low-battery alert per id from `STATUS`.
- `src/overlay.rs`: `rows(ids, status, no_link, name)` builds `Row { name, icon, percent, charging, level, asleep }`
  for ids from `crate::active_ids()` (`PRESENT` + `NO_LINK`, panel order), the icon from `kind_of(id).icon`, the
  name from `Config::display_name`. `size(style, n)` already grows with the row count: Full in height (40 per row),
  Column in height (28 per row), Row in width (74 + 12 per cell). There is no cap.
- `kind_of(id)` returns the first model for an unknown prefix, `Config::register/sorted/forget` and the hotkeys all
  work on ids in `PRESENT`. So system devices must stay out of `PRESENT`, `STATUS` and `config.devices`, or they would
  turn up in the panel and the config as a bogus model.
- The `windows-sys` feature `Win32_Devices_DeviceAndDriverInstallation` is already on (`src/hid.rs` uses SetupDi).
  The overlay does not read the battery on show (0001, decision 9) because that would wake devices; reading an OS
  property is not talking to the device, see the plan.

## Scope

- A new read-only source of system batteries (new module `src/sysbat.rs`): Bluetooth devices for which Windows holds a
  battery level (PnP device property `DEVPKEY_Bluetooth_Battery`, the value Settings shows), with name, connection
  state and class.
- Overlay rows for them: Windows friendly name, an icon by the device class, percent, same low/critical colours.
  Charging mark is not shown (Windows gives no charging state for this property).
- Connected devices only; disconnected ones are hidden (decision 2).
- A cap on the number of rows so the plate always fits the work area.
- Read on the 5-min poller tick, when the overlay is shown, and every 60 s while it is visible.
- An opt-out: tray item "System devices" (on by default) and `overlay_system` in `config.toml`.
- A CLI probe `--system-batteries <out>` listing every Bluetooth device node with the property found, whether
  connected, class, and the id the app would give it. It is the first step of the work (it settles whether the XM6
  exposes a level) and the tool for anyone whose device does not show up.
- `--snapshot` demo variable for system devices; README feature line (all `README*.md`); `CLAUDE.md` (CLI and demo variable).
- Unit tests for the hardware-free logic (see the plan).

## Out of scope

- Any message to a device: no GATT connection, no HFP/AVRCP/RFCOMM, no vendor protocol (Sony). Only OS properties are read.
- BLE GATT Battery Service through WinRT, Xbox controllers through XInput, USB HID batteries, `Win32_Battery`
  (the laptop's own battery). They would plug into the same module later (decision 5) if the probe shows a device
  that Windows holds no property for.
- Showing system devices in the panel, the tray icon tooltip, hotkeys, lighting or `config.devices`.
- Low-battery alert for them (decision 3) and renaming them in the app (decision 4).
- Changing the id scheme, `STATUS`, `NO_LINK`, `PRESENT` or `Driver`.

## Acceptance

1. With the XM6 connected and Windows showing its level in Settings, the overlay shows a row with its Windows name,
   a headphones icon and the same percent as Settings (within the update lag of Windows). Verified by the owner.
2. Switching the headphones off or disconnecting them removes the row within one refresh (60 s while the overlay is
   visible); connecting them adds it. A disconnected device never shows its stale percent.
3. Nothing is sent to any device and no device is woken: the code only calls SetupDi/CM property getters. No
   Bluetooth radio traffic is caused by the app (checked by reading; the owner sees no change in the headphones).
4. The app's own devices are unchanged: same rows, order and colours. System rows come after them, sorted by name,
   then id. A system device that is the same product as an app device (decision 6) is shown once, as the app's.
5. All three styles lay out correctly with 0, 1, 5 and the maximum number of rows, in dark and light themes and RU/EN;
   with the cap exceeded the plate still fits the work area at 100% to 200% scaling.
6. The panel, the tray menu list of devices, hotkeys, `config.devices` and the low-battery alert see no system
   device (no new entries in `config.toml` except `overlay_system = false` when the user turns it off).
7. `overlay_system` is not written when on; a config with it is read by older versions (unknown field ignored); a
   garbled value reads as on and never makes the config unreadable.
8. The tray item turns the system rows off and on at once, also while the plate is visible.
9. A machine without Bluetooth, or with Bluetooth off, behaves as before (empty list, no log spam: errors are logged
   once per change, not every minute).
10. `cargo fmt --check`, `cargo clippy --release --all-targets -- -D warnings`, `cargo test --release` and
    `cargo build --release` pass; new tests cover the logic in the plan.
11. The owner confirmed on real hardware (plan, "Hardware check").

## Owner decisions

Decided by the owner on 2026-11-21:

- Q1 answered by a read-only probe (Get-PnpDeviceProperty, 2026-11-21): Windows reports DEVPKEY_Bluetooth_Battery for
  "WH-1000XM6 Hands-Free AG" (BTHENUM HFP node, 94%), with the parent BTHENUM\DEV_58186221E9F9 node
  DEVPKEY_Device_IsConnected = true. Also present, disconnected: Xbox Wireless Controller (BTHLE, 80%),
  SRS-XB33 (BTHENUM HFP, 10%), and Naga V2 Pro over BLE (100%, the dedup case). The battery sits on the
  service (HFP) node and the connection state on the device (DEV_) node: join them by Bluetooth address.
- Q2: hide disconnected devices (as recommended).
- Q3: YES, the low-battery alert also covers system devices, the same as the app's own (10% / 5%, same snooze).
  This changes the recommended default; the plan must cover it.
- Q7: tray item "System devices", on by default (as recommended).
- Q4, Q5, Q6, Q8: as recommended.


Questions that matter, each with the recommended default (the plan follows the defaults):

1. Does Windows Settings show the XM6 battery level today? If not, the PnP property is empty and this approach gives
   nothing for the headphones (Sony may report the level only over its own protocol or LE Audio GATT).
   Recommend: the owner runs the probe (`--system-batteries`) first; if the XM6 is missing there, this task stops
   and a follow-up task (GATT Battery Service over WinRT) is planned instead of guessing.
2. Disconnected paired devices: hide, or show dimmed like a sleeping app device?
   Recommend: hide. A PC usually has many paired Bluetooth devices, mostly not in range; a dimmed row for each would
   crowd the plate, and Windows keeps a stale percent for a disconnected device.
3. Low-battery alert (10% / 5%) for system devices too?
   Recommend: no in this task, overlay only. The alert is per device id with a snooze and re-arming kept in
   `main.rs`; adding system ids there is a small follow-up if wanted (Windows may already warn for headsets).
4. Can a system device be renamed in the app?
   Recommend: no. The name is the Windows one (rename it in Windows settings). Saving names would put system ids
   into `config.devices`, which `register`/`sorted`/`forget` treat as lighting devices.
5. Which sources, now and later?
   Recommend: now only the Bluetooth battery property (covers classic headsets over HFP and BLE devices Windows
   already reads). Keep the module shaped so a second source (GATT, XInput, USB HID) can be added without touching
   the overlay. Do not add WinRT in this task.
6. A device that is both ours and a Bluetooth device (e.g. a Razer or Logitech mouse paired over Bluetooth)?
   Recommend: the app's row wins; a system device is dropped when its name equals, or contains / is contained in
   (4 or more characters, ignoring case), the display name or model name of any device the app has. Because
   disconnected devices are hidden, a mouse in 2.4 GHz mode (app) and Bluetooth (system) rarely appears twice
   anyway. No manual hide list in this task.
7. Opt-out: tray item "System devices" on by default?
   Recommend: yes. Cheap, and avoids a surprise for someone whose paired gadgets clutter the plate.
8. Maximum rows?
   Recommend: 10 rows in total in every style; the app's devices are always kept, system devices beyond the cap are
   left out (logged once). Sized so Row style (about 870 DIP wide) fits a 1366 px screen at 100% and a 1920 px one at 200%.

## Notes

- No significant architectural decision: one new read-only module and one config bool. No ADR needed.
- `src/config.rs` and `src/main.rs` are security-shaped paths (settings compatibility, polling, shared state):
  extra care in review. The new code calls Win32 property APIs only; it must not open device handles
  (`CreateFile`) on Bluetooth devices.
