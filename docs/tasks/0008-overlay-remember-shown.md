# 0008. The battery overlay comes back after a restart

## Goal

A battery overlay the user left on is shown again when the app starts. Only the user switches it off: by its hotkey or
the tray item.

## Why

Whether the plate is shown lived only in memory, so it was gone after every restart of Windows or of the app (an update
too) and had to be switched on by hand.

## Scope

- `overlay_shown` in the config: absent = off (nothing changes for those who don't use the overlay), a garbled value
  reads as off, older versions ignore it.
- `overlay::toggle` (hotkey, shared hotkey, tray item) saves it; `overlay::restore` at start shows the plate.
- The plate closing with the app does not change it. A plate that could not be opened does not change it either.
- Loading a settings file keeps the current state.

## Out of scope

- Position and style (already saved), and a plate whose monitor is not there yet at start (default corner, as before).
