# Device profiles

**English** · [Русский](profiles.ru.md)

A profile describes a device model of a protocol RGB Tray already speaks, in a small TOML file. No compiling is needed: put the file into `%APPDATA%\rgbtray\devices\` and restart RGB Tray (tray icon → Exit, then start it again).

The built-in profiles are in [`profiles/`](../profiles) in the repository. A file of yours with the same `key` replaces the built-in one, so you can fix a model without waiting for a release. When a profile of yours works, please send it as a pull request or attach it to a device request, so that it becomes built in for everyone.

If a file has a mistake, RGB Tray skips it and says why in the device report (tray icon → Device report…, the "Device profiles" line) and in `%APPDATA%\rgbtray\rgbtray.log`.

Protocols profiles support today: **Razer** and **QMK/VIA**.

## Example

```toml
# Razer Naga V2 Pro: wireless (HyperSpeed dongle) and wired.
key = "naga"
name = "Razer Naga V2 Pro"
class = "mouse"
tested = true

[[zones]]
led = 0x04
name = "Logo"
name_ru = "Логотип"

[[zones]]
led = 0x05
name = "Side panel"
name_ru = "Боковая панель"

[razer]
pids = [0x00A8, 0x00A7]
transaction_id = 0x1F
leds = [0x01, 0x04, 0x05]
battery = true
```

## Fields

| Field | Required | What it is |
|---|---|---|
| `key` | yes | Short id of the model: small latin letters and digits. Device ids are `<key>-<serial>`, and settings are stored under them. **Don't change it later**, or the device loses its settings. |
| `name` | yes | Name on the panel, in English. |
| `name_ru` | no | Name in Russian, if it differs. |
| `class` | yes | `keyboard`, `mouse` or `other`. Picks the icon and wording ("All" of a mouse is "Вся мышь" in Russian). |
| `icon` | no | `keyboard`, `mouse`, `dock`, `laptop`, `macropad` or `light`. By default it follows `class`. |
| `effects` | no | Effects the device can show, from `static`, `breathe`, `cycle`, `off`. All four by default; `off` is required. The panel greys out the rest. |
| `tested` | no | `true` once someone has checked the model on real hardware. Until then its card says "untested", so that its owner knows a report is welcome. |
| `[[zones]]` | no | Parts the panel can color separately: `led` (an id from the protocol section), `name`, `name_ru`. Without zones, the device has one color. |

Numbers can be written in hex (`0x1F`) or decimal (`31`).

One file can describe several models: put each one in a `[[device]]` section, and its tables become `[[device.zones]]` and `[device.razer]` (see `profiles/razer-mice.toml`).

## `[razer]`: Razer mice, keyboards and accessories

Most Razer devices use the 90-byte control report that [OpenRazer](https://github.com/openrazer/openrazer) documents. The values for a model can be taken from OpenRazer's source (`driver/razer*_driver.h` for product ids, `razer*_driver.c` for the transaction id and LED ids).

| Field | Required | What it is |
|---|---|---|
| `pids` | yes | USB product ids (vendor 1532). A wireless device usually has two: dongle and cable. The device report shows the ids of your device. |
| `transaction_id` | yes | The report's transaction id: `0x1F` for most newer devices, `0xFF` for older ones and many accessories; a few use other values (see OpenRazer). |
| `leds` | yes | LED ids that light up when the whole device gets one color, as OpenRazer names them: `0x00` the only LED of simple devices, `0x01` scroll wheel, `0x04` logo, `0x05` backlight (the side panel on the Naga), `0x10`/`0x11` right/left side strips. Ids a device doesn't have are skipped. |
| `battery` | no | `true` if the device reports its battery level (wireless mice). |

The Razer report has no effect speed, so the panel shows no speed slider for these devices.

## `[via]`: QMK keyboards with VIA

Any QMK keyboard with VIA and RGB lighting already works without a profile: the app finds it, and draws breathing and spectrum itself. A profile gives it a name, an icon and, if its firmware has a good rainbow effect, uses that for "Spectrum".

| Field | Required | What it is |
|---|---|---|
| `vid`, `pid` | yes | USB vendor and product id of the keyboard (in the device report). |
| `cycle_effect` | no | Number of the firmware effect to use for "Spectrum" (QMK RGB Matrix / RGBLight mode). Without it the app draws the spectrum. |

QMK has no serial number, so a keyboard's settings are tied to its USB port. `[[zones]]` aren't supported for VIA keyboards. Example: [`profiles/geekboard-macropad.toml`](../profiles/geekboard-macropad.toml).
