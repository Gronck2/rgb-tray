# 0004. MSI Vigor GK50 Elite: lighting and per-key colors

## Goal

RGB Tray drives the MSI Vigor GK50 Elite (USB `0DB0:0B5B`): static color, breathing, spectrum, off and per-key
colors in the key editor (full-size layout), without MSI Center.

## Why

A user's keyboard; the protocol is not documented anywhere, so it was read from a USB capture of MSI Center
(USBPcap, 2026-11-22).

## Protocol (from the capture)

- Vendor collection `FF00:0001` (interface `MI_01`), 64-byte output reports without a report id; the keyboard answers
  each report on its IN endpoint.
- Every change is a transaction: `41 80` (begin), `56 21 <page> 00 <data>` (write a settings page),
  `51 28 00 00 <mode>`, `41` (end). MSI Center reads the page first (`56 20 <page>`); that is not needed.
- Modes: `01` static (page 01, color and brightness at data bytes 44-47), `02` breathe and `03` wave (page 02,
  speed in byte 15, not used yet), `0B` per-key (page 05 header, pages 06-0C: 132 RGB slots), `0C` off.
- MSI Center sends `50 55` before the end. It is most likely the save to flash and is never sent: without it the
  lighting lives in RAM (after a replug the keyboard shows its saved lighting again; checked on the device).
- Per-key slots go column by column, 6 rows each, 22 columns. The first six slots were checked on the device
  (Esc, `` ` ``, Tab, Caps Lock, left Shift, left Ctrl); the rest follows from the gaps of the full-size layout,
  which MSI Center reads back as black (104 lit slots). Fn sits in the slot of the right Win key.

## Scope

- `src/devices/msi.rs`: a native driver (`gk50-<path hash>`, the id the `[hid]` profile used during testing),
  static and off through page 01, breathing and spectrum drawn by the app with the static reports, per-key colors
  through mode `0B`; `update_keys` keeps the shown table per keyboard. A failed write after the begin still sends the end report.
- `keymap::code_of`: the code of a key by its layout name.
- The model in `NATIVE` (layout `full`), the driver in `DRIVERS`, every README.

## Checked on the device

Static colors, brightness, app-drawn breathing and spectrum, off; no flash write. Per-key colors in the key editor
(different colors on different keys): checked by the contributor on their GK50, reported to the maintainer on
23.11.2026.
