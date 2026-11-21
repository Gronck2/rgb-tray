# Plan 0002. System devices on the battery overlay

Task: `docs/tasks/0002-system-batteries-overlay.md`. Defaults below follow the recommendations in its "Owner decisions";
if the owner answers differently, adjust the matching step. Step 1 is a gate: do the rest only if the XM6 shows up there.

## Source: what Windows offers (researched, to be verified by step 1)

| Source | Covers | Read-only, no wake | Cost | Verdict |
|---|---|---|---|---|
| PnP property `DEVPKEY_Bluetooth_Battery` `{104EA319-6EE2-4701-BD47-8DDBF425BBE5}`, pid 2 (BYTE, 0..100) on the Bluetooth device node | what Settings shows: classic headsets whose level reaches Windows through HFP, and BLE devices with a GATT Battery Service that the Windows BLE stack reads | yes: a property getter reads the PnP store, no radio traffic | SetupDi/CM, already linked; a few ms | chosen |
| GATT Battery Service 0x180F via WinRT (`Windows.Devices.Bluetooth`) | BLE only; opens a GATT session, can conflict with the vendor app and with Windows' own reads | no: connects to the device | new `windows` features (Devices_Bluetooth*, Foundation, Storage_Streams), async | rejected for now; fallback follow-up |
| `Win32_Battery` / WMI | the laptop's own battery | yes | WMI/COM | not relevant (not an accessory) |
| XInput `XInputGetBatteryInformation` | Xbox-protocol pads | yes | needs a user index, not Bluetooth property | later source |
| Vendor protocols (Sony etc.) | the exact level | no (talks to the device) | reverse engineering | out of scope |

Property facts to check in step 1 (the plan assumes them, the probe confirms):

- The battery property is on the device node of the device (`BTHENUM\DEV_<12 hex address>` for classic, `BTHLE\DEV_<address>`
  for LE), not on every service node. The probe lists every node under both enumerators with the property so we
  see which one carries it for the XM6.
- Connection: `DEVPKEY_Device_IsConnected` `{83DA6326-97A6-4088-9453-A1923F573B29}`, pid 15 (BOOLEAN) on the same node.
  A disconnected device keeps its old battery value in the store, so this flag decides visibility.
- Name: `DEVPKEY_Device_FriendlyName` `{A45C254E-DF1C-4EFD-8020-67D146A850E0}`, pid 14, fallback `DEVPKEY_NAME`
  `{B725F130-47EF-101A-A5F1-02608C9EEBAC}`, pid 10, fallback "Bluetooth device".
- Class: `DEVPKEY_Bluetooth_ClassOfDevice` `{2BD67D8B-8BEB-48D5-87E0-6CDA3428040A}`, pid 10 (UINT32, Bluetooth Class of
  Device; classic devices only; LE devices may have none).
- None of these constants are in the `windows-sys` headers as ready consts for the Bluetooth ones, so define them as
  `DEVPROPKEY { fmtid: GUID, pid }` consts in `sysbat.rs`.

## Approach

A self-contained module `src/sysbat.rs` with a pure part (class to icon, id, merge, dedup, cap) and a thin Windows part
(enumerate nodes, read properties). It owns one static, `SYSTEM: Mutex<Vec<SysDevice>>`, written only by its refresh
and read by the overlay. System devices are deliberately not put into `PRESENT`, `STATUS`, `NO_LINK` or `config.devices`,
which keeps the panel, hotkeys, alert, config migration and `kind_of` unaware of them. The overlay merges the two
sources when it builds rows.

```rust
pub struct SysDevice { pub id: String, pub name: String, pub class: SysClass, pub percent: u8 }
pub enum SysClass { Headphones, Headset, Speaker, Mouse, Keyboard, Gamepad, Phone, Other }
```

Only connected devices with a valid level (0..=100) are stored. No `charging` field (not available).

### Ids

`system-<8 uppercase hex>`: FNV-1a (same function family as `devices::path_hash`, which takes UTF-16 paths) of the
lower-cased 12-hex Bluetooth address taken from the instance id (`DEV_xxxxxxxxxxxx`); if no address can be parsed,
of the whole lower-cased instance id. The address is stable over reconnects, driver reinstalls and across the classic
and LE nodes of one dual-mode device, and an instance id is not guaranteed to be. The id is never saved, but is used
as the stable sort and dedup key and in logs. If two nodes give the same id (dual-mode device), keep the one
with a level (the connected one first).

### Class to icon (Segoe Fluent Icons, same font as the panel)

Pure `fn class_of(cod: Option<u32>) -> SysClass` and `fn glyph(SysClass) -> &'static str`.

Class of Device: major = `(cod >> 8) & 0x1F`, minor = `(cod >> 2) & 0x3F`.

- major 0x04 (audio/video): minor 0x01 wearable headset, 0x02 hands-free, 0x06 headphones, 0x07 portable audio, 0x05
  loudspeaker, 0x04 microphone; the headset/headphone minors map to Headphones, hands-free/mic to Headset, the
  loudspeaker minors to Speaker, anything else audio to Headphones.
- major 0x05 (peripheral): bits 6-7 of the minor (cod bits 8..) give keyboard (0x10), pointing device (0x20), combo
  keyboard/pointer (keyboard); the low four minor bits 0x01 joystick, 0x02 gamepad map to Gamepad.
- major 0x02 phone: Phone. Everything else and a missing class: Other.

Glyphs (pick these, then check them in a snapshot; reuse the panel's `E765` keyboard and `E962` mouse so they match):
Headphones `E7F6`, Headset `E95B` (fallback `E7F6` if the font lacks it), Speaker `E7F5`, Mouse `E962`, Keyboard `E765`,
Gamepad `E7FC`, Phone `E8EA`, Other (Bluetooth) `E702`. Any codepoint missing from "Segoe Fluent Icons" on Windows 10
must be replaced by one that exists in both it and Segoe MDL2 Assets (the font the panel falls back to on 10).

### Visibility rules (pure `fn visible(...)`)

A device is listed when: setting on, `connected`, has a level, and `!is_duplicate(sys_name, app_names)`.
`is_duplicate`: lower-cased, trimmed names; equal, or one contains the other and the shorter has at least 4
characters. `app_names` are the display name and the model name (`kind_of(id).names`) of each device in `active_ids()`.

### Merge for the overlay

`Row` gets its icon as given instead of from `kind_of`. Change `rows()` to take the system devices too:

```rust
fn rows(ids, status, no_link, name, system: &[SysDevice], app_names...) -> Vec<Row>
```

App rows are built exactly as today (order unchanged). System rows follow, sorted by name (case-insensitive) then
id, with `asleep = false`, `charging = false`, `level` by the same thresholds. The cap (`MAX_ROWS = 10`) keeps all app
rows and fills the rest with system rows; with more app rows than the cap nothing changes from today (no new cap for
the app's own devices). Dropping beyond the cap is logged once per change.

### Layout

`size(style, n)` already scales with `n`: Full height `16 + 40n`, Column height `16 + 28n`, Row width `24 + 74n + 12(n-1)`.
At the cap of 10: Full 416, Column 296, Row 872 DIP, all inside a 1366x768 work area at 100%; at 200% scaling of a
1920x1080 screen the work area is 960x540 DIP and Full (416) and Row (872) still fit. Add a pure test that
`size(style, MAX_ROWS)` is below 920 wide and 500 high. `placed()` already clamps the position into the work
area; the growth-away-from-the-anchored-corner logic (0001 addendum) handles the plate getting bigger when a device
connects. Nothing new in the drawing code except the icon source.

### Refresh and threading

- `sysbat::refresh()` enumerates and reads, replaces `SYSTEM`, returns whether the list changed (compare the old and
  new vectors; PartialEq). Runs on a worker thread (`sysbat::refresh_async()` spawns one and posts `WM_BATTERY`
  when changed), never on the UI thread, never under `APPLY_LOCK` (it touches no HID).
- Called: (1) in `spawn_battery_poller` once per tick, outside the idle skip (reading OS properties wakes nothing,
  and the user may be watching a film with the headphones on), (2) from `overlay::toggle()` when showing, (3) a 60 s
  timer of the overlay window while it is visible (`TIMER_SYSTEM`, next to `TIMER_MODIFIERS`).
- Also after `WM_DEVICECHANGE` / `DBT_DEVNODES_CHANGED` (already handled at `main.rs`), but only if the overlay is
  visible, debounced to once per 2 s; connecting headphones then shows at once. Check that Bluetooth connection changes
  do raise that broadcast; if not, the 60 s timer is what is left (acceptance 2 allows 60 s).
- This differs from 0001 decision 9 ("no poll on show") on purpose: that rule protects sleeping wireless devices
  from being asked; here the OS store is read, the device is not asked.
- Errors (no Bluetooth, access denied) give an empty list; log only when the error text changes.

### Windows part

Enumerate with `CM_Get_Device_ID_List_SizeW` / `CM_Get_Device_ID_ListW` using `CM_GETIDLIST_FILTER_ENUMERATOR` for
`BTHENUM` and `BTHLE` (present devices only: no `CM_GETIDLIST_FILTER_PRESENT` needed if the list is of present nodes; use
the PRESENT flag where the API has it), then for each id `CM_Locate_DevNodeW` and `CM_Get_DevNode_PropertyW` for the
four properties. Alternative if CM proves awkward: `SetupDiGetClassDevsW(null, "BTHENUM", ..., DIGCF_ALLCLASSES |
DIGCF_PRESENT)` and `SetupDiGetDevicePropertyW`. Both are in `windows-sys` `Win32_Devices_DeviceAndDriverInstallation`
(already enabled); `DEVPROPKEY` needs `Win32_Devices_Properties`, which that feature pulls in, but add it to
`Cargo.toml` explicitly if the compiler asks. No `windows` crate feature is needed, no new crate. Only property getters
are called: no `CreateFile`, no IOCTL.
Parse the property buffer by its `DEVPROPTYPE`: BYTE for the level (ignore other types and values above 100),
BOOLEAN for connected (`0xFF` or non-zero is true).

## Steps

1. Gate, probe: `sysbat.rs` Windows part and `--system-batteries <out>` (`main.rs` CLI chain, written to a file like
   the other tools): for every node found under `BTHENUM` and `BTHLE`: instance id, friendly name, class of device,
   connected, battery (or "none"), derived id. English only. Build it and ask the owner to run it with the XM6
   connected and with it off; record the facts (which node carries the level, class value, whether the level changes
   when the headphones' battery changes, and the lag) in the task file's Owner decisions. If the XM6 shows no level:
   stop, report, and plan the GATT follow-up.
2. `sysbat.rs` pure part with tests: `class_of`, `glyph`, `id_for`, `is_duplicate`, `visible`, `merge`/cap, sort.
3. `config.rs`: `pub overlay_system: bool` (default true in `impl Default`; `#[serde(deserialize_with = "lenient_bool_true",
   skip_serializing_if = "is_true")]`, so a garbled value reads as true, true is not written). Test round trip, absent,
   garbled, not written when on.
4. `overlay.rs`: `Row.icon` filled by the caller; `current_rows()` also takes `sysbat::snapshot()` and the setting
   (`rows()` stays pure and testable); `MAX_ROWS`; the 60 s timer started in `open()` and killed in `close()`; `refresh()` re-lays-out.
   Make sure `kind_of` is never called with a system id.
5. `main.rs`: `mod sysbat;`, the poller call, `--system-batteries`, the debounced `WM_DEVICECHANGE` refresh, the demo
   variable, and `cycle`-style `toggle_overlay_system()` (flip, save, `overlay::refresh()`), mirroring `cycle_overlay_style`.
6. `traymenu.rs` and `i18n`: item "System devices" / "Системные устройства" under "Overlay style", with a check or "on/off" label like the other toggles in the menu.
   Make sure the menu width fits both languages.
7. Snapshot: `RGBTRAY_DEMO_SYSTEM=<name>:<class>:<percent>[,...]` fills `SYSTEM` in the snapshot branch only (class one of
   headphones, headset, speaker, mouse, keyboard, gamepad, phone, other; a name containing no `:` or `,`), e.g.
   `WH-1000XM6:headphones:72,MX Anywhere:mouse:9`. Document it and `--system-batteries` in `CLAUDE.md`.
8. README: feature line in `README.md` and every `README.*.md` (ru, zh-CN, es, de, fr, pt-BR, ja): the overlay also
   shows Bluetooth devices Windows reports a level for; says nothing is sent to them. `ROADMAP.md` is the owner's.

## Files likely touched

`src/sysbat.rs` (new), `src/overlay.rs`, `src/main.rs`, `src/config.rs`, `src/traymenu.rs`, `CLAUDE.md`, `README*.md`,
maybe `Cargo.toml` (one feature name). Not touched: `src/devices/**`, `src/hid.rs`, `src/alert.rs`, `src/ui.rs`.

## Tests (hardware-free, inline `#[cfg(test)]`)

- `sysbat.rs`: `class_of` for headphones (0x240418), a headset (0x240404), a mouse (0x002580), a keyboard (0x002540),
  a gamepad (0x002508), a phone (0x5A020C), None and an unknown value; `glyph` returns a non-empty private-use
  character for each class; `id_for` is stable, case-insensitive, and the same for a classic and LE node with the same
  address, different for different addresses and starts with `system-`; `is_duplicate` (equal, contained with 4+
  characters, too short a name does not match, no match for different products); `visible` hides disconnected,
  levels above 100 and the setting off; sort order; the cap keeps app rows and drops system rows last.
- `overlay.rs`: `rows()` with only app devices is identical to today's result (existing tests keep passing); with
  system devices they come after, carry the given icon, are never asleep and never charging; low/critical colours
  apply to them; `size(style, MAX_ROWS)` fits (acceptance 5).
- `config.rs`: `overlay_system` as in step 3.

## How to verify without a screen

```
$env:RGBTRAY_DEMO_IDS="logitech-DEMO1,naga-DEMO2"
$env:RGBTRAY_DEMO_BATTERY="logitech-DEMO1:82,naga-DEMO2:8"
$env:RGBTRAY_DEMO_SYSTEM="WH-1000XM6:headphones:72,Keychron K3:keyboard:40,Pad:gamepad:4"
target\release\rgbtray.exe --snapshot <scratch>\overlay-dark.png dark
```

For each of `RGBTRAY_DEMO_OVERLAY_STYLE=full|row|column`, dark and light, `RGBTRAY_LANG=ru|en`, 0, 1 and 10 system devices,
a very long name (must be truncated, not overflow), and with `RGBTRAY_DEMO_OVERLAY_BG=split`. Look at the PNGs: icons
render (no tofu boxes), the system rows follow the app's, colours of the 4% row. Snapshot `*traymenu*` for the new item.

## Hardware check (the owner)

1. Run `rgbtray.exe --system-batteries <file>` with the XM6 connected (step 1); then off; send or paste the file.
2. With the XM6 connected: the overlay shows its row and percent equal to Settings; headphones icon.
3. Turn the headphones off: the row goes within a minute; on: it returns. Watch that nothing changes on the
   headphones (no beep, no reconnect).
4. A second Bluetooth device with a level (mouse, keyboard, phone is not expected) shows with its icon, if one is at hand.
5. The G915 and Naga rows, their order and colours are as before; none of them is duplicated.
6. Tray item "System devices" off: the rows vanish at once with the plate open; restart keeps the choice; on again restores them.
7. Open the panel and the settings of `config.toml`: no new device cards, no `system-` entries.
8. Bluetooth off in Windows: no error, no row, the log has no repeating lines.

## Tradeoffs considered

- PnP property vs GATT Battery Service over WinRT: the property is a cache read (no connection, no conflict with Windows'
  own session or the vendor app, no new features), but only has what Windows itself already read. GATT could show more
  BLE devices and be fresher but connects to the device, adds async WinRT and may fight other clients. Property first,
  GATT only if the probe shows a gap.
- Separate store vs adding system devices to `STATUS` / `PRESENT`: a separate store keeps every existing consumer
  (panel, hotkeys, alert, `Config::register`, `kind_of`) unchanged and the config clean; the price is that the overlay
  merges two sources. Putting them in `STATUS` would need `kind_of`, naming, `forget` and alert handling to learn
  about a second kind of device.
- Hide vs dim disconnected: hide (the paired list is long, the stored percent is stale).
- Address-based id vs instance id: address is stable across classic/LE nodes; ids are used as keys only in memory.
- Name-based dedup with the app's devices: crude but safe (worst case a device appears twice or an unrelated device
  with a name inside another is hidden, which the 4-character floor limits). Matching by VID/PID would need each driver to
  expose its Bluetooth identity and is not worth it for a rare overlap.
- Polling on the existing 5-min tick only: too slow for connect/disconnect feedback while the overlay is open,
  hence the show-time read, the 60 s timer and the debounced device-change refresh.
