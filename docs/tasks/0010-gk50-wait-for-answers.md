# 0010. MSI Vigor GK50 Elite: wait for the answers

## Goal

The lighting of the GK50 does not hang while breathing or spectrum runs.

## Why

On 24.11.2026 the lighting channel of a GK50 stopped taking reports: every apply ended with `GK50: send failed` after the
1 s write timeout, a plain `41 80` sent with `--hid-send` failed too, the animation had ended and the lights stayed as
they were. Only a replug helped. After one start of the app the first failure came 95 s after the first apply.

The driver sent the four reports of a frame back to back at 60 fps (about 250 reports a second) and never read the
answers. In the USB capture MSI Center sends the next report only after the answer to the last one (the only exception is
the end report after its save `50 55`, which the app never sends) and leaves about 14 ms between reports. The flood is the
likely cause; it was not proven. MSI's `LEDKeeper2.exe` was running too.

## Scope

- `send` waits up to 100 ms for the answer to each report. A missing answer alone does not fail the transaction.
- App-drawn frames are at least 40 ms apart, and nothing is sent while the color stays.

## Out of scope

- Restarting an animation whose device stopped taking reports: still the next apply.
