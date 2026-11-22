# 0005. Battery overlay: Logitech mice and headsets over HID++

## Goal

The battery overlay lists wireless Logitech mice and headsets the app doesn't light, with the level read from the device:
for example a G502 LIGHTSPEED on its receiver and a G535 headset on its own dongle.

## Why

The overlay showed only the app's own devices and Bluetooth devices whose level Windows holds. Mice and headsets on
LIGHTSPEED dongles are neither of those.

## Read on the devices (2026-11-22)

- G502 LIGHTSPEED (receiver `046D:C539`, collection `FF00:0002`, slot 1): `0x1001` battery voltage, 3920 mV.
- G535 (`046D:0AC4`, collection `FF43:0202`, device index `0xFF`, long reports): no `0x1000`/`0x1001`/`0x1004`;
  `0x1F20` ADC measurement at index 05 answers mV and flags (`0F E9 01` = 4073 mV, bit 0 = battery present,
  bit 1 = charging, as Solaar reads it). `0x0005` gives name and type.

## Scope

- `logitech::other_batteries`: every Logitech `FF00:0002` collection of a LIGHTSPEED receiver (slots 1-2) or a cable (0xFF)
  and every `FF43:0202` collection; skips every slot the app's keyboard search drives and keyboards by type; level from `0x1004`, `0x1001` or `0x1F20`
  (voltage through the existing curve); name from `0x0005` without the generic words at the end; class from its type.
  Feature lookups and reads only, its own software id (`--system-batteries` another one).
- Unifying (`C52B` etc.) and Bolt (`C548`) receivers are not covered: not read at all.
- `sysbat::refresh` adds them to the system devices (same setting, same refresh times, worker thread);
  `--system-batteries` lists them.

## Out of scope

- A charging mark on system rows (the overlay has none for them).
- Lighting of these devices.
