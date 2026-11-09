<!-- Thanks! For a device, fill in all sections; for a fix, delete what doesn't apply. -->

## What

<!-- The device (exact model, VID:PID) or the problem this fixes. -->

## How

- [ ] Profile (`profiles/*.toml`, added to `BUILT_IN`)
- [ ] Driver (`src/devices/*.rs`, `Kind` in `NATIVE`, `DRIVER` in `DRIVERS`)
- [ ] Other change

Where the protocol came from (documentation, OpenRGB / OpenRazer source, USB capture, probing):

## Checked on hardware

<!-- What you saw on the device: static colors, breathing, spectrum, off, zones, brightness, battery, after sleep / replug. -->

## Checks

- [ ] `cargo fmt --check`, `cargo clippy --release --all-targets -- -D warnings`, `cargo test --release` pass
- [ ] `README.md` lists the device (the translations `README.*.md` too if you can; the maintainer updates the rest)
- [ ] Device report (`rgbtray --diagnose`) attached or pasted below, if this adds a device
