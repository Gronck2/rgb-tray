# Device profiles

**English** · [Русский](profiles.ru.md)

A profile describes a device model of a protocol RGB Tray already speaks, in a small TOML file. No compiling is needed: put the file into `%APPDATA%\rgbtray\devices\` and restart RGB Tray (tray icon → Exit, then start it again).

The built-in profiles are in [`profiles/`](../profiles) in the repository. A file of yours with the same `key` replaces the built-in one, so you can fix a model without waiting for a release. When a profile of yours works, please send it as a pull request or attach it to a device request, so that it becomes built in for everyone.

If a file has a mistake, RGB Tray skips it and says why in the device report (tray icon → Device report…, the "Device profiles" line) and in `%APPDATA%\rgbtray\rgbtray.log`.

Protocols profiles support today: **Razer**, **QMK/VIA**, and **[hid]**: report templates for any device with a simple protocol.

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
| `effects` | no | Effects the device can show, from `static`, `breathe`, `cycle`, `off`, and for `[razer]` also `wave`, `reactive` (keys light up when pressed), `starlight` (keys twinkle). The basic four by default; `off` is required. The panel greys out the rest. Extra effects show in the key editor of keyboards that have one, which Razer keyboards don't yet. Add them only for a model that has them in its firmware (OpenRazer's `razerkbd_driver.c` shows which keyboards get wave, reactive and starlight). |
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

## `[hid]`: any device with a simple protocol

For a device no driver knows, if its lighting is set by one or a few HID reports with the color in them. The reports are written as byte templates; the app fills in the color when it sends them. Working out the bytes is the hard part: [CONTRIBUTING.md](../CONTRIBUTING.md#adding-a-device) explains how, and the `/add-device` skill lets [Claude Code](https://claude.com/claude-code) do it with you.

```toml
# A made-up LED strip with two zones.
key = "mystrip"
name = "My LED strip"
class = "other"

[[zones]]
led = 0x01
name = "Left"

[[zones]]
led = 0x02
name = "Right"

[hid]
vid = 0x1234
pids = [0x5678]
usage_page = 0xFF00
report = "feature"
static = ["07 0A {zone} {r} {g} {b}", "07 0B"]
```

| Field | Required | What it is |
|---|---|---|
| `vid`, `pids` | yes | USB vendor id and product ids (in the device report). |
| `usage_page` | yes | Usage page of the HID collection the reports go to, usually a vendor one (`0xFF00` and up). The device report lists the collections of each device. |
| `usage` | no | Its usage, if the device has several collections on that page. Without it, the first matching collection of each device is used. |
| `report` | yes | `feature` (sent with HidD_SetFeature) or `output` (written to the device). Each report is padded with zeros to the collection's report length. |
| `static` | yes | Reports that show a steady color, in order. |
| `off` | no | Reports that switch the lighting off. By default `static` with black. |
| `breathe`, `cycle` | no | Reports that start the firmware's breathing or spectrum. Without them the app draws the effect by sending `static` about 60 times a second. |
| `delay_ms` | no | Pause between reports, for devices that drop reports sent too fast. Effects the app draws then send no more often than that either. |

A report is hex bytes separated by spaces, starting with the report id (`00` if the device has no report ids). Placeholders:

| Placeholder | Value |
|---|---|
| `{r}` `{g}` `{b}` | The color scaled by the brightness slider. |
| `{red}` `{green}` `{blue}` | The color as chosen, for devices that take brightness separately. |
| `{brightness}` | Brightness 0–255. |
| `{percent}` | Brightness 0–100. |
| `{speed}` | Effect speed 0–255, about 127 normal, higher is faster. |
| `{zone}` | The zone's `led` id. A report with `{zone}` is sent once per zone, with that zone's color. `[[zones]]` need it in a `static` report, and it needs `[[zones]]`. |

The panel shows a speed slider only if the app draws breathing or spectrum (no `breathe` or `cycle` reports for an effect the device has) or a report has `{speed}`. Zone `led` ids must differ.

The device has no serial number to the app, so its settings are tied to its USB port. If a protocol needs answers from the device, a checksum or a handshake, a profile can't describe it: that is a driver (CONTRIBUTING.md, "A new protocol").

### Trying a profile before installing it

```
rgbtray --hid-send out.txt 1234:5678:FF00 feature "07 0A 01 FF 00 00" "07 0B"
rgbtray --profile-test out.txt my-strip.toml
```

`--hid-send` sends single reports to the first collection that matches `vid:pid[:usage_page[:usage]]` and appends what it sent and what the device answered to `out.txt` (`feature`, `output`, or `get` to read a feature report by id; any other mode sends nothing). `--profile-test` plays red, green, blue, dim white, each zone in turn, breathing, spectrum and off with a [hid] profile, a step every few seconds, and logs each step. Watch the device while it runs.
