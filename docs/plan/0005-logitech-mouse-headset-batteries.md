# Plan 0005. Logitech mice and headsets over HID++

Task: `docs/tasks/0005-logitech-mouse-headset-batteries.md`.

## Approach

`logitech::other_batteries(sw)` walks the Logitech HID++ collections (`FF00:0002` of LIGHTSPEED receivers, slots 1-2, and
cables, index 0xFF; `FF43:0202` of headset dongles), finds the battery feature (0x1004, 0x1001, 0x1F20) with a lookup,
reads it, and reads the name and type (0x0005). `sysbat::refresh` merges the rows into the system devices.

- 0x1004: getCapabilities says whether getStatus carries a state of charge; without it the level bitmask is mapped
  (full 90, good 50, low 20, critical 5), as Solaar does.
- A device that doesn't answer keeps its last level for 10 minutes; a held row is dropped when a fresh read of the same
  pass has the same name (a mouse on its charging cable answers on the cable and on the receiver slot).

## Safety rules (the owner's G915 sits on a LIGHTSPEED receiver)

1. The app's own devices get no traffic from this feature: slots where `Logitech::search` found a keyboard are recorded
   (`DRIVEN`) and skipped before any request; a slot that turns out to be a keyboard or has no battery is remembered and
   not asked again (`OTHERS_SKIP`). The refresh holds `APPLY_LOCK`, so it follows the startup discovery and never runs
   at the same time as an apply or a poll.
2. The HID++ part runs only while the user is active (`idle_seconds() <= IDLE_SKIP`) and `overlay_system` is on; otherwise
   the last rows are kept (setting on) or dropped (setting off). The Bluetooth OS-property part is unchanged.
3. Only LIGHTSPEED receivers: Unifying and Bolt answer a cabled-device request with a HID++ 1.0 error, so they are skipped.
4. Reads only; software id 0x0D in the tray app, 0x0E in `--system-batteries` (another process).

## Known limit

A G915 asleep at startup is not in `DRIVEN` until a search sees it; the first read of its slot then finds a keyboard by
type and remembers to skip it.
