# 0007. Hotkeys shared with another program still work

## Goal

When Windows refuses a RGB Tray hotkey because another program already owns it (`RegisterHotKey` fails with
`ERROR_HOTKEY_ALREADY_REGISTERED`, 1409), RGB Tray still reacts to that key press, without taking the key from the owner.
One mechanism for every hotkey of the app: the battery overlay, presets and per-device hotkeys.

## Why

The owner's macropad has one key that sends Ctrl+Alt+Shift+O, G-Helper's overlay hotkey. VIA macros do not work on the pad,
so the key can only send one combination. The owner set RGB Tray's `overlay_hotkey` to the same combination
(`{mods=7, vk=79}`). Today `RegisterHotKey` fails, so only G-Helper's overlay opens. The wish: one key opens both overlays.

## Scope

- On error 1409 the hotkey goes to a "shared" list. A low-level keyboard hook (`WH_KEYBOARD_LL`) only observes: it always
  calls `CallNextHookEx` and never suppresses a key.
- A key-down matches a shared hotkey when the vk is equal and the Ctrl/Alt/Shift/Win state matches exactly (no extra, no
  missing modifier; left/right variants count as the same). It fires once per physical press: auto-repeat does not fire again;
  the key is armed again on key up.
- The hook callback does no work: it posts a message to the hidden window, and the handler dispatches what `WM_HOTKEY`
  would (overlay toggle, preset, device toggle).
- The hook is installed only while at least one shared hotkey exists. It is removed when `register_hotkeys` finds none
  (hotkeys changed or freed) and on exit.
- Other `RegisterHotKey` failures (any error but 1409) keep today's behaviour: logged, no fallback.
- The label "Клавишу занимает другая программа" (`ui.rs`, hotkey section title of the overlay menu) is replaced by wording that the
  key is shared and still works (ru + en through `tr!`).
- Unit tests for the hardware-free part: matching a key event against hotkeys with modifiers, repeat suppression and re-arming.

## Out of scope

- Suppressing the key for the other program, or any way to take a hotkey from its owner.
- Hotkeys that fail for another reason than 1409.
- Changing the config format (`Hotkey { mods, vk }` stays as is) or the id scheme.
- Making the owner's pad send anything else.

## Acceptance

- `cargo fmt --check`, `cargo clippy --release --all-targets -- -D warnings`, `cargo test --release` pass; new tests cover exact
  modifier matching (a press with an extra modifier or one missing does not match) and once-per-press with re-arm on key up.
- The log tells which hotkeys fell back to the shared mode.
- `--snapshot` of the overlay hotkey menu shows the new label if a demo state exists, otherwise the existing views still render.
- Owner on hardware, after the PR: the macropad's lower-left key toggles both overlays (G-Helper's and RGB Tray's);
  a hotkey nobody else owns still works as before; the key still reaches G-Helper. Merge only after this is confirmed.

## Open owner decisions

None open; the owner decided the points above. Notes for review:

- Security-shaped path: `src/main.rs` and a global keyboard hook. The hook must only compare against the saved combinations,
  keep no key log and send nothing anywhere. Recommended: keep state to "is this vk currently down".
- No ADR needed: the choice is local to hotkey handling. Flag to the main thread if it disagrees.
